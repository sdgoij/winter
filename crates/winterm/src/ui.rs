//! The egui shell — the only module that knows a GUI toolkit exists.
//!
//! ADR 018 records the boundary: this module turns egui events into `term`
//! vocabulary and `grid` snapshots into paint calls. Replacing egui with a
//! native backend means replacing this file, not the session or the grid.

use std::time::Instant;

use eframe::egui;
use server::pty::{PtyCommand, PtySize};
use term::Terminal;
use term::input::{
    Key, KeyEvent, Modifiers as KeyModifiers, MouseButton, MouseButtons, MouseEvent, MouseEventKind, WHEEL_DELTA,
    WheelAxis,
};
use types::color_table::{ColorAlias, ColorTable};
use types::text_color::{ColorType, TextColor};

use crate::{grid, session::Session, view::View};

/// The font size the grid is measured and painted at.
const FONT_SIZE: f32 = 16.0;
/// The colour used when the palette has no default background.
const FALLBACK_BACKGROUND: egui::Color32 = egui::Color32::from_rgb(12, 12, 12);
/// The colour used when the palette has no default foreground.
const FALLBACK_FOREGROUND: egui::Color32 = egui::Color32::from_rgb(204, 204, 204);
/// The fill behind selected cells. Opaque, because the text is painted over it.
const SELECTION_BACKGROUND: egui::Color32 = egui::Color32::from_rgb(38, 79, 120);

/// Starts the window and the shell inside it.
pub fn run() -> eframe::Result {
    let options = eframe::NativeOptions {
        // Ctrl+C, Ctrl+X and Ctrl+V must reach the shell. The application owns
        // its clipboard chords (Ctrl+Shift+C/V) instead.
        clipboard_shortcuts: false,
        viewport: egui::ViewportBuilder::default()
            .with_inner_size([900.0, 560.0])
            .with_title("winterm"),
        ..Default::default()
    };

    eframe::run_native("winterm", options, Box::new(|cc| Ok(Box::new(App::new(&cc.egui_ctx)?))))
}

struct App {
    terminal: Terminal,
    session: Session,
    view: View,
    clipboard: Option<arboard::Clipboard>,
    /// The grid size the terminal and pty were last told about.
    size: (i32, i32),
    /// The cell the pointer was last forwarded from, for motion deduplication.
    last_mouse_cell: Option<(i32, i32)>,
    /// Right Alt is held, which is what makes an AltGr chord distinct from
    /// Ctrl+Alt (ADR 017's `is_alt_gr`).
    right_alt: bool,
    /// Right Control is held.
    right_ctrl: bool,
    /// The output stream ended.
    exited: bool,
}

impl App {
    fn new(ctx: &egui::Context) -> Result<Self, Box<dyn std::error::Error + Send + Sync>> {
        let size = PtySize::DEFAULT;
        let repaint = ctx.clone();
        let session = Session::spawn_with(&default_command(), size, move || repaint.request_repaint())?;
        let mut terminal = Terminal::new();
        terminal.resize(i32::from(size.cols), i32::from(size.rows));

        Ok(Self {
            terminal,
            session,
            view: View::new(),
            clipboard: arboard::Clipboard::new().ok(),
            size: (i32::from(size.cols), i32::from(size.rows)),
            last_mouse_cell: None,
            right_alt: false,
            right_ctrl: false,
            exited: false,
        })
    }

    /// Moves bytes from the pty into the parser and answers from the adapter.
    fn pump_output(&mut self) {
        let mut bytes = Vec::new();
        if self.session.drain_into(&mut bytes) {
            self.exited = true;
        }
        if bytes.is_empty() {
            return;
        }

        self.terminal.process_bytes(&bytes);
        for response in self.terminal.take_responses() {
            self.write(&response);
        }
    }

