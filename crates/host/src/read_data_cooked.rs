//! `COOKED_READ_DATA` — the line editor behind a cooked read.
//!
//! Ported from `src/host/readDataCooked.cpp`. Upstream also owns the pager that
//! paints the line by emitting VT, the drawing half of the four history
//! popups, and the `ReadData` / `IWaitRoutine` completion path. Painting belongs
//! to the screen layer and the wait path to `server` (Phase 7); this module is
//! the model they drive: the edit line, the popups' state and input handling,
//! the alias expansion of a finished line (`alias.cpp`), and the text the read
//! hands back.
//!
//! The key classification is ported from `IsCommandLineEditingKey` and
//! `IsCommandLinePopupKey` in `_stream.cpp`, which is what `GetChar` uses to
//! decide whether a key record is a character or an editing action.

use crate::alias::AliasTable;
use crate::cmdline::delimiter_class;
use crate::history::{CommandHistory, Index, SearchDirection};
use crate::input_buffer::{InputBuffer, InputMode};
use types::virtual_key::{
    VK_DELETE, VK_DOWN, VK_END, VK_ESCAPE, VK_F1, VK_F2, VK_F3, VK_F4, VK_F5, VK_F6, VK_F7, VK_F8, VK_F9, VK_F10,
    VK_HOME, VK_INSERT, VK_LEFT, VK_NEXT, VK_PRIOR, VK_RIGHT, VK_UP,
};
use types::{ControlKeyState, InputRecord, KeyEventRecord};

/// A cursor position meaning "past the end", upstream's `npos`.
const END: usize = usize::MAX;

/// Upstream's `CommandNumberMaxInputLength`.
const COMMAND_NUMBER_MAX_INPUT_LENGTH: usize = 5;

/// Upstream's `EXTKEY_ERASE_PREV_WORD`: Ctrl+Backspace.
const ERASE_PREV_WORD: char = '\u{7f}';

/// Where the read stands.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
enum State {
    /// Still collecting input.
    Accumulating,
    /// A `CtrlWakeupMask` key ended the read.
    DoneWithWakeupMask,
    /// Carriage return ended the read.
    DoneWithCarriageReturn,
}

/// Which popup was opened.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
enum PopupKind {
    /// F2: copy from the previous command up to a character.
    CopyToChar,
    /// F4: erase up to a character.
    CopyFromChar,
    /// F9: recall a command by number.
    CommandNumber,
    /// F7: pick a command from a list.
    CommandList,
}

/// An open popup and the state its drawing and input share.
#[derive(Debug, Clone, PartialEq, Eq)]
pub enum Popup {
    /// F2: the next character terminates the copy.
    CopyToChar,
    /// F4: the next character terminates the erase.
    CopyFromChar,
    /// F9: the digits typed so far.
    CommandNumber {
        /// The digits, at most `COMMAND_NUMBER_MAX_INPUT_LENGTH`.
        digits: Vec<char>,
    },
    /// F7: the visible command list.
    CommandList {
        /// The first history index the list shows; `-1` until it is computed.
        top: Index,
        /// The selected history index.
        selected: Index,
        /// The number of rows the list shows.
        height: i32,
    },
}

/// The text a finished read returns to its client.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct CookedReadOutput {
    /// The line, with the newline a carriage return adds or with its alias
    /// expansion applied.
    pub text: String,
    /// The modifiers of the key that ended the read.
    pub control_key_state: ControlKeyState,
    /// The rest of a multi-line alias expansion, to start the next read with as
    /// its initial text.
    ///
    /// Upstream saves this through `INPUT_READ_HANDLE_DATA`, which needs the
    /// server layer; a portable host hands it back instead.
    pub pending_input: Option<String>,
}

/// A cooked read's edit line and popups.
pub struct CookedReadData<'a> {
    buffer: Vec<char>,
    cursor: usize,
    insert_mode: bool,
    state: State,
    ctrl_wakeup_mask: u32,
    control_key_state: ControlKeyState,
    history: Option<&'a mut CommandHistory>,
    aliases: Option<&'a AliasTable>,
    exe_name: String,
    popups: Vec<Popup>,
    processed_input: bool,
    echo_input: bool,
    virtual_terminal_input: bool,
    viewport_height: i32,
}

impl<'a> CookedReadData<'a> {
    /// Creates a read over `history`.
    ///
    /// `initial` is `CONSOLE_READCONSOLE_CONTROL::nInitialChars`: text the
    /// application has already written and is telling the host to trust.
    /// Upstream searches for the layout origin that ends at the cursor; that
    /// search belongs to the pager, so the portable form takes the text and
    /// places the cursor after it.
    pub fn new(
        modes: InputMode,
        viewport_height: i32,
        ctrl_wakeup_mask: u32,
        initial: &str,
        history: Option<&'a mut CommandHistory>,
    ) -> Self {
        let mut this = Self {
            buffer: Vec::new(),
            cursor: 0,
            insert_mode: false,
            state: State::Accumulating,
            ctrl_wakeup_mask,
            control_key_state: ControlKeyState::default(),
            history,
            aliases: None,
            exe_name: String::new(),
            popups: Vec::new(),
            processed_input: modes.contains(InputMode::PROCESSED),
            echo_input: modes.contains(InputMode::ECHO),
            virtual_terminal_input: modes.contains(InputMode::VIRTUAL_TERMINAL),
            viewport_height,
        };

        if !initial.is_empty() {
            this.buffer.extend(initial.chars());
            this.cursor = this.buffer.len();
        }

        this
    }

    /// Refreshes the input modes the read consults.
    ///
    /// Upstream reads the modes from the live input buffer on every key; a
    /// portable host owns the modes and tells the read when they change.
    pub fn set_modes(&mut self, modes: InputMode) {
        self.processed_input = modes.contains(InputMode::PROCESSED);
        self.echo_input = modes.contains(InputMode::ECHO);
        self.virtual_terminal_input = modes.contains(InputMode::VIRTUAL_TERMINAL);
    }

