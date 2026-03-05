//! Keyboard handler for WASD movement controls that publish TwistStamped messages.
//! 
//! Converts keyboard input to robot velocity commands following teleop conventions:
//! - WASD/arrows for linear/angular motion
//! - QE for strafing
//! - Space for emergency stop
//! - Shift for speed multiplier

use std::io;
use super::lcm::{LcmPublisher, twist_command_now};
use rerun::external::{egui, re_log};

/// LCM channel for twist commands (follows ROS convention)
const CMD_VEL_CHANNEL: &str = "/cmd_vel#geometry_msgs.TwistStamped";

/// Base speeds for keyboard control
const BASE_LINEAR_SPEED: f64 = 0.5;   // m/s
const BASE_ANGULAR_SPEED: f64 = 0.5;  // rad/s
const FAST_MULTIPLIER: f64 = 2.0;     // Shift modifier

/// Tracks which movement keys are currently held down.
#[derive(Debug, Clone, Default)]
struct KeyState {
    forward: bool,   // W or ↑
    backward: bool,  // S or ↓
    left: bool,      // A or ←
    right: bool,     // D or →
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

/// Handles keyboard input and publishes TwistStamped via LCM.
pub struct KeyboardHandler {
    publisher: LcmPublisher,
    state: KeyState,
    was_active: bool,  // true if any key was held last frame (for zero-on-release)
}

impl KeyboardHandler {
    /// Create a new keyboard handler with LCM publisher on CMD_VEL_CHANNEL.
    pub fn new() -> Result<Self, io::Error> {
        let publisher = LcmPublisher::new(CMD_VEL_CHANNEL.to_string())?;
        Ok(Self {
            publisher,
            state: KeyState::new(),
            was_active: false,
        })
    }

    /// Process keyboard input from egui and publish TwistStamped if keys are held.
    /// Called once per frame from DimosApp.ui().
    ///
    /// Returns true if any movement key is active (for UI overlay).
    pub fn process(&mut self, ctx: &egui::Context) -> bool {
        // Check if any text widget has focus - if so, skip keyboard capture
        let text_has_focus = ctx.memory(|m| m.focused().is_some());
        if text_has_focus {
            // If we were active but now text has focus, send stop command
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
            return false;
        }

        // Publish twist command if keys are active, or stop if just released
        if self.state.any_active() {
            if let Err(e) = self.publish_twist() {
                re_log::warn!("Failed to publish twist command: {e:?}");
            }
            self.was_active = true;
        } else if self.was_active {
            // Keys were active last frame but not now - send stop
            if let Err(e) = self.publish_stop() {
                re_log::warn!("Failed to send stop on key release: {e:?}");
            }
            self.was_active = false;
        }

        self.state.any_active()
    }

    /// Read current key state from egui input, update self.state.
    /// Only reads WASD/arrow/QE/Space/Shift — does NOT consume events
    /// so Rerun's own key handling still works.
    fn update_key_state(&mut self, ctx: &egui::Context) {
        ctx.input(|i| {
            // Movement keys (held down)
            self.state.forward = i.key_down(egui::Key::W) || i.key_down(egui::Key::ArrowUp);
            self.state.backward = i.key_down(egui::Key::S) || i.key_down(egui::Key::ArrowDown);
            self.state.left = i.key_down(egui::Key::A) || i.key_down(egui::Key::ArrowLeft);
            self.state.right = i.key_down(egui::Key::D) || i.key_down(egui::Key::ArrowRight);
            self.state.strafe_l = i.key_down(egui::Key::Q);
            self.state.strafe_r = i.key_down(egui::Key::E);
            
            // Speed modifier
            self.state.fast = i.modifiers.shift;
        });
    }

