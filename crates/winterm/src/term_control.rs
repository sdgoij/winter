//! The terminal a pane controls, ported from `ControlCore`.
//!
//! `ControlCore` is the part of a Windows Terminal control that has no XAML: the
//! terminal, its connection, the events it raises and the state the widget asks
//! about. `TermControl` is the XAML half. This port's `TermControl` is `Pane` in
//! `ui.rs`: it translates egui events and paints, and it owns the viewport and
//! the selection, which ADR 019 keeps in the application (ADR 036).
//!
//! The connection is a trait object, so a test drives a whole terminal from a
//! byte buffer with no pty in sight.

use std::sync::{Arc, Mutex};
use std::time::{Duration, Instant};

use server::pty::PtySize;
use term::Terminal;
use term::input::{KeyEvent, MouseEvent};
use types::color_table::ColorTable;

use crate::term_connection::Connection;
use crate::term_events::TerminalEvent;
use crate::view::Selection;
use crate::visual_bell::VisualBell;
use crate::{grid, view::View};

/// A terminal and the shell behind it.
pub struct Control {
    terminal: Terminal,
    connection: Box<dyn Connection>,
    /// The flash a bell started, until it fades.
    bell: VisualBell,
    /// Events found but not yet taken.
    events: Vec<TerminalEvent>,
    /// The title the program set with OSC 0/2.
    title: Option<String>,
    /// The grid size the terminal and the connection were last told about.
    size: (i32, i32),
    /// The output stream ended.
    exited: bool,
    /// Whether the end has already been reported as an event.
    exit_reported: bool,
}

impl Control {
    /// Wraps `connection` in a terminal of `size` cells.
    pub fn new(connection: Box<dyn Connection>, size: (i32, i32)) -> Self {
        let mut terminal = Terminal::new();
        terminal.resize(size.0, size.1);
        // OSC 777 is ignored unless the application asks for it, which is
        // upstream's `DesktopNotification` optional feature.
        terminal.set_optional_features(std::collections::HashSet::from([
            term::OptionalFeature::DesktopNotification,
        ]));

        Self {
            terminal,
            connection,
            bell: VisualBell::default(),
            events: Vec::new(),
            title: None,
            size,
            exited: false,
            exit_reported: false,
        }
    }

    /// Makes the palette the built-in table with `scheme` on top.
    ///
    /// At creation the scheme fills the table before the shell can write to it,
    /// so a program's OSC 10/11 arrives later and wins. On a reload the file is
    /// the later word, so the table is rebuilt and a program's OSC colours go.
    pub fn apply_scheme(&mut self, scheme: Option<&settings::Scheme>) {
        let handle = self.color_table();
        let Ok(mut table) = handle.lock() else {
            return;
        };
        match scheme {
            Some(scheme) => scheme.apply(&mut table),
            None => table.reset(),
        }
    }

    /// The palette the pane paints with.
    pub fn color_table(&self) -> Arc<Mutex<ColorTable>> {
        self.terminal.dispatch().color_table()
    }

    /// Moves bytes from the connection into the parser and answers the program.
    pub fn pump(&mut self) {
        let mut bytes = Vec::new();
        if self.connection.drain_into(&mut bytes) {
            self.exited = true;
        }
        // The stream does not always end when the shell does: a ConPTY keeps
        // its pipe open, so the backend's process is asked too.
        if self.connection.exited() {
            self.exited = true;
        }
        if bytes.is_empty() {
            return;
        }

        self.terminal.process_bytes(&bytes);
        for response in self.terminal.take_responses() {
            self.write(&response);
        }

        if let Some(title) = self.terminal.window_title()
            && self.title.as_deref() != Some(title)
        {
            self.title = Some(title.to_string());
        }
    }

    /// Sends text to the program, as if it had been typed.
    pub fn write(&mut self, text: &str) {
        if text.is_empty() {
            return;
        }
        if self.connection.write(text.as_bytes()).is_err() {
            self.exited = true;
        }
    }

    /// Encodes a key with the terminal's input modes and sends it.
    pub fn send_key(&mut self, event: &KeyEvent) {
        if let Some(bytes) = self.terminal.handle_key(event) {
            self.write(&bytes);
        }
    }

    /// Encodes a mouse event with the terminal's tracking mode and sends it.
    ///
    /// Returns whether the terminal wanted the event; a terminal that is not
    /// tracking the mouse owns the wheel and the clicks for itself.
    pub fn send_mouse(&mut self, event: &MouseEvent) -> bool {
        match self.terminal.handle_mouse(event) {
            Some(bytes) => {
                self.write(&bytes);
                true
            }
            None => false,
        }
    }

    /// Pastes text through the terminal's bracketed-paste filter.
    pub fn paste(&mut self, text: &str) {
        let bytes = self.terminal.paste(text);
        self.write(&bytes);
    }

