//! Keyboard handler for WASD movement controls that publish Twist messages.
//!
//! Supports multi-robot: tracks which robot is "active" for teleop.
//! WASD publishes to that robot's /cmd_vel channel.

use std::io;
use super::lcm::{LcmPublisher, twist_command};
use rerun::external::{egui, re_log};

/// LCM channel suffix for twist commands
const CMD_VEL_SUFFIX: &str = "/cmd_vel#geometry_msgs.Twist";
/// Default channel when no robot is selected
const DEFAULT_CMD_VEL: &str = "/cmd_vel#geometry_msgs.Twist";

/// Base speeds
const BASE_LINEAR_SPEED: f64 = 0.5;
const BASE_ANGULAR_SPEED: f64 = 0.8;
const FAST_MULTIPLIER: f64 = 2.0;

/// Overlay styling
const OVERLAY_X: f32 = 260.0;
const OVERLAY_Y: f32 = 12.0;
const OVERLAY_PADDING: f32 = 10.0;
const OVERLAY_ROUNDING: f32 = 8.0;
const OVERLAY_BG_ACTIVE: egui::Color32 = egui::Color32::from_rgba_premultiplied(20, 20, 30, 220);
const OVERLAY_BG_IDLE: egui::Color32 = egui::Color32::from_rgba_premultiplied(20, 20, 30, 100);
const KEY_SIZE: f32 = 32.0;
const KEY_GAP: f32 = 3.0;
const KEY_ACTIVE_BG: egui::Color32 = egui::Color32::from_rgb(60, 180, 75);
const KEY_INACTIVE_BG: egui::Color32 = egui::Color32::from_rgba_premultiplied(60, 60, 80, 180);
const KEY_IDLE_BG: egui::Color32 = egui::Color32::from_rgba_premultiplied(60, 60, 80, 80);
const KEY_TEXT_COLOR: egui::Color32 = egui::Color32::WHITE;
const KEY_TEXT_IDLE: egui::Color32 = egui::Color32::from_rgba_premultiplied(255, 255, 255, 100);
const LABEL_COLOR: egui::Color32 = egui::Color32::from_rgb(180, 180, 200);
const LABEL_IDLE: egui::Color32 = egui::Color32::from_rgba_premultiplied(180, 180, 200, 80);
const ESTOP_ACTIVE_BG: egui::Color32 = egui::Color32::from_rgb(220, 50, 50);

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
    fn new() -> Self { Default::default() }

    fn any_active(&self) -> bool {
        self.forward || self.backward || self.left || self.right || self.strafe_l || self.strafe_r
    }

    fn reset(&mut self) { *self = Default::default(); }
}

/// Handles keyboard input and publishes Twist via LCM.
/// Supports multi-robot via `set_active_robot()`.
pub struct KeyboardHandler {
    publisher: LcmPublisher,
    state: KeyState,
    was_active: bool,
    estop_flash: bool,
    /// Whether teleop is engaged (robot clicked in 3D view)
    engaged: bool,
    /// Currently selected robot entity path prefix, or None for default /cmd_vel
    active_robot: Option<String>,
}

impl KeyboardHandler {
    pub fn new() -> Result<Self, io::Error> {
        let publisher = LcmPublisher::new(DEFAULT_CMD_VEL.to_string())?;
        Ok(Self {
            publisher,
            state: KeyState::new(),
            was_active: false,
            estop_flash: false,
            engaged: false,
            active_robot: None,
        })
    }

    /// Set the active robot for teleop. Recreates LCM publisher for new channel.
    pub fn set_active_robot(&mut self, robot_prefix: Option<String>) {
        if self.active_robot == robot_prefix { return; }
        if self.was_active {
            let _ = self.publish_stop();
            self.was_active = false;
        }
        let channel = match &robot_prefix {
            Some(prefix) => format!("{prefix}{CMD_VEL_SUFFIX}"),
            None => DEFAULT_CMD_VEL.to_string(),
        };
        match LcmPublisher::new(channel.clone()) {
            Ok(p) => { self.publisher = p; re_log::info!("Teleop target: {channel}"); }
            Err(e) => { re_log::error!("Publisher failed for {channel}: {e}"); return; }
        }
        self.active_robot = robot_prefix;
    }

    pub fn active_robot(&self) -> Option<&str> { self.active_robot.as_deref() }

    /// Whether teleop is currently engaged (robot selected).
    pub fn engaged(&self) -> bool { self.engaged }

    /// Set engaged state. Sends stop when disengaging.
    pub fn set_engaged(&mut self, engaged: bool) {
        if self.engaged && !engaged {
            let _ = self.publish_stop();
            self.state.reset();
            self.was_active = false;
        }
        self.engaged = engaged;
    }