    /// Expands a finished line through `aliases` when the application is
    /// `exe_name`.
    ///
    /// Upstream holds the alias table as process state and consults it from the
    /// read's completion path; the portable form takes it as a dependency, so a
    /// test and a host can supply their own.
    pub fn with_aliases(mut self, aliases: &'a AliasTable, exe_name: &str) -> Self {
        self.aliases = Some(aliases);
        self.exe_name.clear();
        self.exe_name.push_str(exe_name);
        self
    }

    /// Returns the edit line.
    pub fn text(&self) -> String {
        self.buffer.iter().collect()
    }

    /// Returns the cursor's offset into the edit line, in characters.
    pub const fn cursor(&self) -> usize {
        self.cursor
    }

    /// Returns whether the line and every popup are empty.
    pub fn is_empty(&self) -> bool {
        self.buffer.is_empty() && self.popups.is_empty()
    }

    /// Returns whether a popup is open.
    pub fn presenting_popup(&self) -> bool {
        !self.popups.is_empty()
    }

    /// Returns the open popups, oldest first.
    pub fn popups(&self) -> &[Popup] {
        &self.popups
    }

    /// Sets overwrite (`false`) or insert (`true`) mode.
    pub const fn set_insert_mode(&mut self, insert_mode: bool) {
        self.insert_mode = insert_mode;
    }

    /// Returns whether insert mode is on.
    pub const fn insert_mode(&self) -> bool {
        self.insert_mode
    }

    /// Drains one record from `input` at a time until the read finishes or the
    /// buffer runs dry.
    ///
    /// `None` means the read is still accumulating and the caller must wait
    /// for the client to write more records, upstream's
    /// `CONSOLE_STATUS_WAIT`.
    pub fn read(&mut self, input: &mut InputBuffer) -> Option<CookedReadOutput> {
        while self.state == State::Accumulating {
            let mut records = input.read(1);
            let Some(record) = records.pop() else {
                break;
            };
            self.handle_record(record);
        }

        if self.state == State::Accumulating {
            return None;
        }
        Some(self.finish_read())
    }

    /// Applies one console input record.
    pub fn handle_record(&mut self, record: InputRecord) {
        let InputRecord::Key(key) = record else {
            // `GetChar` discards non-key events it reads.
            return;
        };

        let has_popup = self.presenting_popup();
        let editing = if has_popup {
            is_popup_key(&key)
        } else {
            is_editing_key(&key)
        };

        // A character wins over the key code, unless the record is an editing
        // key or is Escape / LF outside VT input, which upstream ignores.
        if key.key_down
            && key.character != '\0'
            && !editing
            && (self.virtual_terminal_input || (key.virtual_key != VK_ESCAPE && key.character != '\n'))
        {
            if has_popup {
                self.popup_handle_input(key.character, 0, key.control_keys);
            } else {
                self.handle_char(key.character, key.control_keys);
            }
            return;
        }

        if key.key_down && editing {
            if has_popup {
                self.popup_handle_input('\0', key.virtual_key, key.control_keys);
            } else {
                self.handle_vkey(key.virtual_key, key.control_keys);
            }
            return;
        }

        // Upstream reaches a NUL character through `VkKeyScanW(0)`. A portable
        // record says directly that Ctrl produced NUL, which is Ctrl+@.
        if key.key_down && key.character == '\0' && key.control_keys.ctrl() && !editing {
            if has_popup {
                self.popup_handle_input('\0', 0, key.control_keys);
            } else {
                self.handle_char('\0', key.control_keys);
            }
        }
    }

    /// Handles character input when no popup is open.
    pub fn handle_char(&mut self, character: char, modifiers: ControlKeyState) {
        if self.ctrl_wakeup_mask != 0 && character < ' ' && (self.ctrl_wakeup_mask & (1 << character as u32)) != 0 {
            // The control character is returned to the client but never
            // painted: the tail is erased first, then the character is
            // inserted, so tab completion does not leave the old tail behind.
            self.replace_range(self.cursor, END, &[]);
            self.replace_range(self.cursor, 0, &[character]);
            self.control_key_state = modifiers;
            self.transition(State::DoneWithWakeupMask);
            return;
        }

        match character {
            '\r' => {
                self.set_cursor(END);
                self.transition(State::DoneWithCarriageReturn);
                return;
            }
            ERASE_PREV_WORD | '\u{8}' if self.processed_input => {
                let cursor = self.cursor;
                let position = if character == ERASE_PREV_WORD {
                    self.word_prev(cursor)
                } else {
                    self.grapheme_prev(cursor)
                };
                self.replace_range(position, cursor - position, &[]);
                return;
            }
            _ => {}
        }

        // With processed input off, the control character falls through and is
        // inserted like any other.
        let remove = if self.insert_mode {
            0
        } else {
            self.grapheme_next(self.cursor) - self.cursor
        };
        self.replace_range(self.cursor, remove, &[character]);
    }

