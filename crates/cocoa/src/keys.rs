//! macOS virtual-key codes and the terminal vocabulary.
//!
//! AppKit reports a physical `keyCode` and the characters the active layout
//! produced. A printable key is typed by its characters, but a chord
//! (Control, Option, Command) needs the key's identity, and so do the arrow and
//! function keys. This module is the one table that turns an AppKit key code
//! into `term`'s [`Key`], so the shell and its tests agree.
//!
//! The codes are Carbon's `kVK_*` constants (`Events.h`), which AppKit still
//! reports as `NSEvent.keyCode`; a US keyboard's table is the base.

use term::input::Key;

/// The key a virtual key code names, or `None` for a key with no terminal
/// meaning (a media key, the Help key, the numpad's Clear).
pub(crate) const fn key_from_code(code: u16) -> Option<Key> {
    Some(match code {
        0 => Key::Char('a'),
        1 => Key::Char('s'),
        2 => Key::Char('d'),
        3 => Key::Char('f'),
        4 => Key::Char('h'),
        5 => Key::Char('g'),
        6 => Key::Char('z'),
        7 => Key::Char('x'),
        8 => Key::Char('c'),
        9 => Key::Char('v'),
        11 => Key::Char('b'),
        12 => Key::Char('q'),
        13 => Key::Char('w'),
        14 => Key::Char('e'),
        15 => Key::Char('r'),
        16 => Key::Char('y'),
        17 => Key::Char('t'),
        18 => Key::Char('1'),
        19 => Key::Char('2'),
        20 => Key::Char('3'),
        21 => Key::Char('4'),
        22 => Key::Char('6'),
        23 => Key::Char('5'),
        24 => Key::Char('='),
        25 => Key::Char('9'),
        26 => Key::Char('7'),
        27 => Key::Char('-'),
        28 => Key::Char('8'),
        29 => Key::Char('0'),
        30 => Key::Char(']'),
        31 => Key::Char('o'),
        32 => Key::Char('u'),
        33 => Key::Char('['),
        34 => Key::Char('i'),
        35 => Key::Char('p'),
        36 => Key::Enter,
        37 => Key::Char('l'),
        38 => Key::Char('j'),
        39 => Key::Char('\''),
        40 => Key::Char('k'),
        41 => Key::Char(';'),
        42 => Key::Char('\\'),
        43 => Key::Char(','),
        44 => Key::Char('/'),
        45 => Key::Char('n'),
        46 => Key::Char('m'),
        47 => Key::Char('.'),
        48 => Key::Tab,
        49 => Key::Space,
        50 => Key::Char('`'),
        51 => Key::Backspace,
        53 => Key::Escape,
        54 => Key::RightSuper,
        55 => Key::LeftSuper,
        56 => Key::LeftShift,
        57 => Key::CapsLock,
        58 => Key::LeftAlt,
        59 => Key::LeftControl,
        60 => Key::RightShift,
        61 => Key::RightAlt,
        62 => Key::RightControl,
        64 => Key::Function(17),
        65 => Key::NumpadDecimal,
        67 => Key::NumpadMultiply,
        69 => Key::NumpadAdd,
        71 => Key::Clear,
        75 => Key::NumpadDivide,
        76 => Key::NumpadEnter,
        78 => Key::NumpadSubtract,
        79 => Key::Function(18),
        80 => Key::Function(19),
        81 => Key::NumpadSeparator,
        82 => Key::Numpad(0),
        83 => Key::Numpad(1),
        84 => Key::Numpad(2),
        85 => Key::Numpad(3),
        86 => Key::Numpad(4),
        87 => Key::Numpad(5),
        88 => Key::Numpad(6),
        89 => Key::Numpad(7),
        91 => Key::Numpad(8),
        92 => Key::Numpad(9),
        96 => Key::Function(5),
        97 => Key::Function(6),
        98 => Key::Function(7),
        99 => Key::Function(3),
        100 => Key::Function(8),
        101 => Key::Function(9),
        103 => Key::Function(11),
        105 => Key::Function(13),
        106 => Key::Function(16),
        107 => Key::Function(14),
        109 => Key::Function(10),
        111 => Key::Function(12),
        113 => Key::Function(15),
        115 => Key::Home,
        116 => Key::PageUp,
        117 => Key::Delete,
        118 => Key::Function(4),
        119 => Key::End,
        120 => Key::Function(2),
        121 => Key::PageDown,
        122 => Key::Function(1),
        123 => Key::Left,
        124 => Key::Right,
        125 => Key::Down,
        126 => Key::Up,
        _ => return None,
    })
}

