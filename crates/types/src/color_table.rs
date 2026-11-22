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

/// Parses an X11 colour name, with an optional numbered variant.
///
/// Ported from `Utils::ColorFromXOrgAppColorName`: the name is lowercased with
/// whitespace removed, a trailing number selects a variant (`yellow3`), and
/// `gray0`..`gray100`/`grey0`..`grey100` are computed rather than stored.
pub fn color_from_x_org_app_color_name(name: &str) -> Option<u32> {
    let mut stem = String::new();
    let mut variant: Option<usize> = None;

    for byte in name.bytes() {
        if !byte.is_ascii() {
            return None;
        }

        if byte.is_ascii_digit() {
            let digit = usize::from(byte - b'0');
            variant = Some(variant.unwrap_or(0).saturating_mul(10).saturating_add(digit));
            continue;
        }

        if matches!(byte, b' ' | b'\x0c' | b'\n' | b'\r' | b'\t' | 0x0b) {
            continue;
        }

        // The variant must be at the end: "yellow3a" is not a colour.
        if variant.is_some() {
            return None;
        }

        stem.push(byte.to_ascii_lowercase() as char);
    }

    // A base name is variant zero (`red` is `red1`), so the variant table is
    // consulted whether or not a number was written.
    let index = variant.unwrap_or(0);
    if let Ok(position) = X11_VARIANT_COLORS.binary_search_by(|entry| entry.0.cmp(stem.as_str()))
        && let Some(&[red, green, blue]) = X11_VARIANT_COLORS[position].1.get(index)
    {
        return Some(pack_colorref([u32::from(red), u32::from(green), u32::from(blue)]));
    }

    if variant.is_some() && (stem == "gray" || stem == "grey") {
        if index > 100 {
            return None;
        }
        let component = ((index * 255) + 50) / 100;
        let component = component as u32;
        return Some(pack_colorref([component, component, component]));
    }

    X11_COLORS
        .binary_search_by(|entry| entry.0.cmp(stem.as_str()))
        .ok()
        .map(|position| {
            let [red, green, blue] = X11_COLORS[position].1;
            pack_colorref([u32::from(red), u32::from(green), u32::from(blue)])
        })
}

/// Parses an xterm colour spec: an `rgb:`/`#` spec or an X11 colour name.
///
/// Ported from `Utils::ColorFromXTermColor`.
pub fn color_from_xterm_color(spec: &str) -> Option<u32> {
    color_from_xparse_color_spec(spec).or_else(|| color_from_x_org_app_color_name(spec))
}

