//! The command palette's entries, filter and selection.
//!
//! Windows Terminal's `CommandPalette.cpp` offers the window's commands beside
//! a new tab per profile, each row carrying the chord the file binds to it.
//! Every shell draws its own palette — an overlay in the GTK shell — but the
//! list it shows and the way typing narrows it name no toolkit (ADR 038).

use settings::{Action, Settings};

/// What a palette row asks the window to do.
#[derive(Debug, Clone, PartialEq, Eq)]
pub enum Command {
    /// Run this window action.
    Action(Action),
    /// Open a new tab on the named profile.
    Profile(String),
}

/// One row the palette offers.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct Entry {
    /// The row's text.
    pub label: String,
    /// What choosing it asks the window to do.
    pub command: Command,
    /// The chord the file binds to the action, for the row's right-hand side.
    pub chord: Option<String>,
}

/// The palette's state between frames.
#[derive(Debug, Default)]
pub struct Palette {
    /// Whether the palette is showing.
    pub open: bool,
    /// What the user has typed to narrow the list.
    pub filter: String,
    /// The row Enter runs, within the filtered list.
    pub selected: usize,
    /// Whether the field still needs the keyboard focus.
    pub focus: bool,
}

impl Palette {
    /// Opens the palette with an empty query and the first row selected.
    pub fn show(&mut self) {
        self.open = true;
        self.filter.clear();
        self.selected = 0;
        self.focus = true;
    }

    /// Closes the palette and forgets the query.
    pub fn hide(&mut self) {
        self.open = false;
        self.filter.clear();
        self.selected = 0;
        self.focus = false;
    }

    pub fn toggle(&mut self) {
        if self.open { self.hide() } else { self.show() }
    }

    /// The rows the current filter leaves, in display order.
    pub fn rows<'a>(&self, entries: &'a [Entry]) -> Vec<&'a Entry> {
        let filter = self.filter.trim().to_lowercase();
        if filter.is_empty() {
            return entries.iter().collect();
        }
        entries
            .iter()
            .filter(|entry| entry.label.to_lowercase().contains(&filter))
            .collect()
    }

    /// Moves the selection one row, wrapping at both ends.
    pub fn step(&mut self, len: usize, forward: bool) {
        if len == 0 {
            self.selected = 0;
            return;
        }
        self.selected = if forward {
            (self.selected + 1) % len
        } else {
            (self.selected + len - 1) % len
        };
    }

    /// Keeps the selection inside a list of `len` rows.
    pub fn clamp(&mut self, len: usize) {
        self.selected = if len == 0 { 0 } else { self.selected.min(len - 1) };
    }
}

/// Every row the palette offers: one new tab per profile, then the actions the
/// window can run.
///
/// `supported` names the actions this shell performs, in the order the file
/// lists them; the palette itself never offers a row the window would decline.
pub fn entries(settings: &Settings, supported: impl Fn(Action) -> bool) -> Vec<Entry> {
    let mut entries = Vec::new();
    for profile in &settings.profiles {
        entries.push(Entry {
            label: format!("New tab: {}", profile.name),
            command: Command::Profile(profile.name.clone()),
            chord: None,
        });
    }

    for action in Action::ALL.iter().copied() {
        // The palette is not a command to run from the palette, and a chord the
        // file unbound never reaches this list.
        if action == Action::CommandPalette || !supported(action) {
            continue;
        }
        entries.push(Entry {
            label: action_label(action).to_string(),
            command: Command::Action(action),
            chord: settings
                .keybindings
                .iter()
                .find(|binding| binding.action == action)
                .map(|binding| binding.keys.to_string()),
        });
    }

    entries
}

