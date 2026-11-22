//! The console's registry settings, ported from `registry.cpp` and
//! `RegistrySerialization.cpp`.
//!
//! Upstream reads `HKCU\Console` (and the subkey named after the title) into
//! the `Settings` object, coercing each value by the type its mapping declares
//! and handling a handful of keys by hand. The Win32 calls are the server
//! layer's; the schema, the coercion and the quirks are portable and live here,
//! behind a [`Values`] source a test can fill from a map (ADR 037).
//!
//! Not ported, because this port's `Settings` has no field for them: the
//! answerback message, `UseDx`, `CopyColor`, `TextMeasurement`, `MSAADelay`,
//! `UIADelay` and `EnableBuiltinGlyphs`. The code page's East Asian rule needs
//! the host's OEM code page, which a portable console does not have; a key that
//! names a code page is taken as written.

use std::collections::BTreeMap;

use types::CursorType;
use types::color_table::{CURSOR_COLOR, ColorTable, DEFAULT_BACKGROUND, DEFAULT_FOREGROUND};
use types::text_color::TextColor;
use utils::{CoordType, Point, Size};

/// One value a console key may hold.
///
/// The type matters: upstream asks the registry for a `REG_DWORD` or a `REG_SZ`
/// and ignores a value of the other kind, which is how the word-delimiter quirk
/// below is detected.
#[derive(Debug, Clone, PartialEq, Eq)]
pub enum Value {
    /// A `REG_DWORD`.
    Dword(u32),
    /// A `REG_SZ`.
    String(String),
}

/// Where the console's registry values come from.
pub trait Values {
    /// The value named `name`, if the key holds one.
    fn value(&self, name: &str) -> Option<Value>;
}

/// An in-memory key, for tests and for a host with no registry.
#[derive(Debug, Default, Clone, PartialEq, Eq)]
pub struct Map(BTreeMap<String, Value>);

impl Map {
    /// An empty key.
    pub fn new() -> Self {
        Self::default()
    }

    /// A key holding a `REG_DWORD`.
    pub fn dword(mut self, name: &str, value: u32) -> Self {
        self.0.insert(name.to_string(), Value::Dword(value));
        self
    }

    /// A key holding a `REG_SZ`.
    pub fn string(mut self, name: &str, value: &str) -> Self {
        self.0.insert(name.to_string(), Value::String(value.to_string()));
        self
    }
}

impl Values for Map {
    fn value(&self, name: &str) -> Option<Value> {
        self.0.get(name).cloned()
    }
}

/// The word delimiters a key asks for when it names none.
///
/// The space is a delimiter too, but it is handled by the word-selection code
/// and is never part of this list, which is why it is absent here.
pub const DEFAULT_WORD_DELIMITERS: &[char] = &['\\', '+', '!', ':', '=', '/', '.', '<', '>', ';', '|', '&'];

/// What a console key holds, with an absent value left absent.
///
/// Every field is an `Option` because upstream's rule is that a key the registry
/// does not hold leaves the constructor's default in place.
#[derive(Debug, Default, Clone, PartialEq)]
pub struct ConsoleSettings {
    /// `PopupColors`, a `WORD`.
    pub popup_fill_attribute: Option<u16>,
    /// `InsertMode`.
    pub insert_mode: Option<bool>,
    /// `LineSelection`.
    pub line_selection: Option<bool>,
    /// `FilterOnPaste`.
    pub filter_on_paste: Option<bool>,
    /// `LineWrap`.
    pub wrap_text: Option<bool>,
    /// `CtrlKeyShortcutsDisabled`.
    pub ctrl_key_shortcuts_disabled: Option<bool>,
    /// `QuickEdit`.
    pub quick_edit: Option<bool>,
    /// `WindowAlpha`, the low byte of the value.
    pub window_alpha: Option<u8>,
    /// `FontSize`, as a coordinate.
    pub font_size: Option<Size>,
    /// `FontFamily`.
    pub font_family: Option<u32>,
    /// `FontWeight`.
    pub font_weight: Option<u32>,
    /// `FaceName`.
    pub face_name: Option<String>,
    /// `CursorSize`.
    pub cursor_size: Option<u32>,
    /// `HistoryBufferSize`.
    pub history_buffer_size: Option<u32>,
    /// `NumberOfHistoryBuffers`.
    pub number_of_history_buffers: Option<u32>,
    /// `HistoryNoDup`.
    pub history_no_dup: Option<bool>,
    /// `ScrollScale`.
    pub scroll_scale: Option<u32>,
    /// `ScreenColors`, a `WORD`.
    pub fill_attribute: Option<u16>,
    /// `ScreenBufferSize`, as a coordinate.
    pub screen_buffer_size: Option<Size>,
    /// `WindowSize`, as a coordinate.
    pub window_size: Option<Size>,
    /// `TrimLeadingZeros`.
    pub trim_leading_zeros: Option<bool>,
    /// `EnableColorSelection`.
    pub enable_color_selection: Option<bool>,
    /// `WindowPosition`: its presence means the window is not placed by the
    /// console, whatever the coordinate says.
    pub window_origin: Option<Point>,
    /// `CursorType`, when the value names one of the six shapes.
    pub cursor_type: Option<CursorType>,
    /// `InterceptCopyPaste`.
    pub intercept_copy_paste: Option<bool>,
    /// `TerminalScrolling`.
    pub terminal_scrolling: Option<bool>,
    /// `VirtualTerminalLevel`, from the global key.
    pub virt_term_level: Option<u32>,
    /// `CodePage`.
    pub code_page: Option<u32>,
    /// `AllowAltF4Close`; a value above one is ignored, as upstream does.
    pub allow_alt_f4_close: Option<bool>,
    /// `ColorTable00` to `ColorTable15`, as `COLORREF` values.
    pub ansi: [Option<u32>; 16],
    /// `DefaultForeground`.
    pub default_foreground: Option<u32>,
    /// `DefaultBackground`.
    pub default_background: Option<u32>,
    /// `CursorColor`.
    pub cursor_colour: Option<u32>,
    /// The delimiters the key asks for, already resolved.
    pub word_delimiters: Vec<char>,
}

