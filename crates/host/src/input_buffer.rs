//! `InputBuffer` — the console's pending input records.
//!
//! Ported from `src/host/inputBuffer.cpp`. Upstream's buffer also owns the
//! `TerminalInput` encoder and the Win32 read buffering (code-page conversion,
//! wait queues, `NTSTATUS`). In this port the encoder lives in `term` and the
//! input modes in the adapter (ADR 009), so the buffer is the storage, its
//! modes and the coalescing rules; a reader takes what is there and keeps the
//! remainder itself.

use std::collections::VecDeque;

use types::{ControlKeyState, InputRecord, KeyEventRecord, MouseEventFlags};

/// The console's input modes, the `ENABLE_*` flags of `InputMode`.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub struct InputMode(u32);

impl InputMode {
    /// `ENABLE_LINE_INPUT` — reads wait for a carriage return.
    pub const LINE: Self = Self(1 << 0);
    /// `ENABLE_PROCESSED_INPUT` — Ctrl+C and friends are handled by the host.
    pub const PROCESSED: Self = Self(1 << 1);
    /// `ENABLE_ECHO_INPUT` — read characters are echoed.
    pub const ECHO: Self = Self(1 << 2);
    /// `ENABLE_MOUSE_INPUT` — mouse events are reported.
    pub const MOUSE: Self = Self(1 << 3);
    /// `ENABLE_WINDOW_INPUT` — window size changes are reported.
    pub const WINDOW: Self = Self(1 << 4);
    /// `ENABLE_VIRTUAL_TERMINAL_INPUT` — input is translated to VT sequences.
    pub const VIRTUAL_TERMINAL: Self = Self(1 << 5);

    /// The mode a new console starts in.
    pub const fn new_console() -> Self {
        Self(Self::LINE.0 | Self::PROCESSED.0 | Self::ECHO.0 | Self::MOUSE.0)
    }

    /// Returns whether every bit in `other` is set.
    pub const fn contains(self, other: Self) -> bool {
        self.0 & other.0 == other.0
    }

    /// Returns a mode set with `other` added.
    pub const fn with(self, other: Self) -> Self {
        Self(self.0 | other.0)
    }

    /// Returns a mode set with `other` removed.
    pub const fn without(self, other: Self) -> Self {
        Self(self.0 & !other.0)
    }
}

impl Default for InputMode {
    fn default() -> Self {
        Self::new_console()
    }
}

/// The records a client has written but not yet read.
#[derive(Debug, Clone, Default, PartialEq, Eq)]
pub struct InputBuffer {
    modes: InputMode,
    storage: VecDeque<InputRecord>,
}

impl InputBuffer {
    /// Creates an empty buffer with the default console modes.
    pub fn new() -> Self {
        Self::default()
    }

    /// Returns the input modes.
    pub const fn modes(&self) -> InputMode {
        self.modes
    }

    /// Replaces the input modes.
    pub fn set_modes(&mut self, modes: InputMode) {
        self.modes = modes;
    }

    /// Returns whether input is translated to VT sequences.
    pub const fn is_virtual_terminal_input(&self) -> bool {
        self.modes.contains(InputMode::VIRTUAL_TERMINAL)
    }

    /// Returns the number of pending records.
    pub fn len(&self) -> usize {
        self.storage.len()
    }

    /// Returns whether no records are pending.
    pub fn is_empty(&self) -> bool {
        self.storage.is_empty()
    }

    /// Stores one record, coalescing it with the last when it is a repeat.
    ///
    /// Returns the number of records stored, which is one whether the record
    /// was appended or merged.
    pub fn write(&mut self, record: InputRecord) -> usize {
        if self.storage.is_empty() || !self.coalesce(&record) {
            self.storage.push_back(record);
        }
        1
    }

    /// Stores a batch of records.
    ///
    /// Upstream only coalesces a single record at a time, because callers
    /// depend on a batch keeping its boundaries; a batch of one takes the
    /// coalescing path.
    pub fn write_events(&mut self, records: impl IntoIterator<Item = InputRecord>) -> usize {
        let mut records = records.into_iter();
        let Some(first) = records.next() else {
            return 0;
        };

        let rest: Vec<InputRecord> = records.collect();
        if rest.is_empty() {
            return self.write(first);
        }

        self.storage.push_back(first);
        let written = 1 + rest.len();
        self.storage.extend(rest);
        written
    }

    /// Inserts `records` before everything already stored.
    pub fn prepend(&mut self, records: impl IntoIterator<Item = InputRecord>) -> usize {
        let mut existing = std::mem::take(&mut self.storage);
        let written = self.write_events(records);
        self.storage.append(&mut existing);
        written
    }

