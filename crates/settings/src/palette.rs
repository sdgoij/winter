//! Colour schemes: the colours a user may change, and the table entries they
//! fill.
//!
//! The field names are upstream's: the sixteen ANSI names (`black` … `white`,
//! then the `bright*` eight) plus `foreground`, `background`, `cursorColor` and
//! `selectionBackground`. A colour is read with the ported
//! `Utils::ColorFromXParseColorSpec`, so `#RGB`, `#RRGGBB`, `#RRRGGGBBB` and
//! `rgb:r/g/b` all work (ADR 024).

use std::fmt;

use serde::de::Error as _;
use serde::{Deserialize, Deserializer, Serialize, Serializer};
use types::TextColor;
use types::color_table::{ColorTable, color_from_xparse_color_spec};

use crate::sgr_aliases::SchemeColor;

/// A colour as the settings file writes it.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub struct Color(TextColor);

/// A colour spec that names no colour.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct ColorError(String);

impl fmt::Display for ColorError {
    fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        f.write_str(&self.0)
    }
}

impl std::error::Error for ColorError {}

impl Color {
    /// Reads an upstream colour spec.
    ///
    /// This is `colorTable.cpp`'s grammar, not a hex-only one: the same string
    /// a Windows Terminal scheme accepts reads here, and a name the upstream
    /// table has not ported yet is rejected.
    pub fn parse(spec: &str) -> Result<Self, ColorError> {
        color_from_xparse_color_spec(spec)
            .map(|colorref| Self(TextColor::from_colorref(colorref)))
            .ok_or_else(|| ColorError(format!("`{spec}` is not a colour spec")))
    }

    /// The terminal colour the spec names.
    pub const fn text_color(self) -> TextColor {
        self.0
    }
}

impl fmt::Display for Color {
    fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        let color = self.0;
        write!(f, "#{:02X}{:02X}{:02X}", color.get_r(), color.get_g(), color.get_b())
    }
}

impl Serialize for Color {
    fn serialize<S: Serializer>(&self, serializer: S) -> Result<S::Ok, S::Error> {
        serializer.collect_str(self)
    }
}

impl<'de> Deserialize<'de> for Color {
    fn deserialize<D: Deserializer<'de>>(deserializer: D) -> Result<Self, D::Error> {
        let spec = String::deserialize(deserializer)?;
        Self::parse(&spec).map_err(D::Error::custom)
    }
}

/// One named scheme.
///
/// Every colour is optional: a scheme that names only a background keeps the
/// built-in palette for everything else, which is what a partial hand-written
/// file needs. `Default` is the empty scheme an editor starts from.
#[derive(Debug, Clone, PartialEq, Eq, Default, Serialize, Deserialize)]
pub struct Scheme {
    /// The name `default_scheme` refers to.
    #[serde(default)]
    pub name: String,
    /// The default text colour.
    pub foreground: Option<Color>,
    /// The default background colour.
    pub background: Option<Color>,
    /// The cursor colour.
    pub cursor_color: Option<Color>,
    /// The fill behind selected text.
    pub selection_background: Option<Color>,
    /// ANSI 0.
    pub black: Option<Color>,
    /// ANSI 1.
    pub red: Option<Color>,
    /// ANSI 2.
    pub green: Option<Color>,
    /// ANSI 3.
    pub yellow: Option<Color>,
    /// ANSI 4.
    pub blue: Option<Color>,
    /// ANSI 5.
    pub purple: Option<Color>,
    /// ANSI 6.
    pub cyan: Option<Color>,
    /// ANSI 7.
    pub white: Option<Color>,
    /// ANSI 8.
    pub bright_black: Option<Color>,
    /// ANSI 9.
    pub bright_red: Option<Color>,
    /// ANSI 10.
    pub bright_green: Option<Color>,
    /// ANSI 11.
    pub bright_yellow: Option<Color>,
    /// ANSI 12.
    pub bright_blue: Option<Color>,
    /// ANSI 13.
    pub bright_purple: Option<Color>,
    /// ANSI 14.
    pub bright_cyan: Option<Color>,
    /// ANSI 15.
    pub bright_white: Option<Color>,
}

