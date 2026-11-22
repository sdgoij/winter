//! Window themes: the colours the window chrome is painted with.
//!
//! Ported narrowly from upstream's `Theme.cpp` and `Theme.idl`. A theme here
//! names the tab row's fill and a tab's fill, which is what this shell draws
//! itself. Upstream also carries a requested light or dark mode, Mica, a
//! rainbow frame, a settings page theme and an icon style; those are WinUI
//! concepts with no counterpart in the egui shell, and a field no code reads is
//! a field that drifts (ADR 035).

use serde::{Deserialize, Serialize};

use crate::palette::Color;

/// A named set of window colours.
#[derive(Debug, Clone, PartialEq, Eq, Default, Serialize, Deserialize)]
pub struct Theme {
    /// The name `theme` refers to.
    #[serde(default)]
    pub name: String,
    /// The strip above the panes.
    #[serde(default)]
    pub tab_row: TabRowTheme,
    /// A tab.
    #[serde(default)]
    pub tab: TabTheme,
}

impl Theme {
    /// The tab row's fill, for a window that is or is not focused.
    ///
    /// A theme that names only one colour uses it for both, so a one-colour
    /// theme is not a window that changes when it loses focus.
    pub fn tab_row_background(&self, focused: bool) -> Option<Color> {
        if focused {
            self.tab_row.background
        } else {
            self.tab_row.unfocused_background.or(self.tab_row.background)
        }
    }

    /// A tab's fill, for the active tab or any other.
    pub fn tab_background(&self, active: bool) -> Option<Color> {
        if active {
            self.tab.background
        } else {
            self.tab.unfocused_background.or(self.tab.background)
        }
    }
}

/// The strip the tabs sit in.
#[derive(Debug, Clone, PartialEq, Eq, Default, Serialize, Deserialize)]
pub struct TabRowTheme {
    /// The fill while the window has focus.
    pub background: Option<Color>,
    /// The fill while it does not.
    pub unfocused_background: Option<Color>,
}

/// One tab.
#[derive(Debug, Clone, PartialEq, Eq, Default, Serialize, Deserialize)]
pub struct TabTheme {
    /// The fill of the tab whose pane is showing.
    pub background: Option<Color>,
    /// The fill of every other tab.
    pub unfocused_background: Option<Color>,
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn a_theme_reads_from_the_file() {
        let theme: Theme = toml::from_str(
            r##"
                name = "night"

                [tab_row]
                background = "#101010"
                unfocused_background = "#0A0A0A"

                [tab]
                background = "#202020"
            "##,
        )
        .expect("theme");

        assert_eq!(theme.name, "night");
        assert_eq!(
            theme.tab_row.background.map(|color| color.text_color()),
            Some(types::TextColor::rgb(0x10, 0x10, 0x10))
        );
        assert_eq!(
            theme.tab_background(true).map(|c| c.to_string()).as_deref(),
            Some("#202020")
        );
    }

    #[test]
    fn a_missing_section_is_the_empty_one() {
        let theme: Theme = toml::from_str("name = \"bare\"").expect("theme");
        assert_eq!(theme.tab_row_background(true), None);
        assert_eq!(theme.tab_background(false), None);
    }

    #[test]
    fn one_colour_serves_both_states() {
        let theme: Theme = toml::from_str("name = \"one\"\n[tab]\nbackground = \"#123456\"\n").expect("theme");

        assert_eq!(
            theme.tab_background(false).map(|color| color.to_string()),
            Some("#123456".to_string())
        );
    }

    #[test]
    fn the_unfocused_colour_wins_when_it_is_named() {
        let theme: Theme =
            toml::from_str("name = \"two\"\n[tab_row]\nbackground = \"#111111\"\nunfocused_background = \"#222222\"\n")
                .expect("theme");

        assert_eq!(
            theme.tab_row_background(false).map(|color| color.to_string()),
            Some("#222222".to_string())
        );
        assert_eq!(
            theme.tab_row_background(true).map(|color| color.to_string()),
            Some("#111111".to_string())
        );
    }

    #[test]
    fn a_broken_colour_rejects_the_theme() {
        assert!(toml::from_str::<Theme>("name = \"x\"\n[tab]\nbackground = \"nope\"\n").is_err());
    }
}