    /// Stores `text` as one key record per character.
    ///
    /// A NUL character is `Ctrl+@` on a PC keyboard, and is stored as such, as
    /// upstream's `VkKeyScanW(0)` reports.
    pub fn write_text(&mut self, text: &str) -> usize {
        let mut written = 0;
        for character in text.chars() {
            let control_keys = if character == '\0' {
                ControlKeyState {
                    left_ctrl: true,
                    ..ControlKeyState::default()
                }
            } else {
                ControlKeyState::default()
            };
            self.storage.push_back(InputRecord::Key(KeyEventRecord {
                control_keys,
                ..KeyEventRecord::pressed(character)
            }));
            written += 1;
        }
        written
    }

    /// Copies up to `count` records without removing them.
    pub fn peek(&self, count: usize) -> Vec<InputRecord> {
        self.storage.iter().take(count).copied().collect()
    }

    /// Removes and returns up to `count` records.
    pub fn read(&mut self, count: usize) -> Vec<InputRecord> {
        let take = count.min(self.storage.len());
        self.storage.drain(..take).collect()
    }

    /// Drops every pending record.
    pub fn flush(&mut self) {
        self.storage.clear();
    }

    /// Drops every pending record that is not a key event.
    pub fn flush_all_but_keys(&mut self) {
        self.storage.retain(InputRecord::is_key);
    }

