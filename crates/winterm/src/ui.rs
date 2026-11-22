//! The egui shell — the only module that knows a GUI toolkit exists.
//!
//! ADR 018 records the boundary: this module turns egui events into `term`
//! vocabulary and `grid` snapshots into paint calls. Replacing egui with a
//! native backend means replacing this file, not the session or the grid.

use std::time::Duration;

use eframe::egui;
use server::pty::{PtyCommand, PtySize};
use term::Terminal;
use term::input::{Key, KeyEvent, Modifiers as KeyModifiers};
use types::color_table::{ColorAlias, ColorTable};
use types::text_color::{ColorType, TextColor};

use crate::{grid, session::Session};

/// The font size the grid is measured and painted at.
const FONT_SIZE: f32 = 16.0;
/// The colour used when the palette has no default background.
const FALLBACK_BACKGROUND: egui::Color32 = egui::Color32::from_rgb(12, 12, 12);
/// The colour used when the palette has no default foreground.
const FALLBACK_FOREGROUND: egui::Color32 = egui::Color32::from_rgb(204, 204, 204);

/// Starts the window and the shell inside it.
pub fn run() -> eframe::Result {
    let options = eframe::NativeOptions {
        viewport: egui::ViewportBuilder::default()
            .with_inner_size([900.0, 560.0])
            .with_title("winterm"),
        ..Default::default()
    };

    eframe::run_native("winterm", options, Box::new(|_cc| Ok(Box::new(App::new()?))))
}

struct App {
    terminal: Terminal,
    session: Session,
    /// The grid size the terminal and pty were last told about.
    size: (i32, i32),
    /// Right Alt is held, which is what makes an AltGr chord distinct from
    /// Ctrl+Alt (ADR 017's `is_alt_gr`).
    right_alt: bool,
    /// Right Control is held.
    right_ctrl: bool,
    /// The output stream ended.
    exited: bool,
}

impl App {
    fn new() -> Result<Self, Box<dyn std::error::Error + Send + Sync>> {
        let size = PtySize::DEFAULT;
        let session = Session::spawn(&default_command(), size)?;
        let mut terminal = Terminal::new();
        terminal.resize(i32::from(size.cols), i32::from(size.rows));

        Ok(Self {
            terminal,
            session,
            size: (i32::from(size.cols), i32::from(size.rows)),
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
            match event {
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

    fn snapshot(&mut self) -> grid::Screen {
        let buffer = self.terminal.page_manager_mut().active_page().buffer();
        match buffer.read() {
            Ok(buffer) => grid::snapshot(&buffer),
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

        for (index, row) in screen.rows.iter().enumerate() {
            let top = area.min.y + index as f32 * cell.y;

            for run in &row.runs {
                let left = area.min.x + run.column as f32 * cell.x;

                let background = resolve(run.attribute.get_background(), &table, ColorAlias::DefaultBackground)
                    .unwrap_or(FALLBACK_BACKGROUND);
                if background != FALLBACK_BACKGROUND {
                    let rect =
                        egui::Rect::from_min_size(egui::pos2(left, top), egui::vec2(cell.x * run.cells as f32, cell.y));
                    painter.rect_filled(rect, 0, background);
                }

                if run.text.is_empty() {
                    continue;
                }

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
    fn logic(&mut self, ctx: &egui::Context, _frame: &mut eframe::Frame) {
        self.pump_output();
        if !self.exited {
            // The pump thread has no handle to the context yet, so the frame
            // loop polls. A repaint waker replaces this when the session learns
            // to signal the window.
            ctx.request_repaint_after(Duration::from_millis(8));
        }
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
}
