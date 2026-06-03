//! `IInteractDispatch` — Input state machine callback interface.
//!
//! Ported from `IInteractDispatch.hpp` in the Windows Terminal C++ codebase.

/// Interface for input state machine callbacks.
/// When input events occur, they are dispatched to methods on this interface.
pub trait IInteractDispatch: std::fmt::Debug {
    /// Returns whether VT input is enabled.
    fn is_vt_input_enabled(&self) -> bool;

    /// Writes input events to the output.
    fn write_input(&self, input_events: &[InputRecord]);

    /// Writes a control key event.
    fn write_ctrl_key(&self, event: &InputRecord);

    /// Writes a string with translation.
    fn write_string(&self, string: &str);

    /// Writes a raw string without translation.
    fn write_string_raw(&self, string: &str);

    /// Performs window manipulation.
    fn window_manipulation(
        &self,
        function: super::dispatch_types::WindowManipulationType,
        parameter1: super::dispatch_types::VTInt,
        parameter2: super::dispatch_types::VTInt,
    );

    /// Moves the cursor to the specified position.
    fn move_cursor(&self, row: super::dispatch_types::VTInt, col: super::dispatch_types::VTInt);

    /// Reports focus change.
    fn focus_changed(&self, focused: bool);
}

/// Input record for keyboard/mouse events.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct InputRecord {
    /// Event type (keyboard, mouse, etc.).
    pub event_type: InputEventType,
    /// Virtual key code.
    pub virtual_key: u16,
    /// Virtual scan code.
    pub virtual_scan_code: u16,
    /// Unicode character.
    pub unicode_char: char,
    /// Control key state.
    pub control_keys: ControlKeyState,
    /// Window handle (platform-specific).
    pub window_handle: usize,
    /// Screen buffer dimensions.
    pub screen_buffer_dimensions: (i32, i32),
}

/// Input event type.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum InputEventType {
    /// Keyboard event.
    Keyboard,
    /// Mouse event.
    Mouse,
    /// Mouse header event.
    MouseHeader,
    /// Focus event.
    Focus,
    /// Window events.
    Window,
    /// Menu event.
    Menu,
    /// Set event.
    Set,
}

/// Control key state for keyboard events.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
#[derive(Default)]
pub struct ControlKeyState {
    /// Left or right ctrl key is pressed.
    pub left_ctrl_pressed: bool,
    /// Left or right shift key is pressed.
    pub left_shift_pressed: bool,
    /// Right or left ctrl key is pressed.
    pub right_ctrl_pressed: bool,
    /// Right or left shift key is pressed.
    pub right_shift_pressed: bool,
    /// Scroll lock is active.
    pub scroll_lock: bool,
    /// Num lock is active.
    pub num_lock: bool,
    /// Caps lock is active.
    pub caps_lock: bool,
    /// Extended key flag.
    pub extended_key: bool,
    /// Alt key is pressed.
    pub alt_pressed: bool,
    /// Control key is pressed.
    pub control_pressed: bool,
    /// Shift key is pressed.
    pub shift_pressed: bool,
}


#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn test_input_record_default() {
        let record = InputRecord {
            event_type: InputEventType::Keyboard,
            virtual_key: 0,
            virtual_scan_code: 0,
            unicode_char: '\0',
            control_keys: ControlKeyState::default(),
            window_handle: 0,
            screen_buffer_dimensions: (0, 0),
        };
        assert_eq!(record.event_type, InputEventType::Keyboard);
        assert!(!record.control_keys.control_pressed);
    }

    #[test]
    fn test_control_key_state() {
        let state = ControlKeyState::default();
        assert!(!state.left_ctrl_pressed);
        assert!(!state.shift_pressed);
        assert!(!state.control_pressed);
    }
}