    fn handle_input(&mut self, ctx: &egui::Context) {
        let events = ctx.input(|input| input.events.clone());
        for event in events {
            // Typing returns to the live screen, as upstream's TrySnapOnInput.
            if self.view.scroll() != 0
                && matches!(&event, egui::Event::Text(_) | egui::Event::Key { pressed: true, .. })
            {
                self.view.scroll_to_bottom();
            }

            match event {
                egui::Event::Copy => {
                    self.copy_selection();
                }
                egui::Event::Key {
                    key: egui::Key::C,
                    pressed: true,
                    modifiers,
                    ..
                } if modifiers.ctrl && modifiers.shift => {
                    self.copy_selection();
                }
                egui::Event::Key {
                    key: egui::Key::V,
                    pressed: true,
                    modifiers,
                    ..
                } if modifiers.ctrl && modifiers.shift => {
                    self.paste_clipboard();
                }
                egui::Event::Text(text) => {
                    for character in text.chars() {
                        self.send_key(&KeyEvent {
                            key: Key::Unidentified,
                            codepoint: Some(character),
                            ..KeyEvent::down(Key::Unidentified)
                        });
                    }
                }
                egui::Event::Paste(text) => {
                    let bytes = self.terminal.paste(&text);
                    self.write(&bytes);
                }
                egui::Event::Key {
                    key,
                    pressed,
                    repeat,
                    modifiers,
                    ..
                } => {
                    self.track_modifier(key, pressed);

                    let Some(key) = keymap(key) else {
                        continue;
                    };

                    // A printable key arrives again as `Event::Text`; sending
                    // the key event as well would type it twice. Ctrl, Alt and
                    // Command suppress the text event, so those chords go here.
                    let printable = matches!(key, Key::Char(_) | Key::Space);
                    if printable && !modifiers.ctrl && !modifiers.alt && !modifiers.command {
                        continue;
                    }

                    self.send_key(&KeyEvent {
                        key,
                        modifiers: key_modifiers(modifiers, self.right_alt, self.right_ctrl),
                        pressed,
                        repeat,
                        ..KeyEvent::down(key)
                    });
                }
                _ => {}
            }
        }
    }

    fn track_modifier(&mut self, key: egui::Key, pressed: bool) {
        match key {
            egui::Key::AltRight => self.right_alt = pressed,
            egui::Key::ControlRight => self.right_ctrl = pressed,
            _ => {}
        }
    }

    fn send_key(&mut self, event: &KeyEvent) {
        if let Some(bytes) = self.terminal.handle_key(event) {
            self.write(&bytes);
        }
    }

    fn write(&mut self, bytes: &str) {
        if bytes.is_empty() {
            return;
        }
        if self.session.write(bytes.as_bytes()).is_err() {
            self.exited = true;
        }
    }

    fn resize(&mut self, columns: i32, rows: i32) {
        self.size = (columns, rows);
        self.terminal.resize(columns, rows);
        let size = PtySize::new(to_cells(columns), to_cells(rows));
        let _ = self.session.resize(size);
    }