    pub fn process(&mut self, ctx: &egui::Context) -> bool {
        self.estop_flash = false;
        // Only capture keys when engaged and no text field focused
        let text_has_focus = ctx.memory(|m| m.focused().is_some());
        if !self.engaged || text_has_focus {
            if self.was_active { let _ = self.publish_stop(); self.was_active = false; }
            return false;
        }
        self.update_key_state(ctx);
        if ctx.input(|i| i.key_pressed(egui::Key::Space)) {
            self.state.reset();
            let _ = self.publish_stop();
            self.was_active = false;
            self.estop_flash = true;
            return true;
        }
        if self.state.any_active() {
            let _ = self.publish_twist();
            self.was_active = true;
        } else if self.was_active {
            let _ = self.publish_stop();
            self.was_active = false;
        }
        self.state.any_active()
    }

    /// Draw keyboard overlay HUD. Greyed out when idle, bright when active.
    pub fn draw_overlay(&self, ctx: &egui::Context) {
        egui::Area::new("keyboard_hud".into())
            .default_pos(egui::pos2(OVERLAY_X, OVERLAY_Y))
            .movable(true)
            .order(egui::Order::Foreground)
            .interactable(false)
            .show(ctx, |ui| {
                let bg = if self.engaged { OVERLAY_BG_ACTIVE } else { OVERLAY_BG_IDLE };
                egui::Frame::new()
                    .fill(bg)
                    .corner_radius(egui::CornerRadius::same(OVERLAY_ROUNDING as u8))
                    .inner_margin(egui::Margin::same(OVERLAY_PADDING as i8))
                    .show(ui, |ui| self.draw_hud_content(ui));
            });
    }

    fn draw_hud_content(&self, ui: &mut egui::Ui) {
        let (title_color, label_color) = if self.engaged {
            (egui::Color32::WHITE, LABEL_COLOR)
        } else {
            (egui::Color32::from_rgba_premultiplied(255, 255, 255, 80), LABEL_IDLE)
        };
        let title = if self.engaged {
            match &self.active_robot {
                Some(name) => format!("🎮 {}", name.rsplit('/').next().unwrap_or(name)),
                None => "🎮 Teleop".to_string(),
            }
        } else {
            "🎮 Teleop (click robot)".to_string()
        };
        ui.label(egui::RichText::new(title).color(title_color).size(13.0));
        ui.add_space(4.0);

        let key_bg = if self.engaged { KEY_INACTIVE_BG } else { KEY_IDLE_BG };
        let text_col = if self.engaged { KEY_TEXT_COLOR } else { KEY_TEXT_IDLE };
        let rows = [
            [("Q", self.state.strafe_l), ("W", self.state.forward), ("E", self.state.strafe_r)],
            [("A", self.state.left), ("S", self.state.backward), ("D", self.state.right)],
        ];
        for row in &rows {
            ui.horizontal(|ui| {
                ui.spacing_mut().item_spacing.x = KEY_GAP;
                for (label, pressed) in row {
                    let (rect, _) = ui.allocate_exact_size(egui::vec2(KEY_SIZE, KEY_SIZE), egui::Sense::hover());
                    let bg = if *pressed { KEY_ACTIVE_BG } else { key_bg };
                    ui.painter().rect_filled(rect, egui::CornerRadius::same(4), bg);
                    ui.painter().text(rect.center(), egui::Align2::CENTER_CENTER, label, egui::FontId::monospace(14.0), text_col);
                }
            });
        }
        let space_w = KEY_SIZE * 3.0 + KEY_GAP * 2.0;
        let (space_rect, _) = ui.allocate_exact_size(egui::vec2(space_w, KEY_SIZE * 0.7), egui::Sense::hover());
        let space_bg = if self.estop_flash { ESTOP_ACTIVE_BG } else { key_bg };
        ui.painter().rect_filled(space_rect, egui::CornerRadius::same(4), space_bg);
        ui.painter().text(space_rect.center(), egui::Align2::CENTER_CENTER, "STOP", egui::FontId::proportional(11.0), text_col);
        ui.add_space(4.0);
        let (speed_label, speed_color) = if self.state.fast {
            ("⇧ FAST", egui::Color32::from_rgb(255, 200, 50))
        } else { ("⇧ shift=fast", label_color) };
        ui.label(egui::RichText::new(speed_label).color(speed_color).size(10.0));
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

    fn publish_twist(&mut self) -> io::Result<()> {
        let (lx, ly, lz, ax, ay, az) = self.compute_twist();
        self.publisher.publish_twist(&twist_command([lx, ly, lz], [ax, ay, az])).map(|_| ())
    }

    fn publish_stop(&mut self) -> io::Result<()> {
        self.publisher.publish_twist(&twist_command([0.0; 3], [0.0; 3])).map(|_| ())
    }

    fn compute_twist(&self) -> (f64, f64, f64, f64, f64, f64) {
        let mut lx = 0.0;
        let mut ly = 0.0;
        let mut az = 0.0;
        if self.state.forward  { lx += BASE_LINEAR_SPEED; }
        if self.state.backward { lx -= BASE_LINEAR_SPEED; }
        if self.state.strafe_l { ly += BASE_LINEAR_SPEED; }
        if self.state.strafe_r { ly -= BASE_LINEAR_SPEED; }
        if self.state.left     { az += BASE_ANGULAR_SPEED; }
        if self.state.right    { az -= BASE_ANGULAR_SPEED; }
        if self.state.fast { lx *= FAST_MULTIPLIER; ly *= FAST_MULTIPLIER; az *= FAST_MULTIPLIER; }
        (lx, ly, 0.0, 0.0, 0.0, az)
    }
}

impl std::fmt::Debug for KeyboardHandler {
    fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        f.debug_struct("KeyboardHandler")
            .field("active_robot", &self.active_robot)
            .field("state", &self.state)
            .field("was_active", &self.was_active)
            .finish()
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn test_key_state_any_active() {
        let mut state = KeyState::new();
        assert!(!state.any_active());
        state.forward = true;
        assert!(state.any_active());
        state.reset();
        assert!(!state.any_active());
    }