/// Reads a console key.
pub fn load(values: &dyn Values) -> ConsoleSettings {
    ConsoleSettings {
        popup_fill_attribute: word(values, "PopupColors"),
        insert_mode: boolean(values, "InsertMode"),
        line_selection: boolean(values, "LineSelection"),
        filter_on_paste: boolean(values, "FilterOnPaste"),
        wrap_text: boolean(values, "LineWrap"),
        ctrl_key_shortcuts_disabled: boolean(values, "CtrlKeyShortcutsDisabled"),
        quick_edit: boolean(values, "QuickEdit"),
        window_alpha: dword(values, "WindowAlpha").map(|value| value as u8),
        font_size: coordinate(values, "FontSize"),
        font_family: dword(values, "FontFamily"),
        font_weight: dword(values, "FontWeight"),
        face_name: string(values, "FaceName"),
        cursor_size: dword(values, "CursorSize"),
        history_buffer_size: dword(values, "HistoryBufferSize"),
        number_of_history_buffers: dword(values, "NumberOfHistoryBuffers"),
        history_no_dup: boolean(values, "HistoryNoDup"),
        scroll_scale: dword(values, "ScrollScale"),
        fill_attribute: word(values, "ScreenColors"),
        screen_buffer_size: coordinate(values, "ScreenBufferSize"),
        window_size: coordinate(values, "WindowSize"),
        trim_leading_zeros: boolean(values, "TrimLeadingZeros"),
        enable_color_selection: boolean(values, "EnableColorSelection"),
        window_origin: coordinate_point(values, "WindowPosition"),
        cursor_type: dword(values, "CursorType").and_then(cursor_type),
        intercept_copy_paste: boolean(values, "InterceptCopyPaste"),
        terminal_scrolling: boolean(values, "TerminalScrolling"),
        virt_term_level: dword(values, "VirtualTerminalLevel"),
        code_page: dword(values, "CodePage"),
        allow_alt_f4_close: dword(values, "AllowAltF4Close")
            .filter(|value| *value <= 1)
            .map(|value| value == 1),
        ansi: std::array::from_fn(|index| dword(values, &format!("ColorTable{index:02}"))),
        default_foreground: dword(values, "DefaultForeground"),
        default_background: dword(values, "DefaultBackground"),
        cursor_colour: dword(values, "CursorColor"),
        word_delimiters: word_delimiters(values),
    }
}

/// The word delimiters the key asks for.
///
/// The value was `REG_DWORD` when it should have been `REG_SZ`, and Windows read
/// whatever the buffer held rather than the value, which meant "no delimiters".
/// Upstream keeps that behaviour: a DWORD means none, a string means its
/// characters, and a missing value means the defaults.
pub fn word_delimiters(values: &dyn Values) -> Vec<char> {
    match values.value("WordDelimiters") {
        Some(Value::Dword(_)) => Vec::new(),
        Some(Value::String(text)) => text.chars().take_while(|character| *character != '\0').collect(),
        None => DEFAULT_WORD_DELIMITERS.to_vec(),
    }
}