    /// Handles the pointer: wheel, buttons, motion and selection.
    ///
    /// A tracking application owns the mouse; Shift is the user's override and
    /// starts a selection instead.
    fn handle_pointer(&mut self, ctx: &egui::Context, area: egui::Rect, cell: egui::Vec2) {
        if cell.x <= 0.0 || cell.y <= 0.0 {
            return;
        }

        let input = ctx.input(|input| PointerInput {
            hover: input.pointer.hover_pos(),
            primary_pressed: input.pointer.button_pressed(egui::PointerButton::Primary),
            primary_down: input.pointer.button_down(egui::PointerButton::Primary),
            primary_released: input.pointer.button_released(egui::PointerButton::Primary),
            middle_pressed: input.pointer.button_pressed(egui::PointerButton::Middle),
            middle_down: input.pointer.button_down(egui::PointerButton::Middle),
            middle_released: input.pointer.button_released(egui::PointerButton::Middle),
            secondary_pressed: input.pointer.button_pressed(egui::PointerButton::Secondary),
            secondary_down: input.pointer.button_down(egui::PointerButton::Secondary),
            secondary_released: input.pointer.button_released(egui::PointerButton::Secondary),
            scroll: input.smooth_scroll_delta.y,
            modifiers: input.modifiers,
        });

        let Some(position) = input.hover.filter(|position| area.contains(*position)) else {
            self.last_mouse_cell = None;
            return;
        };
        let Some(cell_position) = self.cell_at(area, cell, position) else {
            return;
        };

        let forwards = forwards_to_application(self.terminal.is_tracking_mouse_input(), input.modifiers.shift);

        if input.scroll != 0.0 {
            let lines = (input.scroll / cell.y).round() as i32;
            let lines = if lines == 0 {
                input.scroll.signum() as i32
            } else {
                lines
            };

            // Shift keeps the wheel for the viewport; otherwise a tracking
            // application, or the alternate screen's scroll, owns it.
            let wheel = MouseEvent::wheel(cell_position, WheelAxis::Vertical, lines * WHEEL_DELTA);
            let bytes = if input.modifiers.shift {
                None
            } else {
                self.terminal.handle_mouse(&wheel)
            };
            match bytes {
                Some(bytes) => self.write(&bytes),
                None => {
                    let max = self.max_scroll();
                    self.view.scroll_by(lines, max);
                }
            }
        }

        if forwards {
            let moved = self.last_mouse_cell != Some(cell_position);
            self.last_mouse_cell = Some(cell_position);
            let buttons = mouse_buttons(input.primary_down, input.middle_down, input.secondary_down);
            let modifiers = key_modifiers(input.modifiers, self.right_alt, self.right_ctrl);

            self.send_mouse_buttons(&input, cell_position, buttons, modifiers);

            let dragging = buttons.left || buttons.middle || buttons.right;
            if moved || dragging {
                let event = MouseEvent {
                    position: cell_position,
                    kind: MouseEventKind::Move,
                    buttons,
                    modifiers,
                    delta: 0,
                };
                if let Some(bytes) = self.terminal.handle_mouse(&event) {
                    self.write(&bytes);
                }
            }
        } else {
            self.last_mouse_cell = None;

            if input.primary_pressed {
                self.view.scroll_to_bottom();
                let count = self.view.click(cell_position, Instant::now());

                if count >= 3 {
                    let width = self.screen_width();
                    self.view.select_line(cell_position.1, width);
                } else if count == 2 {
                    let text = self.row_text(cell_position.1);
                    self.view.select_word(&text, cell_position);
                } else {
                    self.view.select(cell_position, input.modifiers.alt);
                }
            } else if input.primary_down && self.view.selection().is_some() {
                self.view.extend(cell_position);
            }
        }
    }

    /// Forwards button presses and releases to a tracking application.
    fn send_mouse_buttons(
        &mut self,
        input: &PointerInput,
        position: (i32, i32),
        buttons: MouseButtons,
        modifiers: KeyModifiers,
    ) {
        for (button, pressed, released) in [
            (MouseButton::Left, input.primary_pressed, input.primary_released),
            (MouseButton::Middle, input.middle_pressed, input.middle_released),
            (MouseButton::Right, input.secondary_pressed, input.secondary_released),
        ] {
            if !pressed && !released {
                continue;
            }

            let event = MouseEvent {
                position,
                kind: if pressed {
                    MouseEventKind::Press(button)
                } else {
                    MouseEventKind::Release(button)
                },
                buttons,
                modifiers,
                delta: 0,
            };
            if let Some(bytes) = self.terminal.handle_mouse(&event) {
                self.write(&bytes);
            }
        }
    }

    /// Maps a window position to a screen cell, if it is inside the grid.
    fn cell_at(&self, area: egui::Rect, cell: egui::Vec2, position: egui::Pos2) -> Option<(i32, i32)> {
        let column = ((position.x - area.min.x) / cell.x).floor() as i32;
        let row = ((position.y - area.min.y) / cell.y).floor() as i32;
        let (columns, rows) = self.size;
        if column < 0 || row < 0 || column >= columns || row >= rows {
            return None;
        }
        Some((column, row))
    }

    fn row_text(&mut self, row: i32) -> String {
        let buffer = self.terminal.page_manager_mut().active_page().buffer();
        match buffer.read() {
            Ok(buffer) => buffer
                .get_row_at_screen_line(row)
                .map(buffer::Row::get_text)
                .unwrap_or_default(),
            Err(_) => String::new(),
        }
    }

    fn screen_width(&mut self) -> i32 {
        let buffer = self.terminal.page_manager_mut().active_page().buffer();
        buffer.read().map(|buffer| buffer.screen_width()).unwrap_or(0)
    }

    fn max_scroll(&mut self) -> i32 {
        let buffer = self.terminal.page_manager_mut().active_page().buffer();
        buffer.read().map(|buffer| buffer.scrollback_len() as i32).unwrap_or(0)
    }