    /// Tells the terminal and the program how large the grid is.
    pub fn resize(&mut self, columns: i32, rows: i32) {
        self.size = (columns, rows);
        self.terminal.resize(columns, rows);
        let size = PtySize::new(cell_count(columns), cell_count(rows));
        let _ = self.connection.resize(size);
    }

    /// The grid size in cells.
    pub fn size(&self) -> (i32, i32) {
        self.size
    }

    /// The title the program set, if it set one.
    pub fn title(&self) -> Option<&str> {
        self.title.as_deref()
    }

    /// Whether the program is tracking the mouse.
    pub fn is_tracking_mouse(&self) -> bool {
        self.terminal.is_tracking_mouse_input()
    }

    /// Takes what happened since the last call.
    ///
    /// A bell is reported once per flash, because the light ignores a bell that
    /// arrives while it is fading; an exit is reported once, because the stream
    /// stays ended.
    pub fn take_events(&mut self, now: Instant) -> Vec<TerminalEvent> {
        let mut events = std::mem::take(&mut self.events);

        if self.terminal.take_bell_requests() > 0 && self.bell.flash(now) {
            events.push(TerminalEvent::Bell);
        }
        for (title, body) in self.terminal.take_notifications() {
            events.push(TerminalEvent::Notification { title, body });
        }
        if self.exited && !self.exit_reported {
            self.exit_reported = true;
            events.push(TerminalEvent::Exited);
        }

        events
    }

    /// Takes the DECPS notes the program asked for.
    pub fn take_sounds(&mut self) -> Vec<audio::Note> {
        self.terminal
            .take_sound_requests()
            .into_iter()
            .map(|note| audio::Note {
                number: note.note,
                velocity: note.velocity,
                duration: Duration::from_micros(u64::from(note.duration_micros)),
            })
            .collect()
    }

    /// The bell flash's opacity at `now`.
    pub fn bell_alpha(&self, now: Instant) -> Option<f32> {
        self.bell.alpha(now)
    }

    /// When a running bell flash needs its next frame.
    pub fn bell_repaint(&self, now: Instant) -> Option<Duration> {
        self.bell.next_repaint(now)
    }

    /// The visible rows, with their marks, for painting and scrolling.
    pub fn snapshot(&mut self, view: &View) -> grid::Screen {
        let scroll = view.scroll();
        let buffer = self.terminal.page_manager_mut().active_page().buffer();
        match buffer.read() {
            Ok(buffer) => grid::snapshot_at(&buffer, scroll),
            Err(_) => grid::Screen::default(),
        }
    }

    /// Copies the visible rows, history included, into `target`.
    ///
    /// The renderer needs a `TextBuffer`, but the terminal's lives behind a
    /// lock. Copying the screen-sized view into the shell's own buffer lets a
    /// frame borrow it without holding the lock (ADR 040).
    pub fn copy_visible_into(&mut self, target: &mut buffer::TextBuffer, scroll: i32) {
        let buffer = self.terminal.page_manager_mut().active_page().buffer();
        let Ok(buffer) = buffer.read() else {
            return;
        };

        let total = buffer.document_height();
        let height = buffer.screen_height();
        let scroll = scroll.clamp(0, (total - height).max(0));
        let first = (total - height - scroll).max(0);

        for row in 0..height {
            let source = buffer.get_row_by_document_line(first + row);
            let destination = target.get_row_at_screen_line_mut(row);
            if let (Some(source), Some(destination)) = (source, destination) {
                destination.copy_from(source);
            }
        }
    }

    /// The selected cells, using the buffer's copy path.
    pub fn selection_text(&mut self, selection: &Selection) -> Option<String> {
        let buffer = self.terminal.page_manager_mut().active_page().buffer();
        let buffer = buffer.read().ok()?;
        let text = selection.text(&buffer);
        (!text.is_empty()).then_some(text)
    }

    /// The matches for `query` and the document height they were found in.
    pub fn find_matches(&mut self, query: &str) -> (Vec<buffer::TextMatch>, i32) {
        let buffer = self.terminal.page_manager_mut().active_page().buffer();
        let buffer = buffer.read().unwrap_or_else(|poisoned| poisoned.into_inner());
        (buffer.find_text(query), buffer.document_height())
    }

    /// The text of one document row, for word selection.
    pub fn row_text(&mut self, document_row: i32) -> String {
        let buffer = self.terminal.page_manager_mut().active_page().buffer();
        match buffer.read() {
            Ok(buffer) => buffer
                .get_row_by_document_line(document_row)
                .map(buffer::Row::get_text)
                .unwrap_or_default(),
            Err(_) => String::new(),
        }
    }

    /// How wide the buffer is, in cells.
    pub fn screen_width(&mut self) -> i32 {
        let buffer = self.terminal.page_manager_mut().active_page().buffer();
        buffer.read().map(|buffer| buffer.screen_width()).unwrap_or(0)
    }

    /// How many rows of history the buffer holds.
    pub fn max_scroll(&mut self) -> i32 {
        let buffer = self.terminal.page_manager_mut().active_page().buffer();
        buffer.read().map(|buffer| buffer.scrollback_len() as i32).unwrap_or(0)
    }