/// The key a control's command selector names, for the text fields.
pub(crate) const fn modifier_key(code: u16) -> Option<Key> {
    Some(match code {
        54 => Key::RightSuper,
        55 => Key::LeftSuper,
        56 => Key::LeftShift,
        57 => Key::CapsLock,
        58 => Key::LeftAlt,
        59 => Key::LeftControl,
        60 => Key::RightShift,
        61 => Key::RightAlt,
        62 => Key::RightControl,
        _ => return None,
    })
}

/// The key an event names, preferring the layout's character for a printable
/// key so a chord follows the user's layout rather than the US table.
///
/// `ignoring` is `charactersIgnoringModifiers`, which is the unshifted
/// character of the physical key in the active layout. Control and Option can
/// leave a control character or nothing there, so the physical table is the
/// fallback.
pub(crate) fn key_of(code: u16, ignoring: Option<&str>) -> Option<Key> {
    let physical = key_from_code(code);

    // Special keys are identified by their code, never by their characters.
    if physical.is_some_and(|key| !matches!(key, Key::Char(_) | Key::Space)) {
        return physical;
    }

    let mut characters = ignoring.unwrap_or_default().chars();
    if let (Some(character), None) = (characters.next(), characters.next())
        && !character.is_control()
    {
        return Some(if character == ' ' {
            Key::Space
        } else {
            Key::Char(character)
        });
    }

    physical
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn the_letters_and_digits_are_the_us_table() {
        assert_eq!(key_from_code(0), Some(Key::Char('a')));
        assert_eq!(key_from_code(8), Some(Key::Char('c')));
        assert_eq!(key_from_code(49), Some(Key::Space));
        assert_eq!(key_from_code(29), Some(Key::Char('0')));
        assert_eq!(key_from_code(24), Some(Key::Char('=')));
    }

    #[test]
    fn the_editing_and_function_keys_are_named() {
        assert_eq!(key_from_code(36), Some(Key::Enter));
        assert_eq!(key_from_code(48), Some(Key::Tab));
        assert_eq!(key_from_code(51), Some(Key::Backspace));
        assert_eq!(key_from_code(53), Some(Key::Escape));
        assert_eq!(key_from_code(117), Some(Key::Delete));
        assert_eq!(key_from_code(123), Some(Key::Left));
        assert_eq!(key_from_code(126), Some(Key::Up));
        assert_eq!(key_from_code(122), Some(Key::Function(1)));
        assert_eq!(key_from_code(120), Some(Key::Function(2)));
        assert_eq!(key_from_code(99), Some(Key::Function(3)));
    }

    #[test]
    fn the_modifier_sides_are_distinguished() {
        assert_eq!(key_from_code(56), Some(Key::LeftShift));
        assert_eq!(key_from_code(60), Some(Key::RightShift));
        assert_eq!(key_from_code(58), Some(Key::LeftAlt));
        assert_eq!(key_from_code(61), Some(Key::RightAlt));
        assert_eq!(key_from_code(59), Some(Key::LeftControl));
        assert_eq!(key_from_code(62), Some(Key::RightControl));
        assert_eq!(key_from_code(55), Some(Key::LeftSuper));
        assert_eq!(key_from_code(54), Some(Key::RightSuper));
    }

    #[test]
    fn the_numpad_is_its_own_key() {
        assert_eq!(key_from_code(82), Some(Key::Numpad(0)));
        assert_eq!(key_from_code(92), Some(Key::Numpad(9)));
        assert_eq!(key_from_code(76), Some(Key::NumpadEnter));
        assert_eq!(key_from_code(65), Some(Key::NumpadDecimal));
    }

    #[test]
    fn an_unknown_code_has_no_key() {
        assert_eq!(key_from_code(63), None);
        assert_eq!(key_from_code(10_000), None);
    }

    #[test]
    fn a_layout_character_wins_over_the_physical_table() {
        // Dvorak puts `x` on the key marked `q`.
        assert_eq!(key_of(12, Some("x")), Some(Key::Char('x')));
        assert_eq!(key_of(12, None), Some(Key::Char('q')));
    }

    #[test]
    fn a_special_key_keeps_its_identity_whatever_the_characters() {
        assert_eq!(key_of(36, Some("\r")), Some(Key::Enter));
        assert_eq!(key_of(123, Some("\u{f702}")), Some(Key::Left));
    }

    #[test]
    fn a_control_character_falls_back_to_the_physical_key() {
        // Control+C reports `\u{3}` as its unmodified character.
        assert_eq!(key_of(8, Some("\u{3}")), Some(Key::Char('c')));
    }
}