    /// Convert current KeyState → TwistCommand and publish via LCM.
    /// Publishes motion when keys are held.
    fn publish_twist(&mut self) -> io::Result<()> {
        let (lin_x, lin_y, lin_z, ang_x, ang_y, ang_z) = self.compute_twist();
        
        let cmd = twist_command_now(
            [lin_x, lin_y, lin_z],
            [ang_x, ang_y, ang_z],
            "base_link"
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
        let cmd = twist_command_now([0.0, 0.0, 0.0], [0.0, 0.0, 0.0], "base_link");
        self.publisher.publish_twist(&cmd)?;
        re_log::debug!("Published stop command");
        Ok(())
    }

    /// Map KeyState to linear/angular velocities.
    /// Applies FAST_MULTIPLIER when shift is held.
    fn compute_twist(&self) -> (f64, f64, f64, f64, f64, f64) {
        let mut linear_x = 0.0;
        let mut linear_y = 0.0;
        let mut angular_z = 0.0;

        // Forward/backward (W/S)
        if self.state.forward {
            linear_x += BASE_LINEAR_SPEED;
        }
        if self.state.backward {
            linear_x -= BASE_LINEAR_SPEED;
        }

        // Strafe left/right (Q/E)
        if self.state.strafe_l {
            linear_y += BASE_LINEAR_SPEED;
        }
        if self.state.strafe_r {
            linear_y -= BASE_LINEAR_SPEED;
        }

        // Turn left/right (A/D)
        if self.state.left {
            angular_z += BASE_ANGULAR_SPEED;
        }
        if self.state.right {
            angular_z -= BASE_ANGULAR_SPEED;
        }

        // Apply speed multiplier if Shift held
        if self.state.fast {
            linear_x *= FAST_MULTIPLIER;
            linear_y *= FAST_MULTIPLIER;
            angular_z *= FAST_MULTIPLIER;
        }

        // Return all 6 DOF (only x,y linear and z angular are used for ground robots)
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
        // Test forward (W)
        let mut state = KeyState::new();
        state.forward = true;
        let handler = KeyboardHandler {
            publisher: LcmPublisher::new("/test".to_string()).unwrap(),
            state,
            was_active: false,
        };
        let (lin_x, lin_y, _, _, _, ang_z) = handler.compute_twist();
        assert_eq!(lin_x, BASE_LINEAR_SPEED);
        assert_eq!(lin_y, 0.0);
        assert_eq!(ang_z, 0.0);
    }

    #[test]
    fn test_turn_left_right_mapping() {
        // Test turn left (A)
        let mut state = KeyState::new();
        state.left = true;
        let handler = KeyboardHandler {
            publisher: LcmPublisher::new("/test".to_string()).unwrap(),
            state,
            was_active: false,
        };
        let (lin_x, lin_y, _, _, _, ang_z) = handler.compute_twist();
        assert_eq!(lin_x, 0.0);
        assert_eq!(lin_y, 0.0);
        assert_eq!(ang_z, BASE_ANGULAR_SPEED);

        // Test turn right (D)
        let mut state = KeyState::new();
        state.right = true;
        let handler = KeyboardHandler {
            publisher: LcmPublisher::new("/test".to_string()).unwrap(),
            state,
            was_active: false,
        };
        let (lin_x, lin_y, _, _, _, ang_z) = handler.compute_twist();
        assert_eq!(lin_x, 0.0);
        assert_eq!(lin_y, 0.0);
        assert_eq!(ang_z, -BASE_ANGULAR_SPEED);
    }

    #[test]
    fn test_strafe_mapping() {
        // Test strafe left (Q)
        let mut state = KeyState::new();
        state.strafe_l = true;
        let handler = KeyboardHandler {
            publisher: LcmPublisher::new("/test".to_string()).unwrap(),
            state,
            was_active: false,
        };
        let (lin_x, lin_y, _, _, _, ang_z) = handler.compute_twist();
        assert_eq!(lin_x, 0.0);
        assert_eq!(lin_y, BASE_LINEAR_SPEED);
        assert_eq!(ang_z, 0.0);

        // Test strafe right (E)
        let mut state = KeyState::new();
        state.strafe_r = true;
        let handler = KeyboardHandler {
            publisher: LcmPublisher::new("/test".to_string()).unwrap(),
            state,
            was_active: false,
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
        state.fast = true;  // Shift held
        let handler = KeyboardHandler {
            publisher: LcmPublisher::new("/test".to_string()).unwrap(),
            state,
            was_active: false,
        };
        let (lin_x, lin_y, _, _, _, ang_z) = handler.compute_twist();
        assert_eq!(lin_x, BASE_LINEAR_SPEED * FAST_MULTIPLIER);
        assert_eq!(lin_y, 0.0);
        assert_eq!(ang_z, 0.0);
    }

    #[test]
    fn test_simultaneous_keys() {
        // Test forward + turn left (W + A)
        let mut state = KeyState::new();
        state.forward = true;
        state.left = true;
        let handler = KeyboardHandler {
            publisher: LcmPublisher::new("/test".to_string()).unwrap(),
            state,
            was_active: false,
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
        // Test forward + backward should cancel out
        let mut state = KeyState::new();
        state.forward = true;
        state.backward = true;
        let handler = KeyboardHandler {
            publisher: LcmPublisher::new("/test".to_string()).unwrap(),
            state,
            was_active: false,
        };
        let (lin_x, lin_y, _, _, _, ang_z) = handler.compute_twist();
        assert_eq!(lin_x, 0.0);  // Should cancel out
        assert_eq!(lin_y, 0.0);
        assert_eq!(ang_z, 0.0);
    }

    #[test]
    fn test_compute_twist_all_zeros() {
        let handler = KeyboardHandler {
            publisher: LcmPublisher::new("/test".to_string()).unwrap(),
            state: KeyState::new(),
            was_active: false,
        };
        let (lin_x, lin_y, lin_z, ang_x, ang_y, ang_z) = handler.compute_twist();
        
        // All velocities should be zero with no keys pressed
        assert_eq!(lin_x, 0.0);
        assert_eq!(lin_y, 0.0);
        assert_eq!(lin_z, 0.0);
        assert_eq!(ang_x, 0.0);
        assert_eq!(ang_y, 0.0);
        assert_eq!(ang_z, 0.0);
    }
}
