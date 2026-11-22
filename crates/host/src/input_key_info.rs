//! `INPUT_KEY_INFO` — one key press and its modifiers.
//!
//! Ported from `inputKeyInfo.cpp` and the class declaration in `input.h`.
//! Selection and the console's key handling ask this object questions such as
//! "is this Ctrl+Left?" rather than re-deriving them from the record.
//!
//! `input.cpp`'s Win32 helpers (`GetControlKeyState`, `ClearKeyInfo`,
//! `IsSystemKey`, the menu/focus/ctrl handlers) decode window messages and
//! keyboard state; they belong to the input and window layers.

use types::ControlKeyState;
use types::virtual_key::{
    VK_CAPITAL, VK_CONTROL, VK_LWIN, VK_MENU, VK_NUMLOCK, VK_PAUSE, VK_RWIN, VK_SCROLL, VK_SHIFT,
};

/// A key press, as the console's selection and input code sees it.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub struct InputKeyInfo {
    virtual_key: u16,
    control_keys: ControlKeyState,
}

impl InputKeyInfo {
    /// Creates the description of a key press.
    pub const fn new(virtual_key: u16, control_keys: ControlKeyState) -> Self {
        Self {
            virtual_key,
            control_keys,
        }
    }

    /// Returns the virtual key that was pressed.
    ///
    /// This is the key itself, not the modifier, unless the modifier was the
    /// only key pressed.
    pub const fn virtual_key(&self) -> u16 {
        self.virtual_key
    }

    /// Returns the modifiers and locks held at the time.
    pub const fn control_keys(&self) -> ControlKeyState {
        self.control_keys
    }

    /// Returns whether either Ctrl key was held.
    pub const fn is_ctrl_pressed(&self) -> bool {
        self.control_keys.ctrl()
    }

    /// Returns whether either Alt key was held.
    pub const fn is_alt_pressed(&self) -> bool {
        self.control_keys.alt()
    }

    /// Returns whether Shift was held.
    pub const fn is_shift_pressed(&self) -> bool {
        self.control_keys.shift
    }

    /// Returns whether Ctrl was the only modifier.
    pub const fn is_ctrl_only(&self) -> bool {
        self.is_ctrl_pressed() && !self.is_alt_pressed() && !self.is_shift_pressed()
    }

    /// Returns whether Shift was the only modifier.
    pub const fn is_shift_only(&self) -> bool {
        !self.is_ctrl_pressed() && !self.is_alt_pressed() && self.is_shift_pressed()
    }

    /// Returns whether Shift and Ctrl were held together, without Alt.
    pub const fn is_shift_and_ctrl_only(&self) -> bool {
        self.is_ctrl_pressed() && !self.is_alt_pressed() && self.is_shift_pressed()
    }

    /// Returns whether Alt was the only modifier.
    pub const fn is_alt_only(&self) -> bool {
        self.is_alt_pressed() && !self.is_ctrl_pressed() && !self.is_shift_pressed()
    }

    /// Returns whether no modifier was held.
    pub const fn has_no_modifiers(&self) -> bool {
        !self.is_alt_pressed() && !self.is_ctrl_pressed() && !self.is_shift_pressed()
    }
}

/// Returns whether pressing `virtual_key` alone changes a system state.
///
/// Ported from `IsSystemKey` in `input.cpp`. A mouse selection is cancelled by
/// any key that is not one of these, so a modifier press or a lock toggle must
/// not end it.
pub const fn is_system_key(virtual_key: u16) -> bool {
    matches!(
        virtual_key,
        VK_SHIFT | VK_CONTROL | VK_MENU | VK_PAUSE | VK_CAPITAL | VK_LWIN | VK_RWIN | VK_NUMLOCK | VK_SCROLL
    )
}

#[cfg(test)]
mod tests {
    use super::*;

    fn info(control_keys: ControlKeyState) -> InputKeyInfo {
        InputKeyInfo::new(0x25, control_keys)
    }