    /// Handles a key when no popup is open.
    pub fn handle_vkey(&mut self, vkey: u16, modifiers: ControlKeyState) {
        let ctrl = modifiers.ctrl();
        let alt = modifiers.alt();

        match vkey {
            VK_ESCAPE => {
                if !self.buffer.is_empty() {
                    self.replace_range(0, END, &[]);
                }
            }
            VK_HOME => {
                if self.cursor > 0 {
                    if ctrl {
                        self.replace_range(0, self.cursor, &[]);
                    }
                    self.set_cursor(0);
                }
            }
            VK_END => {
                if self.cursor < self.buffer.len() {
                    if ctrl {
                        self.replace_range(self.cursor, END, &[]);
                    }
                    self.set_cursor(END);
                }
            }
            VK_LEFT => {
                if self.cursor != 0 {
                    let position = if ctrl {
                        self.word_prev(self.cursor)
                    } else {
                        self.grapheme_prev(self.cursor)
                    };
                    self.set_cursor(position);
                }
            }
            VK_F1 | VK_RIGHT => {
                if self.cursor != self.buffer.len() {
                    let position = if ctrl && vkey == VK_RIGHT {
                        self.word_next(self.cursor)
                    } else {
                        self.grapheme_next(self.cursor)
                    };
                    self.set_cursor(position);
                } else if let Some(command) = self.last_command() {
                    // Right at the end of the line appends the rest of the
                    // command the reader is looking at, one grapheme at a time.
                    let buffer_size = self.buffer.len();
                    let mut buffer_beg = 0;
                    let mut command_beg = 0;
                    while command_beg < command.len() {
                        let command_end = grapheme_next(&command, command_beg);
                        if buffer_beg >= buffer_size {
                            let piece = command[command_beg..command_end].to_vec();
                            self.replace_range(END, 0, &piece);
                            break;
                        }
                        buffer_beg = grapheme_next(&self.buffer, buffer_beg);
                        command_beg = command_end;
                    }
                }
            }
            VK_INSERT => self.insert_mode = !self.insert_mode,
            VK_DELETE => {
                if self.cursor < self.buffer.len() {
                    let end = self.grapheme_next(self.cursor);
                    self.replace_range(self.cursor, end - self.cursor, &[]);
                }
            }
            VK_UP | VK_F5 => {
                if self.history.as_ref().is_some_and(|history| !history.at_first_command())
                    && let Some(command) = self.retrieve(SearchDirection::Previous)
                {
                    self.replace_all(&command);
                }
            }
            VK_DOWN => {
                if self.history.as_ref().is_some_and(|history| !history.at_last_command())
                    && let Some(command) = self.retrieve(SearchDirection::Next)
                {
                    self.replace_all(&command);
                }
            }
            VK_PRIOR => {
                if self.history.as_ref().is_some_and(|history| !history.at_first_command())
                    && let Some(command) = self.retrieve_nth(0)
                {
                    self.replace_all(&command);
                }
            }
            VK_NEXT => {
                if self.history.as_ref().is_some_and(|history| !history.at_last_command())
                    && let Some(command) = self.retrieve_nth(Index::MAX)
                {
                    self.replace_all(&command);
                }
            }
            VK_F2 => {
                if self.history.is_some() {
                    self.popup_push(PopupKind::CopyToChar);
                }
            }
            VK_F3 => {
                if let Some(command) = self.last_command()
                    && command.len() > self.cursor
                {
                    let piece = command[self.cursor..].to_vec();
                    self.replace_range(self.cursor, END, &piece);
                }
            }
            VK_F4 => self.popup_push(PopupKind::CopyFromChar),
            VK_F6 => {
                // F6 is an alias for ^Z.
                self.handle_char('\u{1a}', modifiers);
            }
            VK_F7 => {
                if !ctrl && !alt {
                    if self.history.as_ref().is_some_and(|history| !history.is_empty()) {
                        self.popup_push(PopupKind::CommandList);
                    }
                } else if alt && let Some(history) = self.history.as_mut() {
                    history.empty();
                }
            }
            VK_F8 => {
                if self.history.is_some() {
                    let cursor = self.cursor;
                    let prefix: String = self.buffer[..cursor].iter().collect();
                    let start = self.history.as_deref().map_or(-1, CommandHistory::last_displayed);
                    let found = self.history.as_mut().and_then(|history| {
                        let (found, index) = history.find_matching(&prefix, start, false, false);
                        found.then_some(index)
                    });
                    if let Some(index) = found
                        && let Some(command) = self.retrieve_nth(index)
                    {
                        self.replace_all(&command);
                        self.set_cursor(cursor);
                    }
                }
            }
            VK_F9 => {
                if self.history.as_ref().is_some_and(|history| !history.is_empty()) {
                    self.popup_push(PopupKind::CommandNumber);
                }
            }
            // Alt+F10 clears cmd.exe's doskey aliases, which `alias.cpp` owns.
            VK_F10 => {}
            _ => {}
        }
    }

    fn finish_read(&mut self) -> CookedReadOutput {
        let mut text = self.text();
        let mut pending_input = None;

        if self.state == State::DoneWithCarriageReturn {
            let newline = if self.processed_input { "\r\n" } else { "\r" };

            // Upstream records the line the user typed, then expands it; the
            // history therefore holds the alias name, not its expansion.
            let mut expansion = None;
            if self.echo_input {
                if let Some(history) = self.history.as_mut() {
                    history.add(&text, false);
                }
                if let Some(aliases) = self.aliases {
                    expansion = aliases.expand(&text, &self.exe_name);
                }
            }

            if let Some(expansion) = expansion {
                text = expansion.text;
                if expansion.line_count > 1 {
                    // Only the first line belongs to this read; the rest starts
                    // the next one. Upstream parks it in the read handle.
                    let first_line_end = text.find('\n').map_or(text.len(), |index| index + 1);
                    pending_input = Some(text[first_line_end..].to_string());
                    text.truncate(first_line_end);
                }
            } else {
                text.push_str(newline);
            }
        }

        CookedReadOutput {
            text,
            control_key_state: self.control_key_state,
            pending_input,
        }
    }

    fn transition(&mut self, state: State) {
        debug_assert_eq!(self.state, State::Accumulating);
        self.state = state;
    }

    /// Replaces `remove` characters at `offset` with `input`.
    ///
    /// `END` as the offset means "past the last character" and as the count
    /// means "to the end", upstream's `npos`.
    fn replace_range(&mut self, offset: usize, remove: usize, input: &[char]) {
        let size = self.buffer.len();
        let offset = offset.min(size);
        let remove = remove.min(size - offset);

        if remove == 0 && input.is_empty() {
            return;
        }

        self.buffer.splice(offset..offset + remove, input.iter().copied());
        self.cursor = offset + input.len();
    }

    fn replace_all(&mut self, characters: &[char]) {
        self.buffer.clear();
        self.buffer.extend_from_slice(characters);
        self.cursor = self.buffer.len();
    }

    fn set_cursor(&mut self, position: usize) {
        self.cursor = position.min(self.buffer.len());
    }

    fn retrieve(&mut self, direction: SearchDirection) -> Option<Vec<char>> {
        self.history
            .as_mut()?
            .retrieve(direction)
            .map(|command| command.chars().collect())
    }

    fn retrieve_nth(&mut self, index: Index) -> Option<Vec<char>> {
        self.history
            .as_mut()?
            .retrieve_nth(index)
            .map(|command| command.chars().collect())
    }

