//! Console input records — the vocabulary an input buffer stores.
//!
//! Upstream's `INPUT_RECORD` is a tagged union of `KEY_EVENT_RECORD`,
//! `MOUSE_EVENT_RECORD`, `WINDOW_BUFFER_SIZE_RECORD`, `FOCUS_EVENT_RECORD` and
//! `MENU_EVENT_RECORD` (see `types/inc/IInputEvent.hpp`). This is the portable
//! form: an enum with one payload per event, and `dwControlKeyState` as plain
//! flags.
//!
//! The types live here rather than in the adapter because both the adapter's
//! `IInteractDispatch` and the host's input buffer use them (ADR 002), and the
//! host's use is what decided their shape (ADR 010).

/// The console's control-key state, as `dwControlKeyState` carries it.
#[derive(Debug, Clone, Copy, PartialEq, Eq, Default)]
pub struct ControlKeyState {
    /// Left Alt is held.
    pub left_alt: bool,
    /// Right Alt is held.
    pub right_alt: bool,
    /// Left Ctrl is held.
    pub left_ctrl: bool,
    /// Right Ctrl is held.
    pub right_ctrl: bool,
    /// Shift is held.
    pub shift: bool,
    /// Num Lock is on.
    pub num_lock: bool,
    /// Scroll Lock is on.
    pub scroll_lock: bool,
    /// Caps Lock is on.
    pub caps_lock: bool,
    /// The key is in the navigation block rather than the numeric keypad.
    pub enhanced: bool,
}

impl ControlKeyState {
    /// Returns a state from the `dwControlKeyState` bits.
    ///
    /// The inverse of [`to_console_flags`](Self::to_console_flags); used where
    /// a record's modifier word arrives from the wire, as win32-input-mode
    /// carries it.
    pub const fn from_console_flags(flags: u16) -> Self {
        Self {
            right_alt: flags & 0x0001 != 0,
            left_alt: flags & 0x0002 != 0,
            right_ctrl: flags & 0x0004 != 0,
            left_ctrl: flags & 0x0008 != 0,
            shift: flags & 0x0010 != 0,
            num_lock: flags & 0x0020 != 0,
            scroll_lock: flags & 0x0040 != 0,
            caps_lock: flags & 0x0080 != 0,
            enhanced: flags & 0x0100 != 0,
        }
    }

    /// Returns whether either Ctrl key is held.
    pub const fn ctrl(&self) -> bool {
        self.left_ctrl || self.right_ctrl
    }

    /// Returns whether either Alt key is held.
    pub const fn alt(&self) -> bool {
        self.left_alt || self.right_alt
    }

    /// Returns the `dwControlKeyState` bits.
    ///
    /// The values are part of the win32-input-mode protocol, which transmits
    /// this word verbatim to the shell.
    pub const fn to_console_flags(&self) -> u16 {
        const RIGHT_ALT_PRESSED: u16 = 0x0001;
        const LEFT_ALT_PRESSED: u16 = 0x0002;
        const RIGHT_CTRL_PRESSED: u16 = 0x0004;
        const LEFT_CTRL_PRESSED: u16 = 0x0008;
        const SHIFT_PRESSED: u16 = 0x0010;
        const NUMLOCK_ON: u16 = 0x0020;
        const SCROLLLOCK_ON: u16 = 0x0040;
        const CAPSLOCK_ON: u16 = 0x0080;
        const ENHANCED_KEY: u16 = 0x0100;

        (self.right_alt as u16 * RIGHT_ALT_PRESSED)
            | (self.left_alt as u16 * LEFT_ALT_PRESSED)
            | (self.right_ctrl as u16 * RIGHT_CTRL_PRESSED)
            | (self.left_ctrl as u16 * LEFT_CTRL_PRESSED)
            | (self.shift as u16 * SHIFT_PRESSED)
            | (self.num_lock as u16 * NUMLOCK_ON)
            | (self.scroll_lock as u16 * SCROLLLOCK_ON)
            | (self.caps_lock as u16 * CAPSLOCK_ON)
            | (self.enhanced as u16 * ENHANCED_KEY)
    }
}

/// A `KEY_EVENT_RECORD`.
///
/// The default is upstream's zeroed record: no key, no character, no modifiers.
#[derive(Debug, Clone, Copy, PartialEq, Eq, Default)]
pub struct KeyEventRecord {
    /// Whether this is a press rather than a release.
    pub key_down: bool,
    /// How many repeats this record stands for.
    pub repeat_count: u16,
    /// The virtual key code, zero when the event is character-only.
    pub virtual_key: u16,
    /// The scan code, zero when the event is character-only.
    pub scan_code: u16,
    /// The character the key produced, `'\0'` when there is none.
    pub character: char,
    /// The modifiers and locks held at the time.
    pub control_keys: ControlKeyState,
}

impl KeyEventRecord {
    /// Returns a press of `character` with no modifiers.
    pub fn pressed(character: char) -> Self {
        Self {
            key_down: true,
            repeat_count: 1,
            virtual_key: 0,
            scan_code: 0,
            character,
            control_keys: ControlKeyState::default(),
        }
    }

    /// Returns a release of `character` with no modifiers.
    pub fn released(character: char) -> Self {
        Self {
            key_down: false,
            ..Self::pressed(character)
        }
    }
}

/// The `dwEventFlags` bits of a mouse record.
#[derive(Debug, Clone, Copy, PartialEq, Eq, Default)]
pub struct MouseEventFlags(u8);

