#[expect(clippy::disallowed_methods)]
mod colors {
    pub const OVERLAY_BG: egui::Color32 = egui::Color32::from_rgba_premultiplied(20, 20, 30, 220);
    pub const KEY_ACTIVE_BG: egui::Color32 = egui::Color32::from_rgb(60, 180, 75);
    pub const KEY_INACTIVE_BG: egui::Color32 =
        egui::Color32::from_rgba_premultiplied(60, 60, 80, 180);
    pub const KEY_TEXT_COLOR: egui::Color32 = egui::Color32::WHITE;
    pub const LABEL_COLOR: egui::Color32 = egui::Color32::from_rgb(180, 180, 200);
    pub const ESTOP_ACTIVE_BG: egui::Color32 = egui::Color32::from_rgb(220, 50, 50);
    pub const ENGAGED_BORDER: egui::Color32 = egui::Color32::from_rgb(60, 180, 75);
    pub const DISENGAGED_BORDER: egui::Color32 = egui::Color32::from_rgb(80, 80, 100);
    pub const FAST_COLOR: egui::Color32 = egui::Color32::from_rgb(255, 200, 50);
}
use colors::{
    DISENGAGED_BORDER, ENGAGED_BORDER, ESTOP_ACTIVE_BG, FAST_COLOR, KEY_ACTIVE_BG, KEY_INACTIVE_BG,
    KEY_TEXT_COLOR, LABEL_COLOR, OVERLAY_BG,
};

use super::ws::WsPublisher;

const BASE_LINEAR_SPEED: f64 = 0.5;
const BASE_ANGULAR_SPEED: f64 = 0.8;
const FAST_MULTIPLIER: f64 = 2.0;

const OVERLAY_PADDING: f32 = 10.0;
const OVERLAY_ROUNDING: f32 = 8.0;
const KEY_SIZE: f32 = 32.0;
const KEY_GAP: f32 = 3.0;

#[derive(Debug, Clone, Default)]
struct KeyState {
    forward: bool,
    backward: bool,
    left: bool,
    right: bool,
    strafe_l: bool,
    strafe_r: bool,
    fast: bool,
}

impl KeyState {
    fn any_active(&self) -> bool {
        self.forward || self.backward || self.left || self.right || self.strafe_l || self.strafe_r
    }

    fn reset(&mut self) {
        *self = Self::default();
    }
}

pub struct KeyboardHandler {
    ws: WsPublisher,
    state: KeyState,
    was_active: bool,
    estop_flash: bool,
    engaged: bool,
}

impl KeyboardHandler {
    pub fn new(ws: WsPublisher) -> Self {
        Self {
            ws,
            state: KeyState::default(),
            was_active: false,
            estop_flash: false,
            engaged: false,
        }
    }

    pub fn process(&mut self, ctx: &egui::Context) -> bool {
        self.estop_flash = false;

        if !self.engaged {
            if self.was_active {
                if let Err(e) = self.publish_stop() {
                    re_log::warn!("Failed to send stop on disengage: {e}");
                }
                self.was_active = false;
            }
            return false;
        }

        self.update_key_state(ctx);

        if ctx.input(|i| i.key_pressed(egui::Key::Space)) {
            self.state.reset();
            if let Err(e) = self.publish_stop() {
                re_log::warn!("Failed to send emergency stop: {e}");
            }
            self.was_active = false;
            self.estop_flash = true;
            return true;
        }

        if self.state.any_active() {
            if let Err(e) = self.publish_twist() {
                re_log::warn!("Failed to publish twist command: {e}");
            }
            self.was_active = true;
        } else if self.was_active {
            if let Err(e) = self.publish_stop() {
                re_log::warn!("Failed to send stop on key release: {e}");
            }
            self.was_active = false;
        }

        self.state.any_active()
    }

    pub fn draw_overlay(&mut self, ctx: &egui::Context) {
        let area_response = egui::Area::new("dimos_keyboard_hud_br".into())
            .anchor(egui::Align2::RIGHT_BOTTOM, egui::vec2(-5.0, -5.0))
            .order(egui::Order::Foreground)
            .interactable(true)
            .sense(egui::Sense::click_and_drag())
            .show(ctx, |ui| {
                let border_color = if self.engaged {
                    ENGAGED_BORDER
                } else {
                    DISENGAGED_BORDER
                };

                let response = egui::Frame::new()
                    .fill(OVERLAY_BG)
                    .corner_radius(egui::CornerRadius::same(OVERLAY_ROUNDING as u8))
                    .inner_margin(egui::Margin::same(OVERLAY_PADDING as i8))
                    .stroke(egui::Stroke::new(2.0, border_color))
                    .show(ui, |ui| {
                        self.draw_hud_content(ui);
                    });

                let click_response = ui.interact(
                    response.response.rect,
                    ui.id().with("wasd_click"),
                    egui::Sense::click(),
                );

                if click_response.hovered() {
                    ctx.set_cursor_icon(egui::CursorIcon::Default);
                }

                if click_response.clicked() {
                    self.engaged = !self.engaged;
                    if !self.engaged {
                        if let Err(err) = self.publish_stop() {
                            re_log::warn!("Failed to send stop on disengage: {err}");
                        }
                        self.state.reset();
                        self.was_active = false;
                    }
                }
            })
            .response;

        if self.engaged
            && !ctx.rect_contains_pointer(area_response.layer_id, area_response.interact_rect)
            && ctx.input(|i| i.pointer.primary_clicked())
        {
            self.engaged = false;
            if let Err(err) = self.publish_stop() {
                re_log::warn!("Failed to send stop on outside click: {err}");
            }
            self.state.reset();
            self.was_active = false;
        }
    }

