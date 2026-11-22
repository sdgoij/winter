//! `ColorTable` — the VT colour table and DEC colour conversions.
//!
//! Ported from `colorTable.cpp` and the colour helpers in `utils.cpp`: the
//! Campbell ANSI palette, the xterm 6×6×6 cube and greys for entries 16–255,
//! the reserved alias slots, and the DEC HLS/percentage conversions that
//! DECRQTSR and DECRSTS use.
//!
//! Upstream keeps this in `RenderSettings`; the entries and alias indices are
//! the same, but the render-mode flags stay with the renderer when it exists.

use crate::text_color::TextColor;

/// The number of table entries, including the reserved alias slots.
pub const TABLE_SIZE: usize = 267;

/// Table slot aliased to the default foreground.
pub const DEFAULT_FOREGROUND: usize = 261;
/// Table slot aliased to the default background.
pub const DEFAULT_BACKGROUND: usize = 262;
/// Table slot aliased to the frame foreground.
pub const FRAME_FOREGROUND: usize = 263;
/// Table slot aliased to the frame background.
pub const FRAME_BACKGROUND: usize = 264;
/// Table slot for the cursor colour.
pub const CURSOR_COLOR: usize = 265;
/// Table slot for the selection background.
pub const SELECTION_BACKGROUND: usize = 266;

/// The Campbell palette, the default for entries 0–15.
const CAMPBELL: [(u8, u8, u8); 16] = [
    (0x0C, 0x0C, 0x0C),
    (0xC5, 0x0F, 0x1F),
    (0x13, 0xA1, 0x0E),
    (0xC1, 0x9C, 0x00),
    (0x00, 0x37, 0xDA),
    (0x88, 0x17, 0x98),
    (0x3A, 0x96, 0xDD),
    (0xCC, 0xCC, 0xCC),
    (0x76, 0x76, 0x76),
    (0xE7, 0x48, 0x56),
    (0x16, 0xC6, 0x0C),
    (0xF9, 0xF1, 0xA5),
    (0x3B, 0x78, 0xFF),
    (0xB4, 0x00, 0x9E),
    (0x61, 0xD6, 0xD6),
    (0xF2, 0xF2, 0xF2),
];

/// A colour slot that a program can point at another table entry.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum ColorAlias {
    /// The default text foreground.
    DefaultForeground,
    /// The default text background.
    DefaultBackground,
    /// The window frame foreground.
    FrameForeground,
    /// The window frame background.
    FrameBackground,
}

impl ColorAlias {
    /// The number of aliases.
    pub const COUNT: usize = 4;

    /// The alias as an array index.
    pub const fn index(self) -> usize {
        match self {
            Self::DefaultForeground => 0,
            Self::DefaultBackground => 1,
            Self::FrameForeground => 2,
            Self::FrameBackground => 3,
        }
    }
}

/// The VT colour table: 256 palette entries, six reserved slots and four
/// aliases, with the unresolved slots left empty.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct ColorTable {
    entries: [Option<TextColor>; TABLE_SIZE],
    alias_indices: [usize; ColorAlias::COUNT],
}

impl ColorTable {
    /// Creates the default table.
    pub fn new() -> Self {
        let mut entries = [None; TABLE_SIZE];
        for (index, (red, green, blue)) in CAMPBELL.iter().enumerate() {
            entries[index] = Some(TextColor::rgb(*red, *green, *blue));
        }

        // Entries 16–231 are the 6×6×6 colour cube; 232–255 are greys.
        const SCALE: [u8; 6] = [0, 0x5F, 0x87, 0xAF, 0xD7, 0xFF];
        for i in 0..216 {
            let red = SCALE[(i / 36) % 6];
            let green = SCALE[(i / 6) % 6];
            let blue = SCALE[i % 6];
            entries[i + 16] = Some(TextColor::rgb(red, green, blue));
        }
        for i in 0..24 {
            let level = (i * 10 + 8) as u8;
            entries[i + 232] = Some(TextColor::rgb(level, level, level));
        }

        Self {
            entries,
            alias_indices: [
                TextColor::DARK_WHITE.get_index() as usize,
                TextColor::DARK_BLACK.get_index() as usize,
                FRAME_FOREGROUND,
                FRAME_BACKGROUND,
            ],
        }
    }

