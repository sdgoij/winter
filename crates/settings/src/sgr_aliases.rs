//! The names a scheme's colours are written as, and the slot each one fills.
//!
//! A scheme is twenty colours: SGR's sixteen ANSI colours in SGR order, and the
//! four the palette keeps apart — the default foreground and background, the
//! cursor, and the selection fill. The file names them (`black` … `bright_white`,
//! `foreground` …), the palette stores them by slot, and the editor edits them
//! as a list, so one table has to hold the mapping. This is that table: applying
//! a scheme, the editor's field list, and the scheme's getter and setter all
//! read it, and nothing else knows a colour's name or its slot (ADR 034).

use types::color_table::{CURSOR_COLOR, ColorAlias, DEFAULT_BACKGROUND, DEFAULT_FOREGROUND, SELECTION_BACKGROUND};

use crate::palette::{Color, Scheme};

/// One of the twenty colours a scheme may name.
#[derive(Debug, Clone, Copy, PartialEq, Eq, Hash)]
pub enum SchemeColor {
    /// The default text colour.
    Foreground,
    /// The default background colour.
    Background,
    /// The cursor's colour.
    CursorColor,
    /// The fill behind selected text.
    SelectionBackground,
    /// ANSI 0.
    Black,
    /// ANSI 1.
    Red,
    /// ANSI 2.
    Green,
    /// ANSI 3.
    Yellow,
    /// ANSI 4.
    Blue,
    /// ANSI 5.
    Purple,
    /// ANSI 6.
    Cyan,
    /// ANSI 7.
    White,
    /// ANSI 8.
    BrightBlack,
    /// ANSI 9.
    BrightRed,
    /// ANSI 10.
    BrightGreen,
    /// ANSI 11.
    BrightYellow,
    /// ANSI 12.
    BrightBlue,
    /// ANSI 13.
    BrightPurple,
    /// ANSI 14.
    BrightCyan,
    /// ANSI 15.
    BrightWhite,
}

impl SchemeColor {
    /// Every colour a scheme may name, in the order the file lists them.
    pub const ALL: [Self; 20] = [
        Self::Foreground,
        Self::Background,
        Self::CursorColor,
        Self::SelectionBackground,
        Self::Black,
        Self::Red,
        Self::Green,
        Self::Yellow,
        Self::Blue,
        Self::Purple,
        Self::Cyan,
        Self::White,
        Self::BrightBlack,
        Self::BrightRed,
        Self::BrightGreen,
        Self::BrightYellow,
        Self::BrightBlue,
        Self::BrightPurple,
        Self::BrightCyan,
        Self::BrightWhite,
    ];

    /// The name the settings file writes for this colour.
    pub const fn name(self) -> &'static str {
        match self {
            Self::Foreground => "foreground",
            Self::Background => "background",
            Self::CursorColor => "cursor_color",
            Self::SelectionBackground => "selection_background",
            Self::Black => "black",
            Self::Red => "red",
            Self::Green => "green",
            Self::Yellow => "yellow",
            Self::Blue => "blue",
            Self::Purple => "purple",
            Self::Cyan => "cyan",
            Self::White => "white",
            Self::BrightBlack => "bright_black",
            Self::BrightRed => "bright_red",
            Self::BrightGreen => "bright_green",
            Self::BrightYellow => "bright_yellow",
            Self::BrightBlue => "bright_blue",
            Self::BrightPurple => "bright_purple",
            Self::BrightCyan => "bright_cyan",
            Self::BrightWhite => "bright_white",
        }
    }

    /// The palette slot this colour fills.
    ///
    /// The four special colours fill the reserved slots a program also reaches
    /// with the SGR defaults and OSC 12/17; the sixteen ANSI colours fill their
    /// own index.
    pub const fn slot(self) -> usize {
        match self {
            Self::Foreground => DEFAULT_FOREGROUND,
            Self::Background => DEFAULT_BACKGROUND,
            Self::CursorColor => CURSOR_COLOR,
            Self::SelectionBackground => SELECTION_BACKGROUND,
            Self::Black => 0,
            Self::Red => 1,
            Self::Green => 2,
            Self::Yellow => 3,
            Self::Blue => 4,
            Self::Purple => 5,
            Self::Cyan => 6,
            Self::White => 7,
            Self::BrightBlack => 8,
            Self::BrightRed => 9,
            Self::BrightGreen => 10,
            Self::BrightYellow => 11,
            Self::BrightBlue => 12,
            Self::BrightPurple => 13,
            Self::BrightCyan => 14,
            Self::BrightWhite => 15,
        }
    }

    /// Whether this colour is one of SGR's sixteen ANSI colours.
    pub const fn is_ansi(self) -> bool {
        self.slot() < 16
    }

    /// The alias this colour moves, for the two a program can repoint.
    ///
    /// A scheme's foreground and background are stored in the slots OSC 10/11
    /// alias, so a program that later sets its own default foreground overrides
    /// the scheme rather than fighting a second copy of the colour.
    pub const fn alias(self) -> Option<ColorAlias> {
        match self {
            Self::Foreground => Some(ColorAlias::DefaultForeground),
            Self::Background => Some(ColorAlias::DefaultBackground),
            _ => None,
        }
    }
}