    fn last_command(&self) -> Option<Vec<char>> {
        self.history
            .as_ref()?
            .last_command()
            .map(|command| command.chars().collect())
    }

    fn grapheme_next(&self, position: usize) -> usize {
        grapheme_next(&self.buffer, position)
    }

    fn grapheme_prev(&self, position: usize) -> usize {
        grapheme_prev(&self.buffer, position)
    }

    /// The classic Windows word-wise cursor movement: skip one character, skip
    /// the run of spaces, then skip the run of the same delimiter class.
    fn word_prev(&self, position: usize) -> usize {
        let chars = &self.buffer;
        let mut position = position.min(chars.len());

        if position != 0 {
            position -= 1;
            while position != 0 && chars[position] == ' ' {
                position -= 1;
            }

            let class = delimiter_class(chars[position]);
            while position != 0 && delimiter_class(chars[position - 1]) == class {
                position -= 1;
            }
        }

        position
    }

    /// The mirror of [`word_prev`](Self::word_prev), including the same
    /// inconsistent treatment of spaces and delimiters.
    fn word_next(&self, position: usize) -> usize {
        let chars = &self.buffer;
        let mut position = position.min(chars.len());

        if position < chars.len() {
            position += 1;
            let class = delimiter_class(chars[position - 1]);
            while position != chars.len() && class == delimiter_class(chars[position]) {
                position += 1;
            }
            while position != chars.len() && chars[position] == ' ' {
                position += 1;
            }
        }

        position
    }

    /// Returns the number of rows the command list shows: half the viewport,
    /// at least one and at most twenty, and no more than the history.
    fn popup_height(&self) -> i32 {
        let history_len = self.history.as_ref().map_or(0, |history| history.len() as i32);
        let half_viewport = (self.viewport_height / 2 - 1).clamp(1, 20);
        history_len.min(half_viewport)
    }

    fn popup_push(&mut self, kind: PopupKind) {
        let popup = match kind {
            PopupKind::CopyToChar => Popup::CopyToChar,
            PopupKind::CopyFromChar => Popup::CopyFromChar,
            PopupKind::CommandNumber => Popup::CommandNumber { digits: Vec::new() },
            PopupKind::CommandList => Popup::CommandList {
                top: -1,
                selected: self.history.as_deref().map_or(-1, CommandHistory::last_displayed),
                height: self.popup_height(),
            },
        };
        self.popups.push(popup);
    }

    /// Routes a key to the topmost popup. The topmost popup is the last one
    /// pushed, as upstream's `_popups.back()` is.
    fn popup_handle_input(&mut self, character: char, vkey: u16, modifiers: ControlKeyState) {
        let Some(popup) = self.popups.pop() else {
            debug_assert!(false, "no popup is open");
            return;
        };

        let kept = match popup {
            Popup::CopyToChar => {
                if vkey != 0 {
                    if vkey == VK_ESCAPE {
                        None
                    } else {
                        Some(Popup::CopyToChar)
                    }
                } else {
                    if let Some(command) = self.last_command() {
                        let cursor = self.cursor;
                        if let Some(index) = find_char(&command, cursor, character) {
                            let count = index - cursor;
                            let piece = command[cursor..cursor + count].to_vec();
                            self.replace_range(cursor, count, &piece);
                        }
                    }
                    None
                }
            }
            Popup::CopyFromChar => {
                if vkey != 0 {
                    if vkey == VK_ESCAPE {
                        None
                    } else {
                        Some(Popup::CopyFromChar)
                    }
                } else {
                    let cursor = self.cursor;
                    let index = find_char(&self.buffer, cursor, character).unwrap_or(self.buffer.len());
                    self.replace_range(cursor, index - cursor, &[]);
                    None
                }
            }
            Popup::CommandNumber { mut digits } => {
                if vkey != 0 {
                    if vkey == VK_ESCAPE {
                        None
                    } else {
                        Some(Popup::CommandNumber { digits })
                    }
                } else if character == '\r' {
                    let text: String = digits.iter().collect();
                    if let Ok(index) = text.parse::<Index>()
                        && let Some(command) = self.retrieve_nth(index)
                    {
                        self.replace_all(&command);
                    }
                    None
                } else {
                    if character.is_ascii_digit() && digits.len() < COMMAND_NUMBER_MAX_INPUT_LENGTH {
                        digits.push(character);
                    } else if character == '\u{8}' {
                        digits.pop();
                    }
                    Some(Popup::CommandNumber { digits })
                }
            }
            Popup::CommandList {
                top,
                mut selected,
                mut height,
            } => {
                if character == '\r' {
                    if let Some(command) = self.retrieve_nth(selected) {
                        self.replace_all(&command);
                    }
                    self.handle_char('\r', modifiers);
                    return;
                }

                match vkey {
                    VK_ESCAPE => return,
                    VK_F9 => {
                        // F9 stacks the command-number prompt on the list.
                        self.popups.push(Popup::CommandList { top, selected, height });
                        self.popup_push(PopupKind::CommandNumber);
                        return;
                    }
                    VK_DELETE => {
                        if let Some(history) = self.history.as_mut() {
                            history.remove(selected);
                        }
                        if self.history.as_deref().is_some_and(CommandHistory::is_empty) {
                            return;
                        }
                    }
                    VK_LEFT | VK_RIGHT => {
                        if let Some(command) = self.retrieve_nth(selected) {
                            self.replace_all(&command);
                        }
                        return;
                    }
                    VK_UP => {
                        if modifiers.shift
                            && let Some(history) = self.history.as_mut()
                        {
                            history.swap(selected, selected - 1);
                        }
                        selected -= 1;
                    }
                    VK_DOWN => {
                        if modifiers.shift
                            && let Some(history) = self.history.as_mut()
                        {
                            history.swap(selected, selected + 1);
                        }
                        selected += 1;
                    }
                    VK_HOME => selected = 0,
                    VK_END => selected = Index::MAX,
                    VK_PRIOR => selected -= height,
                    VK_NEXT => selected += height,
                    _ => return,
                }

                // The drawing path clamps the selection into the ring and
                // updates the height it measured; without a drawing path the
                // input handler has to.
                let history_len = self.history.as_ref().map_or(0, |history| history.len() as i32);
                if history_len > 0 {
                    selected = selected.clamp(0, history_len - 1);
                }
                height = self.popup_height();

                Some(Popup::CommandList { top, selected, height })
            }
        };

        if let Some(popup) = kept {
            self.popups.push(popup);
        }
    }
}

