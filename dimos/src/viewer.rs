//! DimOS Interactive Viewer — custom Rerun viewer with LCM click-to-navigate and WASD teleop.
//!
//! Accepts ALL stock Rerun CLI flags and adds DimOS-specific behavior:
//! - Click-to-navigate: clicks publish PointStamped LCM on /clicked_point
//! - WASD keyboard teleop: publishes Twist LCM on /cmd_vel
//!
//! ```bash
//! dimos-viewer                                              # standalone
//! dimos-viewer --connect rerun+http://127.0.0.1:9876/proxy  # connect to source
//! dimos-viewer --port 9877 --memory-limit 2GB               # custom port/memory
//! dimos-viewer --serve-web                                  # web viewer + gRPC
//! dimos-viewer --serve-grpc                                 # headless gRPC only
//! dimos-viewer recording.rrd                                # open recording
//! ```

use dimos_viewer::interaction::KeyboardHandler;
use rerun::external::{eframe, egui, re_memory, re_viewer};

#[global_allocator]
static GLOBAL: re_memory::AccountingAllocator<mimalloc::MiMalloc> =
    re_memory::AccountingAllocator::new(mimalloc::MiMalloc);

/// Wraps re_viewer::App to add keyboard teleop and click-to-nav overlay.
struct DimosApp {
    inner: re_viewer::App,
    keyboard: KeyboardHandler,
}

impl DimosApp {
    fn new(inner: re_viewer::App, keyboard: KeyboardHandler) -> Self {
        Self { inner, keyboard }
    }
}

impl eframe::App for DimosApp {
    fn ui(&mut self, ui: &mut egui::Ui, frame: &mut eframe::Frame) {
        self.keyboard.process(ui.ctx());
        self.keyboard.draw_overlay(ui.ctx());
        self.inner.ui(ui, frame);
    }

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

fn main() -> Result<(), Box<dyn std::error::Error>> {
    // Delegate ALL CLI handling to Rerun's entrypoint with our DimosApp wrapper.
    //
    // `run_with_app_wrapper` handles:
    //   - Full Rerun CLI arg parsing (--connect, --port, --memory-limit, etc.)
    //   - --version, subcommands (reset, rrd, auth, etc.)
    //   - Data source routing (--serve-grpc, --serve-web, .rrd files, etc.)
    //   - Native viewer startup (where our wrapper injects DimosApp)
    //
    // The wrapper is ONLY called for the native viewer path. All other modes
    // (--serve-grpc, --serve-web, --save, etc.) work identically to stock Rerun.
    let main_thread_token = re_viewer::MainThreadToken::i_promise_i_am_on_the_main_thread();
    let build_info = re_viewer::build_info();

    let wrapper: rerun::AppWrapper = Box::new(|app| {
        let keyboard = KeyboardHandler::new()
            .map_err(|e| -> Box<dyn std::error::Error + Send + Sync> { Box::new(e) })?;
        Ok(Box::new(DimosApp::new(app, keyboard)))
    });

    let exit_code = rerun::run_with_app_wrapper(
        main_thread_token,
        build_info,
        rerun::CallSource::Cli,
        std::env::args(),
        Some(wrapper),
    )?;

    std::process::exit(exit_code.into());
}
