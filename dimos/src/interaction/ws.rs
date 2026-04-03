//! WebSocket client for remote (non-LCM) mode.
//!
//! When `dimos-viewer` is started with `--connect`, LCM multicast is unavailable
//! across machines. This module connects to a WebSocket server (typically the
//! Python `RerunWebSocketServer` module) and sends click, twist, and stop events
//! as JSON.
//!
//! Message format (JSON objects with a `"type"` discriminant):
//!
//! ```json
//! {"type":"click","x":1.0,"y":2.0,"z":3.0,"entity_path":"/world","timestamp_ms":1234567890}
//! {"type":"twist","linear_x":0.5,"linear_y":0.0,"linear_z":0.0,"angular_x":0.0,"angular_y":0.0,"angular_z":0.8}
//! {"type":"stop"}
//! ```

use std::time::Duration;

use rerun::external::re_log;
use serde::Serialize;
use tokio::sync::mpsc;

/// Error returned when a WebSocket event cannot be sent.
#[derive(Debug)]
pub enum SendError {
    /// The send queue is full; the event was dropped.
    QueueFull,
    /// Failed to serialize the event to JSON.
    Serialize(String),
}

impl std::fmt::Display for SendError {
    fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        match self {
            Self::QueueFull => write!(f, "send queue full, event dropped"),
            Self::Serialize(e) => write!(f, "serialization error: {e}"),
        }
    }
}

/// JSON message variants sent over the WebSocket.
#[derive(Debug, Clone, Serialize)]
#[serde(tag = "type", rename_all = "snake_case")]
pub enum WsEvent {
    Click {
        x: f64,
        y: f64,
        z: f64,
        entity_path: String,
        timestamp_ms: u64,
    },
    Twist {
        linear_x: f64,
        linear_y: f64,
        linear_z: f64,
        angular_x: f64,
        angular_y: f64,
        angular_z: f64,
    },
    Stop,
}

/// Sends `WsEvent`s (serialized to JSON) to a remote WebSocket server.
///
/// Maintains a persistent connection with automatic reconnection. The
/// internal sender is `Clone`, so you can hand copies to multiple producers
/// (keyboard handler, click handler, …).
#[derive(Clone)]
pub struct WsPublisher {
    tx: mpsc::Sender<String>,
}

impl WsPublisher {
    /// Spawn the WebSocket client task and return a publisher.
    ///
    /// The client connects to `url` (e.g. `ws://127.0.0.1:3030/ws`) and
    /// reconnects automatically whenever the connection drops.
    ///
    /// This spawns a dedicated background thread with its own tokio runtime,
    /// so it works even when called from a non-async context (like the eframe UI).
    pub fn connect(url: String) -> Self {
        let (tx, rx) = mpsc::channel::<String>(256);

        // Spawn a dedicated thread with its own tokio runtime.
        // This allows WsPublisher to work from the eframe UI thread which
        // doesn't have a tokio runtime.
        std::thread::Builder::new()
            .name("ws-publisher".to_string())
            .spawn(move || {
                let rt = tokio::runtime::Builder::new_current_thread()
                    .enable_all()
                    .build()
                    .expect("failed to create tokio runtime for WsPublisher");
                rt.block_on(run_client(url, rx));
            })
            .expect("failed to spawn WsPublisher thread");

        Self { tx }
    }

    /// Publish a click event.
    pub fn send_click(&self, x: f64, y: f64, z: f64, entity_path: &str, timestamp_ms: u64) -> Result<(), SendError> {
        let event = WsEvent::Click {
            x,
            y,
            z,
            entity_path: entity_path.to_string(),
            timestamp_ms,
        };
        self.broadcast(event)
    }

    /// Publish a twist (velocity) command.
    pub fn send_twist(
        &self,
        linear_x: f64,
        linear_y: f64,
        linear_z: f64,
        angular_x: f64,
        angular_y: f64,
        angular_z: f64,
    ) -> Result<(), SendError> {
        let event = WsEvent::Twist {
            linear_x,
            linear_y,
            linear_z,
            angular_x,
            angular_y,
            angular_z,
        };
        self.broadcast(event)
    }

    /// Publish a stop command.
    pub fn send_stop(&self) -> Result<(), SendError> {
        self.broadcast(WsEvent::Stop)
    }

    fn broadcast(&self, event: WsEvent) -> Result<(), SendError> {
        let json = serde_json::to_string(&event).map_err(|e| SendError::Serialize(e.to_string()))?;
        // Non-blocking: error if the channel is full rather than block the UI thread.
        self.tx.try_send(json).map_err(|_| SendError::QueueFull)
    }
}

/// Returns true if `DIMOS_DEBUG` is set to `1`.
fn is_debug() -> bool {
    std::env::var("DIMOS_DEBUG").is_ok_and(|v| v == "1")
}

/// Background task: connect → send → reconnect loop.
async fn run_client(url: String, mut rx: mpsc::Receiver<String>) {
    use futures_util::{SinkExt, StreamExt};
    use tokio_tungstenite::{connect_async, tungstenite::Message};

    let debug = is_debug();

    loop {
        if debug {
            eprintln!("[DIMOS_DEBUG] WsPublisher: connecting to {url}");
        }

        match connect_async(&url).await {
            Ok((ws_stream, _)) => {
                if debug {
                    eprintln!("[DIMOS_DEBUG] WsPublisher: connected to {url}");
                }

                let (mut writer, mut reader) = ws_stream.split();

                // Read task: consume incoming frames (ping → auto pong) so the
                // server's keepalive pings get answered and the connection stays
                // alive. Exits when the server closes or an error occurs.
                let debug_read = debug;
                let mut read_handle = tokio::spawn(async move {
                    while let Some(frame) = reader.next().await {
                        match frame {
                            Ok(Message::Close(_)) => {
                                if debug_read {
                                    eprintln!("[DIMOS_DEBUG] WsPublisher: server sent close frame");
                                }
                                break;
                            }
                            Err(err) => {
                                if debug_read {
                                    eprintln!("[DIMOS_DEBUG] WsPublisher: read error: {err}");
                                }
                                break;
                            }
                            _ => {} // Ping/Pong handled by tungstenite internally
                        }
                    }
                });

                // Write loop: drain the channel into the WebSocket.
                let disconnected = loop {
                    tokio::select! {
                        msg = rx.recv() => {
                            match msg {
                                Some(text) => {
                                    if let Err(err) = writer.send(Message::text(text)).await {
                                        if debug {
                                            eprintln!("[DIMOS_DEBUG] WsPublisher: send error: {err} — reconnecting");
                                        }
                                        break false;
                                    }
                                }
                                None => break true, // rx closed → task is done
                            }
                        }
                        _ = &mut read_handle => {
                            // Reader exited → server closed the connection.
                            if debug {
                                eprintln!("[DIMOS_DEBUG] WsPublisher: server closed connection — reconnecting");
                            }
                            break false;
                        }
                    }
                };

                if disconnected {
                    if debug {
                        eprintln!("[DIMOS_DEBUG] WsPublisher: channel closed, shutting down");
                    }
                    break;
                }
            }
            Err(err) => {
                if debug {
                    eprintln!("[DIMOS_DEBUG] WsPublisher: connection failed: {err} — retrying in 1s");
                }
            }
        }

        // Drain any stale commands queued during the disconnect — sending
        // outdated velocity commands on reconnect would be dangerous.
        while rx.try_recv().is_ok() {}

        tokio::time::sleep(Duration::from_secs(1)).await;
    }
}
