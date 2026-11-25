//! `InputModes` — the snapshot of input-affecting modes.
//!
//! Upstream's `TerminalInput` owns these flags and the adapter writes into it
//! through a reference. Here the adapter is the single home for terminal state
//! (ADR 002), so the input engine receives a copy for each event (ADR 009).
//!
//! The default is upstream's initial set: ANSI mode, auto-repeat and alternate
//! scroll are enabled, everything else is not.

/// The input-affecting modes, as a snapshot of the terminal's state.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub struct InputModes {
    /// LNM — a line feed also returns the carriage.
    pub line_feed: bool,
    /// DECANM — ANSI mode rather than VT52.
    pub ansi: bool,
    /// DECARM — keys repeat while held.
    pub auto_repeat: bool,
    /// DECNKM — the keypad sends application sequences.
    pub keypad: bool,
    /// DECCKM — the cursor keys send application sequences.
    pub cursor_key: bool,
    /// DECBKM — Backspace sends DEL rather than BS.
    pub backarrow_key: bool,
    /// Mode 9001 — send Win32 input records to the shell.
    pub win32_input: bool,
    /// S8C1T — send eight-bit control codes.
    pub send_c1: bool,
    /// Mode 1005 — UTF-8 mouse coordinates.
    pub utf8_mouse: bool,
    /// Mode 1006 — SGR mouse coordinates.
    pub sgr_mouse: bool,
    /// Mode 1000 — report button presses and releases.
    pub default_mouse: bool,
    /// Mode 1002 — report motion while a button is held.
    pub button_event_mouse: bool,
    /// Mode 1003 — report all motion.
    pub any_event_mouse: bool,
    /// Mode 1004 — report focus changes.
    pub focus_event: bool,
    /// Mode 1007 — the wheel moves the cursor on the alternate screen.
    pub alternate_scroll: bool,
    /// The alternate screen buffer is active.
    pub in_alternate_buffer: bool,
    /// The kitty keyboard protocol flags in effect.
    pub kitty_flags: u8,
}

impl Default for InputModes {
    /// Upstream's `_inputMode` starts as `{ Ansi, AutoRepeat, AlternateScroll }`.
    fn default() -> Self {
        Self {
            line_feed: false,
            ansi: true,
            auto_repeat: true,
            keypad: false,
            cursor_key: false,
            backarrow_key: false,
            win32_input: false,
            send_c1: false,
            utf8_mouse: false,
            sgr_mouse: false,
            default_mouse: false,
            button_event_mouse: false,
            any_event_mouse: false,
            focus_event: false,
            alternate_scroll: true,
            in_alternate_buffer: false,
            kitty_flags: 0,
        }
    }
}

/// `KittyKeyboardProtocolFlags::ReportAllKeysAsEscapeCodes`.
const KITTY_REPORT_ALL_KEYS: u8 = 1 << 3;

impl InputModes {
    /// Returns whether any mouse tracking mode is enabled.
    pub const fn is_tracking_mouse(&self) -> bool {
        self.default_mouse || self.button_event_mouse || self.any_event_mouse
    }

    /// Whether the program asked for every key as an escape code.
    ///
    /// A key it asked for travels as a key, so its release has an encoding too;
    /// one it did not ask for travels as the character it typed, which is why a
    /// host that only has text for a printable key still gets a press through.
    pub const fn reports_all_keys(&self) -> bool {
        self.kitty_flags & KITTY_REPORT_ALL_KEYS != 0
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn the_default_matches_the_initial_upstream_set() {
        let modes = InputModes::default();
        assert!(modes.ansi);
        assert!(modes.auto_repeat);
        assert!(modes.alternate_scroll);
        assert!(!modes.line_feed);
        assert!(!modes.cursor_key);
        assert!(!modes.is_tracking_mouse());
        assert_eq!(modes.kitty_flags, 0);
    }

    #[test]
    fn every_mouse_tracking_mode_counts_as_tracking() {
        for enable in [
            |modes: &mut InputModes| modes.default_mouse = true,
            |modes: &mut InputModes| modes.button_event_mouse = true,
            |modes: &mut InputModes| modes.any_event_mouse = true,
        ] {
            let mut modes = InputModes::default();
            enable(&mut modes);
            assert!(modes.is_tracking_mouse());
        }
    }
}