    /// The document height and its prompt ticks, read under one lock.
    pub fn scroll_facts(&mut self) -> (i32, Vec<grid::MarkTick>) {
        let buffer = self.terminal.page_manager_mut().active_page().buffer();
        let buffer = buffer.read().unwrap_or_else(|poisoned| poisoned.into_inner());
        (buffer.document_height(), grid::mark_ticks(&buffer))
    }
}

/// Turns a cell count into the `u16` a pty size carries.
fn cell_count(value: i32) -> u16 {
    value.clamp(1, i32::from(u16::MAX)) as u16
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::term_connection::tests::Buffer;

    /// A control whose connection produces `output`, and a handle to that
    /// connection so the test can see what was sent.
    fn control_with(output: &[u8]) -> (Control, Buffer) {
        let connection = Buffer::with_output(output);
        (Control::new(Box::new(connection.clone()), (80, 24)), connection)
    }

    fn screen_text(control: &mut Control) -> String {
        let view = View::new();
        control
            .snapshot(&view)
            .rows
            .iter()
            .flat_map(|row| row.runs.iter())
            .map(|run| run.text.as_str())
            .collect()
    }

    #[test]
    fn a_pump_feeds_the_parser_and_the_snapshot_shows_it() {
        let (mut control, _) = control_with(b"hello");

        control.pump();
        assert!(screen_text(&mut control).contains("hello"));
    }

    #[test]
    fn a_bell_is_one_event_per_flash() {
        let now = Instant::now();
        let (mut control, _) = control_with(b"\x07\x07");

        control.pump();
        assert_eq!(control.take_events(now), [TerminalEvent::Bell]);
        assert!(control.bell_alpha(now).is_some(), "the flash started");

        // A second bell during the flash is ignored, so the window is not asked
        // to beep again.
        control.pump();
        assert_eq!(control.take_events(now + Duration::from_millis(100)), []);
    }

    #[test]
    fn a_bell_after_the_flash_is_a_new_event() {
        let now = Instant::now();
        let (mut control, connection) = control_with(b"\x07");
        control.pump();
        assert_eq!(control.take_events(now), [TerminalEvent::Bell]);

        // The fake produces output once; a second bell arrives as new bytes.
        connection.send(b"\x07");
        control.pump();
        assert_eq!(
            control.take_events(now + crate::visual_bell::FLASH + Duration::from_millis(1)),
            [TerminalEvent::Bell]
        );
    }

    #[test]
    fn a_notification_names_its_title_and_body() {
        let (mut control, _) = control_with(b"\x1b]777;notify;build;failed\x07");

        control.pump();
        assert_eq!(
            control.take_events(Instant::now()),
            [TerminalEvent::Notification {
                title: "build".to_string(),
                body: "failed".to_string(),
            }]
        );
    }

    #[test]
    fn an_exit_is_reported_once() {
        let connection = Buffer::ended();
        let mut control = Control::new(Box::new(connection), (80, 24));

        control.pump();
        assert_eq!(control.take_events(Instant::now()), [TerminalEvent::Exited]);

        control.pump();
        assert_eq!(control.take_events(Instant::now()), []);
    }

    #[test]
    fn typed_text_and_pastes_reach_the_connection() {
        let (mut control, connection) = control_with(b"");

        control.write("ls\r");
        control.paste("pasted");

        let written = String::from_utf8_lossy(&connection.written()).to_string();
        assert!(written.starts_with("ls\r"), "{written:?}");
        assert!(written.ends_with("pasted"), "{written:?}");
    }

    #[test]
    fn a_resize_reaches_the_terminal_and_the_connection() {
        let (mut control, connection) = control_with(b"");
        control.resize(100, 30);

        assert_eq!(control.size(), (100, 30));
        assert_eq!(connection.resized(), [PtySize::new(100, 30)]);

        let view = View::new();
        assert_eq!(control.snapshot(&view).width, 100);
    }

    #[test]
    fn a_cell_count_is_clamped_for_the_pty() {
        assert_eq!(cell_count(0), 1);
        assert_eq!(cell_count(120), 120);
        assert_eq!(cell_count(i32::MAX), u16::MAX);
    }

    #[test]
    fn a_title_is_kept_until_the_program_sets_another() {
        let (mut control, _) = control_with(b"\x1b]2;first\x07");
        control.pump();
        assert_eq!(control.title(), Some("first"));

        control.pump();
        assert_eq!(control.title(), Some("first"));
    }

    #[test]
    fn a_response_goes_back_to_the_program() {
        // DA1 is answered by the terminal itself, so the connection is written
        // to without any input from the window.
        let (mut control, connection) = control_with(b"\x1b[c");
        control.pump();

        let written = connection.written();
        assert!(written.starts_with(b"\x1b["), "{written:?}");
        assert!(written.ends_with(b"c"), "{written:?}");
    }
}