    #[test]
    fn test_wasd_to_twist_mapping() {
        let h = KeyboardHandler {
            publisher: LcmPublisher::new("/test".to_string()).unwrap(),
            state: KeyState { forward: true, ..Default::default() },
            was_active: false, estop_flash: false, engaged: true, active_robot: None,
        };
        let (lx, ly, _, _, _, az) = h.compute_twist();
        assert_eq!(lx, BASE_LINEAR_SPEED);
        assert_eq!(ly, 0.0);
        assert_eq!(az, 0.0);
    }

    #[test]
    fn test_turn_left_right_mapping() {
        let h = KeyboardHandler {
            publisher: LcmPublisher::new("/test".to_string()).unwrap(),
            state: KeyState { left: true, ..Default::default() },
            was_active: false, estop_flash: false, engaged: true, active_robot: None,
        };
        assert_eq!(h.compute_twist().5, BASE_ANGULAR_SPEED);
        let h = KeyboardHandler {
            publisher: LcmPublisher::new("/test".to_string()).unwrap(),
            state: KeyState { right: true, ..Default::default() },
            was_active: false, estop_flash: false, engaged: true, active_robot: None,
        };
        assert_eq!(h.compute_twist().5, -BASE_ANGULAR_SPEED);
    }

    #[test]
    fn test_strafe_mapping() {
        let h = KeyboardHandler {
            publisher: LcmPublisher::new("/test".to_string()).unwrap(),
            state: KeyState { strafe_l: true, ..Default::default() },
            was_active: false, estop_flash: false, engaged: true, active_robot: None,
        };
        assert_eq!(h.compute_twist().1, BASE_LINEAR_SPEED);
    }

    #[test]
    fn test_shift_doubles_speed() {
        let h = KeyboardHandler {
            publisher: LcmPublisher::new("/test".to_string()).unwrap(),
            state: KeyState { forward: true, fast: true, ..Default::default() },
            was_active: false, estop_flash: false, engaged: true, active_robot: None,
        };
        assert_eq!(h.compute_twist().0, BASE_LINEAR_SPEED * FAST_MULTIPLIER);
    }

    #[test]
    fn test_simultaneous_keys() {
        let h = KeyboardHandler {
            publisher: LcmPublisher::new("/test".to_string()).unwrap(),
            state: KeyState { forward: true, left: true, ..Default::default() },
            was_active: false, estop_flash: false, engaged: true, active_robot: None,
        };
        let t = h.compute_twist();
        assert_eq!(t.0, BASE_LINEAR_SPEED);
        assert_eq!(t.5, BASE_ANGULAR_SPEED);
    }

    #[test]
    fn test_key_reset() {
        let mut s = KeyState { forward: true, left: true, fast: true, ..Default::default() };
        s.reset();
        assert!(!s.any_active());
        assert!(!s.fast);
    }

    #[test]
    fn test_keyboard_handler_creation() {
        let h = KeyboardHandler::new().unwrap();
        assert!(!h.was_active);
        assert!(!h.engaged);
        assert!(h.active_robot.is_none());
    }

    #[test]
    fn test_opposite_keys_cancel() {
        let h = KeyboardHandler {
            publisher: LcmPublisher::new("/test".to_string()).unwrap(),
            state: KeyState { forward: true, backward: true, ..Default::default() },
            was_active: false, estop_flash: false, engaged: true, active_robot: None,
        };
        assert_eq!(h.compute_twist().0, 0.0);
    }

    #[test]
    fn test_compute_twist_all_zeros() {
        let h = KeyboardHandler {
            publisher: LcmPublisher::new("/test".to_string()).unwrap(),
            state: KeyState::new(),
            was_active: false, estop_flash: false, engaged: true, active_robot: None,
        };
        let (lx, ly, lz, ax, ay, az) = h.compute_twist();
        assert!(lx == 0.0 && ly == 0.0 && lz == 0.0 && ax == 0.0 && ay == 0.0 && az == 0.0);
    }

    #[test]
    fn test_set_active_robot() {
        let mut h = KeyboardHandler::new().unwrap();
        assert!(h.active_robot().is_none());
        assert!(!h.engaged());
        h.set_active_robot(Some("/world/go2".to_string()));
        h.set_engaged(true);
        assert_eq!(h.active_robot(), Some("/world/go2"));
        assert!(h.engaged());
        h.set_engaged(false);
        assert!(!h.engaged());
        h.set_active_robot(None);
        assert!(h.active_robot().is_none());
    }
}
