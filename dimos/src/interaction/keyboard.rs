//! Keyboard handler for WASD movement controls that publish Twist messages.
//! 
//! Converts keyboard input to robot velocity commands following teleop conventions:
//! - WASD/arrows for linear/angular motion
//! - QE for strafing
//! - Space for emergency stop
//! - Shift for speed multiplier

use std::io;
use super::lcm::{LcmPublisher, twist_command};
use rerun::external::{egui, re_log};

/// LCM channel for twist commands (matches DimOS convention)
const CMD_VEL_CHANNEL: &str = "/cmd_vel#geometry_msgs.Twist";

/// Base speeds for keyboard control
const BASE_LINEAR_SPEED: f64 = 0.5;   // m/s
const BASE_ANGULAR_SPEED: f64 = 0.8;  // rad/s
const FAST_MULTIPLIER: f64 = 2.0;     // Shift modifier

/// Overlay styling
const OVERLAY_MARGIN: f32 = 12.0;
const OVERLAY_PADDING: f32 = 10.0;
const OVERLAY_ROUNDING: f32 = 8.0;
const OVERLAY_BG: egui::Color32 = egui::Color32::from_rgba_premultiplied(20, 20, 30, 220);
const KEY_SIZE: f32 = 32.0;
const KEY_GAP: f32 = 3.0;
const KEY_ACTIVE_BG: egui::Color32 = egui::Color32::from_rgb(60, 180, 75);
const KEY_INACTIVE_BG: egui::Color32 = egui::Color32::from_rgba_premultiplied(60, 60, 80, 180);
const KEY_TEXT_COLOR: egui::Color32 = egui::Color32::WHITE;
const LABEL_COLOR: egui::Color32 = egui::Color32::from_rgb(180, 180, 200);
const ESTOP_ACTIVE_BG: egui::Color32 = egui::Color32::from_rgb(220, 50, 50);

/// Tracks which movement keys are currently held down.
#[derive(Debug, Clone, Default)]
struct KeyState {
    forward: bool,   // W or Up
    backward: bool,  // S or Down
    left: bool,      // A or Left
    right: bool,     // D or Right
    strafe_l: bool,  // Q
    strafe_r: bool,  // E
    fast: bool,      // Shift held
}

impl KeyState {
    fn new() -> Self {
        Default::default()
    }

    /// Returns true if any movement key is currently active
    fn any_active(&self) -> bool {
        self.forward || self.backward || self.left || self.right || self.strafe_l || self.strafe_r
    }

    /// Reset all key states (used for emergency stop)
    fn reset(&mut self) {
        self.forward = false;
        self.backward = false;
        self.left = false;
        self.right = false;
        self.strafe_l = false;
        self.strafe_r = false;
        self.fast = false;
    }
}

/// Handles keyboard input and publishes Twist via LCM.
pub struct KeyboardHandler {
    publisher: LcmPublisher,
    state: KeyState,
    was_active: bool,
    estop_flash: bool,  // true briefly after space pressed
}

impl KeyboardHandler {
    /// Create a new keyboard handler with LCM publisher on CMD_VEL_CHANNEL.
    pub fn new() -> Result<Self, io::Error> {
        let publisher = LcmPublisher::new(CMD_VEL_CHANNEL.to_string())?;
        Ok(Self {
            publisher,
            state: KeyState::new(),
            was_active: false,
            estop_flash: false,
        })
    }

    /// Process keyboard input from egui and publish Twist if keys are held.
    /// Called once per frame from DimosApp.ui().
    ///
    /// Returns true if any movement key is active (for UI overlay).
    pub fn process(&mut self, ctx: &egui::Context) -> bool {
        self.estop_flash = false;

        // Check if any text widget has focus - if so, skip keyboard capture
        let text_has_focus = ctx.memory(|m| m.focused().is_some());
        if text_has_focus {
            if self.was_active {
                if let Err(e) = self.publish_stop() {
                    re_log::warn!("Failed to send stop command on focus change: {e:?}");
                }
                self.was_active = false;
            }
            return false;
        }

        // Update key state from egui input
        self.update_key_state(ctx);

        // Check for emergency stop (Space key pressed - one-shot action)
        if ctx.input(|i| i.key_pressed(egui::Key::Space)) {
            self.state.reset();
            if let Err(e) = self.publish_stop() {
                re_log::warn!("Failed to send emergency stop: {e:?}");
            }
            self.was_active = false;
            self.estop_flash = true;
            return true; // return true so overlay shows the e-stop flash
        }

        // Publish twist command if keys are active, or stop if just released
        if self.state.any_active() {
            if let Err(e) = self.publish_twist() {
                re_log::warn!("Failed to publish twist command: {e:?}");
            }
            self.was_active = true;
        } else if self.was_active {
            if let Err(e) = self.publish_stop() {
                re_log::warn!("Failed to send stop on key release: {e:?}");
            }
            self.was_active = false;
        }

        self.state.any_active()
    }

