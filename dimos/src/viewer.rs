use std::cell::RefCell;
use std::rc::Rc;
use std::time::{Duration, Instant, SystemTime, UNIX_EPOCH};

use clap::Parser;
use dimos_viewer::interaction::{LcmPublisher, click_event_from_ms};
use rerun::external::{eframe, re_crash_handler, re_grpc_server, re_log, re_memory, re_viewer};

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
    let rx_log = re_grpc_server::spawn_with_recv(
        listen_addr.parse()?,
        Default::default(),
        re_grpc_server::shutdown::never(),
    );

    // Create LCM publisher for click events
    let lcm_publisher = LcmPublisher::new(LCM_CHANNEL.to_string())
        .expect("Failed to create LCM publisher");
    re_log::info!("LCM publisher created for channel: {LCM_CHANNEL}");

    // State for debouncing and rapid click detection
    let last_click_time = Rc::new(RefCell::new(Instant::now()));
    let rapid_click_count = Rc::new(RefCell::new(0usize));

    let mut native_options = re_viewer::native::eframe_options(None);
    native_options.viewport = native_options
        .viewport
        .with_app_id("rerun_example_custom_callback");

    let app_env = re_viewer::AppEnvironment::Custom("DimOS Interactive Viewer".to_owned());

    let startup_options = re_viewer::StartupOptions {
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

                                // Build click event and publish via LCM
                                let click = click_event_from_ms(
                                    [pos.x, pos.y, pos.z],
                                    &entity_path.to_string(),
                                    timestamp_ms,
                                );

                                match lcm_publisher.publish(&click) {
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
                                        re_log::error!("Failed to publish LCM click event: {err:?}");
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

            Ok(Box::new(rerun_app))
        }),
    )?;

    Ok(())
}
