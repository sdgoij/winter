//! `TerminalInput` — key events to VT input sequences.
//!
//! Ported from `src/terminal/input/terminalInput.cpp`. The Win32
//! `INPUT_RECORD` becomes [`KeyEvent`] and the class's mode set becomes a
//! [`InputModes`] snapshot (ADR 009); the bytes produced are upstream's.
//!
//! The mouse half of the upstream class lives in `mouse_input.rs`.

use types::InputModes;

use super::mouse_input::MouseState;

/// `KittyKeyboardProtocolFlags::DisambiguateEscapeCodes`.
const KITTY_DISAMBIGUATE: u8 = 1 << 0;
/// `KittyKeyboardProtocolFlags::ReportEventTypes`.
const KITTY_REPORT_EVENT_TYPES: u8 = 1 << 1;
/// `KittyKeyboardProtocolFlags::ReportAlternateKeys`.
const KITTY_REPORT_ALTERNATE_KEYS: u8 = 1 << 2;
/// `KittyKeyboardProtocolFlags::ReportAllKeysAsEscapeCodes`.
const KITTY_REPORT_ALL_KEYS: u8 = 1 << 3;
/// `KittyKeyboardProtocolFlags::ReportAssociatedText`.
const KITTY_REPORT_ASSOCIATED_TEXT: u8 = 1 << 4;

/// No codepoint, as upstream's `InvalidCodepoint`.
const INVALID_CODEPOINT: u32 = 0x11_0000;
/// The key is a text key, so kitty falls back to the legacy encoding.
const KITTY_TEXT_SENTINEL: u32 = 0;
/// The key is a legacy functional key, so kitty falls back to the legacy encoding.
const KITTY_LEGACY_SENTINEL: u32 = 1;

/// The lowest kitty keypad code (`KP_0`).
const KITTY_KP_FIRST: u32 = 57399;
/// The highest kitty keypad code (`KP_BEGIN`).
const KITTY_KP_LAST: u32 = 57427;
/// The PUA base the kitty functional key codes live above.
const KITTY_PUA_BASE: u32 = 0xE000;

/// A key, named by its identity rather than by a platform code.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum Key {
    /// A key whose physical identity is not known; the codepoint carries the text.
    Unidentified,
    /// A printable key, named by its US-layout unshifted character.
    ///
    /// This is the information the kitty "base layout key" parameter needs:
    /// `Key::Char('q')` is the key that produces `q` on a US keyboard.
    Char(char),
    Space,
    Backspace,
    Tab,
    Enter,
    Escape,
    Left,
    Right,
    Up,
    Down,
    Home,
    End,
    PageUp,
    PageDown,
    Insert,
    Delete,
    Clear,
    /// F1 through F24.
    Function(u8),
    /// The numeric keypad digits, 0 through 9.
    Numpad(u8),
    NumpadEnter,
    NumpadDecimal,
    NumpadDivide,
    NumpadMultiply,
    NumpadSubtract,
    NumpadAdd,
    NumpadSeparator,
    Shift,
    LeftShift,
    RightShift,
    Control,
    LeftControl,
    RightControl,
    Alt,
    LeftAlt,
    RightAlt,
    Super,
    LeftSuper,
    RightSuper,
    CapsLock,
    NumLock,
    ScrollLock,
    PrintScreen,
    Pause,
    Menu,
    MediaPlayPause,
    MediaStop,
    MediaNextTrack,
    MediaPreviousTrack,
    VolumeUp,
    VolumeDown,
    VolumeMute,
}

impl Key {
    /// Returns whether the key is one of the modifier keys.
    ///
    /// Upstream tests `VK_SHIFT..=VK_MENU` and `VK_LSHIFT..=VK_RMENU`, which
    /// covers Shift, Control and Alt but not the Windows keys.
    pub const fn is_modifier(self) -> bool {
        matches!(
            self,
            Self::Shift
                | Self::LeftShift
                | Self::RightShift
                | Self::Control
                | Self::LeftControl
                | Self::RightControl
                | Self::Alt
                | Self::LeftAlt
                | Self::RightAlt
        )
    }
}

/// The modifier and lock state of a key event.
#[derive(Debug, Clone, Copy, PartialEq, Eq, Default)]
pub struct Modifiers {
    /// Shift is held.
    pub shift: bool,
    /// Left Ctrl is held.
    pub left_ctrl: bool,
    /// Right Ctrl is held.
    pub right_ctrl: bool,
    /// Left Alt is held.
    pub left_alt: bool,
    /// Right Alt is held.
    pub right_alt: bool,
    /// Caps Lock is on.
    pub caps_lock: bool,
    /// Num Lock is on.
    pub num_lock: bool,
    /// The key came from the navigation block rather than the numeric keypad.
    pub enhanced: bool,
}

impl Modifiers {
    /// Returns whether either Ctrl key is held.
    pub const fn ctrl(&self) -> bool {
        self.left_ctrl || self.right_ctrl
    }

    /// Returns whether either Alt key is held.
    pub const fn alt(&self) -> bool {
        self.left_alt || self.right_alt
    }

    /// Returns whether exactly one Alt key, and nothing else, is set.
    ///
    /// This is upstream's `dwControlKeyState == LEFT_ALT_PRESSED` test for an
    /// Alt-Numpad composition.
    const fn is_only_one_alt(&self) -> bool {
        !self.shift
            && !self.ctrl()
            && !self.caps_lock
            && !self.num_lock
            && !self.enhanced
            && (self.left_alt ^ self.right_alt)
    }
}

/// A platform-neutral key event.
///
/// The platform supplies what only it knows: the translated character, the
/// layout's base and shifted characters when available, repeat and release
/// state, and whether the key is part of an AltGr chord.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub struct KeyEvent {
    /// The key's identity.
    pub key: Key,
    /// The character the key produced in the active layout, if any.
    pub codepoint: Option<char>,
    /// The layout's unshifted character for the key, if the platform reports it.
    pub base_codepoint: Option<char>,
    /// The layout's shifted character for the key, if the platform reports it.
    pub shifted_codepoint: Option<char>,
    /// The modifier and lock state.
    pub modifiers: Modifiers,
    /// Whether this is a press rather than a release.
    pub pressed: bool,
    /// Whether this is an auto-repeat rather than the first press.
    pub repeat: bool,
    /// Whether the key is part of an AltGr chord.
    pub alt_gr: bool,
    /// The platform virtual key code, used only by win32-input-mode.
    pub virtual_key: u16,
    /// The platform scan code, used only by win32-input-mode.
    pub scan_code: u16,
    /// The platform repeat count, used only by win32-input-mode.
    pub repeat_count: u16,
}