    /// Copies the selected cells, using the buffer's copy path.
    fn copy_selection(&mut self) {
        let text = {
            let buffer = self.terminal.page_manager_mut().active_page().buffer();
            let Ok(buffer) = buffer.read() else {
                return;
            };
            let Some(selection) = self.view.selection() else {
                return;
            };
            selection.text(&buffer)
        };

        if text.is_empty() {
            return;
        }
        if let Some(clipboard) = self.clipboard.as_mut() {
            let _ = clipboard.set_text(text);
        }
    }

    /// Pastes the platform clipboard as filtered text for the pty.
    fn paste_clipboard(&mut self) {
        let Some(clipboard) = self.clipboard.as_mut() else {
            return;
        };
        let Ok(text) = clipboard.get_text() else {
            return;
        };

        let bytes = self.terminal.paste(&text);
        self.write(&bytes);
    }

    fn snapshot(&mut self) -> grid::Screen {
        let scroll = self.view.scroll();
        let buffer = self.terminal.page_manager_mut().active_page().buffer();
        match buffer.read() {
            Ok(buffer) => grid::snapshot_at(&buffer, scroll),
            Err(_) => grid::Screen::default(),
        }
    }

    fn paint(&self, ui: &egui::Ui, area: egui::Rect, cell: egui::Vec2, screen: &grid::Screen) {
        let painter = ui.painter().with_clip_rect(area);
        painter.rect_filled(area, 0, FALLBACK_BACKGROUND);

        if cell.x <= 0.0 || cell.y <= 0.0 {
            return;
        }

        let table = self.terminal.dispatch().color_table();
        let Ok(table) = table.lock() else {
            return;
        };
        let font = egui::FontId::monospace(FONT_SIZE);

        // Backgrounds first, so the selection can cover them and the text can
        // be drawn over both.
        for (index, row) in screen.rows.iter().enumerate() {
            let top = area.min.y + index as f32 * cell.y;
            for run in &row.runs {
                let background = resolve(run.attribute.get_background(), &table, ColorAlias::DefaultBackground)
                    .unwrap_or(FALLBACK_BACKGROUND);
                if background == FALLBACK_BACKGROUND {
                    continue;
                }
                let left = area.min.x + run.column as f32 * cell.x;
                let rect =
                    egui::Rect::from_min_size(egui::pos2(left, top), egui::vec2(cell.x * run.cells as f32, cell.y));
                painter.rect_filled(rect, 0, background);
            }
        }

        if let Some(selection) = self.view.selection() {
            for row in 0..screen.height {
                let Some((start, end)) = selection.row_span(row) else {
                    continue;
                };
                let start = start.max(0);
                let end = end.min(screen.width - 1);
                if end < start {
                    continue;
                }

                let left = area.min.x + start as f32 * cell.x;
                let top = area.min.y + row as f32 * cell.y;
                let rect = egui::Rect::from_min_size(
                    egui::pos2(left, top),
                    egui::vec2((end - start + 1) as f32 * cell.x, cell.y),
                );
                painter.rect_filled(rect, 0, SELECTION_BACKGROUND);
            }
        }

        for (index, row) in screen.rows.iter().enumerate() {
            let top = area.min.y + index as f32 * cell.y;
            for run in &row.runs {
                if run.text.is_empty() {
                    continue;
                }
                let left = area.min.x + run.column as f32 * cell.x;
                let foreground = resolve(run.attribute.get_foreground(), &table, ColorAlias::DefaultForeground)
                    .unwrap_or(FALLBACK_FOREGROUND);
                painter.text(
                    egui::pos2(left, top),
                    egui::Align2::LEFT_TOP,
                    &run.text,
                    font.clone(),
                    foreground,
                );
            }
        }

        if screen.cursor_visible {
            let column = screen.cursor.0.clamp(0, screen.width.max(1) - 1);
            let row = screen.cursor.1.clamp(0, screen.height.max(1) - 1);
            let origin = egui::pos2(area.min.x + column as f32 * cell.x, area.min.y + row as f32 * cell.y);
            painter.rect_filled(
                egui::Rect::from_min_size(origin, cell),
                0,
                egui::Color32::from_rgba_unmultiplied(230, 230, 230, 110),
            );
        }
    }
}