impl Scheme {
    /// Makes `table` the built-in palette with this scheme's colours on top.
    ///
    /// A scheme describes the whole palette, not an edit of it, so the table is
    /// reset first: a colour the scheme leaves out gets its built-in value
    /// rather than keeping one from a scheme that ran before, or a program's
    /// OSC 4. The defaults go into the reserved slots — the ones OSC 10/11 also
    /// use — so a program that later sets its own default foreground overrides
    /// the scheme instead of fighting it.
    pub fn apply(&self, table: &mut ColorTable) {
        table.reset();

        for color in SchemeColor::ALL {
            let Some(written) = self.get(color) else {
                continue;
            };
            // The two the palette aliases are moved before they are filled, so
            // a program's own default foreground lands in this scheme's slot.
            if let Some(alias) = color.alias() {
                table.set_alias_index(alias, color.slot());
            }
            table.set(color.slot(), written.text_color());
        }
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use types::color_table::{CURSOR_COLOR, ColorAlias, DEFAULT_BACKGROUND, DEFAULT_FOREGROUND, SELECTION_BACKGROUND};

    #[test]
    fn a_color_spec_is_read_with_upstreams_grammar() {
        assert_eq!(
            Color::parse("#C50F1F").expect("colour").text_color(),
            TextColor::rgb(0xC5, 0x0F, 0x1F)
        );
        assert_eq!(
            Color::parse("#fff").expect("colour").text_color(),
            TextColor::rgb(0xF0, 0xF0, 0xF0)
        );
        assert_eq!(
            Color::parse("rgb:cc/cc/cc").expect("colour").text_color(),
            TextColor::rgb(0xCC, 0xCC, 0xCC)
        );

        assert!(Color::parse("#12345").is_err());
        assert!(Color::parse("campbell").is_err());
        assert!(Color::parse("").is_err());
    }

    #[test]
    fn a_color_serialises_as_canonical_hex() {
        #[derive(Debug, PartialEq, Serialize, Deserialize)]
        struct Holder {
            colour: Color,
        }

        let holder = Holder {
            colour: Color::parse("#c50f1f").expect("colour"),
        };
        assert_eq!(toml::to_string(&holder).expect("toml"), "colour = \"#C50F1F\"\n");

        let back: Holder = toml::from_str("colour = \"#123\"").expect("colour");
        assert_eq!(back.colour.text_color(), TextColor::rgb(0x10, 0x20, 0x30));
    }

    fn scheme() -> Scheme {
        toml::from_str(
            r##"
                name = "test"
                foreground = "#010203"
                background = "#101112"
                cursor_color = "#A0B0C0"
                selection_background = "#202122"
                red = "#FF0000"
            "##,
        )
        .expect("scheme")
    }

    #[test]
    fn a_scheme_sets_only_the_colours_it_names() {
        let mut table = ColorTable::new();
        let green = table.get(2);

        scheme().apply(&mut table);

        assert_eq!(table.get(1), Some(TextColor::rgb(0xFF, 0, 0)));
        assert_eq!(table.get(2), green, "a colour the scheme leaves out is untouched");
        assert_eq!(table.get(0), Some(TextColor::rgb(0x0C, 0x0C, 0x0C)));
        assert_eq!(table.get(CURSOR_COLOR), Some(TextColor::rgb(0xA0, 0xB0, 0xC0)));
        assert_eq!(table.get(SELECTION_BACKGROUND), Some(TextColor::rgb(0x20, 0x21, 0x22)));
    }

    #[test]
    fn a_scheme_moves_the_default_aliases_to_its_own_slots() {
        let mut table = ColorTable::new();
        scheme().apply(&mut table);

        assert_eq!(table.alias_index(ColorAlias::DefaultForeground), DEFAULT_FOREGROUND);
        assert_eq!(table.alias_index(ColorAlias::DefaultBackground), DEFAULT_BACKGROUND);
        assert_eq!(table.get(DEFAULT_FOREGROUND), Some(TextColor::rgb(1, 2, 3)));
        assert_eq!(table.get(DEFAULT_BACKGROUND), Some(TextColor::rgb(0x10, 0x11, 0x12)));
    }

    #[test]
    fn a_second_scheme_drops_the_colours_the_first_one_set() {
        let mut table = ColorTable::new();
        scheme().apply(&mut table);
        assert_eq!(table.get(1), Some(TextColor::rgb(0xFF, 0, 0)));

        // A program's OSC 4 is gone too: the file is the user's answer.
        table.set(2, TextColor::rgb(1, 1, 1));

        let bare: Scheme = toml::from_str("name = \"bare\"").expect("scheme");
        bare.apply(&mut table);

        assert_eq!(table.get(1), Some(TextColor::rgb(0xC5, 0x0F, 0x1F)));
        assert_eq!(table.get(2), Some(TextColor::rgb(0x13, 0xA1, 0x0E)));
        assert_eq!(
            table.alias_index(ColorAlias::DefaultForeground),
            TextColor::DARK_WHITE.get_index() as usize
        );
    }

    #[test]
    fn an_ansi_scheme_is_applied_in_table_order() {
        let mut scheme = scheme();
        scheme.bright_white = Some(Color::parse("#FFFFFF").expect("colour"));

        let table = {
            let mut table = ColorTable::new();
            scheme.apply(&mut table);
            table
        };

        assert_eq!(table.get(15), Some(TextColor::rgb(0xFF, 0xFF, 0xFF)));
    }
}
