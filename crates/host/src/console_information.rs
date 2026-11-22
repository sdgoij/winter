//! `CONSOLE_INFORMATION` — the console's shared state.
//!
//! Ported from `consoleInformation.cpp` and the class declaration in
//! `server.h`. The Win32 build reaches for the screen buffers, the VT I/O
//! object, the MIDI player, the code pages and the console window; those belong
//! to `screenInfo.cpp`, the server and the UI layer, and are named here rather
//! than stubbed.
//!
//! What remains is the state the host's own modules read: the input buffer, the
//! per-application command histories, the titles, the flags, bracketed paste and
//! the pending clipboard text.

use bitflags::bitflags;
use utils::ticket_lock::RecursiveTicketLock;

use crate::history::CommandHistoryList;
use crate::input_buffer::InputBuffer;

bitflags! {
    /// `CONSOLE_INFORMATION::Flags` from `server.h`.
    ///
    /// The values are part of the console's observable state, not a local
    /// convenience, so they keep their documented numbers.
    #[derive(Debug, Clone, Copy, PartialEq, Eq, Hash)]
    pub struct ConsoleFlags: u32 {
        /// The window is minimized.
        const IS_ICONIC = 0x0000_0001;
        /// Output is suspended.
        const OUTPUT_SUSPENDED = 0x0000_0002;
        /// The window has focus.
        const HAS_FOCUS = 0x0000_0004;
        /// The next mouse event is ignored.
        const IGNORE_NEXT_MOUSE_INPUT = 0x0000_0008;
        /// The user is selecting text.
        const SELECTING = 0x0000_0010;
        /// The user is scrolling.
        const SCROLLING = 0x0000_0020;
        /// The scroll bars are being recalculated.
        const UPDATING_SCROLL_BARS = 0x0000_0400;
        /// Quick-edit selection is enabled.
        const QUICK_EDIT_MODE = 0x0000_0800;
        /// The console is attached to an emulator.
        const CONNECTED_TO_EMULATOR = 0x0000_2000;
        /// A quit message has been posted.
        const QUIT_POSTED = 0x0000_8000;
        /// The window positions itself automatically.
        const AUTO_POSITION = 0x0001_0000;
        /// The next key-up event is ignored.
        const IGNORE_NEXT_KEYUP = 0x0002_0000;
        /// Duplicate commands are not added to a history.
        const HISTORY_NODUP = 0x0010_0000;
        /// The user is dragging a scroll-bar thumb.
        const SCROLLBAR_TRACKING = 0x0020_0000;
        /// The console is applying a window size.
        const SETTING_WINDOW_SIZE = 0x0080_0000;
        /// The private console flags API is enabled.
        const USE_PRIVATE_FLAGS = 0x2000_0000;
        /// The console finished initializing.
        const INITIALIZED = 0x8000_0000;
    }
}

/// The console, as the host's modules see it.
#[derive(Debug)]
pub struct ConsoleInformation {
    input_buffer: InputBuffer,
    history: CommandHistoryList,
    flags: ConsoleFlags,
    title: String,
    title_prefix: String,
    title_and_prefix: String,
    original_title: String,
    link_title: String,
    bracketed_paste_mode: bool,
    pending_clipboard_text: Option<String>,
    /// `None` when no read is pending; `Some(presenting_popup)` when one is.
    cooked_read: Option<bool>,
    lock: RecursiveTicketLock,
}

impl ConsoleInformation {
    /// Creates an empty console.
    pub fn new() -> Self {
        Self {
            input_buffer: InputBuffer::new(),
            history: CommandHistoryList::new(),
            flags: ConsoleFlags::empty(),
            title: String::new(),
            title_prefix: String::new(),
            title_and_prefix: String::new(),
            original_title: String::new(),
            link_title: String::new(),
            bracketed_paste_mode: false,
            pending_clipboard_text: None,
            cooked_read: None,
            lock: RecursiveTicketLock::new(),
        }
    }

    /// Acquires the console lock. Recursive, as upstream's is.
    pub fn lock(&self) {
        self.lock.lock();
    }

    /// Releases one acquisition of the console lock.
    pub fn unlock(&self) {
        self.lock.unlock();
    }