    /// Merges `incoming` into the last record when the two describe one event.
    fn coalesce(&mut self, incoming: &InputRecord) -> bool {
        let Some(last) = self.storage.back_mut() else {
            return false;
        };

        match (last, incoming) {
            (InputRecord::Mouse(last), InputRecord::Mouse(incoming))
                if last.event_flags.contains(MouseEventFlags::MOVED)
                    && incoming.event_flags.contains(MouseEventFlags::MOVED) =>
            {
                last.position = incoming.position;
                true
            }
            (InputRecord::WindowSize(last), InputRecord::WindowSize(incoming)) => {
                *last = *incoming;
                true
            }
            // A repeat of the same key press adds to the repeat count. Upstream
            // also merges when the scan code differs under an IME conversion
            // flag, which has no portable counterpart. A `char` cannot be a lone
            // surrogate, so upstream's surrogate-pair exclusion is structural.
            (InputRecord::Key(last), InputRecord::Key(incoming))
                if last.key_down
                    && incoming.key_down
                    && last.scan_code == incoming.scan_code
                    && last.character == incoming.character
                    && last.control_keys == incoming.control_keys =>
            {
                last.repeat_count = last.repeat_count.saturating_add(incoming.repeat_count);
                true
            }
            _ => false,
        }
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use types::{FocusEventRecord, KeyEventRecord, MouseEventRecord, WindowSizeEventRecord};

    fn keys(buffer: &InputBuffer) -> Vec<char> {
        buffer
            .storage
            .iter()
            .filter_map(InputRecord::key)
            .map(|record| record.character)
            .collect()
    }

    #[test]
    fn a_new_console_has_the_default_modes() {
        let buffer = InputBuffer::new();
        assert!(buffer.is_empty());
        assert!(buffer.modes().contains(InputMode::LINE));
        assert!(buffer.modes().contains(InputMode::PROCESSED));
        assert!(buffer.modes().contains(InputMode::ECHO));
        assert!(buffer.modes().contains(InputMode::MOUSE));
        assert!(!buffer.is_virtual_terminal_input());
    }

    #[test]
    fn a_written_record_can_be_read_back() {
        let mut buffer = InputBuffer::new();
        assert_eq!(buffer.write(InputRecord::Key(KeyEventRecord::pressed('a'))), 1);

        let read = buffer.read(1);
        assert_eq!(read.len(), 1);
        assert_eq!(read[0].key().map(|record| record.character), Some('a'));
        assert!(buffer.is_empty());
    }

    #[test]
    fn a_repeated_key_press_adds_to_the_repeat_count() {
        let mut buffer = InputBuffer::new();
        buffer.write(InputRecord::Key(KeyEventRecord::pressed('a')));
        buffer.write(InputRecord::Key(KeyEventRecord::pressed('a')));

        assert_eq!(buffer.len(), 1);
        let record = buffer.peek(1)[0].key().copied().unwrap_or_default();
        assert_eq!(record.repeat_count, 2);
    }

    #[test]
    fn a_different_key_is_stored_separately() {
        let mut buffer = InputBuffer::new();
        buffer.write(InputRecord::Key(KeyEventRecord::pressed('a')));
        buffer.write(InputRecord::Key(KeyEventRecord::pressed('b')));

        assert_eq!(buffer.len(), 2);
        assert_eq!(keys(&buffer), ['a', 'b']);
    }

    #[test]
    fn a_key_release_is_not_coalesced() {
        let mut buffer = InputBuffer::new();
        buffer.write(InputRecord::Key(KeyEventRecord::pressed('a')));
        buffer.write(InputRecord::Key(KeyEventRecord::released('a')));

        assert_eq!(buffer.len(), 2);
    }

    #[test]
    fn a_mouse_move_updates_the_last_position() {
        let mut buffer = InputBuffer::new();
        buffer.write(InputRecord::Mouse(MouseEventRecord::moved((1, 2))));
        buffer.write(InputRecord::Mouse(MouseEventRecord::moved((3, 4))));

        assert_eq!(buffer.len(), 1);
        let InputRecord::Mouse(record) = buffer.peek(1)[0] else {
            panic!("the record is a mouse event");
        };
        assert_eq!(record.position, (3, 4));
    }

    #[test]
    fn a_window_size_event_replaces_the_previous_one() {
        let mut buffer = InputBuffer::new();
        buffer.write(InputRecord::WindowSize(WindowSizeEventRecord { size: (80, 24) }));
        buffer.write(InputRecord::WindowSize(WindowSizeEventRecord { size: (100, 40) }));

        assert_eq!(buffer.len(), 1);
        assert_eq!(
            buffer.peek(1)[0],
            InputRecord::WindowSize(WindowSizeEventRecord { size: (100, 40) })
        );
    }

    #[test]
    fn a_batch_keeps_its_boundaries() {
        let mut buffer = InputBuffer::new();
        let written = buffer.write_events([
            InputRecord::Key(KeyEventRecord::pressed('a')),
            InputRecord::Key(KeyEventRecord::pressed('a')),
        ]);

        assert_eq!(written, 2);
        assert_eq!(buffer.len(), 2);
    }

    #[test]
    fn a_batch_of_one_coalesces() {
        let mut buffer = InputBuffer::new();
        buffer.write(InputRecord::Key(KeyEventRecord::pressed('a')));
        buffer.write_events([InputRecord::Key(KeyEventRecord::pressed('a'))]);

        assert_eq!(buffer.len(), 1);
    }

    #[test]
    fn written_text_becomes_one_record_per_character() {
        let mut buffer = InputBuffer::new();
        assert_eq!(buffer.write_text("ab"), 2);
        assert_eq!(keys(&buffer), ['a', 'b']);
    }

    #[test]
    fn a_nul_character_is_ctrl_at() {
        let mut buffer = InputBuffer::new();
        buffer.write_text("\0");

        let record = buffer.peek(1)[0].key().copied().unwrap_or_default();
        assert_eq!(record.character, '\0');
        assert!(record.control_keys.left_ctrl);
    }

    #[test]
    fn prepending_puts_the_records_first() {
        let mut buffer = InputBuffer::new();
        buffer.write(InputRecord::Key(KeyEventRecord::pressed('b')));
        buffer.prepend([InputRecord::Key(KeyEventRecord::pressed('a'))]);

        assert_eq!(keys(&buffer), ['a', 'b']);
    }

    #[test]
    fn peek_does_not_consume_and_read_does() {
        let mut buffer = InputBuffer::new();
        buffer.write_text("ab");

        assert_eq!(buffer.peek(1).len(), 1);
        assert_eq!(buffer.len(), 2);
        assert_eq!(buffer.read(1).len(), 1);
        assert_eq!(buffer.len(), 1);
    }

    #[test]
    fn flushing_drops_everything() {
        let mut buffer = InputBuffer::new();
        buffer.write_text("ab");
        buffer.flush();
        assert!(buffer.is_empty());
    }

    #[test]
    fn flushing_all_but_keys_keeps_the_keys() {
        let mut buffer = InputBuffer::new();
        buffer.write(InputRecord::Key(KeyEventRecord::pressed('a')));
        buffer.write(InputRecord::Focus(FocusEventRecord { focused: true }));
        buffer.write(InputRecord::Mouse(MouseEventRecord::moved((1, 1))));

        buffer.flush_all_but_keys();
        assert_eq!(buffer.len(), 1);
        assert!(buffer.peek(1)[0].is_key());
    }

    #[test]
    fn reading_past_the_end_returns_what_is_there() {
        let mut buffer = InputBuffer::new();
        buffer.write_text("a");
        assert_eq!(buffer.read(5).len(), 1);
        assert!(buffer.read(5).is_empty());
    }
}