impl ConsoleSettings {
    /// Writes the colour entries onto `table`.
    ///
    /// A registry colour is a `COLORREF` (`0x00BBGGRR`), which is the form
    /// `TextColor::from_colorref` reads.
    pub fn apply_colours(&self, table: &mut ColorTable) {
        for (index, entry) in self.ansi.iter().enumerate() {
            if let Some(value) = entry {
                table.set(index, TextColor::from_colorref(*value));
            }
        }
        if let Some(value) = self.default_foreground {
            table.set(DEFAULT_FOREGROUND, TextColor::from_colorref(value));
        }
        if let Some(value) = self.default_background {
            table.set(DEFAULT_BACKGROUND, TextColor::from_colorref(value));
        }
        if let Some(value) = self.cursor_colour {
            table.set(CURSOR_COLOR, TextColor::from_colorref(value));
        }
    }
}

fn dword(values: &dyn Values, name: &str) -> Option<u32> {
    match values.value(name) {
        Some(Value::Dword(value)) => Some(value),
        _ => None,
    }
}

fn word(values: &dyn Values, name: &str) -> Option<u16> {
    dword(values, name).map(|value| value as u16)
}

fn boolean(values: &dyn Values, name: &str) -> Option<bool> {
    dword(values, name).map(|value| value != 0)
}

fn string(values: &dyn Values, name: &str) -> Option<String> {
    match values.value(name) {
        Some(Value::String(value)) => Some(value),
        _ => None,
    }
}

/// A coordinate: the low word is x, the high word is y.
fn size(value: u32) -> Size {
    Size::new(
        CoordType::from(value as u16 as i16),
        CoordType::from((value >> 16) as u16 as i16),
    )
}

fn coordinate(values: &dyn Values, name: &str) -> Option<Size> {
    dword(values, name).map(size)
}

fn coordinate_point(values: &dyn Values, name: &str) -> Option<Point> {
    let size = coordinate(values, name)?;
    let (width, height) = size.into_tuple();
    Some(Point { x: width, y: height })
}

fn cursor_type(value: u32) -> Option<CursorType> {
    Some(match value {
        0 => CursorType::Legacy,
        1 => CursorType::VerticalBar,
        2 => CursorType::Underscore,
        3 => CursorType::EmptyBox,
        4 => CursorType::FullBox,
        5 => CursorType::DoubleUnderscore,
        _ => return None,
    })
}

#[cfg(test)]
mod tests {
    use super::*;

    fn full() -> Map {
        Map::new()
            .dword("PopupColors", 0x5F)
            .dword("InsertMode", 0)
            .dword("LineSelection", 0)
            .dword("FilterOnPaste", 1)
            .dword("LineWrap", 0)
            .dword("CtrlKeyShortcutsDisabled", 1)
            .dword("QuickEdit", 0)
            .dword("WindowAlpha", 0x1FF)
            .dword("FontSize", 16 << 16)
            .dword("FontFamily", 54)
            .dword("FontWeight", 400)
            .string("FaceName", "Consolas")
            .dword("CursorSize", 25)
            .dword("HistoryBufferSize", 50)
            .dword("NumberOfHistoryBuffers", 4)
            .dword("HistoryNoDup", 1)
            .dword("ScrollScale", 3)
            .dword("ScreenColors", 0x07)
            .dword("ScreenBufferSize", (25 << 16) | 80)
            .dword("WindowSize", (30 << 16) | 120)
            .dword("TrimLeadingZeros", 1)
            .dword("EnableColorSelection", 1)
            .dword("WindowPosition", (10 << 16) | 20)
            .dword("CursorType", 4)
            .dword("InterceptCopyPaste", 1)
            .dword("TerminalScrolling", 0)
            .dword("VirtualTerminalLevel", 1)
            .dword("CodePage", 65001)
            .dword("AllowAltF4Close", 0)
            .dword("ColorTable01", 0x0000FF)
            .dword("DefaultForeground", 0x00C0C0C0)
            .dword("DefaultBackground", 0x00000000)
            .dword("CursorColor", 0x00FFFFFF)
    }

    #[test]
    fn every_mapped_value_is_coerced_to_its_field() {
        let settings = load(&full());

        assert_eq!(settings.popup_fill_attribute, Some(0x5F));
        assert_eq!(settings.insert_mode, Some(false));
        assert_eq!(settings.line_selection, Some(false));
        assert_eq!(settings.filter_on_paste, Some(true));
        assert_eq!(settings.wrap_text, Some(false));
        assert_eq!(settings.ctrl_key_shortcuts_disabled, Some(true));
        assert_eq!(settings.quick_edit, Some(false));
        // A byte keeps the low eight bits.
        assert_eq!(settings.window_alpha, Some(0xFF));
        assert_eq!(settings.font_size, Some(Size::new(0, 16)));
        assert_eq!(settings.font_family, Some(54));
        assert_eq!(settings.font_weight, Some(400));
        assert_eq!(settings.face_name.as_deref(), Some("Consolas"));
        assert_eq!(settings.cursor_size, Some(25));
        assert_eq!(settings.history_buffer_size, Some(50));
        assert_eq!(settings.number_of_history_buffers, Some(4));
        assert_eq!(settings.history_no_dup, Some(true));
        assert_eq!(settings.scroll_scale, Some(3));
        assert_eq!(settings.fill_attribute, Some(0x07));
        assert_eq!(settings.screen_buffer_size, Some(Size::new(80, 25)));
        assert_eq!(settings.window_size, Some(Size::new(120, 30)));
        assert_eq!(settings.trim_leading_zeros, Some(true));
        assert_eq!(settings.enable_color_selection, Some(true));
        assert_eq!(settings.window_origin, Some(Point { x: 20, y: 10 }));
        assert_eq!(settings.cursor_type, Some(CursorType::FullBox));
        assert_eq!(settings.intercept_copy_paste, Some(true));
        assert_eq!(settings.terminal_scrolling, Some(false));
        assert_eq!(settings.virt_term_level, Some(1));
        assert_eq!(settings.code_page, Some(65001));
        assert_eq!(settings.allow_alt_f4_close, Some(false));
    }