impl KeyEvent {
    /// Returns a press of `key` with no character and no modifiers.
    pub fn down(key: Key) -> Self {
        Self {
            key,
            codepoint: None,
            base_codepoint: None,
            shifted_codepoint: None,
            modifiers: Modifiers::default(),
            pressed: true,
            repeat: false,
            alt_gr: false,
            virtual_key: 0,
            scan_code: 0,
            repeat_count: 1,
        }
    }

    /// Returns a release of `key` with no character and no modifiers.
    pub fn up(key: Key) -> Self {
        Self {
            pressed: false,
            ..Self::down(key)
        }
    }
}

/// Translates key events into the bytes a program reads from its pty.
#[derive(Debug, Default, Clone, Copy, PartialEq, Eq)]
pub struct TerminalInput {
    pub(super) mouse: MouseState,
}

impl TerminalInput {
    /// Creates an input engine.
    pub const fn new() -> Self {
        Self {
            mouse: MouseState::new(),
        }
    }

    /// Translates a key event, or returns `None` when it has no encoding.
    ///
    /// `None` is upstream's empty optional: the caller may handle the key in
    /// another way.
    pub fn handle_key(&self, event: &KeyEvent, modes: &InputModes) -> Option<String> {
        // Win32 input mode sees every key, but only while the kitty protocol is
        // not in use: otherwise the shell would never see kitty sequences.
        if modes.win32_input && modes.kitty_flags == 0 {
            return Some(self.win32_output(event, modes));
        }

        if event.pressed {
            // A key with no identity carries only its character.
            if event.key == Key::Unidentified {
                return event.codepoint.map(String::from);
            }

            // The keys of an Alt-Numpad composition are swallowed; the
            // composed character arrives on the Alt release.
            if matches!(event.key, Key::Numpad(0..=9)) && event.modifiers.is_only_one_alt() {
                return None;
            }
        }

        if event.repeat && (event.key.is_modifier() || !modes.auto_repeat) {
            return None;
        }

        // Modifier keys are only reported when the kitty protocol asks.
        if modes.kitty_flags & KITTY_REPORT_ALL_KEYS == 0 && event.key.is_modifier() {
            return None;
        }

        if !event.pressed {
            // With Num Lock on, an Alt release carrying a character is the
            // second half of an Alt-Numpad composition.
            if event.modifiers.num_lock
                && event.key == Key::Alt
                && let Some(character) = event.codepoint
            {
                return Some(character.to_string());
            }

            if modes.kitty_flags & KITTY_REPORT_EVENT_TYPES == 0 {
                return None;
            }

            // Enter, Tab and Backspace only report releases when every key is
            // reported as an escape code.
            if modes.kitty_flags & KITTY_REPORT_ALL_KEYS == 0
                && ((event.key == Key::Enter && !event.modifiers.enhanced)
                    || event.key == Key::Tab
                    || event.key == Key::Backspace)
            {
                return None;
            }
        }

        let class = classify(event);

        let mut enc = Encoding {
            csi_modifier: csi_modifier_bits(&class),
            ..Encoding::default()
        };

        let legacy = modes.kitty_flags == 0 || !self.encode_kitty(&mut enc, event, &class, modes);
        if legacy {
            self.encode_regular(&mut enc, event, &class, modes);
        }

        if let Some(sequence) = format_encoding(&enc, &class, modes) {
            return Some(sequence);
        }

        format_fallback(event, &class, modes)
    }

    /// Translates a focus change, when focus reporting is enabled.
    pub fn handle_focus(&self, focused: bool, modes: &InputModes) -> Option<String> {
        if !modes.focus_event {
            return None;
        }
        let csi = introducer(modes);
        Some(format!("{csi}{}", if focused { 'I' } else { 'O' }))
    }

    /// Encodes a cursor key without the kitty, win32 or filter prelude.
    ///
    /// Upstream's `_makeAlternateScrollOutput` calls `_encodeRegular` and
    /// `_formatEncodingHelper` directly; this is that path.
    pub(super) fn encode_cursor_key(&self, key: Key, modes: &InputModes) -> Option<String> {
        let event = KeyEvent::down(key);
        let class = Classification {
            ctrl: false,
            alt: false,
            shift: false,
            enhanced: false,
        };
        let mut enc = Encoding::default();
        self.encode_regular(&mut enc, &event, &class, modes);
        format_encoding(&enc, &class, modes)
    }

    /// Encodes an event as a win32-input-mode record.
    fn win32_output(&self, event: &KeyEvent, modes: &InputModes) -> String {
        let character = event.codepoint.map_or(0, |character| character as u16);
        let down = u16::from(event.pressed);
        format!(
            "{}{};{};{};{};{};{}_",
            introducer(modes),
            event.virtual_key,
            event.scan_code,
            character,
            down,
            control_key_state(event.modifiers),
            event.repeat_count,
        )
    }
}

/// The modifier decisions the encoders share.
struct Classification {
    ctrl: bool,
    alt: bool,
    shift: bool,
    enhanced: bool,
}

fn classify(event: &KeyEvent) -> Classification {
    let modifiers = event.modifiers;
    let any_ctrl = modifiers.ctrl();
    let any_alt = modifiers.alt();
    let both_ctrl = modifiers.left_ctrl && modifiers.right_ctrl;
    let both_alt = modifiers.left_alt && modifiers.right_alt;
    // A graphical character with both modifiers is an AltGr layer, not a
    // deliberate Ctrl+Alt chord; upstream uses the same heuristic because
    // Windows sends no character for a genuine Ctrl+Alt combination.
    let graphical = event
        .codepoint
        .is_some_and(|character| character > '\u{20}' && character != '\u{7f}');
    let alt_gr = event.alt_gr || (any_alt && any_ctrl && graphical);

    Classification {
        ctrl: both_ctrl || (any_ctrl && !alt_gr),
        alt: both_alt || (any_alt && !alt_gr),
        shift: modifiers.shift,
        enhanced: modifiers.enhanced,
    }
}

const fn csi_modifier_bits(class: &Classification) -> u32 {
    (class.shift as u32) | ((class.alt as u32) << 1) | ((class.ctrl as u32) << 2)
}

fn introducer(modes: &InputModes) -> &'static str {
    if modes.send_c1 { "\u{9b}" } else { "\x1b[" }
}

fn string_terminator(modes: &InputModes) -> &'static str {
    if modes.send_c1 { "\u{8f}" } else { "\x1bO" }
}

