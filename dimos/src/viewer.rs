use std::cell::{Cell, RefCell};
use std::rc::Rc;
use std::time::{Duration, Instant, SystemTime, UNIX_EPOCH};

use clap::Parser;
use dimos_viewer::interaction::{LcmPublisher, KeyboardHandler, click_event_from_ms};
use rerun::external::{eframe, egui, re_crash_handler, re_grpc_server, re_log, re_memory, re_viewer};

#[global_allocator]
static GLOBAL: re_memory::AccountingAllocator<mimalloc::MiMalloc> =
    re_memory::AccountingAllocator::new(mimalloc::MiMalloc);

const LCM_CHANNEL: &str = "/clicked_point#geometry_msgs.PointStamped";
const CLICK_DEBOUNCE_MS: u64 = 100;
const DEFAULT_PORT: u16 = 9877;

/// Entity path prefixes that are considered "robot" entities.
/// Clicking these activates teleop for that robot.
const ROBOT_PREFIXES: &[&str] = &["/world/go2", "/world/g1", "/world/robot"];

#[derive(Parser, Debug)]
#[command(name = "dimos-viewer", version, about)]
struct Args {
    #[arg(long, default_value_t = DEFAULT_PORT)]
    port: u16,
    #[arg(long, default_value = "75%")]
    memory_limit: String,
    #[arg(long, default_value = "1GiB")]
    server_memory_limit: String,
    #[arg(long)]
    hide_welcome_screen: bool,
    #[arg(long)]
    expect_data_soon: bool,
}

/// Wraps re_viewer::App with keyboard teleop and click-to-nav.
struct DimosApp {
    inner: re_viewer::App,
    keyboard: Rc<RefCell<KeyboardHandler>>,
    ctrl_held: Rc<Cell<bool>>,
}

impl eframe::App for DimosApp {
    fn ui(&mut self, ui: &mut egui::Ui, frame: &mut eframe::Frame) {
        self.ctrl_held.set(ui.ctx().input(|i| i.modifiers.ctrl || i.modifiers.mac_cmd));
        self.keyboard.borrow_mut().process(ui.ctx());
        self.keyboard.borrow().draw_overlay(ui.ctx());
        self.inner.ui(ui, frame);
    }

    fn save(&mut self, storage: &mut dyn eframe::Storage) { self.inner.save(storage); }
    fn clear_color(&self, visuals: &egui::Visuals) -> [f32; 4] { self.inner.clear_color(visuals) }
    fn persist_egui_memory(&self) -> bool { self.inner.persist_egui_memory() }
    fn auto_save_interval(&self) -> Duration { self.inner.auto_save_interval() }
    fn raw_input_hook(&mut self, ctx: &egui::Context, raw_input: &mut egui::RawInput) {
        self.inner.raw_input_hook(ctx, raw_input);
    }
}

/// Check if an entity path belongs to a known robot.
/// Returns the robot prefix (e.g. "/world/go2") if matched.
fn robot_prefix_for(entity_path: &str) -> Option<&'static str> {
    ROBOT_PREFIXES.iter().find(|&&p| entity_path.starts_with(p)).copied()
}

#[tokio::main]
async fn main() -> Result<(), Box<dyn std::error::Error>> {
    let args = Args::parse();
    let main_thread_token = re_viewer::MainThreadToken::i_promise_i_am_on_the_main_thread();
    re_log::setup_logging();
    re_crash_handler::install_crash_handlers(re_viewer::build_info());

    let listen_addr = format!("0.0.0.0:{}", args.port);
    re_log::info!("Listening for SDK connections on {listen_addr}");
    let rx_log = re_grpc_server::spawn_with_recv(
        listen_addr.parse()?,
        re_grpc_server::ServerOptions {
            memory_limit: re_memory::MemoryLimit::parse(&args.server_memory_limit)
                .expect("Bad --server-memory-limit"),
            ..Default::default()
        },
        re_grpc_server::shutdown::never(),
    );

    let lcm_publisher = LcmPublisher::new(LCM_CHANNEL.to_string())
        .expect("Failed to create LCM publisher");

    // Shared keyboard handler: DimosApp uses it for process/draw,
    // on_event callback uses it to engage/disengage teleop on robot click
    let keyboard = Rc::new(RefCell::new(
        KeyboardHandler::new().expect("Failed to create keyboard handler")
    ));
    let keyboard_for_callback = keyboard.clone();

    let ctrl_held = Rc::new(Cell::new(false));
    let ctrl_for_callback = ctrl_held.clone();
    let last_click_time = Rc::new(RefCell::new(Instant::now()));

    let memory_limit = re_memory::MemoryLimit::parse(&args.memory_limit)
        .expect("Bad --memory-limit");
    re_log::info!("Memory limit: {memory_limit}");

    let mut native_options = re_viewer::native::eframe_options(None);
    native_options.viewport = native_options.viewport
        .with_app_id("rerun_example_custom_callback");

    let startup_options = re_viewer::StartupOptions {
        memory_limit,
        on_event: Some(Rc::new(move |event: re_viewer::ViewerEvent| {
            if let re_viewer::ViewerEventKind::SelectionChange { items } = event.kind {
                for item in &items {
                    if let re_viewer::SelectionChangeItem::Entity { entity_path, position, .. } = item {
                        let path = entity_path.to_string();

                        // Check if clicked entity is a robot → engage teleop
                        if let Some(prefix) = robot_prefix_for(&path) {
                            let mut kb = keyboard_for_callback.borrow_mut();
                            kb.set_active_robot(Some(prefix.to_string()));
                            kb.set_engaged(true);
                            re_log::info!("Teleop engaged: {prefix}");
                            return; // Robot click = engage only, not nav goal
                        }

                        // Not a robot entity: disengage teleop
                        {
                            let mut kb = keyboard_for_callback.borrow_mut();
                            if kb.engaged() {
                                kb.set_engaged(false);
                                re_log::info!("Teleop disengaged");
                            }
                        }

                        // Ctrl+click on non-robot entity with position → nav goal
                        if ctrl_for_callback.get() {
                            if let Some(pos) = position {
                                let now = Instant::now();
                                if now.duration_since(*last_click_time.borrow()) < Duration::from_millis(CLICK_DEBOUNCE_MS) {
                                    continue;
                                }
                                *last_click_time.borrow_mut() = now;

                                let ts = SystemTime::now().duration_since(UNIX_EPOCH)
                                    .unwrap_or_default().as_millis() as u64;
                                let click = click_event_from_ms([pos.x, pos.y, pos.z], "world", ts);
                                if let Err(e) = lcm_publisher.publish(&click) {
                                    re_log::error!("Nav goal failed: {e:?}");
                                } else {
                                    re_log::info!("Nav goal: ({:.2}, {:.2}, {:.2})", pos.x, pos.y, pos.z);
                                }
                            }
                        }
                    }
                }
            }
        })),
        ..Default::default()
    };

    eframe::run_native(
        "DimOS Interactive Viewer",
        native_options,
        Box::new(move |cc| {
            re_viewer::customize_eframe_and_setup_renderer(cc)?;
            let mut rerun_app = re_viewer::App::new(
                main_thread_token,
                re_viewer::build_info(),
                re_viewer::AppEnvironment::Custom("DimOS Interactive Viewer".to_owned()),
                startup_options,
                cc,
                None,
                re_viewer::AsyncRuntimeHandle::from_current_tokio_runtime_or_wasmbindgen()?,
            );
            rerun_app.add_log_receiver(rx_log);
            Ok(Box::new(DimosApp { inner: rerun_app, keyboard, ctrl_held }))
        }),
    )?;
    Ok(())
}
