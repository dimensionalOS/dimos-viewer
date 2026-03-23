use std::cell::RefCell;
use std::rc::Rc;
use std::time::{Duration, Instant, SystemTime, UNIX_EPOCH};

use clap::Parser;
use dimos_viewer::interaction::{KeyboardHandler, WsPublisher};
use rerun::external::{eframe, egui, re_crash_handler, re_grpc_client, re_grpc_server, re_log, re_memory, re_uri, re_viewer};

#[global_allocator]
static GLOBAL: re_memory::AccountingAllocator<mimalloc::MiMalloc> =
    re_memory::AccountingAllocator::new(mimalloc::MiMalloc);

/// Minimum time between click events (debouncing)
const CLICK_DEBOUNCE_MS: u64 = 100;

/// Maximum rapid clicks to log as warning
const RAPID_CLICK_THRESHOLD: usize = 5;

/// Default gRPC listen port (9877 to avoid conflict with stock Rerun on 9876)
const DEFAULT_PORT: u16 = 9877;

/// Default WebSocket server URL to connect to in --connect mode
const DEFAULT_WS_URL: &str = "ws://127.0.0.1:3030/ws";

/// DimOS Interactive Viewer — a custom Rerun viewer with click-to-navigate.
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

    /// WebSocket server URL to connect to for publishing click/keyboard events.
    ///
    /// The viewer connects as a WebSocket CLIENT to this URL and sends JSON
    /// events (click, twist, stop). The server is typically the Python
    /// `RerunWebSocketServer` DimOS module.
    ///
    /// When provided explicitly this flag overrides the default URL.
    /// When omitted, defaults to ws://127.0.0.1:3030/ws.
    #[arg(long)]
    ws_url: Option<String>,
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

    // Resolve the WebSocket URL (always WebSocket, no LCM):
    //   - explicit --ws-url  → use that URL
    //   - otherwise          → default URL
    let ws_url = args.ws_url.unwrap_or_else(|| DEFAULT_WS_URL.to_string());
    let ws_publisher = WsPublisher::connect(ws_url.clone());
    re_log::info!("WebSocket client connecting to {ws_url}");
    let keyboard_handler = KeyboardHandler::new(ws_publisher.clone());


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

                                ws_publisher.send_click(
                                    pos.x as f64,
                                    pos.y as f64,
                                    pos.z as f64,
                                    &entity_path.to_string(),
                                    timestamp_ms,
                                );
                                re_log::debug!(
                                    "Click event published: entity={}, pos=({:.2}, {:.2}, {:.2})",
                                    entity_path,
                                    pos.x,
                                    pos.y,
                                    pos.z
                                );
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