/// The documented win32-input-mode control-key bits.
fn control_key_state(modifiers: Modifiers) -> u16 {
    const RIGHT_ALT_PRESSED: u16 = 0x0001;
    const LEFT_ALT_PRESSED: u16 = 0x0002;
    const RIGHT_CTRL_PRESSED: u16 = 0x0004;
    const LEFT_CTRL_PRESSED: u16 = 0x0008;
    const SHIFT_PRESSED: u16 = 0x0010;
    const NUMLOCK_ON: u16 = 0x0020;
    const CAPSLOCK_ON: u16 = 0x0080;
    const ENHANCED_KEY: u16 = 0x0100;

    let mut state = 0;
    if modifiers.right_alt {
        state |= RIGHT_ALT_PRESSED;
    }
    if modifiers.left_alt {
        state |= LEFT_ALT_PRESSED;
    }
    if modifiers.right_ctrl {
        state |= RIGHT_CTRL_PRESSED;
    }
    if modifiers.left_ctrl {
        state |= LEFT_CTRL_PRESSED;
    }
    if modifiers.shift {
        state |= SHIFT_PRESSED;
    }
    if modifiers.num_lock {
        state |= NUMLOCK_ON;
    }
    if modifiers.caps_lock {
        state |= CAPSLOCK_ON;
    }
    if modifiers.enhanced {
        state |= ENHANCED_KEY;
    }
    state
}

/// The fields a sequence is assembled from, as upstream's `EncodingHelper`.
#[derive(Default)]
struct Encoding {
    csi_key_code: u32,
    csi_alt_shifted: u32,
    csi_alt_base: u32,
    csi_modifier: u32,
    csi_event_type: u32,
    csi_text: u32,
    csi_final: char,
    ss3_final: char,
    alt_prefix: bool,
    plain: &'static str,
}

/// Assembles the sequence the [`Encoding`] describes.
fn format_encoding(enc: &Encoding, class: &Classification, modes: &InputModes) -> Option<String> {
    let mut sequence = String::new();
    if enc.alt_prefix && class.alt && modes.ansi {
        sequence.push('\x1b');
    }

    if enc.csi_final != '\0' {
        let mut modifier = enc.csi_modifier;
        let mut key_code = enc.csi_key_code;
        if modifier != 0 || enc.csi_event_type != 0 {
            // The CSI modifier value is one-based.
            modifier += 1;
            key_code = key_code.max(1);
        }

        sequence.push_str(introducer(modes));
        if key_code != 0 {
            sequence.push_str(&key_code.to_string());
        }
        if enc.csi_alt_shifted != 0 || enc.csi_alt_base != 0 {
            sequence.push(':');
            if enc.csi_alt_shifted != 0 {
                sequence.push_str(&enc.csi_alt_shifted.to_string());
            }
            if enc.csi_alt_base != 0 {
                sequence.push(':');
                sequence.push_str(&enc.csi_alt_base.to_string());
            }
        }
        if modifier != 0 || enc.csi_text != 0 {
            sequence.push(';');
            if modifier != 0 {
                sequence.push_str(&modifier.to_string());
                if enc.csi_event_type != 0 {
                    sequence.push(':');
                    sequence.push_str(&enc.csi_event_type.to_string());
                }
            }
            if enc.csi_text != 0 {
                sequence.push(';');
                sequence.push_str(&enc.csi_text.to_string());
            }
        }
        sequence.push(enc.csi_final);
        return Some(sequence);
    }

    if enc.ss3_final != '\0' {
        sequence.push_str(string_terminator(modes));
        sequence.push(enc.ss3_final);
        return Some(sequence);
    }

    if !enc.plain.is_empty() {
        sequence.push_str(enc.plain);
        return Some(sequence);
    }

    None
}

/// Appends the character or control code a key produced.
fn format_fallback(event: &KeyEvent, class: &Classification, modes: &InputModes) -> Option<String> {
    // A release carries no text, whatever the mode.
    if !event.pressed {
        return None;
    }

    // Ctrl+Space is always NUL, however the layout translates the space key.
    let ctrl_space = class.ctrl && event.key == Key::Space;
    let codepoint = if let Some(character) = event.codepoint.filter(|_| !ctrl_space) {
        let codepoint = u32::from(character);
        if class.ctrl {
            make_ctrl_char(codepoint)
        } else {
            codepoint
        }
    } else if class.alt || class.ctrl {
        // Without a translated character, the layout's base key decides what
        // was pressed. Upstream asks `ToUnicodeEx`; here the platform reports
        // the base character, or the US layout supplies it.
        let base = event.base_codepoint.or_else(|| us_base_character(event.key))?;
        let mut codepoint = u32::from(base);
        if class.ctrl {
            codepoint = make_ctrl_char(codepoint);
            // Alphanumeric keys keep their control mapping even when the
            // layout translated them somewhere else.
            if codepoint >= 0x20
                && let Key::Char(character) = event.key
            {
                codepoint = make_ctrl_char(u32::from(character));
            }
        }
        codepoint
    } else {
        return None;
    };

    let mut sequence = String::new();
    if class.alt && modes.ansi {
        sequence.push('\x1b');
    }
    if let Some(character) = char::from_u32(codepoint) {
        sequence.push(character);
    }
    Some(sequence)
}

/// Maps a character to the control code a PC keyboard produces, as upstream's
/// `_makeCtrlChar`.
fn make_ctrl_char(ch: u32) -> u32 {
    match ch {
        0x40..=0x7e => ch & 0x1f,
        0x20 => 0x00,
        0x2f => 0x1f,
        0x3f => 0x7f,
        0x32..=0x38 => [0x00, 0x1b, 0x1c, 0x1d, 0x1e, 0x1f, 0x7f][(ch - 0x32) as usize],
        _ => ch,
    }
}

/// Returns the US-layout unshifted character for a key.
fn us_base_character(key: Key) -> Option<char> {
    match key {
        Key::Char(character) => Some(character),
        Key::Space => Some(' '),
        _ => us_base_key(key).and_then(char::from_u32),
    }
}

/// Returns the US-layout shifted character for a key.
fn us_shifted_character(key: Key) -> Option<char> {
    let Key::Char(character) = key else {
        return None;
    };
    Some(match character {
        'a'..='z' => character.to_ascii_uppercase(),
        '0' => ')',
        '1' => '!',
        '2' => '@',
        '3' => '#',
        '4' => '$',
        '5' => '%',
        '6' => '^',
        '7' => '&',
        '8' => '*',
        '9' => '(',
        '`' => '~',
        '-' => '_',
        '=' => '+',
        '[' => '{',
        ']' => '}',
        '\\' => '|',
        ';' => ':',
        '\'' => '"',
        ',' => '<',
        '.' => '>',
        '/' => '?',
        other => other,
    })
}