impl eframe::App for App {
    fn logic(&mut self, _ctx: &egui::Context, _frame: &mut eframe::Frame) {
        // The pump asks for a repaint when output arrives; there is no poll.
        self.pump_output();
    }

    fn ui(&mut self, ui: &mut egui::Ui, _frame: &mut eframe::Frame) {
        let font = egui::FontId::monospace(FONT_SIZE);
        let cell = ui
            .ctx()
            .fonts_mut(|fonts| egui::vec2(fonts.glyph_width(&font, 'M'), fonts.row_height(&font)));

        let area = ui.available_rect_before_wrap();
        let columns = ((area.width() / cell.x).floor() as i32).max(1);
        let rows = ((area.height() / cell.y).floor() as i32).max(1);
        if (columns, rows) != self.size {
            self.resize(columns, rows);
        }

        self.handle_pointer(ui.ctx(), area, cell);
        self.handle_input(ui.ctx());
        let screen = self.snapshot();
        self.paint(ui, area, cell, &screen);

        if self.exited {
            ui.painter().text(
                area.left_bottom() + egui::vec2(4.0, -4.0),
                egui::Align2::LEFT_BOTTOM,
                "[session ended]",
                font,
                FALLBACK_FOREGROUND,
            );
        }
    }
}

/// The pointer state one frame needs.
struct PointerInput {
    hover: Option<egui::Pos2>,
    primary_pressed: bool,
    primary_down: bool,
    primary_released: bool,
    middle_pressed: bool,
    middle_down: bool,
    middle_released: bool,
    secondary_pressed: bool,
    secondary_down: bool,
    secondary_released: bool,
    scroll: f32,
    modifiers: egui::Modifiers,
}

/// Returns whether the application, rather than the selection, gets the mouse.
///
/// Shift is the user's override: it selects even while an application tracks
/// the mouse, which is how a person copies text out of a full-screen program.
const fn forwards_to_application(tracking: bool, shift: bool) -> bool {
    tracking && !shift
}

/// Maps the pointer's buttons to the protocol's set of held buttons.
const fn mouse_buttons(primary: bool, middle: bool, secondary: bool) -> MouseButtons {
    MouseButtons {
        left: primary,
        middle,
        right: secondary,
    }
}

fn default_command() -> PtyCommand {
    #[cfg(windows)]
    let shell = std::env::var_os("COMSPEC").unwrap_or_else(|| "cmd.exe".into());
    #[cfg(unix)]
    let shell = std::env::var_os("SHELL").unwrap_or_else(|| "/bin/sh".into());

    PtyCommand::new(shell)
}

/// Turns a cell count into the `u16` a pty size carries.
fn to_cells(value: i32) -> u16 {
    value.clamp(1, i32::from(u16::MAX)) as u16
}

/// Resolves a terminal colour through the adapter's palette.
fn resolve(color: TextColor, table: &ColorTable, alias: ColorAlias) -> Option<egui::Color32> {
    let entry = match color.get_type() {
        ColorType::Rgb { .. } => color,
        ColorType::Index16(_) | ColorType::Index256(_) => table.get(usize::from(color.get_index()))?,
        ColorType::Default => table.get(table.alias_index(alias))?,
    };

    match entry.get_type() {
        ColorType::Rgb { r, g, b } => Some(egui::Color32::from_rgb(r, g, b)),
        _ => None,
    }
}