    /// Draw keyboard overlay HUD. Always shown (dim when idle, bright when active).
    pub fn draw_overlay(&self, ctx: &egui::Context) {
        egui::Area::new("keyboard_hud".into())
            .fixed_pos(egui::pos2(OVERLAY_MARGIN, OVERLAY_MARGIN))
            .order(egui::Order::Foreground)
            .interactable(false)
            .show(ctx, |ui| {
                egui::Frame::new()
                    .fill(OVERLAY_BG)
                    .corner_radius(egui::CornerRadius::same(OVERLAY_ROUNDING as u8))
                    .inner_margin(egui::Margin::same(OVERLAY_PADDING as i8))
                    .show(ui, |ui| {
                        self.draw_hud_content(ui);
                    });
            });
    }

    fn draw_hud_content(&self, ui: &mut egui::Ui) {
        let active = self.state.any_active() || self.estop_flash;

        // Title
        let title_color = if active {
            egui::Color32::WHITE
        } else {
            egui::Color32::from_rgb(120, 120, 140)
        };
        ui.label(egui::RichText::new("🎮 Keyboard Teleop").color(title_color).size(13.0));
        ui.add_space(4.0);

        // Key grid:  [Q] [W] [E]
        //            [A] [S] [D]
        //            [  SPACE  ]
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

        // Row 1
        ui.horizontal(|ui| {
            ui.spacing_mut().item_spacing.x = KEY_GAP;
            for (label, pressed) in &row1 {
                self.draw_key(ui, label, *pressed);
            }
        });

        // Row 2
        ui.horizontal(|ui| {
            ui.spacing_mut().item_spacing.x = KEY_GAP;
            for (label, pressed) in &row2 {
                self.draw_key(ui, label, *pressed);
            }
        });

        // Space bar (e-stop)
        let space_width = KEY_SIZE * 3.0 + KEY_GAP * 2.0;
        let space_rect = ui.allocate_exact_size(
            egui::vec2(space_width, KEY_SIZE * 0.7),
            egui::Sense::hover(),
        ).0;
        let space_bg = if self.estop_flash {
            ESTOP_ACTIVE_BG
        } else {
            KEY_INACTIVE_BG
        };
        ui.painter().rect_filled(space_rect, egui::CornerRadius::same(4), space_bg);
        ui.painter().text(
            space_rect.center(),
            egui::Align2::CENTER_CENTER,
            "STOP",
            egui::FontId::proportional(11.0),
            KEY_TEXT_COLOR,
        );

        ui.add_space(4.0);

        // Speed indicator
        let speed_label = if self.state.fast { "⇧ FAST" } else { "⇧ shift=fast" };
        let speed_color = if self.state.fast {
            egui::Color32::from_rgb(255, 200, 50)
        } else {
            LABEL_COLOR
        };
        ui.label(egui::RichText::new(speed_label).color(speed_color).size(10.0));
    }

    fn draw_key(&self, ui: &mut egui::Ui, label: &str, pressed: bool) {
        let (rect, _) = ui.allocate_exact_size(
            egui::vec2(KEY_SIZE, KEY_SIZE),
            egui::Sense::hover(),
        );
        let bg = if pressed { KEY_ACTIVE_BG } else { KEY_INACTIVE_BG };
        ui.painter().rect_filled(rect, egui::CornerRadius::same(4), bg);
        ui.painter().text(
            rect.center(),
            egui::Align2::CENTER_CENTER,
            label,
            egui::FontId::monospace(14.0),
            KEY_TEXT_COLOR,
        );
    }

    /// Read current key state from egui input, update self.state.
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

    /// Convert current KeyState to Twist and publish via LCM.
    fn publish_twist(&mut self) -> io::Result<()> {
        let (lin_x, lin_y, lin_z, ang_x, ang_y, ang_z) = self.compute_twist();

        let cmd = twist_command(
            [lin_x, lin_y, lin_z],
            [ang_x, ang_y, ang_z],
        );

        self.publisher.publish_twist(&cmd)?;

        re_log::trace!(
            "Published twist: lin=({:.2},{:.2},{:.2}) ang=({:.2},{:.2},{:.2})",
            lin_x, lin_y, lin_z, ang_x, ang_y, ang_z
        );

        Ok(())
    }

    /// Publish all-zero twist (stop command)
    fn publish_stop(&mut self) -> io::Result<()> {
        let cmd = twist_command([0.0, 0.0, 0.0], [0.0, 0.0, 0.0]);
        self.publisher.publish_twist(&cmd)?;
        re_log::debug!("Published stop command");
        Ok(())
    }

    /// Map KeyState to linear/angular velocities.
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