    /// Returns whether the calling thread holds the console lock.
    pub fn is_console_locked(&self) -> bool {
        self.lock.is_locked()
    }

    /// Returns the calling thread's lock recursion depth.
    pub fn recursion_depth(&self) -> u32 {
        self.lock.recursion_depth()
    }

    /// Returns the console's input modes and pending records.
    pub const fn input_buffer(&self) -> &InputBuffer {
        &self.input_buffer
    }

    /// Returns the console's input buffer mutably.
    pub const fn input_buffer_mut(&mut self) -> &mut InputBuffer {
        &mut self.input_buffer
    }

    /// Returns the per-application command histories.
    pub const fn history(&self) -> &CommandHistoryList {
        &self.history
    }

    /// Returns the per-application command histories mutably.
    pub const fn history_mut(&mut self) -> &mut CommandHistoryList {
        &mut self.history
    }

    /// Returns the console flags.
    pub const fn flags(&self) -> ConsoleFlags {
        self.flags
    }

    /// Returns the console flags mutably.
    pub const fn flags_mut(&mut self) -> &mut ConsoleFlags {
        &mut self.flags
    }

    /// Sets the title without the prefix.
    pub fn set_title(&mut self, title: &str) {
        self.title.clear();
        self.title.push_str(title);
        self.recompose_title();
    }

    /// Sets the title prefix, the "Mark", "Select" or "Scroll" a window shows.
    pub fn set_title_prefix(&mut self, prefix: &str) {
        self.title_prefix.clear();
        self.title_prefix.push_str(prefix);
        self.recompose_title();
    }

    /// Sets the title the console launched with.
    pub fn set_original_title(&mut self, original_title: &str) {
        self.original_title.clear();
        self.original_title.push_str(original_title);
    }

    /// Sets the shortcut's title, empty when the console was not launched from
    /// one.
    pub fn set_link_title(&mut self, link_title: &str) {
        self.link_title.clear();
        self.link_title.push_str(link_title);
    }

    /// Returns the title without the prefix.
    pub fn title(&self) -> &str {
        &self.title
    }

    /// Returns the title with its prefix, as a window shows it.
    pub fn title_and_prefix(&self) -> &str {
        &self.title_and_prefix
    }

    /// Returns the title the console launched with.
    pub fn original_title(&self) -> &str {
        &self.original_title
    }

    /// Returns the shortcut's title.
    pub fn link_title(&self) -> &str {
        &self.link_title
    }

    /// Returns whether pasted text is bracketed.
    pub const fn bracketed_paste_mode(&self) -> bool {
        self.bracketed_paste_mode
    }

    /// Turns bracketed paste on or off.
    pub const fn set_bracketed_paste_mode(&mut self, enabled: bool) {
        self.bracketed_paste_mode = enabled;
    }

    /// Offers `text` to the window's clipboard.
    ///
    /// Upstream posts a message to the GUI thread because only it may touch the
    /// clipboard. The portable host keeps the text here and the window layer
    /// takes it with [`use_pending_clipboard_text`](Self::use_pending_clipboard_text).
    pub fn copy_text_to_clipboard(&mut self, text: &str) {
        self.pending_clipboard_text = Some(text.to_owned());
    }

    /// Takes the pending clipboard text, leaving none.
    pub fn use_pending_clipboard_text(&mut self) -> Option<String> {
        self.pending_clipboard_text.take()
    }

    /// Records whether a cooked read is pending, and whether it shows a popup.
    ///
    /// Upstream keeps a non-owning pointer to the read, which lives on the
    /// stack of the routine driving it. The console therefore only records what
    /// these two predicates report.
    pub const fn set_cooked_read_data(&mut self, presenting_popup: Option<bool>) {
        self.cooked_read = presenting_popup;
    }

    /// Returns whether a cooked read is pending.
    pub const fn has_pending_cooked_read(&self) -> bool {
        self.cooked_read.is_some()
    }

    /// Returns whether the pending cooked read is showing a popup.
    pub const fn has_pending_popup(&self) -> bool {
        matches!(self.cooked_read, Some(true))
    }

