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
