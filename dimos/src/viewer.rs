use clap::Parser;
use dimos_viewer::interaction::KeyboardHandler;
use rerun::external::{eframe, egui, re_crash_handler, re_grpc_server, re_log, re_memory, re_viewer};

#[global_allocator]
static GLOBAL: re_memory::AccountingAllocator<mimalloc::MiMalloc> =
    re_memory::AccountingAllocator::new(mimalloc::MiMalloc);

/// Default gRPC listen port (9877 to avoid conflict with stock Rerun on 9876)
const DEFAULT_PORT: u16 = 9877;

/// DimOS Interactive Viewer — a custom Rerun viewer with WASD keyboard teleop.
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

    // Listen for gRPC connections from Rerun's logging SDKs.
    let listen_addr = format!("0.0.0.0:{}", args.port);
    re_log::info!("Listening for SDK connections on {listen_addr}");
    let server_memory_limit = re_memory::MemoryLimit::parse(&args.server_memory_limit)
        .expect("Bad --server-memory-limit");
    let rx_log = re_grpc_server::spawn_with_recv(
        listen_addr.parse()?,
        re_grpc_server::ServerOptions {
            memory_limit: server_memory_limit,
            ..Default::default()
        },
        re_grpc_server::shutdown::never(),
    );

    // Create keyboard handler
    let keyboard_handler = KeyboardHandler::new()
        .expect("Failed to create keyboard handler");
    re_log::info!("Keyboard handler initialized for WASD controls on /cmd_vel");

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
