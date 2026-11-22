//! The commands a key chord can run.
//!
//! Upstream's `KeyChordSerialization.cpp` schema names a command as a string and
//! passes its arguments in a sibling field (`{ "action": "moveFocus",
//! "direction": "left" }`). Every command this shell implements takes no
//! argument, so the name is flat — `focusLeft` — and there is one shape to
//! validate instead of two (ADR 023).

use serde::{Deserialize, Serialize};

/// What a keybinding asks the window to do.
#[derive(Debug, Clone, Copy, PartialEq, Eq, Hash, Serialize, Deserialize)]
#[serde(rename_all = "camelCase")]
pub enum Action {
    /// Open the default profile in a new tab.
    NewTab,
    /// Close the focused pane, or the tab when it is the last one.
    CloseTab,
    /// Activate the next tab.
    NextTab,
    /// Activate the previous tab.
    PrevTab,
    /// Split the focused pane side by side.
    SplitHorizontal,
    /// Split the focused pane into stacked panes.
    SplitVertical,
    /// Move focus to the pane on the left.
    FocusLeft,
    /// Move focus to the pane on the right.
    FocusRight,
    /// Move focus to the pane above.
    FocusUp,
    /// Move focus to the pane below.
    FocusDown,
    /// Give the left neighbour some of the focused pane's width.
    ResizeLeft,
    /// Give the right neighbour some of the focused pane's width.
    ResizeRight,
    /// Give the upper neighbour some of the focused pane's height.
    ResizeUp,
    /// Give the lower neighbour some of the focused pane's height.
    ResizeDown,
    /// Copy the selection to the clipboard.
    Copy,
    /// Paste the clipboard into the focused pane.
    Paste,
    /// Show or hide the settings window.
    OpenSettings,
    /// Remove the chord named beside it. Resolved away at load; it never
    /// reaches the binding list the window searches.
    Unbound,
}

impl Action {
    /// Every action a binding can name, in the order the window lists them.
    ///
    /// `Unbound` is absent: the window removes a binding by deleting its row,
    /// and the file is where `unbound` takes a default chord away (ADR 031).
    pub const ALL: &[Self] = &[
        Self::NewTab,
        Self::CloseTab,
        Self::NextTab,
        Self::PrevTab,
        Self::SplitHorizontal,
        Self::SplitVertical,
        Self::FocusLeft,
        Self::FocusRight,
        Self::FocusUp,
        Self::FocusDown,
        Self::ResizeLeft,
        Self::ResizeRight,
        Self::ResizeUp,
        Self::ResizeDown,
        Self::Copy,
        Self::Paste,
        Self::OpenSettings,
    ];

    /// The name the file writes, which is also the window's label.
    pub const fn name(self) -> &'static str {
        match self {
            Self::NewTab => "newTab",
            Self::CloseTab => "closeTab",
            Self::NextTab => "nextTab",
            Self::PrevTab => "prevTab",
            Self::SplitHorizontal => "splitHorizontal",
            Self::SplitVertical => "splitVertical",
            Self::FocusLeft => "focusLeft",
            Self::FocusRight => "focusRight",
            Self::FocusUp => "focusUp",
            Self::FocusDown => "focusDown",
            Self::ResizeLeft => "resizeLeft",
            Self::ResizeRight => "resizeRight",
            Self::ResizeUp => "resizeUp",
            Self::ResizeDown => "resizeDown",
            Self::Copy => "copy",
            Self::Paste => "paste",
            Self::OpenSettings => "openSettings",
            Self::Unbound => "unbound",
        }
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::keybindings::{Chord, Keybinding};

    #[test]
    fn every_name_is_the_one_the_file_writes() {
        for action in Action::ALL.iter().copied().chain([Action::Unbound]) {
            let binding = Keybinding {
                keys: Chord::parse("ctrl+t").expect("chord"),
                action,
            };
            let text = toml::to_string(&binding).expect("toml");
            assert!(
                text.contains(&format!("action = \"{}\"", action.name())),
                "{} wrote {text}",
                action.name()
            );
            assert_eq!(toml::from_str::<Keybinding>(&text).expect("binding").action, action);
        }
    }

    #[test]
    fn the_dropdown_list_is_unique_and_leaves_out_unbound() {
        assert!(!Action::ALL.contains(&Action::Unbound));

        let names: std::collections::BTreeSet<_> = Action::ALL.iter().map(|action| action.name()).collect();
        assert_eq!(names.len(), Action::ALL.len());
    }
}
