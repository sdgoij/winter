//! `CommandHistory` — the per-application command ring.
//!
//! Ported from `src/host/history.cpp`. Upstream keeps a process-wide list keyed
//! by client process handle and trims it under memory pressure through Win32
//! hooks; a portable host keys the list by application name and leaves its
//! lifecycle to the owner, so this module is the ring and the list.

use std::collections::VecDeque;

/// The number of commands a host keeps per application by default.
pub const DEFAULT_MAX_COMMANDS: i32 = 50;

/// An index into the ring, with `-1` meaning "before the first command".
pub type Index = i32;

/// Which way [`CommandHistory::retrieve`] walks.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum SearchDirection {
    /// Towards older commands.
    Previous,
    /// Towards newer commands.
    Next,
}

/// One application's command history.
///
/// The ring holds at most `max_commands` entries and drops the oldest first.
/// `last_displayed` is the entry the reader is looking at, and `reset` is
/// upstream's `CLE_RESET`: the next [`retrieve`](Self::retrieve) returns the
/// current command instead of stepping back one.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct CommandHistory {
    app_name: String,
    max_commands: i32,
    commands: VecDeque<String>,
    last_displayed: Index,
    reset: bool,
}

impl CommandHistory {
    /// Creates an empty history for `app_name`.
    pub fn new(app_name: impl Into<String>, max_commands: i32) -> Self {
        Self {
            app_name: app_name.into(),
            max_commands: max_commands.max(0),
            commands: VecDeque::new(),
            last_displayed: -1,
            reset: true,
        }
    }

    /// Returns the application this history belongs to.
    pub fn app_name(&self) -> &str {
        &self.app_name
    }

    /// Returns whether two application names name the same history.
    ///
    /// Upstream compares with `CompareStringOrdinal(..., TRUE)`, which ignores
    /// case.
    pub fn app_name_matches(&self, other: &str) -> bool {
        self.app_name.eq_ignore_ascii_case(other)
    }

    /// Returns the most commands the ring holds.
    pub const fn max_commands(&self) -> i32 {
        self.max_commands
    }

    /// Returns the number of stored commands.
    pub fn len(&self) -> usize {
        self.commands.len()
    }

    /// Returns whether the ring is empty.
    pub fn is_empty(&self) -> bool {
        self.commands.is_empty()
    }

    /// Iterates over the commands, oldest first.
    pub fn commands(&self) -> impl Iterator<Item = &str> {
        self.commands.iter().map(String::as_str)
    }

    /// Adds a command, skipping an empty one and the one already at the end.
    ///
    /// With `suppress_duplicates`, an earlier identical command is removed and
    /// the new copy is appended, so the most recent use is always last.
    pub fn add(&mut self, command: &str, suppress_duplicates: bool) {
        if self.max_commands == 0 || command.is_empty() {
            return;
        }

        if self.commands.back().is_some_and(|last| last == command) {
            // The same command again; upstream still marks a command returned.
            self.reset = true;
            return;
        }

        let mut reuse = None;
        if suppress_duplicates {
            let (found, index) = self.find_matching(command, self.last_displayed, true, false);
            if found {
                reuse = self.remove(index);
            }
        }

        if self.len() as i32 == self.max_commands {
            self.commands.pop_front();
            // Stay pointing at the same command after the shift.
            self.last_displayed -= 1;
        }

        self.commands.push_back(reuse.unwrap_or_else(|| command.to_string()));

        let displayed = self.commands.get(self.last_displayed as usize).map(String::as_str);
        if self.last_displayed == -1 || displayed != Some(command) {
            self.reset_display();
        }
        // "Remember that we've returned a command."
        self.reset = true;
    }

    /// Returns the entry at `index`, if there is one.
    pub fn get_nth(&self, index: Index) -> Option<&str> {
        if index < 0 {
            return None;
        }
        self.commands.get(index as usize).map(String::as_str)
    }

    /// Returns the command the reader is looking at.
    pub fn last_command(&self) -> Option<&str> {
        self.get_nth(self.last_displayed)
    }

    /// Steps to the previous or next command and returns it.
    ///
    /// The first previous after an add returns the latest command; later ones
    /// walk back. A walk past either end clamps there.
    pub fn retrieve(&mut self, direction: SearchDirection) -> Option<&str> {
        match direction {
            SearchDirection::Previous => {
                if self.reset {
                    self.reset = false;
                } else {
                    self.last_displayed -= 1;
                }
            }
            SearchDirection::Next => self.last_displayed += 1,
        }
        let index = self.last_displayed;
        self.retrieve_nth(index)
    }