    /// Returns the colour at `index`, or None when the slot is unset.
    pub const fn get(&self, index: usize) -> Option<TextColor> {
        if index < TABLE_SIZE { self.entries[index] } else { None }
    }

    /// Sets a table entry. Out-of-range indices are ignored.
    pub fn set(&mut self, index: usize, color: TextColor) {
        if index < TABLE_SIZE {
            self.entries[index] = Some(color);
        }
    }

    /// Restores one entry to its default value or unset state.
    pub fn reset_entry(&mut self, index: usize) {
        if index < TABLE_SIZE {
            self.entries[index] = Self::default_entry(index);
        }
    }

    /// Restores the 256 programmable entries, which is what OSC 104 asks for.
    pub fn reset_indexed(&mut self) {
        for index in 0..256 {
            self.entries[index] = Self::default_entry(index);
        }
    }

    /// Restores every entry and alias, which is what a hard reset asks for.
    pub fn reset(&mut self) {
        *self = Self::new();
    }

    /// Returns the table slot an alias currently points at.
    pub const fn alias_index(&self, alias: ColorAlias) -> usize {
        self.alias_indices[alias.index()]
    }

    /// Points an alias at another table entry.
    pub fn set_alias_index(&mut self, alias: ColorAlias, index: usize) {
        if index < TABLE_SIZE {
            self.alias_indices[alias.index()] = index;
        }
    }

    /// Restores an alias to the slot it pointed at by default.
    pub fn restore_default_alias_index(&mut self, alias: ColorAlias) {
        self.alias_indices[alias.index()] = match alias {
            ColorAlias::DefaultForeground => TextColor::DARK_WHITE.get_index() as usize,
            ColorAlias::DefaultBackground => TextColor::DARK_BLACK.get_index() as usize,
            ColorAlias::FrameForeground => FRAME_FOREGROUND,
            ColorAlias::FrameBackground => FRAME_BACKGROUND,
        };
    }

    /// The default value of one slot.
    fn default_entry(index: usize) -> Option<TextColor> {
        match index {
            0..=15 => {
                let (red, green, blue) = CAMPBELL[index];
                Some(TextColor::rgb(red, green, blue))
            }
            16..=231 => {
                const SCALE: [u8; 6] = [0, 0x5F, 0x87, 0xAF, 0xD7, 0xFF];
                let i = index - 16;
                Some(TextColor::rgb(SCALE[(i / 36) % 6], SCALE[(i / 6) % 6], SCALE[i % 6]))
            }
            232..=255 => {
                let level = ((index - 232) * 10 + 8) as u8;
                Some(TextColor::rgb(level, level, level))
            }
            _ => None,
        }
    }
}

impl Default for ColorTable {
    fn default() -> Self {
        Self::new()
    }
}

/// Converts an RGB colour to DEC HLS components.
///
/// Ported from `Utils::ColorToHLS`. DEC's hue mapping puts blue at 0°, red at
/// 120° and green at 240° (see DEC STD 070, ReGIS §8.6.2.2.2).
pub fn color_to_hls(color: TextColor) -> (i32, i32, i32) {
    let red = f32::from(color.get_r()) / 255.0;
    let green = f32::from(color.get_g()) / 255.0;
    let blue = f32::from(color.get_b()) / 255.0;

    let max_comp = red.max(green).max(blue);
    let min_comp = red.min(green).min(blue);
    let chroma = max_comp - min_comp;
    let luma2 = max_comp + min_comp;

    let divisor = luma2.min(2.0 - luma2);
    let saturation = if divisor > 0.0 { chroma / divisor } else { 0.0 };

    let mut hue = 0.0;
    if chroma != 0.0 {
        if max_comp == red {
            hue = (green - blue) / chroma + 2.0;
        } else if max_comp == green {
            hue = (blue - red) / chroma + 4.0;
        } else {
            hue = (red - green) / chroma + 6.0;
        }
    }

    let h = ((hue * 60.0 + 0.5) as i32) % 360;
    let l = (luma2 * 50.0 + 0.5) as i32;
    let s = (saturation * 100.0 + 0.5) as i32;
    (h, l, s)
}