/// Maps an egui key to the key's identity in `term`.
///
/// Punctuation is named by the character it produces, which differs from the
/// US-layout base key `term` documents; the difference only matters for a
/// Ctrl chord on punctuation, where the text event is suppressed.
fn keymap(key: egui::Key) -> Option<Key> {
    use egui::Key as E;

    Some(match key {
        E::ArrowDown => Key::Down,
        E::ArrowLeft => Key::Left,
        E::ArrowRight => Key::Right,
        E::ArrowUp => Key::Up,
        E::Escape => Key::Escape,
        E::Tab => Key::Tab,
        E::Backspace => Key::Backspace,
        E::Enter => Key::Enter,
        E::Space => Key::Space,
        E::Insert => Key::Insert,
        E::Delete => Key::Delete,
        E::Home => Key::Home,
        E::End => Key::End,
        E::PageUp => Key::PageUp,
        E::PageDown => Key::PageDown,
        E::Clear => Key::Clear,
        E::Colon => Key::Char(':'),
        E::Comma => Key::Char(','),
        E::Backslash => Key::Char('\\'),
        E::Slash => Key::Char('/'),
        E::Pipe => Key::Char('|'),
        E::Questionmark => Key::Char('?'),
        E::Exclamationmark => Key::Char('!'),
        E::OpenBracket => Key::Char('['),
        E::CloseBracket => Key::Char(']'),
        E::OpenCurlyBracket => Key::Char('{'),
        E::CloseCurlyBracket => Key::Char('}'),
        E::Backtick => Key::Char('`'),
        E::Minus => Key::Char('-'),
        E::Period => Key::Char('.'),
        E::Plus => Key::Char('+'),
        E::Equals => Key::Char('='),
        E::Semicolon => Key::Char(';'),
        E::Quote => Key::Char('\''),
        E::IntlBackslash => Key::Char('\\'),
        E::Num0 => Key::Char('0'),
        E::Num1 => Key::Char('1'),
        E::Num2 => Key::Char('2'),
        E::Num3 => Key::Char('3'),
        E::Num4 => Key::Char('4'),
        E::Num5 => Key::Char('5'),
        E::Num6 => Key::Char('6'),
        E::Num7 => Key::Char('7'),
        E::Num8 => Key::Char('8'),
        E::Num9 => Key::Char('9'),
        E::A => Key::Char('a'),
        E::B => Key::Char('b'),
        E::C => Key::Char('c'),
        E::D => Key::Char('d'),
        E::E => Key::Char('e'),
        E::F => Key::Char('f'),
        E::G => Key::Char('g'),
        E::H => Key::Char('h'),
        E::I => Key::Char('i'),
        E::J => Key::Char('j'),
        E::K => Key::Char('k'),
        E::L => Key::Char('l'),
        E::M => Key::Char('m'),
        E::N => Key::Char('n'),
        E::O => Key::Char('o'),
        E::P => Key::Char('p'),
        E::Q => Key::Char('q'),
        E::R => Key::Char('r'),
        E::S => Key::Char('s'),
        E::T => Key::Char('t'),
        E::U => Key::Char('u'),
        E::V => Key::Char('v'),
        E::W => Key::Char('w'),
        E::X => Key::Char('x'),
        E::Y => Key::Char('y'),
        E::Z => Key::Char('z'),
        E::F1 => Key::Function(1),
        E::F2 => Key::Function(2),
        E::F3 => Key::Function(3),
        E::F4 => Key::Function(4),
        E::F5 => Key::Function(5),
        E::F6 => Key::Function(6),
        E::F7 => Key::Function(7),
        E::F8 => Key::Function(8),
        E::F9 => Key::Function(9),
        E::F10 => Key::Function(10),
        E::F11 => Key::Function(11),
        E::F12 => Key::Function(12),
        E::F13 => Key::Function(13),
        E::F14 => Key::Function(14),
        E::F15 => Key::Function(15),
        E::F16 => Key::Function(16),
        E::F17 => Key::Function(17),
        E::F18 => Key::Function(18),
        E::F19 => Key::Function(19),
        E::F20 => Key::Function(20),
        E::F21 => Key::Function(21),
        E::F22 => Key::Function(22),
        E::F23 => Key::Function(23),
        E::F24 => Key::Function(24),
        E::MediaPlayPause => Key::MediaPlayPause,
        E::MediaTrackNext => Key::MediaNextTrack,
        E::MediaTrackPrevious => Key::MediaPreviousTrack,
        E::MediaStop => Key::MediaStop,
        E::AudioVolumeMute => Key::VolumeMute,
        E::AudioVolumeUp => Key::VolumeUp,
        E::AudioVolumeDown => Key::VolumeDown,
        E::ShiftLeft => Key::LeftShift,
        E::ShiftRight => Key::RightShift,
        E::ControlLeft => Key::LeftControl,
        E::ControlRight => Key::RightControl,
        E::AltLeft => Key::LeftAlt,
        E::AltRight => Key::RightAlt,
        E::SuperLeft => Key::LeftSuper,
        E::SuperRight => Key::RightSuper,
        E::CapsLock => Key::CapsLock,
        E::NumLock => Key::NumLock,
        E::ScrollLock => Key::ScrollLock,
        E::PrintScreen => Key::PrintScreen,
        E::Pause => Key::Pause,
        E::Menu => Key::Menu,
        _ => return None,
    })
}