    /// Shows the entry at `index`, clamping it into the ring.
    pub fn retrieve_nth(&mut self, index: Index) -> Option<&str> {
        if self.commands.is_empty() {
            self.last_displayed = 0;
            return None;
        }
        self.last_displayed = index.clamp(0, self.len() as i32 - 1);
        self.get_nth(self.last_displayed)
    }

    /// Empties the ring.
    pub fn empty(&mut self) {
        self.commands.clear();
        self.last_displayed = -1;
        self.reset = true;
    }

    /// Returns whether the reader is on the oldest command.
    pub fn at_first_command(&self) -> bool {
        if self.reset {
            return false;
        }
        let mut index = self.last_displayed - 1;
        if index == -1 {
            index = self.len() as i32 - 1;
        }
        index == self.len() as i32 - 1
    }

    /// Returns whether the reader is on the newest command.
    pub fn at_last_command(&self) -> bool {
        self.last_displayed == self.len() as i32 - 1
    }

    /// Changes the ring size, keeping the oldest commands that still fit.
    pub fn realloc(&mut self, max_commands: i32) {
        if self.max_commands == max_commands {
            return;
        }
        self.commands.truncate(max_commands.max(0) as usize);
        self.reset = true;
        self.last_displayed = self.len() as i32 - 1;
        self.max_commands = max_commands;
    }

    /// Removes the entry at `index` and returns it.
    pub fn remove(&mut self, index: Index) -> Option<String> {
        if index < 0 || index as usize >= self.commands.len() {
            return None;
        }
        let command = self.commands.remove(index as usize);
        if self.last_displayed == index {
            self.last_displayed = -1;
        } else if self.last_displayed > index {
            self.last_displayed -= 1;
        }
        command
    }

    /// Finds the most recent command that matches `given`, starting at
    /// `starting_index` and walking towards older entries.
    ///
    /// With `exact` the command must equal `given`; otherwise `given` may be a
    /// prefix, which is how Tab completion finds a command. `just_looking`
    /// leaves the reset flag and the caller's index untouched, as upstream's
    /// `MatchOptions::JustLooking` does.
    pub fn find_matching(
        &mut self,
        given: &str,
        starting_index: Index,
        exact: bool,
        just_looking: bool,
    ) -> (bool, Index) {
        let mut index = starting_index;
        if self.commands.is_empty() {
            return (false, index);
        }

        if !just_looking && self.reset {
            self.reset = false;
        } else {
            index = self.previous_index(index);
        }

        if given.is_empty() {
            return (true, index);
        }

        for _ in 0..self.commands.len() {
            let stored = &self.commands[index as usize];
            let matches = if exact {
                stored == given
            } else {
                stored.starts_with(given)
            };
            if matches {
                return (true, index);
            }
            index = self.previous_index(index);
        }

        (false, index)
    }

    fn reset_display(&mut self) {
        self.last_displayed = self.len() as i32 - 1;
        self.reset = true;
    }

    fn previous_index(&self, index: Index) -> Index {
        let mut index = index;
        if index <= 0 {
            index = self.len() as i32;
        }
        index - 1
    }
}

/// The host's histories, one per application.
#[derive(Debug, Clone, Default, PartialEq, Eq)]
pub struct CommandHistoryList {
    histories: Vec<CommandHistory>,
}

impl CommandHistoryList {
    /// Creates an empty list.
    pub const fn new() -> Self {
        Self { histories: Vec::new() }
    }

    /// Returns the history for `app_name`, creating it if it is new.
    pub fn find_or_create(&mut self, app_name: &str, max_commands: i32) -> &mut CommandHistory {
        if let Some(index) = self
            .histories
            .iter()
            .position(|history| history.app_name_matches(app_name))
        {
            return &mut self.histories[index];
        }

        self.histories.push(CommandHistory::new(app_name, max_commands));
        let index = self.histories.len() - 1;
        &mut self.histories[index]
    }

    /// Resizes every history.
    pub fn resize_all(&mut self, max_commands: i32) {
        for history in &mut self.histories {
            history.realloc(max_commands);
        }
    }

    /// Resizes the histories of one application, as upstream's
    /// `s_ReallocExeToFront` does for the process in front.
    pub fn realloc_exe_to_front(&mut self, app_name: &str, max_commands: i32) {
        for history in &mut self.histories {
            if history.app_name_matches(app_name) {
                history.realloc(max_commands);
            }
        }
    }

    /// Empties every history.
    pub fn empty_all(&mut self) {
        for history in &mut self.histories {
            history.empty();
        }
    }

    /// Returns the number of applications with a history.
    pub fn len(&self) -> usize {
        self.histories.len()
    }