    fn draw_hud_content(&self, ui: &mut egui::Ui) {
        ui.label(
            egui::RichText::new("Keyboard Teleop")
                .color(LABEL_COLOR)
                .size(13.0),
        );
        ui.add_space(4.0);

        let row1 = [
            ("Q", self.state.strafe_l),
            ("W", self.state.forward),
            ("E", self.state.strafe_r),
        ];
        let row2 = [
            ("A", self.state.left),
            ("S", self.state.backward),
            ("D", self.state.right),
        ];

        ui.horizontal(|ui| {
            ui.spacing_mut().item_spacing.x = KEY_GAP;
            for (label, pressed) in &row1 {
                Self::draw_key(ui, label, *pressed);
            }
        });

        ui.horizontal(|ui| {
            ui.spacing_mut().item_spacing.x = KEY_GAP;
            for (label, pressed) in &row2 {
                Self::draw_key(ui, label, *pressed);
            }
        });

        let space_width = KEY_SIZE * 3.0 + KEY_GAP * 2.0;
        let space_rect = ui
            .allocate_exact_size(
                egui::vec2(space_width, KEY_SIZE * 0.7),
                egui::Sense::hover(),
            )
            .0;
        let space_bg = if self.estop_flash {
            ESTOP_ACTIVE_BG
        } else {
            KEY_INACTIVE_BG
        };
        ui.painter()
            .rect_filled(space_rect, egui::CornerRadius::same(4), space_bg);
        ui.painter().text(
            space_rect.center(),
            egui::Align2::CENTER_CENTER,
            "STOP",
            egui::FontId::proportional(11.0),
            KEY_TEXT_COLOR,
        );

        ui.add_space(4.0);

        let speed_label = if self.state.fast {
            "\u{21e7} FAST"
        } else {
            "\u{21e7} shift=fast"
        };
        let speed_color = if self.state.fast {
            FAST_COLOR
        } else {
            LABEL_COLOR
        };
        ui.label(
            egui::RichText::new(speed_label)
                .color(speed_color)
                .size(10.0),
        );
    }

    fn draw_key(ui: &mut egui::Ui, label: &str, pressed: bool) {
        let (rect, _) =
            ui.allocate_exact_size(egui::vec2(KEY_SIZE, KEY_SIZE), egui::Sense::hover());
        let bg = if pressed {
            KEY_ACTIVE_BG
        } else {
            KEY_INACTIVE_BG
        };
        ui.painter()
            .rect_filled(rect, egui::CornerRadius::same(4), bg);
        ui.painter().text(
            rect.center(),
            egui::Align2::CENTER_CENTER,
            label,
            egui::FontId::monospace(14.0),
            KEY_TEXT_COLOR,
        );
    }

    fn update_key_state(&mut self, ctx: &egui::Context) {
        ctx.input(|i| {
            self.state.forward = i.key_down(egui::Key::W) || i.key_down(egui::Key::ArrowUp);
            self.state.backward = i.key_down(egui::Key::S) || i.key_down(egui::Key::ArrowDown);
            self.state.left = i.key_down(egui::Key::A) || i.key_down(egui::Key::ArrowLeft);
            self.state.right = i.key_down(egui::Key::D) || i.key_down(egui::Key::ArrowRight);
            self.state.strafe_l = i.key_down(egui::Key::Q);
            self.state.strafe_r = i.key_down(egui::Key::E);
            self.state.fast = i.modifiers.shift;
        });
    }

    fn publish_twist(&self) -> Result<(), super::ws::SendError> {
        let (lin_x, lin_y, lin_z, ang_x, ang_y, ang_z) = self.compute_twist();
        self.ws.send_twist(lin_x, lin_y, lin_z, ang_x, ang_y, ang_z)
    }

    fn publish_stop(&self) -> Result<(), super::ws::SendError> {
        self.ws.send_stop()
    }

    fn compute_twist(&self) -> (f64, f64, f64, f64, f64, f64) {
        let mut linear_x = 0.0;
        let mut linear_y = 0.0;
        let mut angular_z = 0.0;

        if self.state.forward {
            linear_x += BASE_LINEAR_SPEED;
        }
        if self.state.backward {
            linear_x -= BASE_LINEAR_SPEED;
        }
        if self.state.strafe_l {
            linear_y += BASE_LINEAR_SPEED;
        }
        if self.state.strafe_r {
            linear_y -= BASE_LINEAR_SPEED;
        }
        if self.state.left {
            angular_z += BASE_ANGULAR_SPEED;
        }
        if self.state.right {
            angular_z -= BASE_ANGULAR_SPEED;
        }
        if self.state.fast {
            linear_x *= FAST_MULTIPLIER;
            linear_y *= FAST_MULTIPLIER;
            angular_z *= FAST_MULTIPLIER;
        }

        (linear_x, linear_y, 0.0, 0.0, 0.0, angular_z)
    }
}

impl std::fmt::Debug for KeyboardHandler {
    fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        f.debug_struct("KeyboardHandler")
            .field("state", &self.state)
            .field("was_active", &self.was_active)
            .finish_non_exhaustive()
    }
}