/// The physical key in the standard PC-101 layout, as upstream's
/// `getKittyUSBaseKey` table.
fn us_base_key(key: Key) -> Option<u32> {
    Some(match key {
        Key::Char(character) => u32::from(character),
        Key::Space => 0x20,
        Key::Escape => 0x1B,
        Key::Backspace => 0x7F,
        Key::Tab => 0x09,
        Key::Enter => 0x0D,
        Key::NumpadEnter => 57414,
        Key::Shift | Key::LeftShift => 57441,
        Key::RightShift => 57447,
        Key::Control | Key::LeftControl => 57442,
        Key::RightControl => 57448,
        Key::Alt | Key::LeftAlt => 57443,
        Key::RightAlt => 57449,
        Key::Super | Key::LeftSuper => 57444,
        Key::RightSuper => 57450,
        Key::CapsLock => 57358,
        Key::ScrollLock => 57359,
        Key::NumLock => 57360,
        Key::PrintScreen => 57361,
        Key::Pause => 57362,
        Key::Menu => 57363,
        Key::Home => 57423,
        Key::Up => 57419,
        Key::PageUp => 57421,
        Key::Left => 57417,
        Key::Right => 57418,
        Key::End => 57424,
        Key::Down => 57420,
        Key::PageDown => 57422,
        Key::Insert => 57425,
        Key::Delete => 57426,
        Key::Numpad(digit) => KITTY_KP_FIRST + u32::from(digit),
        Key::NumpadDecimal => 57409,
        Key::NumpadDivide => 57410,
        Key::NumpadMultiply => 57411,
        Key::NumpadSubtract => 57412,
        Key::NumpadAdd => 57413,
        Key::NumpadSeparator => 57416,
        Key::Function(number) if number >= 13 => 57376 + u32::from(number) - 13,
        Key::MediaPlayPause => 57430,
        Key::MediaStop => 57432,
        Key::MediaNextTrack => 57435,
        Key::MediaPreviousTrack => 57436,
        Key::VolumeDown => 57438,
        Key::VolumeUp => 57439,
        Key::VolumeMute => 57440,
        _ => return None,
    })
}

/// Returns the kitty functional key code, or one of the sentinels.
fn kitty_functional_key_code(key: Key, enhanced: bool) -> u32 {
    let navigation = |code: u32| if enhanced { KITTY_LEGACY_SENTINEL } else { code };
    match key {
        Key::Escape => 27,
        Key::Enter | Key::NumpadEnter => {
            if enhanced {
                57414
            } else {
                13
            }
        }
        Key::Tab => 9,
        Key::Backspace => 127,
        Key::Left => navigation(57417),
        Key::Right => navigation(57418),
        Key::Up => navigation(57419),
        Key::Down => navigation(57420),
        Key::PageUp => navigation(57421),
        Key::PageDown => navigation(57422),
        Key::Home => navigation(57423),
        Key::End => navigation(57424),
        Key::Insert => navigation(57425),
        Key::Delete => navigation(57426),
        Key::Clear => KITTY_LEGACY_SENTINEL,
        Key::Function(number) if (1..=12).contains(&number) => KITTY_LEGACY_SENTINEL,
        Key::Function(number) if (13..=24).contains(&number) => 57376 + u32::from(number) - 13,
        Key::Numpad(digit) => KITTY_KP_FIRST + u32::from(digit),
        Key::NumpadDecimal => 57409,
        Key::NumpadDivide => 57410,
        Key::NumpadMultiply => 57411,
        Key::NumpadSubtract => 57412,
        Key::NumpadAdd => 57413,
        Key::NumpadSeparator => 57416,
        Key::CapsLock => 57358,
        Key::ScrollLock => 57359,
        Key::NumLock => 57360,
        Key::PrintScreen => 57361,
        Key::Pause => 57362,
        Key::Menu => 57363,
        Key::Shift | Key::LeftShift => 57441,
        Key::RightShift => 57447,
        Key::Control | Key::LeftControl => {
            if enhanced {
                57448
            } else {
                57442
            }
        }
        Key::RightControl => 57448,
        Key::Alt | Key::LeftAlt => {
            if enhanced {
                57449
            } else {
                57443
            }
        }
        Key::RightAlt => 57449,
        Key::Super | Key::LeftSuper => 57444,
        Key::RightSuper => 57450,
        Key::MediaPlayPause => 57430,
        Key::MediaStop => 57432,
        Key::MediaNextTrack => 57435,
        Key::MediaPreviousTrack => 57436,
        Key::VolumeDown => 57438,
        Key::VolumeUp => 57439,
        Key::VolumeMute => 57440,
        _ => KITTY_TEXT_SENTINEL,
    }
}

fn is_kitty_functional(codepoint: u32) -> bool {
    (codepoint > KITTY_LEGACY_SENTINEL && codepoint <= 0x1f)
        || (0x7f..=0x9f).contains(&codepoint)
        || (KITTY_PUA_BASE..=0xF8FF).contains(&codepoint)
}

fn is_kitty_valid_text(codepoint: u32) -> bool {
    (codepoint > 0x1f && codepoint < 0x7f) || (codepoint > 0x9f && codepoint < INVALID_CODEPOINT)
}

/// Lowercases a codepoint, or returns `INVALID_CODEPOINT` when the result is
/// not a single character. As upstream's `_codepointToLower`.
fn lowercase(codepoint: u32) -> u32 {
    if codepoint >= INVALID_CODEPOINT {
        return codepoint;
    }
    let Some(character) = char::from_u32(codepoint) else {
        return INVALID_CODEPOINT;
    };
    let mut lowered = character.to_lowercase();
    match (lowered.next(), lowered.next()) {
        (Some(only), None) => u32::from(only),
        _ => INVALID_CODEPOINT,
    }
}

/// Returns the kitty "base key": the key without Ctrl, Alt or Shift.
fn kitty_base_key(event: &KeyEvent) -> u32 {
    let base = event
        .base_codepoint
        .or_else(|| us_base_character(event.key))
        .or(event.codepoint);
    match base {
        Some(character) => lowercase(u32::from(character)),
        None => INVALID_CODEPOINT,
    }
}

/// Returns the kitty "shifted key": the key with Shift and without Ctrl or Alt.
fn kitty_shifted_key(event: &KeyEvent) -> u32 {
    let shifted = event
        .shifted_codepoint
        .or_else(|| us_shifted_character(event.key))
        .or(event.codepoint);
    match shifted {
        Some(character) => u32::from(character),
        None => INVALID_CODEPOINT,
    }
}