/// Composes the terminal's modifier state from egui's collapsed state and the
/// sides the shell has tracked.
///
/// egui reports that *an* Alt is held but not which; tracking the right key is
/// what lets `Modifiers::is_alt_gr` recognise an AltGr chord.
fn key_modifiers(modifiers: egui::Modifiers, right_alt: bool, right_ctrl: bool) -> KeyModifiers {
    KeyModifiers {
        shift: modifiers.shift,
        left_ctrl: modifiers.ctrl && !right_ctrl,
        right_ctrl: modifiers.ctrl && right_ctrl,
        left_alt: modifiers.alt && !right_alt,
        right_alt: modifiers.alt && right_alt,
        caps_lock: false,
        num_lock: false,
        enhanced: false,
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn the_keymap_names_the_keys_the_terminal_encodes() {
        assert_eq!(keymap(egui::Key::ArrowLeft), Some(Key::Left));
        assert_eq!(keymap(egui::Key::F5), Some(Key::Function(5)));
        assert_eq!(keymap(egui::Key::Q), Some(Key::Char('q')));
        assert_eq!(keymap(egui::Key::AltRight), Some(Key::RightAlt));
        assert_eq!(keymap(egui::Key::BrowserBack), None);
    }

    #[test]
    fn a_right_alt_chord_is_distinguishable_from_left_alt() {
        let chord = egui::Modifiers {
            alt: true,
            ctrl: true,
            ..egui::Modifiers::default()
        };

        assert!(key_modifiers(chord, true, false).is_alt_gr());
        assert!(!key_modifiers(chord, false, false).is_alt_gr());
    }

    #[test]
    fn the_modifier_sides_come_from_the_tracked_keys() {
        let ctrl = egui::Modifiers {
            ctrl: true,
            ..egui::Modifiers::default()
        };

        let left = key_modifiers(ctrl, false, false);
        assert!(left.left_ctrl);
        assert!(!left.right_ctrl);

        let right = key_modifiers(ctrl, false, true);
        assert!(!right.left_ctrl);
        assert!(right.right_ctrl);
    }

    #[test]
    fn colours_resolve_through_the_palette_and_rgb_passes_through() {
        let table = ColorTable::new();

        assert_eq!(
            resolve(TextColor::rgb(1, 2, 3), &table, ColorAlias::DefaultForeground),
            Some(egui::Color32::from_rgb(1, 2, 3))
        );

        // A default colour follows the alias; the Campbell defaults are the
        // palette's white and black.
        assert_eq!(
            resolve(TextColor::new(), &table, ColorAlias::DefaultForeground),
            Some(egui::Color32::from_rgb(0xCC, 0xCC, 0xCC))
        );
        assert_eq!(
            resolve(TextColor::new(), &table, ColorAlias::DefaultBackground),
            Some(egui::Color32::from_rgb(0x0C, 0x0C, 0x0C))
        );
    }

    #[test]
    fn a_cell_count_is_clamped_for_the_pty() {
        assert_eq!(to_cells(0), 1);
        assert_eq!(to_cells(120), 120);
        assert_eq!(to_cells(i32::MAX), u16::MAX);
    }

    #[test]
    fn shift_takes_the_mouse_back_from_a_tracking_application() {
        assert!(forwards_to_application(true, false));
        assert!(!forwards_to_application(true, true));
        assert!(!forwards_to_application(false, false));
    }

    #[test]
    fn the_pointer_buttons_map_to_the_protocol_buttons() {
        assert_eq!(
            mouse_buttons(true, false, false),
            MouseButtons {
                left: true,
                middle: false,
                right: false,
            }
        );
        assert_eq!(
            mouse_buttons(false, true, false),
            MouseButtons {
                left: false,
                middle: true,
                right: false,
            }
        );
        assert_eq!(
            mouse_buttons(false, false, true),
            MouseButtons {
                left: false,
                middle: false,
                right: true,
            }
        );
    }
}