    const NONE: ControlKeyState = ControlKeyState {
        left_alt: false,
        right_alt: false,
        left_ctrl: false,
        right_ctrl: false,
        shift: false,
        num_lock: false,
        scroll_lock: false,
        caps_lock: false,
        enhanced: false,
    };

    #[test]
    fn the_virtual_key_and_state_are_preserved() {
        let state = ControlKeyState {
            left_ctrl: true,
            ..NONE
        };
        let info = InputKeyInfo::new(0x26, state);
        assert_eq!(info.virtual_key(), 0x26);
        assert_eq!(info.control_keys(), state);
    }

    #[test]
    fn with_no_modifiers_only_has_no_modifiers_is_true() {
        let info = info(NONE);
        assert!(!info.is_ctrl_pressed());
        assert!(!info.is_alt_pressed());
        assert!(!info.is_shift_pressed());
        assert!(!info.is_ctrl_only());
        assert!(!info.is_shift_only());
        assert!(!info.is_shift_and_ctrl_only());
        assert!(!info.is_alt_only());
        assert!(info.has_no_modifiers());
    }

    #[test]
    fn ctrl_alone_is_ctrl_only() {
        let info = info(ControlKeyState {
            left_ctrl: true,
            ..NONE
        });
        assert!(info.is_ctrl_pressed());
        assert!(info.is_ctrl_only());
        assert!(!info.is_shift_only());
        assert!(!info.is_alt_only());
        assert!(!info.has_no_modifiers());
    }

    #[test]
    fn either_ctrl_key_counts() {
        let info = info(ControlKeyState {
            right_ctrl: true,
            ..NONE
        });
        assert!(info.is_ctrl_pressed());
        assert!(info.is_ctrl_only());
    }

    #[test]
    fn shift_alone_is_shift_only() {
        let info = info(ControlKeyState { shift: true, ..NONE });
        assert!(info.is_shift_pressed());
        assert!(info.is_shift_only());
        assert!(!info.is_ctrl_only());
        assert!(!info.is_alt_only());
    }

    #[test]
    fn alt_alone_is_alt_only() {
        let info = info(ControlKeyState {
            right_alt: true,
            ..NONE
        });
        assert!(info.is_alt_pressed());
        assert!(info.is_alt_only());
        assert!(!info.is_ctrl_only());
        assert!(!info.is_shift_only());
    }

    #[test]
    fn ctrl_and_shift_together_are_shift_and_ctrl_only() {
        let info = info(ControlKeyState {
            left_ctrl: true,
            shift: true,
            ..NONE
        });
        assert!(info.is_shift_and_ctrl_only());
        // Each of the single-modifier predicates is false when another is held.
        assert!(!info.is_ctrl_only());
        assert!(!info.is_shift_only());
        assert!(!info.is_alt_only());
    }

    #[test]
    fn adding_alt_clears_every_only_predicate() {
        let info = info(ControlKeyState {
            left_ctrl: true,
            shift: true,
            left_alt: true,
            ..NONE
        });
        assert!(info.is_ctrl_pressed());
        assert!(info.is_shift_pressed());
        assert!(info.is_alt_pressed());
        assert!(!info.is_ctrl_only());
        assert!(!info.is_shift_only());
        assert!(!info.is_shift_and_ctrl_only());
        assert!(!info.is_alt_only());
        assert!(!info.has_no_modifiers());
    }

    #[test]
    fn locks_are_not_modifiers() {
        let info = info(ControlKeyState {
            caps_lock: true,
            num_lock: true,
            scroll_lock: true,
            ..NONE
        });
        assert!(info.has_no_modifiers());
    }

    #[test]
    fn the_modifiers_and_locks_are_system_keys() {
        for virtual_key in [
            VK_SHIFT, VK_CONTROL, VK_MENU, VK_PAUSE, VK_CAPITAL, VK_LWIN, VK_RWIN, VK_NUMLOCK, VK_SCROLL,
        ] {
            assert!(
                is_system_key(virtual_key),
                "expected {virtual_key:#x} to be a system key"
            );
        }

        assert!(!is_system_key(types::virtual_key::VK_ESCAPE));
        assert!(!is_system_key(u16::from(b'a')));
    }
}
