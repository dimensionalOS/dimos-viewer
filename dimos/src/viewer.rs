use std::cell::RefCell;
use std::rc::Rc;
use std::time::{Duration, Instant, SystemTime, UNIX_EPOCH};

use clap::Parser;
use dimos_viewer::interaction::{LcmPublisher, KeyboardHandler, WsPublisher, click_event_from_ms};
use rerun::external::{eframe, egui, re_crash_handler, re_grpc_client, re_grpc_server, re_log, re_memory, re_uri, re_viewer};

#[global_allocator]
static GLOBAL: re_memory::AccountingAllocator<mimalloc::MiMalloc> =
    re_memory::AccountingAllocator::new(mimalloc::MiMalloc);

/// LCM channel for click events (follows RViz convention)
const LCM_CHANNEL: &str = "/clicked_point#geometry_msgs.PointStamped";

/// Minimum time between click events (debouncing)
const CLICK_DEBOUNCE_MS: u64 = 100;

/// Maximum rapid clicks to log as warning
const RAPID_CLICK_THRESHOLD: usize = 5;

/// Default gRPC listen port (9877 to avoid conflict with stock Rerun on 9876)
const DEFAULT_PORT: u16 = 9877;

/// Default WebSocket event port used in --connect mode
const DEFAULT_WS_PORT: u16 = 3030;

/// DimOS Interactive Viewer — a custom Rerun viewer with LCM click-to-navigate.
///
/// Accepts the same CLI flags as the stock `rerun` binary so it can be spawned
/// seamlessly via `rerun_bindings.spawn(executable_name="dimos-viewer")`.
#[derive(Parser, Debug)]
#[command(name = "dimos-viewer", version, about)]
struct Args {
    /// The gRPC port to listen on for incoming SDK connections.
    #[arg(long, default_value_t = DEFAULT_PORT)]
    port: u16,

    /// An upper limit on how much memory the viewer should use.
    /// When this limit is reached, the oldest data will be dropped.
    /// Examples: "75%", "16GB".
    #[arg(long, default_value = "75%")]
    memory_limit: String,

    /// An upper limit on how much memory the gRPC server should use.
    /// Examples: "1GiB", "50%".
    #[arg(long, default_value = "1GiB")]
    server_memory_limit: String,

    /// Hide the Rerun welcome screen.
    #[arg(long)]
    hide_welcome_screen: bool,

    /// Hint that data will arrive shortly (suppresses "waiting for data" message).
    #[arg(long)]
    expect_data_soon: bool,

    /// Do not start a local gRPC server; instead connect to an existing one.
    ///
    /// Optionally accepts a URL to a gRPC proxy server.
    /// The scheme must be one of `rerun://`, `rerun+http://`, or `rerun+https://`,
    /// and the pathname must be `/proxy`.
    ///
    /// Defaults to `rerun+http://127.0.0.1:<port>/proxy`.
    #[arg(long)]
    connect: Option<Option<String>>,

    /// WebSocket port for publishing click/keyboard events in --connect mode.
    ///
    /// A WebSocket server is started on `ws://0.0.0.0:<ws_port>/ws` and
    /// broadcasts JSON events to all connected clients.
    /// Ignored when not using --connect.
    #[arg(long, default_value_t = DEFAULT_WS_PORT)]
    ws_port: u16,
}

/// Wraps re_viewer::App to add keyboard control interception.
struct DimosApp {
    inner: re_viewer::App,
    keyboard: KeyboardHandler,
}

impl DimosApp {
    fn new(
        inner: re_viewer::App,
        keyboard: KeyboardHandler,
    ) -> Self {
        Self {
            inner,
            keyboard,
        }
    }
}

impl eframe::App for DimosApp {
    fn ui(&mut self, ui: &mut egui::Ui, frame: &mut eframe::Frame) {
        // Process keyboard input before delegating to Rerun
        self.keyboard.process(ui.ctx());

        // Draw the keyboard HUD overlay (click to engage/disengage)
        self.keyboard.draw_overlay(ui.ctx());

        // Delegate to Rerun's main ui method
        self.inner.ui(ui, frame);
    }

    // Delegate all other methods to inner re_viewer::App
    fn save(&mut self, storage: &mut dyn eframe::Storage) {
        self.inner.save(storage);
    }

    fn clear_color(&self, visuals: &egui::Visuals) -> [f32; 4] {
        self.inner.clear_color(visuals)
    }

    fn persist_egui_memory(&self) -> bool {
        self.inner.persist_egui_memory()
    }

    fn auto_save_interval(&self) -> std::time::Duration {
        self.inner.auto_save_interval()
    }

    fn raw_input_hook(&mut self, ctx: &egui::Context, raw_input: &mut egui::RawInput) {
        self.inner.raw_input_hook(ctx, raw_input);
    }
}

