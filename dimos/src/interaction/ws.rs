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

/// Sends `WsEvent`s (serialised to JSON) to a remote WebSocket server.
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
    pub fn connect(url: String) -> Self {
        let (tx, rx) = mpsc::channel::<String>(256);

        tokio::spawn(async move {
            run_client(url, rx).await;
        });

        Self { tx }
    }

    /// Publish a click event.
    pub fn send_click(&self, x: f64, y: f64, z: f64, entity_path: &str, timestamp_ms: u64) {
        let event = WsEvent::Click {
            x,
            y,
            z,
            entity_path: entity_path.to_string(),
            timestamp_ms,
        };
        self.broadcast(event);
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
    ) {
        let event = WsEvent::Twist {
            linear_x,
            linear_y,
            linear_z,
            angular_x,
            angular_y,
            angular_z,
        };
        self.broadcast(event);
    }

    /// Publish a stop command.
    pub fn send_stop(&self) {
        self.broadcast(WsEvent::Stop);
    }

    fn broadcast(&self, event: WsEvent) {
        if let Ok(json) = serde_json::to_string(&event) {
            // Non-blocking: drop message if the channel is full rather than block the UI thread.
            if self.tx.try_send(json).is_err() {
                re_log::warn!("WsPublisher: send queue full, dropped event");
            }
        }
    }
}

/// Background task: connect → send → reconnect loop.
async fn run_client(url: String, mut rx: mpsc::Receiver<String>) {
    use tokio_tungstenite::{connect_async, tungstenite::Message};
    use futures_util::SinkExt;

    loop {
        re_log::info!("WsPublisher: connecting to {url}");

        match connect_async(&url).await {
            Ok((mut ws_stream, _)) => {
                re_log::info!("WsPublisher: connected to {url}");

                // Drain the channel into the WebSocket until it closes or errors.
                while let Some(msg) = rx.recv().await {
                    if let Err(err) = ws_stream.send(Message::text(msg)).await {
                        re_log::warn!("WsPublisher: send error: {err} — reconnecting");
                        break;
                    }
                }
                // rx closed → task is done
                if rx.is_closed() {
                    break;
                }
            }
            Err(err) => {
                re_log::debug!("WsPublisher: connection failed: {err} — retrying in 1s");
            }
        }

        // Drain any stale commands queued during the disconnect — sending
        // outdated velocity commands on reconnect would be dangerous.
        while rx.try_recv().is_ok() {}

        tokio::time::sleep(Duration::from_secs(1)).await;
    }
}