    /// Returns whether no application has a history.
    pub fn is_empty(&self) -> bool {
        self.histories.is_empty()
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    fn history(max: i32) -> CommandHistory {
        CommandHistory::new("cmd.exe", max)
    }

    #[test]
    fn commands_are_added_in_order() {
        let mut history = history(5);
        history.add("dir", false);
        history.add("cd ..", false);
        history.add("dir", false);

        assert_eq!(history.commands().collect::<Vec<_>>(), ["dir", "cd ..", "dir"]);
        assert_eq!(history.len(), 3);
    }

    #[test]
    fn an_empty_command_is_ignored() {
        let mut history = history(5);
        history.add("", false);
        assert!(history.is_empty());
    }

    #[test]
    fn a_zero_sized_history_keeps_nothing() {
        let mut history = history(0);
        history.add("dir", false);
        assert!(history.is_empty());
    }

    #[test]
    fn the_same_command_twice_is_not_stored_twice() {
        let mut history = history(5);
        history.add("dir", false);
        history.add("dir", false);
        assert_eq!(history.len(), 1);
    }

    #[test]
    fn suppressing_duplicates_moves_the_command_to_the_end() {
        let mut history = history(5);
        history.add("a", false);
        history.add("b", false);
        history.add("a", true);

        assert_eq!(history.commands().collect::<Vec<_>>(), ["b", "a"]);
    }

    #[test]
    fn the_ring_drops_the_oldest_command() {
        let mut history = history(2);
        history.add("a", false);
        history.add("b", false);
        history.add("c", false);

        assert_eq!(history.commands().collect::<Vec<_>>(), ["b", "c"]);
    }

    #[test]
    fn retrieve_walks_back_and_forward() {
        let mut history = history(5);
        history.add("a", false);
        history.add("b", false);

        assert_eq!(history.retrieve(SearchDirection::Previous), Some("b"));
        assert_eq!(history.retrieve(SearchDirection::Previous), Some("a"));
        // The walk clamps at the oldest command.
        assert_eq!(history.retrieve(SearchDirection::Previous), Some("a"));
        assert_eq!(history.retrieve(SearchDirection::Next), Some("b"));
    }

    #[test]
    fn an_empty_history_retrieves_nothing() {
        let mut history = history(5);
        assert_eq!(history.retrieve(SearchDirection::Previous), None);
    }

    #[test]
    fn empty_clears_the_ring() {
        let mut history = history(5);
        history.add("a", false);
        history.empty();

        assert!(history.is_empty());
        assert_eq!(history.last_command(), None);
    }

    #[test]
    fn realloc_keeps_the_oldest_commands() {
        let mut history = history(5);
        for command in ["a", "b", "c"] {
            history.add(command, false);
        }

        history.realloc(2);
        assert_eq!(history.commands().collect::<Vec<_>>(), ["a", "b"]);
        assert_eq!(history.max_commands(), 2);
    }

    #[test]
    fn the_first_and_last_positions_are_known() {
        let mut history = history(5);
        history.add("a", false);
        history.add("b", false);

        // Before any retrieval the reader is not on a command.
        assert!(!history.at_first_command());
        assert!(history.at_last_command());

        history.retrieve(SearchDirection::Previous);
        assert!(history.at_last_command());

        history.retrieve(SearchDirection::Previous);
        assert!(history.at_first_command());
    }

    #[test]
    fn a_prefix_search_finds_the_most_recent_match() {
        let mut history = history(5);
        history.add("make", false);
        history.add("git status", false);
        history.add("make test", false);

        let (found, index) = history.find_matching("make", history.last_displayed, false, false);
        assert!(found);
        assert_eq!(history.get_nth(index), Some("make test"));
    }

    #[test]
    fn an_exact_search_skips_a_longer_command() {
        let mut history = history(5);
        history.add("make", false);
        history.add("make test", false);

        let (found, index) = history.find_matching("make", history.last_displayed, true, false);
        assert!(found);
        assert_eq!(history.get_nth(index), Some("make"));
    }

    #[test]
    fn the_list_keeps_one_history_per_application() {
        let mut list = CommandHistoryList::new();
        list.find_or_create("cmd.exe", 5).add("dir", false);

        // The comparison ignores case.
        let history = list.find_or_create("CMD.EXE", 5);
        assert_eq!(history.len(), 1);

        list.find_or_create("powershell.exe", 5).add("Get-ChildItem", false);
        assert_eq!(list.len(), 2);

        list.empty_all();
        assert!(list.find_or_create("cmd.exe", 5).is_empty());
        assert!(list.find_or_create("powershell.exe", 5).is_empty());
    }
}