// The X11 colour-name tables, ported from `colorTable.cpp`.
//
// `_COLORS` holds names whose single value has no numbered variants;
// `_VARIANT_COLORS` holds the base name's five shades (`yellow1`..`yellow4`,
// with index 0 being the base name itself).
const X11_VARIANT_COLORS: &[(&str, [[u8; 3]; 5])] = &[
    (
        "antiquewhite",
        [
            [250, 235, 215],
            [255, 239, 219],
            [238, 223, 204],
            [205, 192, 176],
            [139, 131, 120],
        ],
    ),
    (
        "aquamarine",
        [
            [127, 255, 212],
            [127, 255, 212],
            [118, 238, 198],
            [102, 205, 170],
            [69, 139, 116],
        ],
    ),
    (
        "azure",
        [
            [240, 255, 255],
            [240, 255, 255],
            [224, 238, 238],
            [193, 205, 205],
            [131, 139, 139],
        ],
    ),
    (
        "bisque",
        [
            [255, 228, 196],
            [255, 228, 196],
            [238, 213, 183],
            [205, 183, 158],
            [139, 125, 107],
        ],
    ),
    (
        "blue",
        [[0, 0, 255], [0, 0, 255], [0, 0, 238], [0, 0, 205], [0, 0, 139]],
    ),
    (
        "brown",
        [
            [165, 42, 42],
            [255, 64, 64],
            [238, 59, 59],
            [205, 51, 51],
            [139, 35, 35],
        ],
    ),
    (
        "burlywood",
        [
            [222, 184, 135],
            [255, 211, 155],
            [238, 197, 145],
            [205, 170, 125],
            [139, 115, 85],
        ],
    ),
    (
        "cadetblue",
        [
            [95, 158, 160],
            [152, 245, 255],
            [142, 229, 238],
            [122, 197, 205],
            [83, 134, 139],
        ],
    ),
    (
        "chartreuse",
        [[127, 255, 0], [127, 255, 0], [118, 238, 0], [102, 205, 0], [69, 139, 0]],
    ),
    (
        "chocolate",
        [
            [210, 105, 30],
            [255, 127, 36],
            [238, 118, 33],
            [205, 102, 29],
            [139, 69, 19],
        ],
    ),
    (
        "coral",
        [
            [255, 127, 80],
            [255, 114, 86],
            [238, 106, 80],
            [205, 91, 69],
            [139, 62, 47],
        ],
    ),
    (
        "cornsilk",
        [
            [255, 248, 220],
            [255, 248, 220],
            [238, 232, 205],
            [205, 200, 177],
            [139, 136, 120],
        ],
    ),
    (
        "cyan",
        [
            [0, 255, 255],
            [0, 255, 255],
            [0, 238, 238],
            [0, 205, 205],
            [0, 139, 139],
        ],
    ),
    (
        "darkgoldenrod",
        [
            [184, 134, 11],
            [255, 185, 15],
            [238, 173, 14],
            [205, 149, 12],
            [139, 101, 8],
        ],
    ),
    (
        "darkolivegreen",
        [
            [85, 107, 47],
            [202, 255, 112],
            [188, 238, 104],
            [162, 205, 90],
            [110, 139, 61],
        ],
    ),
    (
        "darkorange",
        [[255, 140, 0], [255, 127, 0], [238, 118, 0], [205, 102, 0], [139, 69, 0]],
    ),
    (
        "darkorchid",
        [
            [153, 50, 204],
            [191, 62, 255],
            [178, 58, 238],
            [154, 50, 205],
            [104, 34, 139],
        ],
    ),
    (
        "darkseagreen",
        [
            [143, 188, 143],
            [193, 255, 193],
            [180, 238, 180],
            [155, 205, 155],
            [105, 139, 105],
        ],
    ),
    (
        "darkslategray",
        [
            [47, 79, 79],
            [151, 255, 255],
            [141, 238, 238],
            [121, 205, 205],
            [82, 139, 139],
        ],
    ),
    (
        "deeppink",
        [
            [255, 20, 147],
            [255, 20, 147],
            [238, 18, 137],
            [205, 16, 118],
            [139, 10, 80],
        ],
    ),
    (
        "deepskyblue",
        [
            [0, 191, 255],
            [0, 191, 255],
            [0, 178, 238],
            [0, 154, 205],
            [0, 104, 139],
        ],
    ),
    (
        "dodgerblue",
        [
            [30, 144, 255],
            [30, 144, 255],
            [28, 134, 238],
            [24, 116, 205],
            [16, 78, 139],
        ],
    ),
    (
        "firebrick",
        [
            [178, 34, 34],
            [255, 48, 48],
            [238, 44, 44],
            [205, 38, 38],
            [139, 26, 26],
        ],
    ),
    (
        "gold",
        [
            [255, 215, 0],
            [255, 215, 0],
            [238, 201, 0],
            [205, 173, 0],
            [139, 117, 0],
        ],
    ),
    (
        "goldenrod",
        [
            [218, 165, 32],
            [255, 193, 37],
            [238, 180, 34],
            [205, 155, 29],
            [139, 105, 20],
        ],
    ),
    (
        "green",
        [[0, 255, 0], [0, 255, 0], [0, 238, 0], [0, 205, 0], [0, 139, 0]],
    ),
    (
        "honeydew",
        [
            [240, 255, 240],
            [240, 255, 240],
            [224, 238, 224],
            [193, 205, 193],
            [131, 139, 131],
        ],
    ),
    (
        "hotpink",
        [
            [255, 105, 180],
            [255, 110, 180],
            [238, 106, 167],
            [205, 96, 144],
            [139, 58, 98],
        ],
    ),
    (
        "indianred",
        [
            [205, 92, 92],
            [255, 106, 106],
            [238, 99, 99],
            [205, 85, 85],
            [139, 58, 58],
        ],
    ),
    (
        "ivory",
        [
            [255, 255, 240],
            [255, 255, 240],
            [238, 238, 224],
            [205, 205, 193],
            [139, 139, 131],
        ],
    ),
    (
        "khaki",
        [
            [240, 230, 140],
            [255, 246, 143],
            [238, 230, 133],
            [205, 198, 115],
            [139, 134, 78],
        ],
    ),
    (
        "lavenderblush",
        [
            [255, 240, 245],
            [255, 240, 245],
            [238, 224, 229],
            [205, 193, 197],
            [139, 131, 134],
        ],
    ),
    (
        "lemonchiffon",
        [
            [255, 250, 205],
            [255, 250, 205],
            [238, 233, 191],
            [205, 201, 165],
            [139, 137, 112],
        ],
    ),
    (
        "lightblue",
        [
            [173, 216, 230],
            [191, 239, 255],
            [178, 223, 238],
            [154, 192, 205],
            [104, 131, 139],
        ],
    ),
    (
        "lightcyan",
        [
            [224, 255, 255],
            [224, 255, 255],
            [209, 238, 238],
            [180, 205, 205],
            [122, 139, 139],
        ],
    ),
    (
        "lightgoldenrod",
        [
            [238, 221, 130],
            [255, 236, 139],
            [238, 220, 130],
            [205, 190, 112],
            [139, 129, 76],
        ],
    ),
    (
        "lightpink",
        [
            [255, 182, 193],
            [255, 174, 185],
            [238, 162, 173],
            [205, 140, 149],
            [139, 95, 101],
        ],
    ),
    (
        "lightsalmon",
        [
            [255, 160, 122],
            [255, 160, 122],
            [238, 149, 114],
            [205, 129, 98],
            [139, 87, 66],
        ],
    ),
    (
        "lightskyblue",
        [
            [135, 206, 250],
            [176, 226, 255],
            [164, 211, 238],
            [141, 182, 205],
            [96, 123, 139],
        ],
    ),
    (
        "lightsteelblue",
        [
            [176, 196, 222],
            [202, 225, 255],
            [188, 210, 238],
            [162, 181, 205],
            [110, 123, 139],
        ],
    ),
    (
        "lightyellow",
        [
            [255, 255, 224],
            [255, 255, 224],
            [238, 238, 209],
            [205, 205, 180],
            [139, 139, 122],
        ],
    ),
    (
        "magenta",
        [
            [255, 0, 255],
            [255, 0, 255],
            [238, 0, 238],
            [205, 0, 205],
            [139, 0, 139],
        ],
    ),
    (
        "maroon",
        [
            [176, 48, 96],
            [255, 52, 179],
            [238, 48, 167],
            [205, 41, 144],
            [139, 28, 98],
        ],
    ),
    (
        "mediumorchid",
        [
            [186, 85, 211],
            [224, 102, 255],
            [209, 95, 238],
            [180, 82, 205],
            [122, 55, 139],
        ],
    ),
    (
        "mediumpurple",
        [
            [147, 112, 219],
            [171, 130, 255],
            [159, 121, 238],
            [137, 104, 205],
            [93, 71, 139],
        ],
    ),
    (
        "mistyrose",
        [
            [255, 228, 225],
            [255, 228, 225],
            [238, 213, 210],
            [205, 183, 181],
            [139, 125, 123],
        ],
    ),
    (
        "navajowhite",
        [
            [255, 222, 173],
            [255, 222, 173],
            [238, 207, 161],
            [205, 179, 139],
            [139, 121, 94],
        ],
    ),
    (
        "olivedrab",
        [
            [107, 142, 35],
            [192, 255, 62],
            [179, 238, 58],
            [154, 205, 50],
            [105, 139, 34],
        ],
    ),
    (
        "orange",
        [[255, 165, 0], [255, 165, 0], [238, 154, 0], [205, 133, 0], [139, 90, 0]],
    ),
    (
        "orangered",
        [[255, 69, 0], [255, 69, 0], [238, 64, 0], [205, 55, 0], [139, 37, 0]],
    ),
    (
        "orchid",
        [
            [218, 112, 214],
            [255, 131, 250],
            [238, 122, 233],
            [205, 105, 201],
            [139, 71, 137],
        ],
    ),
    (
        "palegreen",
        [
            [152, 251, 152],
            [154, 255, 154],
            [144, 238, 144],
            [124, 205, 124],
            [84, 139, 84],
        ],
    ),
    (
        "paleturquoise",
        [
            [175, 238, 238],
            [187, 255, 255],
            [174, 238, 238],
            [150, 205, 205],
            [102, 139, 139],
        ],
    ),
    (
        "palevioletred",
        [
            [219, 112, 147],
            [255, 130, 171],
            [238, 121, 159],
            [205, 104, 137],
            [139, 71, 93],
        ],
    ),
    (
        "peachpuff",
        [
            [255, 218, 185],
            [255, 218, 185],
            [238, 203, 173],
            [205, 175, 149],
            [139, 119, 101],
        ],
    ),
    (
        "pink",
        [
            [255, 192, 203],
            [255, 181, 197],
            [238, 169, 184],
            [205, 145, 158],
            [139, 99, 108],
        ],
    ),
    (
        "plum",
        [
            [221, 160, 221],
            [255, 187, 255],
            [238, 174, 238],
            [205, 150, 205],
            [139, 102, 139],
        ],
    ),
    (
        "purple",
        [
            [160, 32, 240],
            [155, 48, 255],
            [145, 44, 238],
            [125, 38, 205],
            [85, 26, 139],
        ],
    ),
    ("red", [[255, 0, 0], [255, 0, 0], [238, 0, 0], [205, 0, 0], [139, 0, 0]]),
    (
        "rosybrown",
        [
            [188, 143, 143],
            [255, 193, 193],
            [238, 180, 180],
            [205, 155, 155],
            [139, 105, 105],
        ],
    ),
    (
        "royalblue",
        [
            [65, 105, 225],
            [72, 118, 255],
            [67, 110, 238],
            [58, 95, 205],
            [39, 64, 139],
        ],
    ),
    (
        "salmon",
        [
            [250, 128, 114],
            [255, 140, 105],
            [238, 130, 98],
            [205, 112, 84],
            [139, 76, 57],
        ],
    ),
    (
        "seagreen",
        [
            [46, 139, 87],
            [84, 255, 159],
            [78, 238, 148],
            [67, 205, 128],
            [46, 139, 87],
        ],
    ),
    (
        "seashell",
        [
            [255, 245, 238],
            [255, 245, 238],
            [238, 229, 222],
            [205, 197, 191],
            [139, 134, 130],
        ],
    ),
    (
        "sienna",
        [
            [160, 82, 45],
            [255, 130, 71],
            [238, 121, 66],
            [205, 104, 57],
            [139, 71, 38],
        ],
    ),
    (
        "skyblue",
        [
            [135, 206, 235],
            [135, 206, 255],
            [126, 192, 238],
            [108, 166, 205],
            [74, 112, 139],
        ],
    ),
    (
        "slateblue",
        [
            [106, 90, 205],
            [131, 111, 255],
            [122, 103, 238],
            [105, 89, 205],
            [71, 60, 139],
        ],
    ),
    (
        "slategray",
        [
            [112, 128, 144],
            [198, 226, 255],
            [185, 211, 238],
            [159, 182, 205],
            [108, 123, 139],
        ],
    ),
    (
        "snow",
        [
            [255, 250, 250],
            [255, 250, 250],
            [238, 233, 233],
            [205, 201, 201],
            [139, 137, 137],
        ],
    ),
    (
        "springgreen",
        [[0, 255, 127], [0, 255, 127], [0, 238, 118], [0, 205, 102], [0, 139, 69]],
    ),
    (
        "steelblue",
        [
            [70, 130, 180],
            [99, 184, 255],
            [92, 172, 238],
            [79, 148, 205],
            [54, 100, 139],
        ],
    ),
    (
        "tan",
        [
            [210, 180, 140],
            [255, 165, 79],
            [238, 154, 73],
            [205, 133, 63],
            [139, 90, 43],
        ],
    ),
    (
        "thistle",
        [
            [216, 191, 216],
            [255, 225, 255],
            [238, 210, 238],
            [205, 181, 205],
            [139, 123, 139],
        ],
    ),
    (
        "tomato",
        [
            [255, 99, 71],
            [255, 99, 71],
            [238, 92, 66],
            [205, 79, 57],
            [139, 54, 38],
        ],
    ),
    (
        "turquoise",
        [
            [64, 224, 208],
            [0, 245, 255],
            [0, 229, 238],
            [0, 197, 205],
            [0, 134, 139],
        ],
    ),
    (
        "violetred",
        [
            [208, 32, 144],
            [255, 62, 150],
            [238, 58, 140],
            [205, 50, 120],
            [139, 34, 82],
        ],
    ),
    (
        "wheat",
        [
            [245, 222, 179],
            [255, 231, 186],
            [238, 216, 174],
            [205, 186, 150],
            [139, 126, 102],
        ],
    ),
    (
        "yellow",
        [
            [255, 255, 0],
            [255, 255, 0],
            [238, 238, 0],
            [205, 205, 0],
            [139, 139, 0],
        ],
    ),
];