/// Returns the index of the first `character` at or after `from`.
fn find_char(chars: &[char], from: usize, character: char) -> Option<usize> {
    chars
        .iter()
        .enumerate()
        .skip(from)
        .find(|&(_, &candidate)| candidate == character)
        .map(|(index, _)| index)
}

/// The next grapheme boundary after `position`.
fn grapheme_next(chars: &[char], position: usize) -> usize {
    if position >= chars.len() {
        return chars.len();
    }

    let mut next = position + 1;
    while next < chars.len() && !utils::unicode::is_grapheme_boundary(chars[next - 1] as u32, chars[next] as u32) {
        next += 1;
    }
    next
}

/// The previous grapheme boundary before `position`.
fn grapheme_prev(chars: &[char], position: usize) -> usize {
    if position == 0 {
        return 0;
    }

    let mut previous = position - 1;
    while previous > 0 && !utils::unicode::is_grapheme_boundary(chars[previous - 1] as u32, chars[previous] as u32) {
        previous -= 1;
    }
    previous
}

/// Returns whether the record is an editing key while a popup is open.
fn is_popup_key(key: &KeyEventRecord) -> bool {
    if key.control_keys.ctrl() || key.control_keys.alt() {
        return false;
    }

    matches!(
        key.virtual_key,
        VK_ESCAPE
            | VK_PRIOR
            | VK_NEXT
            | VK_END
            | VK_HOME
            | VK_LEFT
            | VK_UP
            | VK_RIGHT
            | VK_DOWN
            | VK_F2
            | VK_F4
            | VK_F7
            | VK_F9
            | VK_DELETE
    )
}

/// Returns whether the record is an editing key when no popup is open.
fn is_editing_key(key: &KeyEventRecord) -> bool {
    if !key.control_keys.ctrl()
        && !key.control_keys.alt()
        && matches!(
            key.virtual_key,
            VK_ESCAPE
                | VK_PRIOR
                | VK_NEXT
                | VK_END
                | VK_HOME
                | VK_LEFT
                | VK_UP
                | VK_RIGHT
                | VK_DOWN
                | VK_INSERT
                | VK_DELETE
                | VK_F1
                | VK_F2
                | VK_F3
                | VK_F4
                | VK_F5
                | VK_F6
                | VK_F7
                | VK_F8
                | VK_F9
        )
    {
        return true;
    }

    if key.control_keys.ctrl() && matches!(key.virtual_key, VK_END | VK_HOME | VK_LEFT | VK_RIGHT) {
        return true;
    }

    key.control_keys.alt() && matches!(key.virtual_key, VK_F7 | VK_F10)
}

#[cfg(test)]
mod tests {
    use super::*;
    use types::FocusEventRecord;

    fn history(commands: &[&str]) -> CommandHistory {
        let mut history = CommandHistory::new("cmd.exe", 50);
        for command in commands {
            history.add(command, false);
        }
        history
    }

    fn ctrl() -> ControlKeyState {
        ControlKeyState {
            left_ctrl: true,
            ..ControlKeyState::default()
        }
    }

    fn shift() -> ControlKeyState {
        ControlKeyState {
            shift: true,
            ..ControlKeyState::default()
        }
    }

