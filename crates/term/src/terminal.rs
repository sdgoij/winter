//! `Terminal` — the parser, engine and adapter behind one object.
//!
//! An application feeds bytes with [`Terminal::process_bytes`] and reads the
//! host-facing queues with the `take_*` methods. `Terminal` owns no I/O: a pty
//! writer, a window and a renderer are the caller's.

use adapter::{AdaptDispatch, PageManager};
use types::VTInt;
use vt::{ParserMode, StateMachine, VtState};

use crate::input::{KeyEvent, MouseEvent, TerminalInput};
use crate::output_engine::OutputStateMachineEngine;

/// A terminal: the VT parser driving an `ITermDispatch`.
pub struct Terminal {
    state: StateMachine<OutputStateMachineEngine<AdaptDispatch>>,
    input: TerminalInput,
}

impl Terminal {
    /// Creates a terminal with a fresh adapter.
    pub fn new() -> Self {
        Self::with_dispatch(AdaptDispatch::new())
    }

    /// Creates a terminal around an existing adapter.
    pub fn with_dispatch(dispatch: AdaptDispatch) -> Self {
        Self {
            state: StateMachine::new(OutputStateMachineEngine::new(dispatch)),
            input: TerminalInput::new(),
        }
    }

    /// Translates a key event into the bytes to write to the pty.
    ///
    /// The input modes come from the adapter, which is their one home
    /// (ADR 009). `None` means the key has no VT encoding and the host may
    /// handle it itself.
    pub fn handle_key(&mut self, event: &KeyEvent) -> Option<String> {
        let modes = self.dispatch().input_modes();
        self.input.handle_key(event, &modes)
    }

    /// Translates a mouse event into the bytes to write to the pty.
    ///
    /// `Some("")` means the event was consumed without producing bytes, as a
    /// partly accumulated wheel turn is.
    pub fn handle_mouse(&mut self, event: &MouseEvent) -> Option<String> {
        let modes = self.dispatch().input_modes();
        self.input.handle_mouse(event, &modes)
    }

    /// Translates a focus change into the bytes to write to the pty.
    pub fn handle_focus(&mut self, focused: bool) -> Option<String> {
        let modes = self.dispatch().input_modes();
        self.input.handle_focus(focused, &modes)
    }

    /// Processes a chunk of terminal output.
    ///
    /// Bytes are fed one at a time so a DECANM or C1-controls change takes
    /// effect on the byte after the sequence that requested it.
    pub fn process_bytes(&mut self, bytes: &[u8]) {
        for &byte in bytes {
            self.state.process_char(byte);

            if let Some(enabled) = self.dispatch_mut().take_ansi_mode_change() {
                self.state.set_parser_mode(ParserMode::Ansi, enabled);
            }
            if let Some(enabled) = self.dispatch_mut().take_c1_controls_change() {
                self.state.set_parser_mode(ParserMode::AcceptC1, enabled);
            }
            self.dispatch_mut().apply_pending_presentation_state();
            self.dispatch_mut().answer_pending_setting_request();
        }
    }

    /// Processes a chunk of UTF-8 text, which is the same as its bytes.
    pub fn process_str(&mut self, text: &str) {
        self.process_bytes(text.as_bytes());
    }

    /// Returns the parser state, for diagnostics.
    pub const fn state(&self) -> VtState {
        self.state.state()
    }

    /// Returns the adapter.
    pub fn dispatch(&self) -> &AdaptDispatch {
        self.state.engine().dispatch()
    }

    /// Returns the adapter mutably.
    pub fn dispatch_mut(&mut self) -> &mut AdaptDispatch {
        self.state.engine_mut().dispatch_mut()
    }

    /// Returns the page manager.
    pub fn page_manager(&self) -> &PageManager {
        self.dispatch().page_manager()
    }

    /// Returns the page manager mutably.
    pub fn page_manager_mut(&mut self) -> &mut PageManager {
        self.dispatch_mut().page_manager_mut()
    }

    /// Resizes every page and the size new pages start at.
    pub fn resize(&mut self, width: VTInt, height: VTInt) {
        self.page_manager_mut().resize_all(width, height);
    }