/// Converts DEC HLS components to an RGB colour.
///
/// Ported from `Utils::ColorFromHLS`; hue wraps at 360° and lightness and
/// saturation are clamped to 100.
pub fn color_from_hls(h: i32, l: i32, s: i32) -> TextColor {
    let hue = h.rem_euclid(360);
    let lum = l.min(100) as f32;
    let sat = s.min(100) as f32;

    let chroma = (50.0 - (lum - 50.0).abs()) * sat / 50.0;
    let x = chroma * (60 - (hue % 120 - 60).abs()) as f32 / 60.0;
    let lightness = lum - chroma / 2.0;

    const SCALE: f32 = 255.0 / 100.0;
    let comp1 = ((chroma + lightness) * SCALE + 0.5) as u8;
    let comp2 = ((x + lightness) * SCALE + 0.5) as u8;
    let comp3 = ((0.0 + lightness) * SCALE + 0.5) as u8;

    let (red, green, blue) = if hue < 60 {
        (comp2, comp3, comp1)
    } else if hue < 120 {
        (comp1, comp3, comp2)
    } else if hue < 180 {
        (comp1, comp2, comp3)
    } else if hue < 240 {
        (comp2, comp1, comp3)
    } else if hue < 300 {
        (comp3, comp1, comp2)
    } else {
        (comp3, comp2, comp1)
    };
    TextColor::rgb(red, green, blue)
}

/// Converts an RGB colour to the 0–100 component scale DEC reports use.
///
/// Ported from `Utils::ColorToRGB100`.
pub fn color_to_rgb100(color: TextColor) -> (i32, i32, i32) {
    let scale = |component: u8| (i32::from(component) * 100 + 128) / 255;
    (scale(color.get_r()), scale(color.get_g()), scale(color.get_b()))
}

/// Converts 0–100 components back to an RGB colour.
///
/// Ported from `Utils::ColorFromRGB100`.
pub fn color_from_rgb100(r: i32, g: i32, b: i32) -> TextColor {
    let scale = |component: i32| ((component.clamp(0, 100) * 255 + 50) / 100) as u8;
    TextColor::rgb(scale(r), scale(g), scale(b))
}

/// Parses an XParseColor spec into a packed `COLORREF` (`0x00BBGGRR`).
///
/// Ported from `Utils::ColorFromXParseColorSpec`: `rgb:<red>/<green>/<blue>`
/// with one to four hex digits per component, or `#<red><green><blue>` with a
/// uniform digit count. The X11 colour-name table is not ported yet, so a name
/// such as `red` returns `None`.
pub fn color_from_xparse_color_spec(spec: &str) -> Option<u32> {
    let bytes = spec.as_bytes();

    if bytes.len() > 4 && bytes[..4].eq_ignore_ascii_case(b"rgb:") {
        // 9 is `rgb:h/h/h`, 18 is `rgb:hhhh/hhhh/hhhh`; anything outside
        // cannot hold three components.
        if !(9..=18).contains(&bytes.len()) {
            return None;
        }

        let mut rest = &spec[4..];
        let mut values = [0u32; 3];
        for (component, value) in values.iter_mut().enumerate() {
            let (digits, tail) = split_hex_prefix(rest, 4)?;
            if component < 2 {
                rest = tail.strip_prefix('/')?;
            } else if !tail.is_empty() {
                return None;
            }

            let parameter = u32::from_str_radix(digits, 16).ok()?;
            let multiplier = 0x11;
            let divisor = (multiplier << 8) >> (4 * (4 - digits.len() as u32));
            *value = parameter * multiplier / divisor;
        }
        return Some(pack_colorref(values));
    }

    if bytes.first() == Some(&b'#') {
        let digits = match bytes.len() {
            4 => 1,
            7 => 2,
            10 => 3,
            13 => 4,
            _ => return None,
        };

        let mut values = [0u32; 3];
        for (component, value) in values.iter_mut().enumerate() {
            let start = 1 + component * digits;
            let part = &spec[start..start + digits];
            if !part.bytes().all(|byte| byte.is_ascii_hexdigit()) {
                return None;
            }
            let parameter = u32::from_str_radix(part, 16).ok()?;
            let multiplier = 0x10;
            let divisor = (multiplier << 8) >> (4 * (4 - digits as u32));
            *value = parameter * multiplier / divisor;
        }
        return Some(pack_colorref(values));
    }

    None
}