impl MouseEventFlags {
    /// No flags.
    pub const NONE: Self = Self(0);
    /// The pointer moved.
    pub const MOVED: Self = Self(1 << 0);
    /// A button was double-clicked.
    pub const DOUBLE_CLICK: Self = Self(1 << 1);
    /// The vertical wheel turned.
    pub const WHEELED: Self = Self(1 << 2);
    /// A horizontal wheel turned.
    pub const HORIZONTAL_WHEELED: Self = Self(1 << 3);

    /// Returns whether every bit in `other` is set.
    pub const fn contains(self, other: Self) -> bool {
        self.0 & other.0 == other.0
    }

    /// Returns the union of two flag sets.
    pub const fn union(self, other: Self) -> Self {
        Self(self.0 | other.0)
    }

    /// Returns the raw bits.
    pub const fn bits(self) -> u8 {
        self.0
    }
}

/// The `dwButtonState` bits and wheel deltas of a mouse record.
pub mod mouse_button {
    /// The leftmost button.
    pub const FROM_LEFT_1ST_BUTTON_PRESSED: u32 = 0x0001;
    /// The rightmost button.
    pub const RIGHTMOST_BUTTON_PRESSED: u32 = 0x0002;
    /// The middle button.
    pub const FROM_LEFT_2ND_BUTTON_PRESSED: u32 = 0x0004;
    /// One wheel notch backwards, carried in the high word of `dwButtonState`.
    pub const SCROLL_DELTA_BACKWARD: u32 = 0xFF80_0000;
    /// One wheel notch forwards.
    pub const SCROLL_DELTA_FORWARD: u32 = 0x0080_0000;
}

/// A `MOUSE_EVENT_RECORD`.
#[derive(Debug, Clone, Copy, PartialEq, Eq, Default)]
pub struct MouseEventRecord {
    /// The cell the pointer is over, zero-based.
    pub position: (i16, i16),
    /// The buttons held and the wheel distance, as the console packs them.
    pub button_state: u32,
    /// The modifiers and locks held at the time.
    pub control_keys: ControlKeyState,
    /// What kind of mouse event this is.
    pub event_flags: MouseEventFlags,
}

impl MouseEventRecord {
    /// Returns a pointer move to `position` with no buttons held.
    pub fn moved(position: (i16, i16)) -> Self {
        Self {
            position,
            button_state: 0,
            control_keys: ControlKeyState::default(),
            event_flags: MouseEventFlags::MOVED,
        }
    }
}

/// A `WINDOW_BUFFER_SIZE_RECORD`.
#[derive(Debug, Clone, Copy, PartialEq, Eq, Default)]
pub struct WindowSizeEventRecord {
    /// The new console size in cells.
    pub size: (i16, i16),
}

/// A `FOCUS_EVENT_RECORD`.
#[derive(Debug, Clone, Copy, PartialEq, Eq, Default)]
pub struct FocusEventRecord {
    /// Whether the window gained focus.
    pub focused: bool,
}

/// A `MENU_EVENT_RECORD`.
#[derive(Debug, Clone, Copy, PartialEq, Eq, Default)]
pub struct MenuEventRecord {
    /// The menu command the window reported.
    pub command_id: u32,
}

/// One console input record.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum InputRecord {
    /// A keyboard event.
    Key(KeyEventRecord),
    /// A mouse event.
    Mouse(MouseEventRecord),
    /// The console was resized.
    WindowSize(WindowSizeEventRecord),
    /// The window gained or lost focus.
    Focus(FocusEventRecord),
    /// A menu command.
    Menu(MenuEventRecord),
}

impl InputRecord {
    /// Returns the keyboard record, if this is one.
    pub const fn key(&self) -> Option<&KeyEventRecord> {
        match self {
            Self::Key(record) => Some(record),
            _ => None,
        }
    }

    /// Returns whether this is a keyboard event.
    pub const fn is_key(&self) -> bool {
        matches!(self, Self::Key(_))
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn a_pressed_key_carries_its_character() {
        let record = KeyEventRecord::pressed('a');
        assert!(record.key_down);
        assert_eq!(record.repeat_count, 1);
        assert_eq!(record.character, 'a');
        assert!(!KeyEventRecord::released('a').key_down);
    }

    #[test]
    fn the_console_flags_are_the_documented_bits() {
        let state = ControlKeyState {
            shift: true,
            left_ctrl: true,
            right_alt: true,
            enhanced: true,
            ..ControlKeyState::default()
        };
        // SHIFT | LEFT_CTRL | RIGHT_ALT | ENHANCED
        assert_eq!(state.to_console_flags(), 0x0010 | 0x0008 | 0x0001 | 0x0100);
        assert!(state.ctrl());
        assert!(state.alt());
    }

    #[test]
    fn a_moved_mouse_record_has_the_moved_flag() {
        let record = MouseEventRecord::moved((3, 4));
        assert_eq!(record.position, (3, 4));
        assert!(record.event_flags.contains(MouseEventFlags::MOVED));
        assert!(!record.event_flags.contains(MouseEventFlags::WHEELED));
    }

    #[test]
    fn records_report_their_kind() {
        let key = InputRecord::Key(KeyEventRecord::pressed('x'));
        assert!(key.is_key());
        assert_eq!(key.key().map(|record| record.character), Some('x'));

        let focus = InputRecord::Focus(FocusEventRecord { focused: true });
        assert!(!focus.is_key());
        assert!(focus.key().is_none());
    }
}