/// The name the palette gives an action.
fn action_label(action: Action) -> &'static str {
    match action {
        Action::NewTab => "New tab",
        Action::CloseTab => "Close tab",
        Action::NextTab => "Next tab",
        Action::PrevTab => "Previous tab",
        Action::SplitHorizontal => "Split pane right",
        Action::SplitVertical => "Split pane down",
        Action::FocusLeft => "Focus pane left",
        Action::FocusRight => "Focus pane right",
        Action::FocusUp => "Focus pane up",
        Action::FocusDown => "Focus pane down",
        Action::ResizeLeft => "Resize pane left",
        Action::ResizeRight => "Resize pane right",
        Action::ResizeUp => "Resize pane up",
        Action::ResizeDown => "Resize pane down",
        Action::Copy => "Copy selection",
        Action::Paste => "Paste",
        Action::ScrollUp => "Scroll up",
        Action::ScrollDown => "Scroll down",
        Action::ScrollUpPage => "Scroll up a page",
        Action::ScrollDownPage => "Scroll down a page",
        Action::ScrollToTop => "Scroll to the top",
        Action::ScrollToBottom => "Scroll to the bottom",
        Action::OpenSettings => "Settings",
        Action::CommandPalette => "Command palette",
        Action::Find => "Find",
        Action::Unbound => "Unbound",
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    fn settings(toml: &str) -> Settings {
        settings::parse(toml).expect("settings")
    }

    fn all(_action: Action) -> bool {
        true
    }

    #[test]
    fn the_rows_are_the_profiles_then_the_supported_actions() {
        let file = settings(
            r##"
                [[profiles]]
                name = "one"
                command = "sh"

                [[profiles]]
                name = "two"
                command = "sh"
            "##,
        );

        let entries = entries(&file, |action| action == Action::NewTab);
        assert_eq!(entries.len(), 3);
        assert_eq!(entries[0].label, "New tab: one");
        assert_eq!(entries[0].command, Command::Profile("one".to_string()));
        assert_eq!(entries[0].chord, None);
        assert_eq!(entries[1].label, "New tab: two");
        // Only the action the predicate accepts is listed, and it carries the
        // chord the file binds to it.
        assert_eq!(entries[2].label, "New tab");
        assert_eq!(entries[2].command, Command::Action(Action::NewTab));
        assert_eq!(entries[2].chord.as_deref(), Some("ctrl+shift+t"));
    }

    #[test]
    fn the_palette_itself_is_not_a_row() {
        let file = settings("[[profiles]]\nname = \"one\"\ncommand = \"sh\"\n");
        let entries = entries(&file, all);
        assert!(
            entries
                .iter()
                .all(|entry| entry.command != Command::Action(Action::CommandPalette))
        );
    }

    #[test]
    fn the_filter_ignores_case_and_surrounding_space() {
        let rows = [
            Entry {
                label: "New tab: one".to_string(),
                command: Command::Profile("one".to_string()),
                chord: None,
            },
            Entry {
                label: "Find".to_string(),
                command: Command::Action(Action::Find),
                chord: None,
            },
        ];

        let mut palette = Palette {
            filter: "  NEW TAB ".to_string(),
            ..Palette::default()
        };
        assert_eq!(palette.rows(&rows).len(), 1);
        palette.filter = "fi".to_string();
        assert_eq!(palette.rows(&rows)[0].label, "Find");
        palette.filter = String::new();
        assert_eq!(palette.rows(&rows).len(), 2);
        palette.filter = "nothing".to_string();
        assert!(palette.rows(&rows).is_empty());
    }

    #[test]
    fn the_selection_wraps_and_stays_inside_the_rows() {
        let mut palette = Palette::default();
        palette.step(3, true);
        assert_eq!(palette.selected, 1);
        palette.step(3, true);
        palette.step(3, true);
        assert_eq!(palette.selected, 0);
        palette.step(3, false);
        assert_eq!(palette.selected, 2);

        palette.selected = 5;
        palette.clamp(3);
        assert_eq!(palette.selected, 2);
        palette.clamp(0);
        assert_eq!(palette.selected, 0);

        palette.step(0, true);
        assert_eq!(palette.selected, 0);
    }

    #[test]
    fn opening_resets_the_query_and_the_selection() {
        let mut palette = Palette {
            filter: "old".to_string(),
            selected: 4,
            ..Palette::default()
        };
        palette.show();
        assert!(palette.open);
        assert!(palette.focus);
        assert!(palette.filter.is_empty());
        assert_eq!(palette.selected, 0);

        palette.hide();
        assert!(!palette.open);
        assert!(!palette.focus);
        palette.toggle();
        assert!(palette.open);
    }
}