    /// Returns the current screen size in characters.
    pub fn screen_size(&self) -> (VTInt, VTInt) {
        self.page_manager().size()
    }

    /// Drains the responses the terminal owes the host.
    pub fn take_responses(&mut self) -> Vec<String> {
        self.dispatch_mut().take_responses()
    }

    /// Takes the clipboard content set by OSC 52.
    pub fn take_clipboard_content(&mut self) -> Option<String> {
        self.dispatch_mut().take_clipboard_content()
    }

    /// Takes the macro sequences queued by DECINVM.
    pub fn take_macro_input(&mut self) -> Vec<Vec<u8>> {
        self.dispatch_mut().take_macro_input()
    }

    /// Takes the window visibility requested by CSI 1/2 t; true means show.
    pub fn take_window_visibility(&mut self) -> Option<bool> {
        self.dispatch_mut().take_window_visibility()
    }

    /// Takes the window size requested by CSI 8 t as `(width, height)`.
    pub fn take_window_resize(&mut self) -> Option<(VTInt, VTInt)> {
        self.dispatch_mut().take_window_resize()
    }

    /// Takes the full-redraw request made by CSI 7 t.
    pub fn take_redraw_request(&mut self) -> bool {
        self.dispatch_mut().take_redraw_request()
    }

    /// Takes the notes requested by DECPS.
    pub fn take_sound_requests(&mut self) -> Vec<adapter::adapt_dispatch::MidiNote> {
        self.dispatch_mut().take_sound_requests()
    }

    /// Takes the number of bells requested since the last call.
    pub fn take_bell_requests(&mut self) -> usize {
        self.dispatch_mut().take_bell_requests()
    }

    /// Takes a pending ANSWERBACK request.
    pub fn take_answerback_request(&mut self) -> bool {
        self.dispatch_mut().take_answerback_request()
    }

    /// Takes the taskbar progress requested by ConEmu's OSC 9;4.
    pub fn take_taskbar_progress(&mut self) -> Option<(adapter::TaskbarState, usize)> {
        self.dispatch_mut().take_taskbar_progress()
    }

    /// Takes the working directory requested by ConEmu's OSC 9;9.
    pub fn take_conemu_working_directory(&mut self) -> Option<String> {
        self.dispatch_mut().take_conemu_working_directory()
    }

    /// Takes the number of shell-integration marks requested since the last call.
    pub fn take_shell_integration_marks(&mut self) -> usize {
        self.dispatch_mut().take_shell_integration_marks()
    }

    /// Takes the completion menu requested by VS Code's OSC 633.
    pub fn take_completions(&mut self) -> Option<(String, u32)> {
        self.dispatch_mut().take_completions()
    }

    /// Takes the missing command requested by Windows Terminal's OSC 9001.
    pub fn take_missing_command(&mut self) -> Option<String> {
        self.dispatch_mut().take_missing_command()
    }

    /// Takes the desktop notifications requested by urxvt's OSC 777.
    pub fn take_notifications(&mut self) -> Vec<(String, String)> {
        self.dispatch_mut().take_notifications()
    }

    /// Takes the number of unrecognised terminal-specific actions since the last call.
    pub fn take_unknown_sequences(&mut self) -> usize {
        self.dispatch_mut().take_unknown_sequences()
    }

    /// Returns the title set by OSC 0/2.
    pub fn window_title(&self) -> Option<&str> {
        self.dispatch().window_title()
    }

    /// Returns the working-directory URI set by OSC 7.
    pub fn current_working_directory(&self) -> Option<&str> {
        self.dispatch().current_working_directory()
    }
}

