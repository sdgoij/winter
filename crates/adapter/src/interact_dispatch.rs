//! `IInteractDispatch` — Input state machine callback interface.
//!
//! Ported from `IInteractDispatch.hpp` in the Windows Terminal C++ codebase.
//!
//! The records it writes are [`InputRecord`], the portable form of upstream's
//! `INPUT_RECORD`; they live in `types` because the host's input buffer stores
//! the same vocabulary (ADR 002, ADR 010).

use types::{DeviceAttributes, InputRecord};

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

    /// Records the DA1 device attributes a terminal reported.
    ///
    /// Only the input engine asks for this: it is the answer to the DA1
    /// request a ConPTY-style host sends while starting up.
    fn set_device_attributes(&self, _attributes: DeviceAttributes) {}
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::dispatch_types::{VTInt, WindowManipulationType};
    use types::{ControlKeyState, KeyEventRecord};

    /// A dispatcher that records what it was given.
    #[derive(Debug, Default)]
    struct Recorder {
        inputs: Vec<InputRecord>,
    }

    impl IInteractDispatch for Recorder {
        fn is_vt_input_enabled(&self) -> bool {
            true
        }

        fn write_input(&self, _input_events: &[InputRecord]) {}

        fn write_ctrl_key(&self, _event: &InputRecord) {}

        fn write_string(&self, _string: &str) {}

        fn write_string_raw(&self, _string: &str) {}

        fn window_manipulation(&self, _function: WindowManipulationType, _parameter1: VTInt, _parameter2: VTInt) {}

        fn move_cursor(&self, _row: VTInt, _col: VTInt) {}

        fn focus_changed(&self, _focused: bool) {}
    }

    #[test]
    fn a_key_record_is_the_shared_vocabulary() {
        let record = InputRecord::Key(KeyEventRecord {
            control_keys: ControlKeyState {
                left_ctrl: true,
                ..ControlKeyState::default()
            },
            ..KeyEventRecord::pressed('c')
        });

        assert!(record.is_key());
        assert_eq!(record.key().map(|key| key.character), Some('c'));
        assert_eq!(
            record.key().map(|key| key.control_keys.to_console_flags()),
            Some(0x0008)
        );
    }

    #[test]
    fn the_trait_is_object_safe() {
        let recorder = Recorder::default();
        let dispatch: &dyn IInteractDispatch = &recorder;
        assert!(dispatch.is_vt_input_enabled());
        assert!(recorder.inputs.is_empty());
    }
}
