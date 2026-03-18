//! WebSocket event publisher for remote (non-LCM) mode.
//!
//! When `dimos-viewer` is started with `--connect`, LCM multicast is unavailable
//! across machines. This module spawns a small WebSocket server on a local port
//! and broadcasts click and twist events as JSON to every connected client.
//!
//! Message format: newline-delimited JSON objects with a `"type"` discriminant.
//!
//! ```json
//! {"type":"heartbeat","timestamp_ms":1234567890}
//! {"type":"click","x":1.0,"y":2.0,"z":3.0,"entity_path":"/world","timestamp_ms":1234567890}
//! {"type":"twist","linear_x":0.5,"linear_y":0.0,"linear_z":0.0,"angular_x":0.0,"angular_y":0.0,"angular_z":0.8}
//! {"type":"stop"}
//! ```

use std::time::{Duration, SystemTime, UNIX_EPOCH};

use axum::{
    Router,
    extract::{
        WebSocketUpgrade,
        ws::{Message, WebSocket},
    },
    routing::get,
};
use rerun::external::re_log;
use serde::Serialize;
use tokio::sync::broadcast;

/// JSON message variants sent over the WebSocket.
#[derive(Debug, Clone, Serialize)]
#[serde(tag = "type", rename_all = "snake_case")]
pub enum WsEvent {
    Heartbeat {
        timestamp_ms: u64,
    },
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

/// Broadcasts `WsEvent`s (serialised to JSON) to all connected WebSocket clients.
///
/// The internal sender is `Clone`, so you can hand copies to multiple producers
/// (keyboard handler, click handler, …).
#[derive(Clone)]
pub struct WsPublisher {
    tx: broadcast::Sender<String>,
}

impl WsPublisher {
    /// Spawn the WebSocket server and heartbeat task, then return a publisher.
    ///
    /// The server listens on `ws://0.0.0.0:<port>/ws`.
    pub fn spawn(port: u16) -> Self {
        let (tx, _rx) = broadcast::channel::<String>(256);

        // WebSocket server
        {
            let tx = tx.clone();
            tokio::spawn(async move {
                let app = Router::new().route(
                    "/ws",
                    get(move |upgrade: WebSocketUpgrade| {
                        let rx = tx.subscribe();
                        async move { upgrade.on_upgrade(move |socket| serve_client(socket, rx)) }
                    }),
                );

                let addr = format!("0.0.0.0:{port}");
                match tokio::net::TcpListener::bind(&addr).await {
                    Ok(listener) => {
                        re_log::info!("WebSocket event server listening on ws://{addr}/ws");
                        if let Err(err) = axum::serve(listener, app).await {
                            re_log::error!("WebSocket server error: {err}");
                        }
                    }
                    Err(err) => {
                        re_log::error!("Failed to bind WebSocket server on {addr}: {err}");
                    }
                }
            });
        }

        // Heartbeat task — 1 Hz
        {
            let tx = tx.clone();
            tokio::spawn(async move {
                let mut ticker = tokio::time::interval(Duration::from_secs(1));
                loop {
                    ticker.tick().await;
                    let ts = SystemTime::now()
                        .duration_since(UNIX_EPOCH)
                        .unwrap_or_default()
                        .as_millis() as u64;
                    let event = WsEvent::Heartbeat { timestamp_ms: ts };
                    if let Ok(json) = serde_json::to_string(&event) {
                        // Ignore send errors — nobody connected yet is fine
                        let _ = tx.send(json);
                    }
                }
            });
        }

        Self { tx }
    }

    /// Publish a click event to all connected clients.
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

    /// Publish a twist (velocity) command to all connected clients.
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

    /// Publish a stop command to all connected clients.
    pub fn send_stop(&self) {
        self.broadcast(WsEvent::Stop);
    }

    /// Return a clone of the underlying broadcast sender for use in other tasks.
    pub fn sender(&self) -> broadcast::Sender<String> {
        self.tx.clone()
    }

    fn broadcast(&self, event: WsEvent) {
        if let Ok(json) = serde_json::to_string(&event) {
            // Ignore if there are no receivers
            let _ = self.tx.send(json);
        }
    }
}

/// Per-client WebSocket task: forward broadcast messages until the client disconnects.
async fn serve_client(mut socket: WebSocket, mut rx: broadcast::Receiver<String>) {
    loop {
        match rx.recv().await {
            Ok(msg) => {
                if socket.send(Message::text(msg)).await.is_err() {
                    break; // client disconnected
                }
            }
            Err(broadcast::error::RecvError::Closed) => break,
            Err(broadcast::error::RecvError::Lagged(n)) => {
                re_log::warn!("WebSocket client lagged, dropped {n} messages");
            }
        }
    }
}