impl TerminalInput {
    /// Fills the kitty keyboard protocol fields of `enc`.
    ///
    /// Returns whether the key is a kitty escape code; `false` means the
    /// legacy encoder runs next.
    fn encode_kitty(&self, enc: &mut Encoding, event: &KeyEvent, class: &Classification, modes: &InputModes) -> bool {
        let flags = modes.kitty_flags;
        let functional = kitty_functional_key_code(event.key, class.enhanced);
        let is_text = functional == KITTY_TEXT_SENTINEL;

        if flags & KITTY_DISAMBIGUATE != 0 {
            // Escape, modifier chords on ASCII keys and every keypad key are
            // reported as `CSI u` so a program can tell them from text.
            if functional > KITTY_LEGACY_SENTINEL
                && (functional == 27
                    || (functional <= 127 && enc.csi_modifier != 0)
                    || (KITTY_KP_FIRST..=KITTY_KP_LAST).contains(&functional))
            {
                enc.csi_key_code = functional;
                enc.csi_final = 'u';
            }
        }

        if flags & KITTY_REPORT_EVENT_TYPES != 0 {
            if !event.pressed {
                enc.csi_event_type = 3;
            } else if event.repeat {
                enc.csi_event_type = 2;
            }
        }

        if flags & KITTY_REPORT_ALL_KEYS != 0
            || (flags & KITTY_DISAMBIGUATE != 0 && is_text && enc.csi_modifier > 1)
            || enc.csi_event_type == 3
        {
            if is_kitty_functional(functional) {
                enc.csi_key_code = functional;
                enc.csi_final = 'u';
            } else if is_text {
                let codepoint = kitty_base_key(event);
                if codepoint < INVALID_CODEPOINT {
                    enc.csi_key_code = codepoint;
                    enc.csi_final = 'u';
                }
            }

            if flags & KITTY_REPORT_ASSOCIATED_TEXT != 0
                && enc.csi_event_type != 3
                && let Some(character) = event
                    .codepoint
                    .filter(|character| is_kitty_valid_text(u32::from(*character)))
            {
                enc.csi_text = u32::from(character);
            }
        }

        if flags & KITTY_REPORT_ALTERNATE_KEYS != 0 && enc.csi_key_code != 0 {
            if is_text && class.shift {
                let shifted = kitty_shifted_key(event);
                if shifted < INVALID_CODEPOINT {
                    enc.csi_alt_shifted = shifted;
                }
            }

            // The base key is the physical key in the standard PC-101 layout,
            // omitted when it is what the key produced anyway.
            if let Some(base) = us_base_key(event.key)
                && base != enc.csi_key_code
            {
                enc.csi_alt_base = base;
            }
        }

        // Lock modifiers are only reported for functional keys, unless every
        // key is reported as an escape code.
        if is_kitty_functional(enc.csi_key_code) || flags & KITTY_REPORT_ALL_KEYS != 0 {
            if event.modifiers.caps_lock {
                enc.csi_modifier |= 64;
            }
            if event.modifiers.num_lock {
                enc.csi_modifier |= 128;
            }
        }

        enc.csi_final != '\0'
    }

    /// Fills `enc` for a key the kitty encoder left to the legacy encoding.
    fn encode_regular(&self, enc: &mut Encoding, event: &KeyEvent, class: &Classification, modes: &InputModes) {
        let modified = enc.csi_modifier != 0;
        let enhanced = class.enhanced;
        let kitty = modes.kitty_flags & (KITTY_DISAMBIGUATE | KITTY_REPORT_EVENT_TYPES | KITTY_REPORT_ALL_KEYS) != 0;
        // Cursor, Home and End only use SS3 when the cursor keys are in
        // application mode and no modifier or kitty mode forces CSI.
        let use_csi = kitty || modified || !modes.cursor_key;

        match event.key {
            Key::Backspace => {
                enc.alt_prefix = true;
                // The Ctrl modifier inverts the Backarrow Key mode.
                enc.plain = if class.ctrl == modes.backarrow_key {
                    "\x7f"
                } else {
                    "\x08"
                };
            }
            Key::Tab => {
                enc.alt_prefix = true;
                enc.plain = if !class.shift {
                    "\t"
                } else if modes.send_c1 {
                    "\u{9b}Z"
                } else {
                    "\x1b[Z"
                };
            }
            Key::Enter | Key::NumpadEnter => {
                enc.alt_prefix = true;
                if modes.keypad && enhanced {
                    if modes.ansi {
                        enc.ss3_final = 'M';
                    } else {
                        enc.plain = "\x1b?M";
                    }
                } else if !class.ctrl {
                    enc.plain = if modes.line_feed { "\r\n" } else { "\r" };
                } else {
                    enc.plain = "\n";
                }
            }
            Key::Pause => enc.plain = "\x1a",
            Key::Function(number @ 1..=4) => {
                if modes.ansi {
                    // The original spec allowed F3 as both `CSI R` and `CSI ~`.
                    if kitty && number == 3 {
                        enc.csi_final = '~';
                        enc.csi_key_code = 13;
                    } else {
                        let final_byte = char::from(b'P' + number - 1);
                        if kitty || modified {
                            enc.csi_final = final_byte;
                        } else {
                            enc.ss3_final = final_byte;
                        }
                    }
                } else {
                    enc.plain = ["\x1bP", "\x1bQ", "\x1bR", "\x1bS"][usize::from(number - 1)];
                }
            }
            Key::Function(number @ 5..=20) => {
                if modes.ansi {
                    const DECFNK: [u32; 16] = [15, 17, 18, 19, 20, 21, 23, 24, 25, 26, 28, 29, 31, 32, 33, 34];
                    enc.csi_final = '~';
                    enc.csi_key_code = DECFNK[usize::from(number - 5)];
                } else {
                    // VT52 defines only F11 through F13.
                    enc.plain = match number {
                        11 => "\x1b",
                        12 => "\x08",
                        13 => "\n",
                        _ => "",
                    };
                }
            }
            Key::Left => set_cursor_final(enc, 'D', use_csi, modes, "\x1bD"),
            Key::Up => set_cursor_final(enc, 'A', use_csi, modes, "\x1bA"),
            Key::Right => set_cursor_final(enc, 'C', use_csi, modes, "\x1bC"),
            Key::Down => set_cursor_final(enc, 'B', use_csi, modes, "\x1bB"),
            Key::Clear => set_cursor_final(enc, 'E', use_csi, modes, "\x1bE"),
            Key::Home => set_cursor_final(enc, 'H', use_csi, modes, "\x1bH"),
            Key::End => set_cursor_final(enc, 'F', use_csi, modes, "\x1bF"),
            Key::Insert | Key::Delete => {
                if modes.ansi {
                    enc.csi_final = '~';
                    enc.csi_key_code = if event.key == Key::Insert { 2 } else { 3 };
                }
            }
            Key::PageUp | Key::PageDown => {
                if modes.ansi {
                    enc.csi_final = '~';
                    enc.csi_key_code = if event.key == Key::PageUp { 5 } else { 6 };
                }
            }
            Key::Numpad(digit) if modes.keypad => {
                if modes.ansi {
                    enc.ss3_final = char::from(b'p' + digit);
                } else {
                    const VT52: [&str; 10] = [
                        "\x1b?p", "\x1b?q", "\x1b?r", "\x1b?s", "\x1b?t", "\x1b?u", "\x1b?v", "\x1b?w", "\x1b?x",
                        "\x1b?y",
                    ];
                    enc.plain = VT52[usize::from(digit)];
                }
            }
            Key::NumpadMultiply
            | Key::NumpadAdd
            | Key::NumpadSeparator
            | Key::NumpadSubtract
            | Key::NumpadDecimal
            | Key::NumpadDivide
                if modes.keypad =>
            {
                let offset = match event.key {
                    Key::NumpadMultiply => 0,
                    Key::NumpadAdd => 1,
                    Key::NumpadSeparator => 2,
                    Key::NumpadSubtract => 3,
                    Key::NumpadDecimal => 4,
                    _ => 5,
                };
                if modes.ansi {
                    enc.ss3_final = char::from(b'j' + offset);
                } else {
                    enc.plain = ["\x1b?j", "\x1b?k", "\x1b?l", "\x1b?m", "\x1b?n", "\x1b?o"][usize::from(offset)];
                }
            }
            _ => {}
        }
    }
}

