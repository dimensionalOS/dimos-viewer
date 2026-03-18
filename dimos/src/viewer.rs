//! DimOS Interactive Viewer — custom Rerun viewer with LCM click-to-navigate and WASD teleop.
//!
//! Accepts ALL stock Rerun CLI flags and adds DimOS-specific behavior:
//! - Click-to-navigate: click any entity with a 3D position → PointStamped LCM on /clicked_point
//! - WASD keyboard teleop: click overlay to engage, then WASD publishes Twist on /cmd_vel

use std::rc::Rc;
use std::cell::RefCell;
use std::time::{Duration, Instant, SystemTime, UNIX_EPOCH};

use dimos_viewer::interaction::{KeyboardHandler, LcmPublisher, click_event_from_ms};
use rerun::external::{eframe, egui, re_memory, re_viewer};

#[global_allocator]
static GLOBAL: re_memory::AccountingAllocator<mimalloc::MiMalloc> =
    re_memory::AccountingAllocator::new(mimalloc::MiMalloc);

/// LCM channel for click events (follows RViz convention)
const LCM_CHANNEL: &str = "/clicked_point#geometry_msgs.PointStamped";
/// Minimum time between click events (debouncing)
const CLICK_DEBOUNCE_MS: u64 = 100;
/// Maximum rapid clicks before logging a warning
const RAPID_CLICK_THRESHOLD: usize = 5;

/// Wraps re_viewer::App to add keyboard teleop overlay.
struct DimosApp {
    inner: re_viewer::App,
    keyboard: KeyboardHandler,
}

impl eframe::App for DimosApp {
    fn ui(&mut self, ui: &mut egui::Ui, frame: &mut eframe::Frame) {
        self.keyboard.process(ui.ctx());
        self.keyboard.draw_overlay(ui.ctx());
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

fn main() -> Result<(), Box<dyn std::error::Error>> {
    let main_thread_token = re_viewer::MainThreadToken::i_promise_i_am_on_the_main_thread();
    let build_info = re_viewer::build_info();

    let lcm_publisher = LcmPublisher::new(LCM_CHANNEL.to_string())
        .expect("Failed to create LCM publisher");

    let last_click_time = Rc::new(RefCell::new(
        Instant::now() - Duration::from_secs(10)
    ));
    let rapid_click_count = Rc::new(RefCell::new(0usize));

    // Plain click (no Ctrl required) fires nav goal on any entity with a 3D position
    let startup_patch = rerun::StartupOptionsPatch {
        on_event: Some(Rc::new(move |event: re_viewer::ViewerEvent| {
            if let re_viewer::ViewerEventKind::SelectionChange { items } = event.kind {
                let mut has_position = false;
                let mut no_position_count = 0;

                for item in &items {
                    match item {
                        re_viewer::SelectionChangeItem::Entity {
                            entity_path,
                            position: Some(pos),
                            ..
                        } => {
                            has_position = true;

                            let now = Instant::now();
                            let elapsed = now.duration_since(*last_click_time.borrow());

                            if elapsed < Duration::from_millis(CLICK_DEBOUNCE_MS) {
                                let mut count = rapid_click_count.borrow_mut();
                                *count += 1;
                                if *count == RAPID_CLICK_THRESHOLD {
                                    rerun::external::re_log::warn!(
                                        "Rapid click detected ({RAPID_CLICK_THRESHOLD} clicks within {CLICK_DEBOUNCE_MS}ms)"
                                    );
                                }
                                continue;
                            } else {
                                *rapid_click_count.borrow_mut() = 0;
                            }
                            *last_click_time.borrow_mut() = now;

                            let ts = SystemTime::now()
                                .duration_since(UNIX_EPOCH)
                                .unwrap_or_default()
                                .as_millis() as u64;

                            let click = click_event_from_ms(
                                [pos.x, pos.y, pos.z],
                                &entity_path.to_string(),
                                ts,
                            );

                            match lcm_publisher.publish(&click) {
                                Ok(_) => rerun::external::re_log::debug!(
                                    "Nav goal: entity={}, pos=({:.2}, {:.2}, {:.2})",
                                    entity_path, pos.x, pos.y, pos.z
                                ),
                                Err(e) => rerun::external::re_log::error!(
                                    "Failed to publish nav goal: {e:?}"
                                ),
                            }
                        }
                        re_viewer::SelectionChangeItem::Entity { position: None, .. } => {
                            no_position_count += 1;
                        }
                        _ => {}
                    }
                }

                if !has_position && no_position_count > 0 {
                    rerun::external::re_log::trace!(
                        "Selection change without position ({no_position_count} items) — normal for hover/keyboard nav."
                    );
                }
            }
        })),
    };

    let wrapper: rerun::AppWrapper = Box::new(move |app| {
        let keyboard = KeyboardHandler::new()
            .map_err(|e| -> Box<dyn std::error::Error + Send + Sync> { Box::new(e) })?;
        Ok(Box::new(DimosApp { inner: app, keyboard }))
    });

    let exit_code = rerun::run_with_app_wrapper(
        main_thread_token,
        build_info,
        rerun::CallSource::Cli,
        std::env::args(),
        Some(wrapper),
        Some(startup_patch),
    )?;

    std::process::exit(exit_code.into());
}