const X11_COLORS: &[(&str, [u8; 3])] = &[
    ("aliceblue", [240, 248, 255]),
    ("aqua", [0, 255, 255]),
    ("beige", [245, 245, 220]),
    ("black", [0, 0, 0]),
    ("blanchedalmond", [255, 235, 205]),
    ("blueviolet", [138, 43, 226]),
    ("cornflowerblue", [100, 149, 237]),
    ("crimson", [220, 20, 60]),
    ("darkblue", [0, 0, 139]),
    ("darkcyan", [0, 139, 139]),
    ("darkgray", [169, 169, 169]),
    ("darkgreen", [0, 100, 0]),
    ("darkgrey", [169, 169, 169]),
    ("darkkhaki", [189, 183, 107]),
    ("darkmagenta", [139, 0, 139]),
    ("darkred", [139, 0, 0]),
    ("darksalmon", [233, 150, 122]),
    ("darkslateblue", [72, 61, 139]),
    ("darkslategrey", [47, 79, 79]),
    ("darkturquoise", [0, 206, 209]),
    ("darkviolet", [148, 0, 211]),
    ("dimgray", [105, 105, 105]),
    ("dimgrey", [105, 105, 105]),
    ("floralwhite", [255, 250, 240]),
    ("forestgreen", [34, 139, 34]),
    ("fuchsia", [255, 0, 255]),
    ("gainsboro", [220, 220, 220]),
    ("ghostwhite", [248, 248, 255]),
    ("gray", [190, 190, 190]),
    ("greenyellow", [173, 255, 47]),
    ("grey", [190, 190, 190]),
    ("indigo", [75, 0, 130]),
    ("lavender", [230, 230, 250]),
    ("lawngreen", [124, 252, 0]),
    ("lightcoral", [240, 128, 128]),
    ("lightgoldenrodyellow", [250, 250, 210]),
    ("lightgray", [211, 211, 211]),
    ("lightgreen", [144, 238, 144]),
    ("lightgrey", [211, 211, 211]),
    ("lightseagreen", [32, 178, 170]),
    ("lightslateblue", [132, 112, 255]),
    ("lightslategray", [119, 136, 153]),
    ("lightslategrey", [119, 136, 153]),
    ("lime", [0, 255, 0]),
    ("limegreen", [50, 205, 50]),
    ("linen", [250, 240, 230]),
    ("mediumaquamarine", [102, 205, 170]),
    ("mediumblue", [0, 0, 205]),
    ("mediumseagreen", [60, 179, 113]),
    ("mediumslateblue", [123, 104, 238]),
    ("mediumspringgreen", [0, 250, 154]),
    ("mediumturquoise", [72, 209, 204]),
    ("mediumvioletred", [199, 21, 133]),
    ("midnightblue", [25, 25, 112]),
    ("mintcream", [245, 255, 250]),
    ("moccasin", [255, 228, 181]),
    ("navy", [0, 0, 128]),
    ("navyblue", [0, 0, 128]),
    ("oldlace", [253, 245, 230]),
    ("olive", [128, 128, 0]),
    ("palegoldenrod", [238, 232, 170]),
    ("papayawhip", [255, 239, 213]),
    ("peru", [205, 133, 63]),
    ("powderblue", [176, 224, 230]),
    ("rebeccapurple", [102, 51, 153]),
    ("saddlebrown", [139, 69, 19]),
    ("sandybrown", [244, 164, 96]),
    ("silver", [192, 192, 192]),
    ("slategrey", [112, 128, 144]),
    ("teal", [0, 128, 128]),
    ("violet", [238, 130, 238]),
    ("webgray", [128, 128, 128]),
    ("webgreen", [0, 128, 0]),
    ("webgrey", [128, 128, 128]),
    ("webmaroon", [128, 0, 0]),
    ("webpurple", [128, 0, 128]),
    ("white", [255, 255, 255]),
    ("whitesmoke", [245, 245, 245]),
    ("x11gray", [190, 190, 190]),
    ("x11green", [0, 255, 0]),
    ("x11grey", [190, 190, 190]),
    ("x11maroon", [176, 48, 96]),
    ("x11purple", [160, 32, 240]),
    ("yellowgreen", [154, 205, 50]),
];

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

    #[test]
    fn x11_names_resolve_to_colours() {
        assert_eq!(color_from_xterm_color("red"), Some(0x0000_00FF));
        assert_eq!(color_from_xterm_color("blue"), Some(0x00FF_0000));
        assert_eq!(color_from_xterm_color("White"), Some(0x00FF_FFFF));
        // Whitespace and case are not significant.
        assert_eq!(
            color_from_xterm_color("Antique White"),
            color_from_xterm_color("antiquewhite")
        );
    }

    #[test]
    fn x11_variant_names_select_a_shade() {
        // `yellow3` is the fourth entry of the yellow variant table.
        assert_eq!(color_from_xterm_color("yellow3"), Some(0x0000_CDCD));
        assert_eq!(color_from_xterm_color("gray50"), Some(0x0080_8080));
        assert_eq!(color_from_xterm_color("grey100"), Some(0x00FF_FFFF));
        assert_eq!(color_from_xterm_color("gray101"), None);
        assert_eq!(color_from_xterm_color("yellow9"), None);
    }

    #[test]
    fn unknown_colour_names_are_not_colours() {
        assert_eq!(color_from_xterm_color("notacolour"), None);
        assert_eq!(color_from_xterm_color(""), None);
        assert_eq!(color_from_xterm_color("yellow3a"), None);
    }
}