    #[test]
    fn an_empty_key_leaves_every_value_absent() {
        let settings = load(&Map::new());

        assert_eq!(settings.popup_fill_attribute, None);
        assert_eq!(settings.face_name, None);
        assert_eq!(settings.window_origin, None);
        assert_eq!(settings.cursor_type, None);
        assert_eq!(settings.ansi, [None; 16]);
        // The delimiters are the one value that is always resolved.
        assert_eq!(settings.word_delimiters, DEFAULT_WORD_DELIMITERS.to_vec());
    }

    #[test]
    fn a_value_of_the_wrong_type_is_ignored() {
        // Upstream asks the registry for a REG_DWORD and gets nothing when the
        // value is a string, and the other way round.
        let settings = load(
            &Map::new()
                .string("InsertMode", "yes")
                .dword("FaceName", 1)
                .string("WindowSize", "120x30"),
        );

        assert_eq!(settings.insert_mode, None);
        assert_eq!(settings.face_name, None);
        assert_eq!(settings.window_size, None);
    }

    #[test]
    fn a_word_keeps_the_low_bits_and_a_coordinate_keeps_both() {
        let settings = load(&Map::new().dword("PopupColors", 0xFFFF_00F5).dword("FontSize", 0));

        assert_eq!(settings.popup_fill_attribute, Some(0x00F5));
        assert_eq!(settings.font_size, Some(Size::new(0, 0)));
    }

    #[test]
    fn a_cursor_type_outside_the_six_shapes_is_ignored() {
        assert_eq!(
            load(&Map::new().dword("CursorType", 5)).cursor_type,
            Some(CursorType::DoubleUnderscore)
        );
        assert_eq!(load(&Map::new().dword("CursorType", 6)).cursor_type, None);
    }

    #[test]
    fn alt_f4_close_only_takes_nought_or_one() {
        assert_eq!(
            load(&Map::new().dword("AllowAltF4Close", 0)).allow_alt_f4_close,
            Some(false)
        );
        assert_eq!(
            load(&Map::new().dword("AllowAltF4Close", 1)).allow_alt_f4_close,
            Some(true)
        );
        assert_eq!(load(&Map::new().dword("AllowAltF4Close", 2)).allow_alt_f4_close, None);
    }

    #[test]
    fn a_dword_word_delimiter_key_means_none() {
        // The load-bearing quirk: the key's type was wrong for years, and the
        // buffer Windows read was empty, so a DWORD means "no delimiters".
        assert!(word_delimiters(&Map::new().dword("WordDelimiters", 0x20)).is_empty());
    }

    #[test]
    fn a_string_word_delimiter_key_means_its_characters() {
        let values = Map::new().string("WordDelimiters", "abc\0def");
        assert_eq!(word_delimiters(&values), vec!['a', 'b', 'c']);
    }

    #[test]
    fn a_missing_word_delimiter_key_means_the_defaults() {
        assert_eq!(word_delimiters(&Map::new()), DEFAULT_WORD_DELIMITERS.to_vec());
    }

    #[test]
    fn the_colour_entries_are_written_as_colorrefs() {
        let settings = load(&full());
        let mut table = ColorTable::new();

        settings.apply_colours(&mut table);

        assert_eq!(table.get(1), Some(TextColor::rgb(0xFF, 0, 0)));
        assert_eq!(table.get(DEFAULT_FOREGROUND), Some(TextColor::rgb(0xC0, 0xC0, 0xC0)));
        assert_eq!(table.get(DEFAULT_BACKGROUND), Some(TextColor::rgb(0, 0, 0)));
        assert_eq!(table.get(CURSOR_COLOR), Some(TextColor::rgb(0xFF, 0xFF, 0xFF)));
        // A colour the key does not name keeps the built-in entry.
        assert_eq!(table.get(2), ColorTable::new().get(2));
    }
}
