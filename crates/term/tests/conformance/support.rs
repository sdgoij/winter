//! A screen a conformance test drives and reads.

use buffer::TextBuffer;
use term::Terminal;
use types::TextAttribute;

/// A terminal with a fixed page size and the accessors the corpus asserts on.
pub struct Screen {
    terminal: Terminal,
    width: i32,
    height: i32,
}

impl Screen {
    /// Creates a blank `width` by `height` screen.
    pub fn new(width: i32, height: i32) -> Self {
        let mut terminal = Terminal::new();
        terminal.resize(width, height);
        Self {
            terminal,
            width,
            height,
        }
    }

    /// Feeds terminal output.
    pub fn feed(&mut self, bytes: &[u8]) {
        self.terminal.process_bytes(bytes);
    }

    /// Feeds terminal output from a string.
    pub fn feed_str(&mut self, text: &str) {
        self.terminal.process_bytes(text.as_bytes());
    }

    /// Returns one row with its trailing blanks removed.
    pub fn line(&mut self, row: i32) -> String {
        let width = self.width;
        self.with_buffer(|buffer| {
            let mut text = String::new();
            for col in 0..width {
                match buffer.get_text_data_at(row, col) {
                    Some(cell) => text.push_str(cell),
                    None => break,
                }
            }
            text.trim_end().to_string()
        })
    }

    /// Returns every row, joined by newlines, with trailing blanks removed.
    pub fn text(&mut self) -> String {
        let height = self.height;
        let mut rows = Vec::new();
        for row in 0..height {
            rows.push(self.line(row));
        }
        rows.join("\n")
    }

    /// Returns the text of one cell.
    pub fn cell(&mut self, row: i32, col: i32) -> String {
        self.with_buffer(|buffer| buffer.get_text_data_at(row, col).unwrap_or("").to_string())
    }

    /// Returns one cell's attributes.
    pub fn attributes(&mut self, row: i32, col: i32) -> TextAttribute {
        self.with_buffer(|buffer| match buffer.get_cell_data_at(row, col) {
            Some(cell) => *cell.get_attributes(),
            None => TextAttribute::new(),
        })
    }

    /// Returns the cursor position as `(row, col)`.
    pub fn cursor(&mut self) -> (i32, i32) {
        self.with_buffer(|buffer| buffer.cursor().position())
    }

    /// Returns whether the cursor is visible.
    pub fn cursor_visible(&mut self) -> bool {
        self.with_buffer(|buffer| buffer.cursor().is_visible())
    }

    /// Drains the responses owed to the program.
    pub fn responses(&mut self) -> Vec<String> {
        self.terminal.take_responses()
    }

    fn with_buffer<R>(&mut self, read: impl FnOnce(&TextBuffer) -> R) -> R {
        let buffer = self.terminal.page_manager_mut().active_page().buffer();
        let guard = match buffer.read() {
            Ok(guard) => guard,
            Err(poisoned) => poisoned.into_inner(),
        };
        read(&guard)
    }
}