    fn recompose_title(&mut self) {
        self.title_and_prefix.clear();
        self.title_and_prefix.push_str(&self.title_prefix);
        self.title_and_prefix.push_str(&self.title);
    }
}

impl Default for ConsoleInformation {
    fn default() -> Self {
        Self::new()
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use types::{InputRecord, KeyEventRecord};

    #[test]
    fn titles_compose_with_the_prefix() {
        let mut console = ConsoleInformation::new();
        console.set_title("Command Prompt");
        assert_eq!(console.title(), "Command Prompt");
        assert_eq!(console.title_and_prefix(), "Command Prompt");

        console.set_title_prefix("Select ");
        assert_eq!(console.title(), "Command Prompt");
        assert_eq!(console.title_and_prefix(), "Select Command Prompt");
    }

    #[test]
    fn changing_the_title_recomposes_the_displayed_one() {
        let mut console = ConsoleInformation::new();
        console.set_title_prefix("Mark ");
        console.set_title("one");
        console.set_title("two");
        assert_eq!(console.title_and_prefix(), "Mark two");
    }

    #[test]
    fn original_and_link_titles_are_separate() {
        let mut console = ConsoleInformation::new();
        console.set_original_title("original");
        console.set_link_title("C:\\shortcut.lnk");

        assert_eq!(console.original_title(), "original");
        assert_eq!(console.link_title(), "C:\\shortcut.lnk");
        assert_eq!(console.title(), "");
    }

    #[test]
    fn bracketed_paste_toggles() {
        let mut console = ConsoleInformation::new();
        assert!(!console.bracketed_paste_mode());
        console.set_bracketed_paste_mode(true);
        assert!(console.bracketed_paste_mode());
    }

    #[test]
    fn the_clipboard_text_is_consumed_once() {
        let mut console = ConsoleInformation::new();
        console.copy_text_to_clipboard("copied");

        assert_eq!(console.use_pending_clipboard_text(), Some("copied".to_string()));
        assert_eq!(console.use_pending_clipboard_text(), None);
    }

    #[test]
    fn a_pending_read_reports_its_popup() {
        let mut console = ConsoleInformation::new();
        assert!(!console.has_pending_cooked_read());
        assert!(!console.has_pending_popup());

        console.set_cooked_read_data(Some(true));
        assert!(console.has_pending_cooked_read());
        assert!(console.has_pending_popup());

        console.set_cooked_read_data(Some(false));
        assert!(console.has_pending_cooked_read());
        assert!(!console.has_pending_popup());

        console.set_cooked_read_data(None);
        assert!(!console.has_pending_cooked_read());
    }

    #[test]
    fn the_lock_is_recursive() {
        let console = ConsoleInformation::new();
        assert!(!console.is_console_locked());

        console.lock();
        console.lock();
        assert!(console.is_console_locked());
        assert_eq!(console.recursion_depth(), 2);

        console.unlock();
        assert!(console.is_console_locked());
        console.unlock();
        assert!(!console.is_console_locked());
    }

    #[test]
    fn the_console_owns_its_input_buffer() {
        let mut console = ConsoleInformation::new();
        console
            .input_buffer_mut()
            .write(InputRecord::Key(KeyEventRecord::pressed('a')));

        let record = console.input_buffer().peek(1);
        assert_eq!(record[0].key().map(|key| key.character), Some('a'));
    }

    #[test]
    fn the_histories_are_keyed_by_application() {
        let mut console = ConsoleInformation::new();
        console.history_mut().find_or_create("cmd.exe", 50).add("dir", false);
        console
            .history_mut()
            .find_or_create("powershell.exe", 50)
            .add("Get-ChildItem", false);

        assert_eq!(console.history().len(), 2);
    }

    #[test]
    fn the_flags_start_empty_and_keep_their_bits() {
        let mut console = ConsoleInformation::new();
        assert!(console.flags().is_empty());

        console
            .flags_mut()
            .insert(ConsoleFlags::QUICK_EDIT_MODE | ConsoleFlags::HISTORY_NODUP);
        assert!(console.flags().contains(ConsoleFlags::QUICK_EDIT_MODE));
        assert_eq!(console.flags().bits(), 0x0010_0800);
    }
}