        state.strafe_l = true;
        assert!(state.any_active());
    }

    #[test]
    fn test_wasd_to_twist_mapping() {
        let mut state = KeyState::new();
        state.forward = true;
        let handler = KeyboardHandler {
            publisher: LcmPublisher::new("/test".to_string()).unwrap(),
            state,
            was_active: false,
            estop_flash: false,
        };
        let (lin_x, lin_y, _, _, _, ang_z) = handler.compute_twist();
        assert_eq!(lin_x, BASE_LINEAR_SPEED);
        assert_eq!(lin_y, 0.0);
        assert_eq!(ang_z, 0.0);
    }

    #[test]
    fn test_turn_left_right_mapping() {
        let mut state = KeyState::new();
        state.left = true;
        let handler = KeyboardHandler {
            publisher: LcmPublisher::new("/test".to_string()).unwrap(),
            state,
            was_active: false,
            estop_flash: false,
        };
        let (lin_x, lin_y, _, _, _, ang_z) = handler.compute_twist();
        assert_eq!(lin_x, 0.0);
        assert_eq!(lin_y, 0.0);
        assert_eq!(ang_z, BASE_ANGULAR_SPEED);

        let mut state = KeyState::new();
        state.right = true;
        let handler = KeyboardHandler {
            publisher: LcmPublisher::new("/test".to_string()).unwrap(),
            state,
            was_active: false,
            estop_flash: false,
        };
        let (lin_x, lin_y, _, _, _, ang_z) = handler.compute_twist();
        assert_eq!(lin_x, 0.0);
        assert_eq!(lin_y, 0.0);
        assert_eq!(ang_z, -BASE_ANGULAR_SPEED);
    }

    #[test]
    fn test_strafe_mapping() {
        let mut state = KeyState::new();
        state.strafe_l = true;
        let handler = KeyboardHandler {
            publisher: LcmPublisher::new("/test".to_string()).unwrap(),
            state,
            was_active: false,
            estop_flash: false,
        };
        let (lin_x, lin_y, _, _, _, ang_z) = handler.compute_twist();
        assert_eq!(lin_x, 0.0);
        assert_eq!(lin_y, BASE_LINEAR_SPEED);
        assert_eq!(ang_z, 0.0);

        let mut state = KeyState::new();
        state.strafe_r = true;
        let handler = KeyboardHandler {
            publisher: LcmPublisher::new("/test".to_string()).unwrap(),
            state,
            was_active: false,
            estop_flash: false,
        };
        let (lin_x, lin_y, _, _, _, ang_z) = handler.compute_twist();
        assert_eq!(lin_x, 0.0);
        assert_eq!(lin_y, -BASE_LINEAR_SPEED);
        assert_eq!(ang_z, 0.0);
    }

    #[test]
    fn test_shift_doubles_speed() {
        let mut state = KeyState::new();
        state.forward = true;
        state.fast = true;
        let handler = KeyboardHandler {
            publisher: LcmPublisher::new("/test".to_string()).unwrap(),
            state,
            was_active: false,
            estop_flash: false,
        };
        let (lin_x, lin_y, _, _, _, ang_z) = handler.compute_twist();
        assert_eq!(lin_x, BASE_LINEAR_SPEED * FAST_MULTIPLIER);
        assert_eq!(lin_y, 0.0);
        assert_eq!(ang_z, 0.0);
    }

    #[test]
    fn test_simultaneous_keys() {
        let mut state = KeyState::new();
        state.forward = true;
        state.left = true;
        let handler = KeyboardHandler {
            publisher: LcmPublisher::new("/test".to_string()).unwrap(),
            state,
            was_active: false,
            estop_flash: false,
        };
        let (lin_x, lin_y, _, _, _, ang_z) = handler.compute_twist();
        assert_eq!(lin_x, BASE_LINEAR_SPEED);
        assert_eq!(lin_y, 0.0);
        assert_eq!(ang_z, BASE_ANGULAR_SPEED);
    }

    #[test]
    fn test_key_reset() {
        let mut state = KeyState::new();
        state.forward = true;
        state.left = true;
        state.fast = true;
        assert!(state.any_active());
        state.reset();
        assert!(!state.forward);
        assert!(!state.left);
        assert!(!state.fast);
        assert!(!state.any_active());
    }

    #[test]
    fn test_keyboard_handler_creation() {
        let handler = KeyboardHandler::new();
        assert!(handler.is_ok());
        let handler = handler.unwrap();
        assert!(!handler.was_active);
        assert!(!handler.state.any_active());
    }

    #[test]
    fn test_opposite_keys_cancel() {
        let mut state = KeyState::new();
        state.forward = true;
        state.backward = true;
        let handler = KeyboardHandler {
            publisher: LcmPublisher::new("/test".to_string()).unwrap(),
            state,
            was_active: false,
            estop_flash: false,
        };
        let (lin_x, lin_y, _, _, _, ang_z) = handler.compute_twist();
        assert_eq!(lin_x, 0.0);
        assert_eq!(lin_y, 0.0);
        assert_eq!(ang_z, 0.0);
    }

    #[test]
    fn test_compute_twist_all_zeros() {
        let handler = KeyboardHandler {
            publisher: LcmPublisher::new("/test".to_string()).unwrap(),
            state: KeyState::new(),
            was_active: false,
            estop_flash: false,
        };
        let (lin_x, lin_y, lin_z, ang_x, ang_y, ang_z) = handler.compute_twist();
        assert_eq!(lin_x, 0.0);
        assert_eq!(lin_y, 0.0);
        assert_eq!(lin_z, 0.0);
        assert_eq!(ang_x, 0.0);
        assert_eq!(ang_y, 0.0);
        assert_eq!(ang_z, 0.0);
    }
}