#[tokio::main]
async fn main() -> Result<(), Box<dyn std::error::Error>> {
    let args = Args::parse();

    let main_thread_token = re_viewer::MainThreadToken::i_promise_i_am_on_the_main_thread();
    re_log::setup_logging();
    re_crash_handler::install_crash_handlers(re_viewer::build_info());

    // Either connect to an existing gRPC proxy, or spawn a local server.
    let rx_log = if let Some(url) = args.connect.clone() {
        let url = url.unwrap_or_else(|| format!("rerun+http://127.0.0.1:{}/proxy", args.port));
        let proxy_uri: re_uri::ProxyUri = url.parse()
            .expect("Bad --connect URL: expected format rerun+http://host:port/proxy");
        re_log::info!("Connecting to existing gRPC proxy at {proxy_uri}");
        re_grpc_client::stream(proxy_uri)
    } else {
        let listen_addr = format!("0.0.0.0:{}", args.port);
        re_log::info!("Listening for SDK connections on {listen_addr}");
        let server_memory_limit = re_memory::MemoryLimit::parse(&args.server_memory_limit)
            .expect("Bad --server-memory-limit");
        re_grpc_server::spawn_with_recv(
            listen_addr.parse()?,
            re_grpc_server::ServerOptions {
                memory_limit: server_memory_limit,
                ..Default::default()
            },
            re_grpc_server::shutdown::never(),
        )
    };

    // In connect mode: use WebSocket for events. Otherwise: use LCM (multicast).
    let (lcm_publisher, ws_publisher, keyboard_handler) = if args.connect.is_some() {
        let ws = WsPublisher::spawn(args.ws_port);
        let kb = KeyboardHandler::new_ws(ws.clone())
            .expect("Failed to create keyboard handler");
        (None::<LcmPublisher>, Some(ws), kb)
    } else {
        let lcm = LcmPublisher::new(LCM_CHANNEL.to_string())
            .expect("Failed to create LCM publisher");
        re_log::info!("LCM publisher created for channel: {LCM_CHANNEL}");
        let kb = KeyboardHandler::new()
            .expect("Failed to create keyboard handler");
        (Some(lcm), None::<WsPublisher>, kb)
    };


    // State for debouncing and rapid click detection
    let last_click_time = Rc::new(RefCell::new(Instant::now()));
    let rapid_click_count = Rc::new(RefCell::new(0usize));

    let mut native_options = re_viewer::native::eframe_options(None);
    native_options.viewport = native_options
        .viewport
        .with_app_id("rerun_example_custom_callback");

    let app_env = re_viewer::AppEnvironment::Custom("DimOS Interactive Viewer".to_owned());

    let memory_limit = re_memory::MemoryLimit::parse(&args.memory_limit)
        .expect("Bad --memory-limit");
    re_log::info!("Memory limit: {memory_limit}");

    let startup_options = re_viewer::StartupOptions {
        memory_limit,
        on_event: Some(Rc::new({
            let last_click_time = last_click_time.clone();
            let rapid_click_count = rapid_click_count.clone();

            move |event: re_viewer::ViewerEvent| {
                if let re_viewer::ViewerEventKind::SelectionChange { items } = event.kind {
                    let mut has_position = false;
                    let mut no_position_count = 0;

                    for item in items {
                        match item {
                            re_viewer::SelectionChangeItem::Entity {
                                entity_path,
                                view_name: _,
                                position: Some(pos),
                                ..
                            } => {
                                has_position = true;

                                // Debouncing
                                let now = Instant::now();
                                let elapsed = now.duration_since(*last_click_time.borrow());

                                if elapsed < Duration::from_millis(CLICK_DEBOUNCE_MS) {
                                    let mut count = rapid_click_count.borrow_mut();
                                    *count += 1;
                                    if *count == RAPID_CLICK_THRESHOLD {
                                        re_log::warn!(
                                            "Rapid click detected ({} clicks within {}ms)",
                                            RAPID_CLICK_THRESHOLD,
                                            CLICK_DEBOUNCE_MS
                                        );
                                    }
                                    continue;
                                } else {
                                    *rapid_click_count.borrow_mut() = 0;
                                }
                                *last_click_time.borrow_mut() = now;

                                let timestamp_ms = SystemTime::now()
                                    .duration_since(UNIX_EPOCH)
                                    .unwrap_or_default()
                                    .as_millis() as u64;

                                if let Some(ws) = &ws_publisher {
                                    // Connect mode: publish click over WebSocket
                                    ws.send_click(
                                        pos.x as f64,
                                        pos.y as f64,
                                        pos.z as f64,
                                        &entity_path.to_string(),
                                        timestamp_ms,
                                    );
                                    re_log::debug!(
                                        "WS click event published: entity={}, pos=({:.2}, {:.2}, {:.2})",
                                        entity_path,
                                        pos.x,
                                        pos.y,
                                        pos.z
                                    );
                                } else if let Some(lcm) = &lcm_publisher {
                                    // Local mode: publish click via LCM
                                    let click = click_event_from_ms(
                                        [pos.x, pos.y, pos.z],
                                        &entity_path.to_string(),
                                        timestamp_ms,
                                    );
                                    match lcm.publish(&click) {
                                        Ok(_) => {
                                            re_log::debug!(
                                                "LCM click event published: entity={}, pos=({:.2}, {:.2}, {:.2})",
                                                entity_path,
                                                pos.x,
                                                pos.y,
                                                pos.z
                                            );
                                        }
                                        Err(err) => {
                                            re_log::error!("Failed to publish LCM click event: {err}");
                                        }
                                    }
                                }
                            }
                            re_viewer::SelectionChangeItem::Entity { position: None, .. } => {
                                no_position_count += 1;
                            }
                            _ => {}
                        }
                    }

                    if !has_position && no_position_count > 0 {
                        re_log::trace!(
                            "Selection change without position data ({no_position_count} items). \
                             This is normal for hover/keyboard navigation."
                        );
                    }
                }
            }
        })),
        ..Default::default()
    };

    let window_title = "DimOS Interactive Viewer";
    eframe::run_native(
        window_title,
        native_options,
        Box::new(move |cc| {
            re_viewer::customize_eframe_and_setup_renderer(cc)?;

            let mut rerun_app = re_viewer::App::new(
                main_thread_token,
                re_viewer::build_info(),
                app_env,
                startup_options,
                cc,
                None,
                re_viewer::AsyncRuntimeHandle::from_current_tokio_runtime_or_wasmbindgen()?,
            );

            rerun_app.add_log_receiver(rx_log);

            let dimos_app = DimosApp::new(rerun_app, keyboard_handler);

            Ok(Box::new(dimos_app))
        }),
    )?;

    Ok(())
}