    fn cooked(history: Option<&mut CommandHistory>) -> CookedReadData<'_> {
        CookedReadData::new(InputMode::new_console(), 24, 0, "", history)
    }

    fn type_text(read: &mut CookedReadData<'_>, text: &str) {
        for character in text.chars() {
            read.handle_char(character, ControlKeyState::default());
        }
    }

    fn key(virtual_key: u16, character: char, modifiers: ControlKeyState) -> InputRecord {
        InputRecord::Key(KeyEventRecord {
            virtual_key,
            control_keys: modifiers,
            ..KeyEventRecord::pressed(character)
        })
    }

    /// Feeds a key the way `_readCharInputLoop` does, so an open popup receives it.
    fn press(read: &mut CookedReadData<'_>, virtual_key: u16, modifiers: ControlKeyState) {
        read.handle_record(key(virtual_key, '\0', modifiers));
    }

    /// Feeds a character the way `_readCharInputLoop` does, so an open popup receives it.
    fn type_character(read: &mut CookedReadData<'_>, character: char) {
        read.handle_record(key(0, character, ControlKeyState::default()));
    }

    fn selected(read: &CookedReadData<'_>) -> Index {
        let Some(Popup::CommandList { selected, .. }) = read.popups().last() else {
            panic!("the command list is open");
        };
        *selected
    }

    #[test]
    fn initial_text_is_placed_before_the_cursor() {
        let read = CookedReadData::new(InputMode::new_console(), 24, 0, "abc", None);
        assert_eq!(read.text(), "abc");
        assert_eq!(read.cursor(), 3);
    }

    #[test]
    fn typing_appends_at_the_cursor() {
        let mut read = cooked(None);
        type_text(&mut read, "abc");
        assert_eq!(read.text(), "abc");
        assert_eq!(read.cursor(), 3);
    }

    #[test]
    fn insert_mode_shifts_the_tail() {
        let mut read = cooked(None);
        read.set_insert_mode(true);
        type_text(&mut read, "abc");
        read.handle_vkey(VK_LEFT, ControlKeyState::default());
        type_text(&mut read, "X");
        assert_eq!(read.text(), "abXc");
    }

    #[test]
    fn overwrite_mode_replaces_the_next_grapheme() {
        let mut read = cooked(None);
        type_text(&mut read, "abc");
        read.handle_vkey(VK_LEFT, ControlKeyState::default());
        read.handle_vkey(VK_LEFT, ControlKeyState::default());
        type_text(&mut read, "X");
        assert_eq!(read.text(), "aXc");
    }

    #[test]
    fn insert_toggles_overwrite_mode() {
        let mut read = cooked(None);
        assert!(!read.insert_mode());
        read.handle_vkey(VK_INSERT, ControlKeyState::default());
        assert!(read.insert_mode());
        read.handle_vkey(VK_INSERT, ControlKeyState::default());
        assert!(!read.insert_mode());
    }

    #[test]
    fn backspace_removes_the_previous_grapheme() {
        let mut read = cooked(None);
        type_text(&mut read, "abc");
        read.handle_char('\u{8}', ControlKeyState::default());
        assert_eq!(read.text(), "ab");
        assert_eq!(read.cursor(), 2);
    }

    #[test]
    fn a_combining_mark_is_one_grapheme() {
        let mut read = cooked(None);
        type_text(&mut read, "a\u{301}b");
        read.handle_char('\u{8}', ControlKeyState::default());
        assert_eq!(read.text(), "a\u{301}");
        read.handle_char('\u{8}', ControlKeyState::default());
        assert_eq!(read.text(), "");
    }

    #[test]
    fn backspace_is_a_character_without_processed_input() {
        let modes = InputMode::new_console().without(InputMode::PROCESSED);
        let mut read = CookedReadData::new(modes, 24, 0, "", None);
        type_text(&mut read, "ab");
        read.handle_char('\u{8}', ControlKeyState::default());
        assert_eq!(read.text(), "ab\u{8}");
    }

    #[test]
    fn ctrl_backspace_removes_the_previous_word() {
        let mut read = cooked(None);
        type_text(&mut read, "echo hello");
        read.handle_char('\u{7f}', ControlKeyState::default());
        assert_eq!(read.text(), "echo ");
    }

    #[test]
    fn ctrl_left_and_right_move_by_word() {
        let mut read = cooked(None);
        type_text(&mut read, "echo hello");
        read.handle_vkey(VK_LEFT, ctrl());
        assert_eq!(read.cursor(), 5);
        read.handle_vkey(VK_RIGHT, ctrl());
        assert_eq!(read.cursor(), 10);
    }

    #[test]
    fn home_and_end_move_the_cursor() {
        let mut read = cooked(None);
        type_text(&mut read, "abc");
        read.handle_vkey(VK_HOME, ControlKeyState::default());
        assert_eq!(read.cursor(), 0);
        assert_eq!(read.text(), "abc");
        read.handle_vkey(VK_END, ControlKeyState::default());
        assert_eq!(read.cursor(), 3);
    }

    #[test]
    fn ctrl_home_erases_to_the_start() {
        let mut read = cooked(None);
        type_text(&mut read, "abc");
        read.handle_vkey(VK_HOME, ctrl());
        assert_eq!(read.text(), "");
        assert_eq!(read.cursor(), 0);
    }

    #[test]
    fn ctrl_end_erases_to_the_end() {
        let mut read = cooked(None);
        type_text(&mut read, "abc");
        read.handle_vkey(VK_HOME, ControlKeyState::default());
        read.handle_vkey(VK_END, ctrl());
        assert_eq!(read.text(), "");
    }

    #[test]
    fn escape_clears_the_line() {
        let mut read = cooked(None);
        type_text(&mut read, "abc");
        read.handle_vkey(VK_ESCAPE, ControlKeyState::default());
        assert_eq!(read.text(), "");
    }

    #[test]
    fn delete_removes_the_grapheme_after_the_cursor() {
        let mut read = cooked(None);
        type_text(&mut read, "abc");
        read.handle_vkey(VK_HOME, ControlKeyState::default());
        read.handle_vkey(VK_DELETE, ControlKeyState::default());
        assert_eq!(read.text(), "bc");
    }

    #[test]
    fn carriage_return_finishes_with_crlf_and_records_the_command() {
        let mut history = CommandHistory::new("cmd.exe", 50);
        {
            let mut read = cooked(Some(&mut history));
            let mut input = InputBuffer::new();
            input.write_text("dir");
            input.write(key(types::virtual_key::VK_RETURN, '\r', ControlKeyState::default()));

            let output = read.read(&mut input).expect("the carriage return ends the read");
            assert_eq!(output.text, "dir\r\n");
        }

        assert_eq!(history.commands().collect::<Vec<_>>(), ["dir"]);
    }

    #[test]
    fn without_processed_input_the_line_ends_with_cr() {
        let modes = InputMode::new_console().without(InputMode::PROCESSED);
        let mut read = CookedReadData::new(modes, 24, 0, "", None);
        type_text(&mut read, "dir");
        read.handle_char('\r', ControlKeyState::default());

        let output = read.read(&mut InputBuffer::new()).expect("the read finished");
        assert_eq!(output.text, "dir\r");
    }

    #[test]
    fn echo_off_does_not_record_the_command() {
        let modes = InputMode::new_console().without(InputMode::ECHO);
        let mut history = CommandHistory::new("cmd.exe", 50);
        {
            let mut read = CookedReadData::new(modes, 24, 0, "", Some(&mut history));
            type_text(&mut read, "dir");
            read.handle_char('\r', ControlKeyState::default());
        }

        assert!(history.is_empty());
    }

    #[test]
    fn a_wakeup_mask_key_erases_the_tail_and_returns_the_character() {
        let modes = InputMode::new_console();
        let mut read = CookedReadData::new(modes, 24, 1 << '\t' as u32, "", None);
        type_text(&mut read, "foo");
        read.handle_vkey(VK_HOME, ControlKeyState::default());
        read.handle_char('\t', ctrl());

        let output = read
            .read(&mut InputBuffer::new())
            .expect("the wakeup key finished the read");
        assert_eq!(output.text, "\t");
        assert_eq!(output.control_key_state, ctrl());
    }

    #[test]
    fn up_and_down_recall_history() {
        let mut history = history(&["echo hello", "dir"]);
        let mut read = cooked(Some(&mut history));

        read.handle_vkey(VK_UP, ControlKeyState::default());
        assert_eq!(read.text(), "dir");
        read.handle_vkey(VK_UP, ControlKeyState::default());
        assert_eq!(read.text(), "echo hello");
        read.handle_vkey(VK_DOWN, ControlKeyState::default());
        assert_eq!(read.text(), "dir");
    }

    #[test]
    fn page_up_and_down_jump_to_the_ends() {
        let mut history = history(&["a", "b", "c"]);
        let mut read = cooked(Some(&mut history));

        read.handle_vkey(VK_PRIOR, ControlKeyState::default());
        assert_eq!(read.text(), "a");
        read.handle_vkey(VK_NEXT, ControlKeyState::default());
        assert_eq!(read.text(), "c");
    }

    #[test]
    fn right_at_the_end_appends_one_grapheme_per_press() {
        let mut history = history(&["echo hello"]);
        let mut read = cooked(Some(&mut history));

        for _ in 0..3 {
            read.handle_vkey(VK_RIGHT, ControlKeyState::default());
        }
        assert_eq!(read.text(), "ech");
    }

    #[test]
    fn f3_copies_the_rest_of_the_command() {
        let mut history = history(&["dir"]);
        let mut read = cooked(Some(&mut history));
        type_text(&mut read, "X");
        read.handle_vkey(VK_F3, ControlKeyState::default());
        assert_eq!(read.text(), "Xir");
    }

    #[test]
    fn f8_finds_a_matching_prefix_and_keeps_the_cursor() {
        let mut history = history(&["make", "git status", "make test"]);
        let mut read = cooked(Some(&mut history));
        type_text(&mut read, "make");
        read.handle_vkey(VK_F8, ControlKeyState::default());
        assert_eq!(read.text(), "make test");
        assert_eq!(read.cursor(), 4);
    }

    #[test]
    fn f2_copies_up_to_a_character() {
        let mut history = history(&["echo hello"]);
        let mut read = cooked(Some(&mut history));
        type_text(&mut read, "ab");
        read.handle_vkey(VK_F2, ControlKeyState::default());
        assert!(read.presenting_popup());

        type_character(&mut read, 'o');
        assert_eq!(read.text(), "abh");
        assert!(!read.presenting_popup());
    }

    #[test]
    fn f2_escape_leaves_the_line_alone() {
        let mut history = history(&["echo hello"]);
        let mut read = cooked(Some(&mut history));
        type_text(&mut read, "ab");
        read.handle_vkey(VK_F2, ControlKeyState::default());
        press(&mut read, VK_ESCAPE, ControlKeyState::default());
        assert_eq!(read.text(), "ab");
        assert!(!read.presenting_popup());
    }

    #[test]
    fn f4_erases_up_to_a_character() {
        let mut read = cooked(None);
        type_text(&mut read, "abcdef");
        read.handle_vkey(VK_HOME, ControlKeyState::default());
        read.handle_vkey(VK_F4, ControlKeyState::default());
        type_character(&mut read, 'c');
        assert_eq!(read.text(), "cdef");
    }

    #[test]
    fn f7_opens_a_command_list_at_the_displayed_command() {
        let mut history = history(&["a", "b", "c"]);
        let mut read = cooked(Some(&mut history));
        read.handle_vkey(VK_F7, ControlKeyState::default());

        assert!(read.presenting_popup());
        assert_eq!(selected(&read), 2);
    }

    #[test]
    fn the_command_list_height_is_half_the_viewport() {
        let commands: Vec<String> = (0..30).map(|index| index.to_string()).collect();
        let commands: Vec<&str> = commands.iter().map(String::as_str).collect();
        let mut history = history(&commands);
        let mut read = cooked(Some(&mut history));
        read.handle_vkey(VK_F7, ControlKeyState::default());

        let Some(Popup::CommandList { height, .. }) = read.popups().last() else {
            panic!("the command list is open");
        };
        // Half of a 24-line viewport is 12, minus the header leaves 11.
        assert_eq!(*height, 11);
    }

    #[test]
    fn up_and_down_move_the_command_list_selection() {
        let mut history = history(&["a", "b", "c"]);
        let mut read = cooked(Some(&mut history));
        read.handle_vkey(VK_F7, ControlKeyState::default());

        press(&mut read, VK_UP, ControlKeyState::default());
        assert_eq!(selected(&read), 1);
        press(&mut read, VK_UP, ControlKeyState::default());
        assert_eq!(selected(&read), 0);
        // The selection clamps at the oldest command.
        press(&mut read, VK_UP, ControlKeyState::default());
        assert_eq!(selected(&read), 0);
        press(&mut read, VK_DOWN, ControlKeyState::default());
        assert_eq!(selected(&read), 1);
    }

    #[test]
    fn shift_up_swaps_the_selected_command_with_its_neighbour() {
        let mut history = history(&["a", "b", "c"]);
        {
            let mut read = cooked(Some(&mut history));
            read.handle_vkey(VK_F7, ControlKeyState::default());
            press(&mut read, VK_UP, shift());
        }

        assert_eq!(history.commands().collect::<Vec<_>>(), ["a", "c", "b"]);
    }

    #[test]
    fn delete_removes_the_selected_command() {
        let mut history = history(&["a", "b", "c"]);
        {
            let mut read = cooked(Some(&mut history));
            read.handle_vkey(VK_F7, ControlKeyState::default());
            press(&mut read, VK_DELETE, ControlKeyState::default());
            assert!(read.presenting_popup());
        }

        assert_eq!(history.commands().collect::<Vec<_>>(), ["a", "b"]);
    }

    #[test]
    fn deleting_the_last_command_closes_the_list() {
        let mut history = history(&["a"]);
        let mut read = cooked(Some(&mut history));
        read.handle_vkey(VK_F7, ControlKeyState::default());
        press(&mut read, VK_DELETE, ControlKeyState::default());
        assert!(!read.presenting_popup());
    }

    #[test]
    fn enter_in_the_command_list_recalls_the_selection_and_finishes() {
        let mut history = history(&["dir", "echo hi"]);
        let mut read = cooked(Some(&mut history));
        read.handle_vkey(VK_F7, ControlKeyState::default());
        type_character(&mut read, '\r');

        let output = read.read(&mut InputBuffer::new()).expect("the read finished");
        assert_eq!(output.text, "echo hi\r\n");
        assert!(!read.presenting_popup());
    }

    #[test]
    fn escape_closes_the_command_list() {
        let mut history = history(&["a"]);
        let mut read = cooked(Some(&mut history));
        read.handle_vkey(VK_F7, ControlKeyState::default());
        press(&mut read, VK_ESCAPE, ControlKeyState::default());
        assert!(!read.presenting_popup());
        assert_eq!(read.text(), "");
    }

    #[test]
    fn f9_recalls_a_command_by_number() {
        let mut history = history(&["a", "b", "c"]);
        let mut read = cooked(Some(&mut history));
        read.handle_vkey(VK_F9, ControlKeyState::default());
        assert!(read.presenting_popup());

        type_character(&mut read, '1');
        type_character(&mut read, '\r');
        assert_eq!(read.text(), "b");
        assert!(!read.presenting_popup());
    }

    #[test]
    fn backspace_edits_the_command_number() {
        let mut history = history(&["a", "b", "c"]);
        let mut read = cooked(Some(&mut history));
        read.handle_vkey(VK_F9, ControlKeyState::default());
        type_character(&mut read, '9');
        type_character(&mut read, '\u{8}');
        type_character(&mut read, '2');
        type_character(&mut read, '\r');
        assert_eq!(read.text(), "c");
    }

    #[test]
    fn a_command_number_is_limited_to_five_digits() {
        let mut history = history(&["a"]);
        let mut read = cooked(Some(&mut history));
        read.handle_vkey(VK_F9, ControlKeyState::default());
        for digit in "123456789".chars() {
            type_character(&mut read, digit);
        }

        let Some(Popup::CommandNumber { digits }) = read.popups().last() else {
            panic!("the command number prompt is open");
        };
        assert_eq!(digits.iter().collect::<String>(), "12345");
    }

    #[test]
    fn f9_stacks_the_number_prompt_on_the_command_list() {
        let mut history = history(&["a"]);
        let mut read = cooked(Some(&mut history));
        read.handle_vkey(VK_F7, ControlKeyState::default());
        read.handle_vkey(VK_F9, ControlKeyState::default());

        assert_eq!(read.popups().len(), 2);
        assert!(matches!(read.popups().last(), Some(Popup::CommandNumber { .. })));
    }

    #[test]
    fn read_waits_when_the_input_buffer_is_empty() {
        let mut read = cooked(None);
        assert!(read.read(&mut InputBuffer::new()).is_none());
    }

    #[test]
    fn read_ignores_non_key_records() {
        let mut read = cooked(None);
        let mut input = InputBuffer::new();
        input.write(InputRecord::Focus(FocusEventRecord { focused: true }));
        input.write_text("a");

        assert!(read.read(&mut input).is_none());
        assert_eq!(read.text(), "a");
    }

    #[test]
    fn control_left_is_an_editing_key() {
        let mut read = cooked(None);
        let mut input = InputBuffer::new();
        input.write_text("echo hello");
        input.write(key(VK_LEFT, '\0', ctrl()));

        assert!(read.read(&mut input).is_none());
        assert_eq!(read.cursor(), 5);
    }

    #[test]
    fn without_history_the_history_keys_do_nothing() {
        let mut read = cooked(None);
        type_text(&mut read, "ab");
        for vkey in [VK_UP, VK_DOWN, VK_PRIOR, VK_NEXT, VK_F2, VK_F7, VK_F9] {
            read.handle_vkey(vkey, ControlKeyState::default());
        }
        assert_eq!(read.text(), "ab");
        assert!(!read.presenting_popup());
    }

    #[test]
    fn an_alias_expands_the_finished_line() {
        let mut aliases = AliasTable::new();
        aliases.add("cmd.exe", "foo", "echo $1").expect("a name");
        let mut history = CommandHistory::new("cmd.exe", 50);

        {
            let mut read = cooked(Some(&mut history)).with_aliases(&aliases, "cmd.exe");
            type_text(&mut read, "foo bar");
            read.handle_char('\r', ControlKeyState::default());

            let output = read.read(&mut InputBuffer::new()).expect("the read finished");
            assert_eq!(output.text, "echo bar\r\n");
            assert_eq!(output.pending_input, None);
        }

        // The history records what the user typed, not the expansion.
        assert_eq!(history.commands().collect::<Vec<_>>(), ["foo bar"]);
    }

    #[test]
    fn an_alias_is_not_expanded_without_echo() {
        let modes = InputMode::new_console().without(InputMode::ECHO);
        let mut aliases = AliasTable::new();
        aliases.add("cmd.exe", "foo", "echo $1").expect("a name");

        let mut read = CookedReadData::new(modes, 24, 0, "", None).with_aliases(&aliases, "cmd.exe");
        type_text(&mut read, "foo bar");
        read.handle_char('\r', ControlKeyState::default());

        let output = read.read(&mut InputBuffer::new()).expect("the read finished");
        assert_eq!(output.text, "foo bar\r\n");
    }

    #[test]
    fn a_multiline_expansion_leaves_the_rest_pending() {
        let mut aliases = AliasTable::new();
        aliases.add("cmd.exe", "foo", "a$tb").expect("a name");

        let mut read = cooked(None).with_aliases(&aliases, "cmd.exe");
        type_text(&mut read, "foo");
        read.handle_char('\r', ControlKeyState::default());

        let output = read.read(&mut InputBuffer::new()).expect("the read finished");
        assert_eq!(output.text, "a\r\n");
        assert_eq!(output.pending_input.as_deref(), Some("b\r\n"));
    }

    #[test]
    fn an_unknown_alias_still_finishes_with_the_line() {
        let mut aliases = AliasTable::new();
        aliases.add("cmd.exe", "foo", "echo $1").expect("a name");

        let mut read = cooked(None).with_aliases(&aliases, "cmd.exe");
        type_text(&mut read, "bar");
        read.handle_char('\r', ControlKeyState::default());

        let output = read.read(&mut InputBuffer::new()).expect("the read finished");
        assert_eq!(output.text, "bar\r\n");
        assert_eq!(output.pending_input, None);
    }
}