/// Sets a cursor-movement final, choosing CSI or SS3 as upstream does.
fn set_cursor_final(enc: &mut Encoding, final_byte: char, use_csi: bool, modes: &InputModes, vt52: &'static str) {
    if !modes.ansi {
        enc.plain = vt52;
    } else if use_csi {
        enc.csi_final = final_byte;
    } else {
        enc.ss3_final = final_byte;
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    fn ctrl() -> Modifiers {
        Modifiers {
            left_ctrl: true,
            ..Modifiers::default()
        }
    }

    fn alt() -> Modifiers {
        Modifiers {
            left_alt: true,
            ..Modifiers::default()
        }
    }

    fn shift() -> Modifiers {
        Modifiers {
            shift: true,
            ..Modifiers::default()
        }
    }

    fn encode(event: &KeyEvent) -> Option<String> {
        TerminalInput::new().handle_key(event, &InputModes::default())
    }

    fn key(key: Key, modifiers: Modifiers) -> KeyEvent {
        KeyEvent {
            modifiers,
            ..KeyEvent::down(key)
        }
    }

    fn character(character: char, modifiers: Modifiers) -> KeyEvent {
        KeyEvent {
            key: Key::Char(character),
            codepoint: Some(character),
            modifiers,
            ..KeyEvent::down(Key::Unidentified)
        }
    }

    #[test]
    fn a_plain_character_is_sent_as_itself() {
        assert_eq!(encode(&character('a', Modifiers::default())).as_deref(), Some("a"));
        assert_eq!(encode(&character('é', Modifiers::default())).as_deref(), Some("é"));
    }

    #[test]
    fn a_key_with_no_identity_is_sent_as_its_character() {
        let event = KeyEvent {
            codepoint: Some('é'),
            ..KeyEvent::down(Key::Unidentified)
        };
        assert_eq!(encode(&event).as_deref(), Some("é"));
    }

    #[test]
    fn enter_is_a_carriage_return_and_lnm_adds_the_line_feed() {
        assert_eq!(encode(&KeyEvent::down(Key::Enter)).as_deref(), Some("\r"));

        let modes = InputModes {
            line_feed: true,
            ..InputModes::default()
        };
        assert_eq!(
            TerminalInput::new()
                .handle_key(&KeyEvent::down(Key::Enter), &modes)
                .as_deref(),
            Some("\r\n")
        );
    }

    #[test]
    fn ctrl_enter_is_a_line_feed() {
        assert_eq!(encode(&key(Key::Enter, ctrl())).as_deref(), Some("\n"));
    }

    #[test]
    fn backspace_is_del_and_the_backarrow_mode_sends_bs() {
        assert_eq!(encode(&KeyEvent::down(Key::Backspace)).as_deref(), Some("\x7f"));

        let modes = InputModes {
            backarrow_key: true,
            ..InputModes::default()
        };
        let input = TerminalInput::new();
        assert_eq!(
            input.handle_key(&KeyEvent::down(Key::Backspace), &modes).as_deref(),
            Some("\x08")
        );
        // Ctrl inverts the mode.
        assert_eq!(encode(&key(Key::Backspace, ctrl())).as_deref(), Some("\x08"));
        assert_eq!(
            input.handle_key(&key(Key::Backspace, ctrl()), &modes).as_deref(),
            Some("\x7f")
        );
        assert_eq!(encode(&key(Key::Backspace, alt())).as_deref(), Some("\x1b\x7f"));
    }

    #[test]
    fn tab_and_shift_tab() {
        assert_eq!(encode(&KeyEvent::down(Key::Tab)).as_deref(), Some("\t"));
        assert_eq!(encode(&key(Key::Tab, shift())).as_deref(), Some("\x1b[Z"));
    }

    #[test]
    fn cursor_keys_use_csi_unless_the_cursor_key_mode_is_set() {
        assert_eq!(encode(&KeyEvent::down(Key::Left)).as_deref(), Some("\x1b[D"));
        assert_eq!(encode(&KeyEvent::down(Key::Up)).as_deref(), Some("\x1b[A"));

        let modes = InputModes {
            cursor_key: true,
            ..InputModes::default()
        };
        assert_eq!(
            TerminalInput::new()
                .handle_key(&KeyEvent::down(Key::Left), &modes)
                .as_deref(),
            Some("\x1bOD")
        );
        // A modifier forces the CSI form again.
        assert_eq!(encode(&key(Key::Left, ctrl())).as_deref(), Some("\x1b[1;5D"));
        assert_eq!(encode(&key(Key::Right, shift())).as_deref(), Some("\x1b[1;2C"));
    }

    #[test]
    fn home_end_and_the_editing_keys() {
        assert_eq!(encode(&KeyEvent::down(Key::Home)).as_deref(), Some("\x1b[H"));
        assert_eq!(encode(&KeyEvent::down(Key::End)).as_deref(), Some("\x1b[F"));
        assert_eq!(encode(&KeyEvent::down(Key::Insert)).as_deref(), Some("\x1b[2~"));
        assert_eq!(encode(&KeyEvent::down(Key::Delete)).as_deref(), Some("\x1b[3~"));
        assert_eq!(encode(&KeyEvent::down(Key::PageUp)).as_deref(), Some("\x1b[5~"));
        assert_eq!(encode(&KeyEvent::down(Key::PageDown)).as_deref(), Some("\x1b[6~"));
    }

    #[test]
    fn function_keys_use_ss3_and_decfnk() {
        assert_eq!(encode(&KeyEvent::down(Key::Function(1))).as_deref(), Some("\x1bOP"));
        assert_eq!(encode(&KeyEvent::down(Key::Function(4))).as_deref(), Some("\x1bOS"));
        assert_eq!(encode(&KeyEvent::down(Key::Function(5))).as_deref(), Some("\x1b[15~"));
        assert_eq!(encode(&KeyEvent::down(Key::Function(12))).as_deref(), Some("\x1b[24~"));
        assert_eq!(encode(&KeyEvent::down(Key::Function(20))).as_deref(), Some("\x1b[34~"));
        // Upstream's legacy table stops at F20; F21-F24 are only reachable
        // through the kitty protocol.
        assert_eq!(encode(&KeyEvent::down(Key::Function(24))), None);
        // A modifier moves F1-F4 to CSI.
        assert_eq!(encode(&key(Key::Function(1), ctrl())).as_deref(), Some("\x1b[1;5P"));
    }

    #[test]
    fn vt52_mode_uses_the_escape_forms() {
        let modes = InputModes {
            ansi: false,
            keypad: true,
            ..InputModes::default()
        };
        let input = TerminalInput::new();
        assert_eq!(
            input.handle_key(&KeyEvent::down(Key::Left), &modes).as_deref(),
            Some("\x1bD")
        );
        assert_eq!(
            input.handle_key(&KeyEvent::down(Key::Home), &modes).as_deref(),
            Some("\x1bH")
        );
        assert_eq!(
            input.handle_key(&KeyEvent::down(Key::Function(1)), &modes).as_deref(),
            Some("\x1bP")
        );
        assert_eq!(
            input.handle_key(&KeyEvent::down(Key::Numpad(1)), &modes).as_deref(),
            Some("\x1b?q")
        );
        assert_eq!(
            input.handle_key(&KeyEvent::down(Key::NumpadAdd), &modes).as_deref(),
            Some("\x1b?k")
        );
        // Alt is not prefixed in VT52 mode.
        assert_eq!(input.handle_key(&character('a', alt()), &modes).as_deref(), Some("a"));
    }

    #[test]
    fn numeric_keypad_mode_sends_ss3_and_otherwise_nothing() {
        let modes = InputModes {
            keypad: true,
            ..InputModes::default()
        };
        let input = TerminalInput::new();
        assert_eq!(
            input.handle_key(&KeyEvent::down(Key::Numpad(5)), &modes).as_deref(),
            Some("\x1bOu")
        );
        assert_eq!(
            input.handle_key(&KeyEvent::down(Key::NumpadAdd), &modes).as_deref(),
            Some("\x1bOk")
        );
        assert_eq!(
            input.handle_key(&KeyEvent::down(Key::NumpadDivide), &modes).as_deref(),
            Some("\x1bOo")
        );
        // Without keypad mode a digit key falls through to its character, and
        // there is none here.
        assert_eq!(encode(&KeyEvent::down(Key::Numpad(5))), None);
    }

    #[test]
    fn ctrl_maps_letters_and_digits_to_control_codes() {
        assert_eq!(encode(&character('a', ctrl())).as_deref(), Some("\x01"));
        assert_eq!(encode(&character('z', ctrl())).as_deref(), Some("\x1a"));
        assert_eq!(encode(&character('[', ctrl())).as_deref(), Some("\x1b"));
        assert_eq!(encode(&character('?', ctrl())).as_deref(), Some("\x7f"));
        assert_eq!(encode(&character('2', ctrl())).as_deref(), Some("\x00"));
        assert_eq!(encode(&character(' ', ctrl())).as_deref(), Some("\x00"));
    }

    #[test]
    fn ctrl_space_is_nul_even_without_a_translated_character() {
        assert_eq!(encode(&key(Key::Space, ctrl())).as_deref(), Some("\x00"));
    }

    #[test]
    fn a_ctrl_chord_without_a_character_uses_the_us_base_key() {
        assert_eq!(encode(&key(Key::Char('a'), ctrl())).as_deref(), Some("\x01"));
    }

    #[test]
    fn alt_prefixes_an_escape() {
        assert_eq!(encode(&character('a', alt())).as_deref(), Some("\x1ba"));
        assert_eq!(encode(&key(Key::Enter, alt())).as_deref(), Some("\x1b\r"));
    }

    #[test]
    fn alt_gr_is_neither_ctrl_nor_alt() {
        let modifiers = Modifiers {
            left_ctrl: true,
            right_alt: true,
            ..Modifiers::default()
        };
        let event = KeyEvent {
            alt_gr: true,
            ..character('€', modifiers)
        };
        assert_eq!(encode(&event).as_deref(), Some("€"));
    }

    #[test]
    fn modifier_keys_are_dropped_unless_every_key_is_reported() {
        assert_eq!(encode(&KeyEvent::down(Key::Shift)), None);

        let modes = InputModes {
            kitty_flags: KITTY_REPORT_ALL_KEYS,
            ..InputModes::default()
        };
        assert_eq!(
            TerminalInput::new()
                .handle_key(&KeyEvent::down(Key::LeftShift), &modes)
                .as_deref(),
            Some("\x1b[57441u")
        );
    }

    #[test]
    fn auto_repeat_can_be_suppressed() {
        let mut event = character('a', Modifiers::default());
        event.repeat = true;
        assert_eq!(encode(&event).as_deref(), Some("a"));

        let modes = InputModes {
            auto_repeat: false,
            ..InputModes::default()
        };
        assert_eq!(TerminalInput::new().handle_key(&event, &modes), None);
    }

    #[test]
    fn an_alt_numpad_composition_key_is_swallowed() {
        assert_eq!(encode(&key(Key::Numpad(1), alt())), None);

        // The Alt release is dropped by the modifier early-return before the
        // composition path runs, exactly as in upstream.
        let release = KeyEvent {
            modifiers: Modifiers {
                num_lock: true,
                ..Modifiers::default()
            },
            codepoint: Some('5'),
            ..KeyEvent::up(Key::Alt)
        };
        assert_eq!(encode(&release), None);
    }

    #[test]
    fn kitty_disambiguates_escape_and_modified_ascii() {
        let modes = InputModes {
            kitty_flags: KITTY_DISAMBIGUATE,
            ..InputModes::default()
        };
        let input = TerminalInput::new();
        assert_eq!(
            input.handle_key(&KeyEvent::down(Key::Escape), &modes).as_deref(),
            Some("\x1b[27u")
        );
        assert_eq!(
            input.handle_key(&character('a', ctrl()), &modes).as_deref(),
            Some("\x1b[97;5u")
        );
        assert_eq!(
            input.handle_key(&key(Key::Tab, shift()), &modes).as_deref(),
            Some("\x1b[9;2u")
        );
        // An unmodified text key stays text.
        assert_eq!(
            input
                .handle_key(&character('a', Modifiers::default()), &modes)
                .as_deref(),
            Some("a")
        );
    }

    #[test]
    fn kitty_reports_event_types() {
        let modes = InputModes {
            kitty_flags: KITTY_REPORT_EVENT_TYPES,
            ..InputModes::default()
        };
        let input = TerminalInput::new();
        let release = KeyEvent {
            codepoint: Some('a'),
            ..KeyEvent::up(Key::Char('a'))
        };
        assert_eq!(input.handle_key(&release, &modes).as_deref(), Some("\x1b[97;1:3u"));

        let mut repeat = character('a', Modifiers::default());
        repeat.repeat = true;
        // Only a release escalates a text key to `CSI u`; a repeat leaves it as
        // text, as upstream does.
        assert_eq!(input.handle_key(&repeat, &modes).as_deref(), Some("a"));

        // A repeat does carry the event type when the sequence is a kitty one
        // already, which is what DisambiguateEscapeCodes does for Escape.
        let both = InputModes {
            kitty_flags: KITTY_DISAMBIGUATE | KITTY_REPORT_EVENT_TYPES,
            ..InputModes::default()
        };
        let mut escape = KeyEvent::down(Key::Escape);
        escape.repeat = true;
        assert_eq!(input.handle_key(&escape, &both).as_deref(), Some("\x1b[27;1:2u"));
    }

    #[test]
    fn kitty_reports_every_key_as_an_escape_code() {
        let modes = InputModes {
            kitty_flags: KITTY_REPORT_ALL_KEYS,
            ..InputModes::default()
        };
        let input = TerminalInput::new();
        assert_eq!(
            input
                .handle_key(&character('a', Modifiers::default()), &modes)
                .as_deref(),
            Some("\x1b[97u")
        );
        assert_eq!(
            input.handle_key(&KeyEvent::down(Key::Function(13)), &modes).as_deref(),
            Some("\x1b[57376u")
        );
        assert_eq!(
            input.handle_key(&KeyEvent::down(Key::Numpad(0)), &modes).as_deref(),
            Some("\x1b[57399u")
        );
    }

    #[test]
    fn kitty_reports_associated_text() {
        let modes = InputModes {
            kitty_flags: KITTY_DISAMBIGUATE | KITTY_REPORT_ALL_KEYS | KITTY_REPORT_ASSOCIATED_TEXT,
            ..InputModes::default()
        };
        assert_eq!(
            TerminalInput::new()
                .handle_key(&character('a', ctrl()), &modes)
                .as_deref(),
            Some("\x1b[97;5;97u")
        );
    }

    #[test]
    fn kitty_reports_the_shifted_and_base_keys() {
        let modes = InputModes {
            kitty_flags: KITTY_DISAMBIGUATE | KITTY_REPORT_ALTERNATE_KEYS | KITTY_REPORT_ALL_KEYS,
            ..InputModes::default()
        };
        let input = TerminalInput::new();
        assert_eq!(
            input.handle_key(&character('a', shift()), &modes).as_deref(),
            Some("\x1b[97:65;2u")
        );

        // A layout whose shifted key differs from the US layout reports it, and
        // a physical key that is not what the layout produced is the third
        // parameter.
        let event = KeyEvent {
            key: Key::Char('z'),
            codepoint: Some('W'),
            base_codepoint: Some('w'),
            shifted_codepoint: Some('W'),
            modifiers: shift(),
            ..KeyEvent::down(Key::Unidentified)
        };
        assert_eq!(input.handle_key(&event, &modes).as_deref(), Some("\x1b[119:87:122;2u"));
    }

    #[test]
    fn win32_input_mode_reports_the_key_record() {
        let modes = InputModes {
            win32_input: true,
            ..InputModes::default()
        };
        let event = KeyEvent {
            virtual_key: 0x41,
            scan_code: 0x1E,
            ..character('a', shift())
        };
        assert_eq!(
            TerminalInput::new().handle_key(&event, &modes).as_deref(),
            Some("\x1b[65;30;97;1;16;1_")
        );
    }

    #[test]
    fn win32_input_mode_yields_to_the_kitty_protocol() {
        let modes = InputModes {
            win32_input: true,
            kitty_flags: KITTY_DISAMBIGUATE,
            ..InputModes::default()
        };
        assert_eq!(
            TerminalInput::new()
                .handle_key(&character('a', ctrl()), &modes)
                .as_deref(),
            Some("\x1b[97;5u")
        );
    }

    #[test]
    fn focus_is_reported_only_when_the_mode_is_set() {
        let input = TerminalInput::new();
        assert_eq!(input.handle_focus(true, &InputModes::default()), None);

        let modes = InputModes {
            focus_event: true,
            ..InputModes::default()
        };
        assert_eq!(input.handle_focus(true, &modes).as_deref(), Some("\x1b[I"));
        assert_eq!(input.handle_focus(false, &modes).as_deref(), Some("\x1b[O"));
    }

    #[test]
    fn s8c1t_sends_single_byte_introducers() {
        let modes = InputModes {
            send_c1: true,
            ..InputModes::default()
        };
        let input = TerminalInput::new();
        assert_eq!(
            input.handle_key(&key(Key::Left, ctrl()), &modes).as_deref(),
            Some("\u{9b}1;5D")
        );
        assert_eq!(
            input.handle_key(&KeyEvent::down(Key::Function(1)), &modes).as_deref(),
            Some("\u{8f}P")
        );
        assert_eq!(
            input.handle_key(&KeyEvent::down(Key::Enter), &modes).as_deref(),
            Some("\r")
        );

        let kitty = InputModes {
            send_c1: true,
            kitty_flags: KITTY_DISAMBIGUATE,
            ..InputModes::default()
        };
        assert_eq!(
            input.handle_key(&KeyEvent::down(Key::Escape), &kitty).as_deref(),
            Some("\u{9b}27u")
        );
    }

    #[test]
    fn a_release_alone_is_not_reported() {
        assert_eq!(encode(&KeyEvent::up(Key::Char('a'))), None);
    }

    #[test]
    fn modifier_recognition_matches_upstream() {
        for modifier in [
            Key::Shift,
            Key::LeftShift,
            Key::RightShift,
            Key::Control,
            Key::LeftControl,
            Key::RightControl,
            Key::Alt,
            Key::LeftAlt,
            Key::RightAlt,
        ] {
            assert!(modifier.is_modifier());
        }
        assert!(!Key::Super.is_modifier());
        assert!(!Key::Char('a').is_modifier());
    }
}
