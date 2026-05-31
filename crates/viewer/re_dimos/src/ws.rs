use re_log;
use serde::Serialize;

#[derive(Debug)]
pub enum SendError {
    QueueFull,
    Serialize(String),
    NotConnected,
}

impl std::fmt::Display for SendError {
    fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        match self {
            Self::QueueFull => write!(f, "send queue full, event dropped"),
            Self::Serialize(e) => write!(f, "serialization error: {e}"),
            Self::NotConnected => write!(f, "WebSocket not connected"),
        }
    }
}

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

// ─── Native implementation (tokio + tokio-tungstenite) ──────────────────────

#[cfg(not(target_arch = "wasm32"))]
mod native {
    use super::{SendError, WsEvent, re_log};
    use std::time::Duration;
    use tokio::sync::mpsc;

    #[derive(Clone)]
    pub struct WsPublisher {
        tx: mpsc::Sender<String>,
    }

    impl WsPublisher {
        pub fn connect(url: String) -> Self {
            let (tx, rx) = mpsc::channel::<String>(256);

            std::thread::Builder::new()
                .name("ws-publisher".to_owned())
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

        pub fn send_click(
            &self,
            x: f64,
            y: f64,
            z: f64,
            entity_path: &str,
            timestamp_ms: u64,
        ) -> Result<(), SendError> {
            self.broadcast(&WsEvent::Click {
                x,
                y,
                z,
                entity_path: entity_path.to_owned(),
                timestamp_ms,
            })
        }

        pub fn send_twist(
            &self,
            linear_x: f64,
            linear_y: f64,
            linear_z: f64,
            angular_x: f64,
            angular_y: f64,
            angular_z: f64,
        ) -> Result<(), SendError> {
            self.broadcast(&WsEvent::Twist {
                linear_x,
                linear_y,
                linear_z,
                angular_x,
                angular_y,
                angular_z,
            })
        }

        pub fn send_stop(&self) -> Result<(), SendError> {
            self.broadcast(&WsEvent::Stop)
        }

        fn broadcast(&self, event: &WsEvent) -> Result<(), SendError> {
            let json =
                serde_json::to_string(event).map_err(|e| SendError::Serialize(e.to_string()))?;
            self.tx.try_send(json).map_err(|_err| SendError::QueueFull)
        }
    }

    async fn run_client(url: String, mut rx: mpsc::Receiver<String>) {
        use futures_util::{SinkExt as _, StreamExt as _};
        use tokio_tungstenite::{connect_async, tungstenite::Message};

        loop {
            match connect_async(&url).await {
                Ok((ws_stream, _)) => {
                    re_log::debug!("WsPublisher: connected to {url}");
                    let (mut writer, mut reader) = ws_stream.split();

                    let mut read_handle = tokio::spawn(async move {
                        while let Some(frame) = reader.next().await {
                            match frame {
                                Ok(Message::Close(_)) | Err(_) => break,
                                _ => {}
                            }
                        }
                    });

                    let disconnected = loop {
                        tokio::select! {
                            msg = rx.recv() => {
                                match msg {
                                    Some(text) => {
                                        if writer.send(Message::text(text)).await.is_err() {
                                            break false;
                                        }
                                    }
                                    None => break true,
                                }
                            }
                            _ = &mut read_handle => {
                                break false;
                            }
                        }
                    };

                    if disconnected {
                        break;
                    }
                }
                Err(err) => {
                    re_log::debug!("WsPublisher: connection failed: {err} — retrying in 1s");
                }
            }

            while rx.try_recv().is_ok() {}
            tokio::time::sleep(Duration::from_secs(1)).await;
        }
    }
}

// ─── WASM implementation (web_sys::WebSocket) ───────────────────────────────

#[cfg(target_arch = "wasm32")]
mod wasm {
    use super::*;
    use std::cell::RefCell;
    use std::collections::VecDeque;
    use std::rc::Rc;
    use wasm_bindgen::prelude::*;

    #[derive(Clone, Copy, PartialEq)]
    enum ConnState {
        Connecting,
        Open,
        Closed,
    }

    struct Inner {
        ws: Option<web_sys::WebSocket>,
        state: ConnState,
        url: String,
        queue: VecDeque<String>,
    }

    #[derive(Clone)]
    pub struct WsPublisher {
        inner: Rc<RefCell<Inner>>,
    }

    impl WsPublisher {
        pub fn connect(url: String) -> Self {
            let inner = Rc::new(RefCell::new(Inner {
                ws: None,
                state: ConnState::Closed,
                url,
                queue: VecDeque::new(),
            }));

            let publisher = Self { inner };
            publisher.start_connection();
            publisher
        }