impl Scheme {
    /// The colour the scheme holds for `color`.
    pub const fn get(&self, color: SchemeColor) -> Option<Color> {
        match color {
            SchemeColor::Foreground => self.foreground,
            SchemeColor::Background => self.background,
            SchemeColor::CursorColor => self.cursor_color,
            SchemeColor::SelectionBackground => self.selection_background,
            SchemeColor::Black => self.black,
            SchemeColor::Red => self.red,
            SchemeColor::Green => self.green,
            SchemeColor::Yellow => self.yellow,
            SchemeColor::Blue => self.blue,
            SchemeColor::Purple => self.purple,
            SchemeColor::Cyan => self.cyan,
            SchemeColor::White => self.white,
            SchemeColor::BrightBlack => self.bright_black,
            SchemeColor::BrightRed => self.bright_red,
            SchemeColor::BrightGreen => self.bright_green,
            SchemeColor::BrightYellow => self.bright_yellow,
            SchemeColor::BrightBlue => self.bright_blue,
            SchemeColor::BrightPurple => self.bright_purple,
            SchemeColor::BrightCyan => self.bright_cyan,
            SchemeColor::BrightWhite => self.bright_white,
        }
    }

    /// Sets, or with `None` clears, the colour the scheme holds for `color`.
    pub fn set(&mut self, color: SchemeColor, value: Option<Color>) {
        match color {
            SchemeColor::Foreground => self.foreground = value,
            SchemeColor::Background => self.background = value,
            SchemeColor::CursorColor => self.cursor_color = value,
            SchemeColor::SelectionBackground => self.selection_background = value,
            SchemeColor::Black => self.black = value,
            SchemeColor::Red => self.red = value,
            SchemeColor::Green => self.green = value,
            SchemeColor::Yellow => self.yellow = value,
            SchemeColor::Blue => self.blue = value,
            SchemeColor::Purple => self.purple = value,
            SchemeColor::Cyan => self.cyan = value,
            SchemeColor::White => self.white = value,
            SchemeColor::BrightBlack => self.bright_black = value,
            SchemeColor::BrightRed => self.bright_red = value,
            SchemeColor::BrightGreen => self.bright_green = value,
            SchemeColor::BrightYellow => self.bright_yellow = value,
            SchemeColor::BrightBlue => self.bright_blue = value,
            SchemeColor::BrightPurple => self.bright_purple = value,
            SchemeColor::BrightCyan => self.bright_cyan = value,
            SchemeColor::BrightWhite => self.bright_white = value,
        }
    }
}

#[cfg(test)]
mod tests {
    use std::collections::HashSet;

    use super::*;

    #[test]
    fn every_colour_has_its_own_name_and_slot() {
        let names: HashSet<&str> = SchemeColor::ALL.iter().map(|color| color.name()).collect();
        let slots: HashSet<usize> = SchemeColor::ALL.iter().map(|color| color.slot()).collect();

        assert_eq!(names.len(), SchemeColor::ALL.len());
        assert_eq!(slots.len(), SchemeColor::ALL.len(), "two colours share a palette slot");
    }

    #[test]
    fn the_sixteen_ansi_colours_are_slots_zero_to_fifteen_in_order() {
        let ansi: Vec<usize> = SchemeColor::ALL
            .iter()
            .filter(|color| color.is_ansi())
            .map(|color| color.slot())
            .collect();

        assert_eq!(ansi, (0..16).collect::<Vec<_>>());
        assert!(SchemeColor::Foreground.slot() >= 16);
    }

    #[test]
    fn only_the_default_colours_move_an_alias() {
        assert_eq!(SchemeColor::Foreground.alias(), Some(ColorAlias::DefaultForeground));
        assert_eq!(SchemeColor::Background.alias(), Some(ColorAlias::DefaultBackground));
        for color in SchemeColor::ALL {
            if !matches!(color, SchemeColor::Foreground | SchemeColor::Background) {
                assert_eq!(color.alias(), None, "{color:?}");
            }
        }
    }

    #[test]
    fn the_table_names_exactly_the_fields_the_file_has() {
        // A scheme with every field set: if a field is added to `Scheme` and not
        // to the table, this fails, which is the drift the table exists to stop.
        let mut scheme = Scheme::default();
        for (index, color) in SchemeColor::ALL.iter().enumerate() {
            scheme.set(*color, Some(color_at(index)));
        }

        let written = toml::to_string(&scheme).expect("toml");
        let mut keys: Vec<&str> = written
            .lines()
            .filter_map(|line| line.split_once(" = "))
            .map(|(key, _)| key)
            .filter(|key| *key != "name")
            .collect();
        keys.sort_unstable();

        let mut table: Vec<&str> = SchemeColor::ALL.iter().map(|color| color.name()).collect();
        table.sort_unstable();

        assert_eq!(keys, table);
    }

    #[test]
    fn a_colour_survives_the_table() {
        let mut scheme = Scheme::default();
        for (index, color) in SchemeColor::ALL.iter().enumerate() {
            scheme.set(*color, Some(color_at(index)));
        }
        for (index, color) in SchemeColor::ALL.iter().enumerate() {
            assert_eq!(scheme.get(*color), Some(color_at(index)), "{color:?}");
        }

        scheme.set(SchemeColor::Red, None);
        assert_eq!(scheme.get(SchemeColor::Red), None);
    }

    #[test]
    fn a_file_written_with_the_tables_names_reads_back_into_the_table() {
        let mut document = String::from("name = \"t\"\n");
        for (index, color) in SchemeColor::ALL.iter().enumerate() {
            document.push_str(&format!("{} = \"#{:02X}0102\"\n", color.name(), index));
        }

        let scheme: Scheme = toml::from_str(&document).expect("scheme");
        for (index, color) in SchemeColor::ALL.iter().enumerate() {
            assert_eq!(scheme.get(*color), Some(color_at(index)), "{color:?}");
        }
    }

    /// A colour that is different for every position in the table.
    fn color_at(index: usize) -> Color {
        Color::parse(&format!("#{:02X}0102", index)).expect("colour")
    }
}