impl Default for Terminal {
    fn default() -> Self {
        Self::new()
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::input::{Key, KeyEvent, MouseButton, MouseEvent, WheelAxis};

    #[test]
    fn a_device_attributes_query_queues_a_response() {
        let mut terminal = Terminal::new();
        terminal.process_str("\x1b[c");
        let responses = terminal.take_responses();
        assert_eq!(responses.len(), 1);
        assert!(responses[0].starts_with("\x1b[?"));
    }

    #[test]
    fn printed_text_lands_in_the_active_page() {
        let mut terminal = Terminal::new();
        terminal.process_str("hi");

        let buffer = terminal.page_manager_mut().active_page().buffer();
        let buffer = buffer.read().unwrap();
        assert_eq!(buffer.get_text_data_at(0, 0), Some("h"));
        assert_eq!(buffer.get_text_data_at(0, 1), Some("i"));
    }

    #[test]
    fn osc_title_and_cwd_are_kept_for_the_host() {
        let mut terminal = Terminal::new();
        terminal.process_str("\x1b]2;title\x07\x1b]7;file:///tmp\x07");
        assert_eq!(terminal.window_title(), Some("title"));
        assert_eq!(terminal.current_working_directory(), Some("file:///tmp"));
    }

    #[test]
    fn osc_clipboard_content_is_drained_once() {
        let mut terminal = Terminal::new();
        terminal.process_str("\x1b]52;c;aGVsbG8=\x07");
        assert_eq!(terminal.take_clipboard_content().as_deref(), Some("hello"));
        assert_eq!(terminal.take_clipboard_content(), None);
    }

    #[test]
    fn resizing_sizes_future_pages_too() {
        let mut terminal = Terminal::new();
        terminal.resize(100, 40);
        assert_eq!(terminal.screen_size(), (100, 40));

        // Activating the alternate screen creates a page at the new size.
        terminal.process_str("\x1b[?1049h");
        let buffer = terminal.page_manager_mut().active_page().buffer();
        let buffer = buffer.read().unwrap();
        assert_eq!(buffer.screen_width(), 100);
        assert_eq!(buffer.screen_height(), 40);
    }

    #[test]
    fn a_mode_set_is_reflected_in_the_decrqm_report() {
        let mut terminal = Terminal::new();
        terminal.process_str("\x1b[?1h");
        terminal.process_str("\x1b[?1$p");
        assert_eq!(terminal.take_responses(), vec!["\x1b[?1;1$y".to_string()]);
    }

    #[test]
    fn decanm_switches_the_parser_to_vt52() {
        let mut terminal = Terminal::new();
        terminal.process_str("\x1b[6;6H");
        terminal.process_str("\x1b[?2l");
        assert!(!terminal.dispatch().is_ansi_mode());

        terminal.process_str("\x1bA");
        let buffer = terminal.page_manager_mut().active_page().buffer();
        let buffer = buffer.read().unwrap();
        assert_eq!(buffer.cursor().position(), (4, 5));
    }

    #[test]
    fn accepting_c1_controls_makes_a_single_byte_csi_parse() {
        let mut terminal = Terminal::new();
        terminal.process_str("\x1b 7");
        terminal.process_bytes(b"\x9b31m");
        terminal.process_str("A");

        let buffer = terminal.page_manager_mut().active_page().buffer();
        let buffer = buffer.read().unwrap();
        let cell = buffer.get_cell_data_at(0, 0).unwrap();
        assert_eq!(cell.get_attributes().get_foreground(), types::TextColor::index16(1));
    }

    #[test]
    fn finalterm_osc_actions_reach_the_adapter() {
        let mut terminal = Terminal::new();
        terminal.process_str("\x1b]133;A\x1b\\\x1b]133;B\x1b\\\x1b]133;C\x1b\\\x1b]133;D;1\x1b\\");

        assert_eq!(terminal.take_shell_integration_marks(), 4);
        let buffer = terminal.page_manager_mut().active_page().buffer();
        let buffer = buffer.read().unwrap();
        assert!(buffer.marks().get_mark_extents(types::MarkKind::Prompt).is_some());
    }

    #[test]
    fn decdwl_reaches_the_buffer() {
        let mut terminal = Terminal::new();
        terminal.process_str("\x1b#6");

        let buffer = terminal.page_manager_mut().active_page().buffer();
        assert_eq!(
            buffer.read().unwrap().get_line_rendition(0),
            types::LineRendition::DoubleWidth
        );
    }

    #[test]
    fn decrsps_applies_a_cursor_report_from_a_dcs_string() {
        let mut terminal = Terminal::new();
        terminal.process_str("\x1b[4;6H\x1b[1m");
        terminal.process_str("\x1bP1$t4;6;1;A;@;@;0;2;@;BBBB\x1b\\");

        let buffer = terminal.page_manager_mut().active_page().buffer();
        let buffer = buffer.read().unwrap();
        assert_eq!(buffer.cursor().position(), (3, 5));
        drop(buffer);
        assert!(terminal.page_manager_mut().active_page().attributes().is_intense());
    }

    #[test]
    fn decdld_through_the_parser_builds_a_soft_font() {
        let mut terminal = Terminal::new();
        terminal.process_str("\x1bP0;0;0;13;1;2;17;0{B~;\x1b\\");

        let buffer = terminal.dispatch().font_buffer();
        assert_eq!(buffer.cell_size(), (13, 17));
        assert_eq!(buffer.designation().as_bytes(), b"B");
    }

    #[test]
    fn decrqss_answers_through_the_parser() {
        let mut terminal = Terminal::new();
        terminal.process_str("\x1b[1;31m\x1bP$qm\x1b\\");
        assert_eq!(terminal.take_responses(), vec!["\x1bP1$r0;1;31m\x1b\\".to_string()]);
    }

    #[test]
    fn input_modes_come_from_the_adapter() {
        let mut terminal = Terminal::new();

        // Without application cursor mode, a cursor key is a CSI sequence.
        assert_eq!(
            terminal.handle_key(&KeyEvent::down(Key::Left)).as_deref(),
            Some("\x1b[D")
        );

        terminal.process_str("\x1b[?1h");
        assert_eq!(
            terminal.handle_key(&KeyEvent::down(Key::Left)).as_deref(),
            Some("\x1bOD")
        );

        // DECBKM makes Backspace send BS rather than DEL.
        terminal.process_str("\x1b[?67h");
        assert_eq!(
            terminal.handle_key(&KeyEvent::down(Key::Backspace)).as_deref(),
            Some("\x08")
        );

        // S8C1T switches the introducers to single-byte C1.
        terminal.process_str("\x1b G");
        assert_eq!(
            terminal
                .handle_key(&KeyEvent {
                    modifiers: crate::input::Modifiers {
                        left_ctrl: true,
                        ..crate::input::Modifiers::default()
                    },
                    ..KeyEvent::down(Key::Left)
                })
                .as_deref(),
            Some("\u{9b}1;5D")
        );

        // Focus reporting is off until the mode is set; S7C1T restores the
        // two-byte introducer.
        terminal.process_str("\x1b F");
        assert_eq!(terminal.handle_focus(true), None);
        terminal.process_str("\x1b[?1004h");
        assert_eq!(terminal.handle_focus(true).as_deref(), Some("\x1b[I"));
    }

    #[test]
    fn mouse_reporting_follows_the_tracking_mode() {
        let mut terminal = Terminal::new();
        let press = MouseEvent::press((2, 3), MouseButton::Left);

        // Nothing is reported until a tracking mode is set.
        assert_eq!(terminal.handle_mouse(&press), None);

        terminal.process_str("\x1b[?1000h");
        assert_eq!(terminal.handle_mouse(&press).as_deref(), Some("\x1b[M #$"));

        // SGR takes over when it is enabled.
        terminal.process_str("\x1b[?1006h");
        assert_eq!(terminal.handle_mouse(&press).as_deref(), Some("\x1b[<0;3;4M"));

        // On the alternate screen the wheel moves the cursor instead.
        terminal.process_str("\x1b[?1000l\x1b[?1006l\x1b[?1049h");
        let wheel = MouseEvent::wheel((0, 0), WheelAxis::Vertical, 120);
        assert_eq!(terminal.handle_mouse(&wheel).as_deref(), Some("\x1b[A"));
    }

    #[test]
    fn bell_and_sound_requests_reach_the_host_queues() {
        let mut terminal = Terminal::new();
        terminal.process_str("\x07");
        assert_eq!(terminal.take_bell_requests(), 1);

        terminal.process_str("\x1b[7;32;1,~");
        let notes = terminal.take_sound_requests();
        assert_eq!(notes.len(), 1);
        assert_eq!(notes[0].note, 72);
        assert_eq!(notes[0].velocity, 127);
    }
}