        fn start_connection(&self) {
            let inner = self.inner.clone();
            let mut borrow = inner.borrow_mut();

            if borrow.state == ConnState::Connecting {
                return;
            }

            let url = borrow.url.clone();
            let ws = match web_sys::WebSocket::new(&url) {
                Ok(ws) => ws,
                Err(err) => {
                    re_log::warn!("WsPublisher: failed to create WebSocket: {err:?}");
                    schedule_reconnect(inner.clone());
                    return;
                }
            };
            ws.set_binary_type(web_sys::BinaryType::Arraybuffer);
            borrow.state = ConnState::Connecting;

            // onopen
            let inner_open = inner.clone();
            let onopen = Closure::<dyn FnMut(JsValue)>::new(move |_| {
                re_log::debug!("WsPublisher: connected");
                let mut borrow = inner_open.borrow_mut();
                borrow.state = ConnState::Open;
                // Flush queued messages
                while let Some(msg) = borrow.queue.pop_front() {
                    if let Some(ref ws) = borrow.ws {
                        if ws.send_with_str(&msg).is_err() {
                            break;
                        }
                    }
                }
            });
            ws.set_onopen(Some(onopen.as_ref().unchecked_ref()));
            onopen.forget();

            // onclose
            let inner_close = inner.clone();
            let onclose = Closure::<dyn FnMut(web_sys::CloseEvent)>::new(move |_| {
                re_log::debug!("WsPublisher: connection closed, reconnecting…");
                {
                    let mut borrow = inner_close.borrow_mut();
                    borrow.state = ConnState::Closed;
                    borrow.ws = None;
                    // Drain stale messages
                    borrow.queue.clear();
                }
                schedule_reconnect(inner_close.clone());
            });
            ws.set_onclose(Some(onclose.as_ref().unchecked_ref()));
            onclose.forget();

            // onerror
            let onerror = Closure::<dyn FnMut(web_sys::ErrorEvent)>::new(move |_| {
                re_log::debug!("WsPublisher: WebSocket error");
            });
            ws.set_onerror(Some(onerror.as_ref().unchecked_ref()));
            onerror.forget();

            borrow.ws = Some(ws);
        }

        pub fn send_click(
            &self,
            x: f64,
            y: f64,
            z: f64,
            entity_path: &str,
            timestamp_ms: u64,
        ) -> Result<(), SendError> {
            self.broadcast(&WsEvent::Click {
                x,
                y,
                z,
                entity_path: entity_path.to_owned(),
                timestamp_ms,
            })
        }

        pub fn send_twist(
            &self,
            linear_x: f64,
            linear_y: f64,
            linear_z: f64,
            angular_x: f64,
            angular_y: f64,
            angular_z: f64,
        ) -> Result<(), SendError> {
            self.broadcast(&WsEvent::Twist {
                linear_x,
                linear_y,
                linear_z,
                angular_x,
                angular_y,
                angular_z,
            })
        }

        pub fn send_stop(&self) -> Result<(), SendError> {
            self.broadcast(&WsEvent::Stop)
        }

        fn broadcast(&self, event: &WsEvent) -> Result<(), SendError> {
            let json =
                serde_json::to_string(event).map_err(|e| SendError::Serialize(e.to_string()))?;
            let mut inner = self.inner.borrow_mut();
            match inner.state {
                ConnState::Open => {
                    if let Some(ref ws) = inner.ws {
                        ws.send_with_str(&json)
                            .map_err(|_| SendError::NotConnected)?;
                    }
                    Ok(())
                }
                ConnState::Connecting => {
                    if inner.queue.len() < 256 {
                        inner.queue.push_back(json);
                    }
                    Ok(())
                }
                ConnState::Closed => Err(SendError::NotConnected),
            }
        }
    }

    fn schedule_reconnect(inner: Rc<RefCell<Inner>>) {
        let cb = Closure::<dyn FnMut()>::new(move || {
            let publisher = WsPublisher {
                inner: inner.clone(),
            };
            publisher.start_connection();
        });

        let global = js_sys::global();
        if let Ok(window) = global.dyn_into::<web_sys::Window>() {
            let _: Result<i32, _> = window.set_timeout_with_callback_and_timeout_and_arguments_0(
                cb.as_ref().unchecked_ref(),
                2000,
            );
        }
        cb.forget();
    }
}

// ─── Re-export the platform-specific type ────────────────────────────────────

#[cfg(not(target_arch = "wasm32"))]
pub use native::WsPublisher;

#[cfg(target_arch = "wasm32")]
pub use wasm::WsPublisher;