fn split_hex_prefix(s: &str, max: usize) -> Option<(&str, &str)> {
    let count = s.bytes().take(max).take_while(u8::is_ascii_hexdigit).count();
    if count == 0 {
        return None;
    }
    Some((&s[..count], &s[count..]))
}

fn pack_colorref(values: [u32; 3]) -> u32 {
    (values[0] & 0xFF) | ((values[1] & 0xFF) << 8) | ((values[2] & 0xFF) << 16)
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn the_default_table_has_the_campbell_palette() {
        let table = ColorTable::new();
        assert_eq!(table.get(0), Some(TextColor::rgb(0x0C, 0x0C, 0x0C)));
        assert_eq!(table.get(7), Some(TextColor::rgb(0xCC, 0xCC, 0xCC)));
        assert_eq!(table.get(15), Some(TextColor::rgb(0xF2, 0xF2, 0xF2)));
    }

    #[test]
    fn the_extended_entries_are_the_cube_and_greys() {
        let table = ColorTable::new();
        assert_eq!(table.get(16), Some(TextColor::rgb(0, 0, 0)));
        assert_eq!(table.get(231), Some(TextColor::rgb(255, 255, 255)));
        assert_eq!(table.get(232), Some(TextColor::rgb(8, 8, 8)));
        assert_eq!(table.get(255), Some(TextColor::rgb(238, 238, 238)));
    }

    #[test]
    fn the_alias_slots_start_unset() {
        let table = ColorTable::new();
        assert_eq!(table.get(DEFAULT_FOREGROUND), None);
        assert_eq!(table.get(SELECTION_BACKGROUND), None);
        assert_eq!(table.get(TABLE_SIZE), None);
    }

    #[test]
    fn the_default_aliases_point_at_white_and_black() {
        let table = ColorTable::new();
        assert_eq!(
            table.alias_index(ColorAlias::DefaultForeground),
            TextColor::DARK_WHITE.get_index() as usize
        );
        assert_eq!(
            table.alias_index(ColorAlias::DefaultBackground),
            TextColor::DARK_BLACK.get_index() as usize
        );
        assert_eq!(table.alias_index(ColorAlias::FrameForeground), FRAME_FOREGROUND);
    }

    #[test]
    fn resetting_an_entry_restores_its_default() {
        let mut table = ColorTable::new();
        table.set(4, TextColor::rgb(1, 2, 3));
        table.set(DEFAULT_FOREGROUND, TextColor::rgb(4, 5, 6));

        table.reset_entry(4);
        table.reset_entry(DEFAULT_FOREGROUND);

        assert_eq!(table.get(4), Some(TextColor::rgb(0x00, 0x37, 0xDA)));
        assert_eq!(table.get(DEFAULT_FOREGROUND), None);
    }

    #[test]
    fn reset_indexed_leaves_the_alias_slots_alone() {
        let mut table = ColorTable::new();
        table.set(1, TextColor::rgb(9, 9, 9));
        table.set(CURSOR_COLOR, TextColor::rgb(8, 8, 8));

        table.reset_indexed();

        assert_eq!(table.get(1), Some(TextColor::rgb(0xC5, 0x0F, 0x1F)));
        assert_eq!(table.get(CURSOR_COLOR), Some(TextColor::rgb(8, 8, 8)));
    }

    #[test]
    fn aliases_can_be_moved_and_restored() {
        let mut table = ColorTable::new();
        table.set_alias_index(ColorAlias::DefaultForeground, 42);
        assert_eq!(table.alias_index(ColorAlias::DefaultForeground), 42);

        table.restore_default_alias_index(ColorAlias::DefaultForeground);
        assert_eq!(
            table.alias_index(ColorAlias::DefaultForeground),
            TextColor::DARK_WHITE.get_index() as usize
        );
    }

    #[test]
    fn rgb100_scales_the_extremes_exactly() {
        assert_eq!(color_to_rgb100(TextColor::rgb(255, 0, 128)), (100, 0, 50));
        assert_eq!(color_from_rgb100(100, 0, 50), TextColor::rgb(255, 0, 128));
    }

    #[test]
    fn hls_puts_red_at_120_degrees() {
        // DEC's hue mapping: blue 0°, red 120°, green 240°.
        assert_eq!(color_to_hls(TextColor::rgb(255, 0, 0)), (120, 50, 100));
        assert_eq!(color_to_hls(TextColor::rgb(0, 0, 255)), (0, 50, 100));
    }

    #[test]
    fn hls_round_trips_primaries() {
        for color in [
            TextColor::rgb(255, 0, 0),
            TextColor::rgb(0, 255, 0),
            TextColor::rgb(0, 0, 255),
            TextColor::rgb(0, 0, 0),
            TextColor::rgb(255, 255, 255),
        ] {
            let (h, l, s) = color_to_hls(color);
            assert_eq!(color_from_hls(h, l, s), color);
        }
    }

    #[test]
    fn xparse_color_accepts_rgb_components_of_any_width() {
        assert_eq!(color_from_xparse_color_spec("rgb:f/0/0"), Some(0x0000_00FF));
        assert_eq!(color_from_xparse_color_spec("rgb:ff/00/00"), Some(0x0000_00FF));
        assert_eq!(color_from_xparse_color_spec("rgb:fff/000/000"), Some(0x0000_00FF));
        assert_eq!(color_from_xparse_color_spec("rgb:ffff/0000/0000"), Some(0x0000_00FF));
        assert_eq!(color_from_xparse_color_spec("RGB:ff/00/00"), Some(0x0000_00FF));
    }

    #[test]
    fn xparse_color_accepts_the_sharp_sign_form() {
        assert_eq!(color_from_xparse_color_spec("#ff0000"), Some(0x0000_00FF));
        assert_eq!(color_from_xparse_color_spec("#00ff00"), Some(0x0000_FF00));
        assert_eq!(color_from_xparse_color_spec("#0000ff"), Some(0x00FF_0000));
        // A single digit scales by 16, as upstream does.
        assert_eq!(color_from_xparse_color_spec("#f00"), Some(0x0000_00F0));
    }

    #[test]
    fn xparse_color_rejects_malformed_specs() {
        assert_eq!(color_from_xparse_color_spec("rgb:ff/00"), None);
        assert_eq!(color_from_xparse_color_spec("rgb:ff/00/00/00"), None);
        assert_eq!(color_from_xparse_color_spec("rgb:gg/00/00"), None);
        assert_eq!(color_from_xparse_color_spec("#ff00"), None);
        assert_eq!(color_from_xparse_color_spec("red"), None);
        assert_eq!(color_from_xparse_color_spec(""), None);
    }
}
