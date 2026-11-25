//! A host for [`winter::embed::Surface`] that speaks a line protocol.
//!
//! The editor demo. A process owns the shell and the pixels; an editor talks to
//! it over stdio. Commands arrive as one JSON object per line, and the process
//! answers on stdout with tagged messages: a frame, or a piece of text for the
//! clipboard (ADR 083).
//!
//! This is not a port, and it is deliberately not a terminal: everything that
//! decides what the screen says is `winter`. What lives here is the smallest
//! host that proves a host can exist outside the shells — the font measurement a
//! shell would do (ADR 029), the pointer policy a shell would apply (ADR 019),
//! and the frame transport an editor needs.

use std::io::{self, BufRead, Write};
use std::path::Path;
use std::time::Instant;

use renderer::{CellSizeInDIP, FontDueRasterizer, FontInfo, Settings};
use term::input::{MouseButton, MouseButtons, MouseEvent, WHEEL_DELTA, WheelAxis};
use utils::Size;
use winter::embed::{Setup, Surface};
use winter::input;
use winter::term_events::TerminalEvent;

/// Measures the host's monospace face at `size` for a `columns` by `rows` grid.
///
/// Font discovery is the host's (ADR 029), so this mirrors what the GTK shell
/// measures, reduced to the pieces an embedding surface wants: the face, its
/// metrics and the atlas settings for the grid the host is about to draw.
pub fn measure(size: f32, columns: i32, rows: i32) -> Option<Setup> {
    let directories = winter::fonts::font_directories();
    let paths: Vec<&Path> = directories.iter().map(std::path::PathBuf::as_path).collect();
    let collection = renderer::FontCollection::from_directories(paths);
    let face = collection.default_monospace(renderer::PREFERRED_MONOSPACE_FAMILIES)?;

    let shaper = FontDueRasterizer::from_file(face.path(), size).ok()?;
    let rasterizer = FontDueRasterizer::from_file(face.path(), size).ok()?;

    let advance = shaper.advance_width('M');
    let (ascent, descent) = shaper.vertical_metrics();
    let cell_width = (advance.ceil() as u16).max(1);
    // The cell is the face's own box, so a glyph cannot spill into the row
    // above it (the GTK shell draws the same box).
    let baseline = (ascent.ceil() as u16).max(1);
    let descender = (descent.ceil() as u16).max(1);
    let cell_height = baseline.saturating_add(descender);

    let mut font = FontInfo::default();
    font.set_from_engine(
        face.family().to_string(),
        0,
        400,
        65001,
        size * 0.75,
        CellSizeInDIP {
            width: f32::from(cell_width),
            height: f32::from(cell_height),
        },
        Size::new(i32::from(cell_width), i32::from(cell_height)),
    );

    let side = |cells: i32, cell: u16| {
        cell.saturating_mul(u16::try_from(cells.max(1)).unwrap_or(u16::MAX))
            .max(1)
    };
    let cells = |count: i32| u16::try_from(count.max(1)).unwrap_or(u16::MAX);

    let settings = Settings {
        font: renderer::FontSettings {
            font_name: face.family().to_string(),
            font_size: size,
            cell_size: (cell_width, cell_height),
            font_weight: 400,
            advance_width: cell_width,
            baseline,
            descender,
            thin_line_width: 1,
            dpi: 96,
            ..renderer::FontSettings::default()
        },
        misc: renderer::MiscellaneousSettings {
            background_color: 0x000C_0C0C,
            foreground_color: 0x00CC_CCCC,
            ..renderer::MiscellaneousSettings::default()
        },
        target_size: (side(columns, cell_width), side(rows, cell_height)),
        viewport_cell_count: (cells(columns), cells(rows)),
        ..Settings::default()
    };

    Some(Setup {
        font,
        settings,
        shaper: Box::new(shaper),
        rasterizer: Box::new(rasterizer),
        columns,
        rows,
    })
}

/// One thing the editor asks the server to do.
#[derive(Debug, Clone, PartialEq, Eq)]
pub enum Command {
    /// Text for the program, as the user typed it.
    Write(String),
    /// A new grid, in cells.
    Resize {
        /// The grid's new width.
        columns: i32,
        /// The grid's new height.
        rows: i32,
    },
    /// Paint and send a frame, if anything changed.
    Render,
    /// A button press at a cell.
    Press {
        /// The cell's column.
        column: i32,
        /// The cell's row.
        row: i32,
        /// 0 left, 1 middle, 2 right.
        button: i32,
        /// Whether Shift is held, which keeps the pointer for the host.
        shift: bool,
    },
    /// A pointer move, with the button that is held or -1 for none.
    Move {
        /// The cell's column.
        column: i32,
        /// The cell's row.
        row: i32,
        /// 0 left, 1 middle, 2 right, -1 nothing held.
        button: i32,
        /// Whether Shift is held.
        shift: bool,
    },
    /// A button release at a cell.
    Release {
        /// The cell's column.
        column: i32,
        /// The cell's row.
        row: i32,
        /// 0 left, 1 middle, 2 right.
        button: i32,
        /// Whether Shift is held.
        shift: bool,
    },
    /// A wheel turn at a cell; positive `lines` scrolls into the history.
    Wheel {
        /// The cell's column.
        column: i32,
        /// The cell's row.
        row: i32,
        /// How many lines, signed.
        lines: i32,
        /// Whether Shift is held, which keeps the wheel for the host.
        shift: bool,
    },
    /// Answer with the selection, for the clipboard.
    Copy,
    /// End the session.
    Quit,
}

/// Reads one command, or `None` when the line names nothing this server knows.
///
/// An unknown line is skipped rather than fatal: a host may send a command this
/// build predates, and dropping the session then would be worse than ignoring
/// the line.
pub fn parse(line: &str) -> Option<Command> {
    let value: serde_json::Value = serde_json::from_str(line).ok()?;
    let shift = || value.get("shift").and_then(serde_json::Value::as_bool).unwrap_or(false);
    let cell = || {
        Some((
            i32::try_from(value.get("col")?.as_i64()?).ok()?,
            i32::try_from(value.get("row")?.as_i64()?).ok()?,
        ))
    };
    let button = |default: i32| {
        value
            .get("button")
            .and_then(serde_json::Value::as_i64)
            .and_then(|button| i32::try_from(button).ok())
            .unwrap_or(default)
    };

    Some(match value.get("cmd")?.as_str()? {
        "write" => Command::Write(value.get("text")?.as_str()?.to_string()),
        "resize" => Command::Resize {
            columns: i32::try_from(value.get("cols")?.as_i64()?).ok()?,
            rows: i32::try_from(value.get("rows")?.as_i64()?).ok()?,
        },
        "render" => Command::Render,
        "press" => {
            let (column, row) = cell()?;
            Command::Press {
                column,
                row,
                button: button(0),
                shift: shift(),
            }
        }
        "move" => {
            let (column, row) = cell()?;
            Command::Move {
                column,
                row,
                button: button(-1),
                shift: shift(),
            }
        }
        "release" => {
            let (column, row) = cell()?;
            Command::Release {
                column,
                row,
                button: button(0),
                shift: shift(),
            }
        }
        "wheel" => {
            let (column, row) = cell()?;
            Command::Wheel {
                column,
                row,
                lines: value
                    .get("lines")
                    .and_then(serde_json::Value::as_i64)
                    .and_then(|lines| i32::try_from(lines).ok())
                    .unwrap_or(3),
                shift: shift(),
            }
        }
        "copy" => Command::Copy,
        "quit" => Command::Quit,
        _ => return None,
    })
}

/// Serves commands from `input`, answering on `output`.
///
/// A frame is `u32 0`, width, height, cell width, cell height, byte length, then
/// that many RGBA bytes. A text answer is `u32 1`, byte length, then UTF-8 —
/// which is how a copy reaches the host's clipboard.
///
/// A frame with a zero width is the answer to a render whose terminal did not
/// change: the host keeps the picture it already has.
pub fn serve(input: impl BufRead, output: &mut impl Write, surface: &mut Surface) -> io::Result<()> {
    let start = Instant::now();

    for line in input.lines() {
        let line = line?;
        if line.trim().is_empty() {
            continue;
        }
        let Some(command) = parse(&line) else {
            continue;
        };

        // Every command reads the screen, so the program's output is drained
        // first: a copy after a burst of output must see that output.
        surface.pump();

        match command {
            Command::Write(text) => surface.write(&text),
            Command::Resize { columns, rows } => surface.resize(columns, rows).map_err(io::Error::other)?,
            Command::Render => {
                let now = Instant::now();
                let cursor = cursor_on(surface, start, now);
                if write_frame(surface, output, cursor)? {
                    output.flush()?;
                }
                if surface.take_events(now).contains(&TerminalEvent::Exited) {
                    break;
                }
            }
            Command::Press {
                column,
                row,
                button,
                shift,
            } => {
                let cell = (column, row);
                if forwards(surface, shift) {
                    surface.mouse(&with_shift(MouseEvent::press(cell, mouse_button(button)), shift));
                } else if button == 0 {
                    surface.press(cell, Instant::now());
                }
            }
            Command::Move {
                column,
                row,
                button,
                shift,
            } => {
                let cell = (column, row);
                if forwards(surface, shift) {
                    let mut event = MouseEvent::moved(cell);
                    event.buttons = mouse_buttons(button);
                    event.modifiers.shift = shift;
                    surface.mouse(&event);
                } else if button == 0 {
                    // A drag extends the selection the press started.
                    surface.drag(cell);
                }
            }
            Command::Release {
                column,
                row,
                button,
                shift,
            } => {
                let cell = (column, row);
                if forwards(surface, shift) {
                    surface.mouse(&with_shift(MouseEvent::release(cell, mouse_button(button)), shift));
                }
            }
            Command::Wheel {
                column,
                row,
                lines,
                shift,
            } => {
                let cell = (column, row);
                if forwards(surface, shift) {
                    let delta = WHEEL_DELTA * lines.signum();
                    let event = MouseEvent::wheel(cell, WheelAxis::Vertical, delta);
                    surface.mouse(&with_shift(event, shift));
                } else {
                    surface.scroll_by(lines);
                }
            }
            Command::Copy => {
                let text = surface.selection_text().unwrap_or_default();
                write_text(output, &text)?;
                output.flush()?;
            }
            Command::Quit => break,
        }
    }
    Ok(())
}

/// Whether the pointer belongs to the program rather than to this host.
fn forwards(surface: &Surface, shift: bool) -> bool {
    input::forwards_to_application(surface.is_tracking_mouse(), shift)
}

/// The cursor phase at `now`, or `true` while the cursor is not drawn.
///
/// The half-period is the shells' 500 ms, and the phase is constant while the
/// cursor is hidden, so an idle pane stops asking for frames.
fn cursor_on(surface: &Surface, start: Instant, now: Instant) -> bool {
    if !surface.cursor_visible() {
        return true;
    }
    (now.saturating_duration_since(start).as_millis() / 500).is_multiple_of(2)
}

fn mouse_button(button: i32) -> MouseButton {
    match button {
        1 => MouseButton::Middle,
        2 => MouseButton::Right,
        _ => MouseButton::Left,
    }
}

fn mouse_buttons(button: i32) -> MouseButtons {
    let mut buttons = MouseButtons::default();
    match button {
        0 => buttons.left = true,
        1 => buttons.middle = true,
        2 => buttons.right = true,
        _ => {}
    }
    buttons
}

fn with_shift(mut event: MouseEvent, shift: bool) -> MouseEvent {
    event.modifiers.shift = shift;
    event
}

/// Paints a frame and writes it, or writes a zero-width header when nothing
/// changed.
///
/// Returns whether a frame was painted; a host that reads a zero-width header
/// keeps the picture it already has.
pub fn write_frame(surface: &mut Surface, output: &mut impl Write, cursor: bool) -> io::Result<bool> {
    if !surface.render(cursor).map_err(io::Error::other)? {
        output.write_all(&0u32.to_be_bytes())?;
        output.write_all(&[0u8; 20])?;
        return Ok(false);
    }

    let (width, height) = surface.frame_size();
    let (cell_width, cell_height) = surface.cell_size();
    let pixels = surface.pixels();

    let mut rgba = Vec::with_capacity(pixels.len() * 4);
    for pixel in pixels.iter() {
        let [_, red, green, blue] = pixel.to_be_bytes();
        // The pane is opaque: a canvas with `alpha = 0` would show through.
        rgba.extend_from_slice(&[red, green, blue, 255]);
    }

    output.write_all(&0u32.to_be_bytes())?;
    output.write_all(&width.to_be_bytes())?;
    output.write_all(&height.to_be_bytes())?;
    output.write_all(&u32::from(cell_width).to_be_bytes())?;
    output.write_all(&u32::from(cell_height).to_be_bytes())?;
    output.write_all(&(rgba.len() as u32).to_be_bytes())?;
    output.write_all(&rgba)?;
    Ok(true)
}

/// Writes a text answer, which is how a copy reaches the host.
pub fn write_text(output: &mut impl Write, text: &str) -> io::Result<()> {
    output.write_all(&1u32.to_be_bytes())?;
    output.write_all(&(text.len() as u32).to_be_bytes())?;
    output.write_all(text.as_bytes())
}

#[cfg(test)]
mod tests {
    use renderer::{CellShaping, FontId, GlyphBitmap, GlyphRasterizer};
    use winter::embed::Surface;
    use winter::term_connection::Queue;

    use super::*;

    /// A rasteriser that paints the whole cell, so any glyph is visible.
    struct SolidRasterizer;

    impl GlyphRasterizer for SolidRasterizer {
        fn rasterize(&mut self, _font: FontId, _glyph: u16) -> Option<GlyphBitmap> {
            Some(GlyphBitmap {
                width: 8,
                height: 16,
                left: 0,
                top: 0,
                coverage: vec![255; 8 * 16],
            })
        }
    }

    fn fixture(columns: i32, rows: i32) -> (Surface, Queue) {
        let mut font = FontInfo::default();
        font.set_from_engine(
            "Test".to_string(),
            0,
            400,
            65001,
            16.0,
            CellSizeInDIP {
                width: 8.0,
                height: 16.0,
            },
            Size::new(8, 16),
        );

        let settings = Settings {
            font: renderer::FontSettings {
                cell_size: (8, 16),
                font_size: 16.0,
                advance_width: 8,
                baseline: 12,
                descender: 4,
                thin_line_width: 1,
                ..renderer::FontSettings::default()
            },
            target_size: (columns as u16 * 8, rows as u16 * 16),
            viewport_cell_count: (columns as u16, rows as u16),
            ..Settings::default()
        };

        let queue = Queue::default();
        let surface = Surface::new(
            Setup {
                font,
                settings,
                shaper: Box::new(CellShaping),
                rasterizer: Box::new(SolidRasterizer),
                columns,
                rows,
            },
            Box::new(queue.clone()),
        );
        (surface, queue)
    }

    /// The frame header is `u32 0` then five more `u32`s.
    const FRAME_HEADER: usize = 24;
    /// A text answer is `u32 1` and a length.
    const TEXT_HEADER: usize = 8;

    fn kind(message: &[u8]) -> u32 {
        u32::from_be_bytes(message[0..4].try_into().expect("four bytes"))
    }

    fn width(frame: &[u8]) -> u32 {
        u32::from_be_bytes(frame[4..8].try_into().expect("four bytes"))
    }

    fn height(frame: &[u8]) -> u32 {
        u32::from_be_bytes(frame[8..12].try_into().expect("four bytes"))
    }

    fn text_of(message: &[u8]) -> String {
        assert_eq!(kind(message), 1, "not a text answer");
        let length = u32::from_be_bytes(message[4..8].try_into().expect("four bytes")) as usize;
        String::from_utf8(message[TEXT_HEADER..TEXT_HEADER + length].to_vec()).expect("utf-8")
    }

    #[test]
    fn a_render_command_asks_for_a_frame() {
        assert_eq!(parse(r#"{"cmd":"render"}"#), Some(Command::Render));
        // A host may still send the phase it used to own; it is ignored now.
        assert_eq!(parse(r#"{"cmd":"render","cursor":false}"#), Some(Command::Render));
    }

    #[test]
    fn a_line_that_names_nothing_this_server_knows_is_ignored() {
        assert_eq!(parse("hello"), None);
        assert_eq!(parse(r#"{"cmd":"nope"}"#), None);
        // A resize missing a side cannot be honoured.
        assert_eq!(parse(r#"{"cmd":"resize","cols":1}"#), None);
        assert_eq!(parse(r#"{"cmd":"press","col":1}"#), None);
    }

    #[test]
    fn a_pointer_command_names_its_cell_button_and_shift() {
        assert_eq!(
            parse(r#"{"cmd":"press","col":2,"row":3,"button":2,"shift":true}"#),
            Some(Command::Press {
                column: 2,
                row: 3,
                button: 2,
                shift: true
            })
        );
        // A move with no button named is a move with nothing held.
        assert_eq!(
            parse(r#"{"cmd":"move","col":0,"row":0}"#),
            Some(Command::Move {
                column: 0,
                row: 0,
                button: -1,
                shift: false
            })
        );
        assert_eq!(
            parse(r#"{"cmd":"wheel","col":0,"row":0}"#),
            Some(Command::Wheel {
                column: 0,
                row: 0,
                lines: 3,
                shift: false
            })
        );
    }

    #[test]
    fn a_write_command_reaches_the_program() {
        let (mut surface, queue) = fixture(20, 2);
        let mut out = Vec::new();

        serve(br#"{"cmd":"write","text":"ls\r"}"#.as_slice(), &mut out, &mut surface).expect("serve");

        assert_eq!(queue.take_written(), b"ls\r");
        assert!(out.is_empty(), "a write is not a frame");
    }

    #[test]
    fn a_render_command_writes_a_tagged_header_and_rgba_pixels() {
        let (mut surface, queue) = fixture(4, 2);
        // Ink on the second row, with the cursor hidden so it can only be text.
        queue.feed(b"\x1b[?25l\r\nhi");
        let mut out = Vec::new();

        serve(br#"{"cmd":"render","cursor":true}"#.as_slice(), &mut out, &mut surface).expect("serve");

        assert_eq!(kind(&out), 0);
        assert_eq!((width(&out), height(&out)), (32, 32));
        assert_eq!(u32::from_be_bytes(out[12..16].try_into().expect("four bytes")), 8);
        assert_eq!(u32::from_be_bytes(out[16..20].try_into().expect("four bytes")), 16);
        let length = u32::from_be_bytes(out[20..24].try_into().expect("four bytes")) as usize;
        assert_eq!(length, 32 * 32 * 4);
        assert_eq!(out.len(), FRAME_HEADER + length);

        let rgba = &out[FRAME_HEADER..];
        let inked = rgba[16 * 32 * 4..]
            .as_chunks::<4>()
            .0
            .iter()
            .any(|pixel| pixel[0] > 0x80 && pixel[3] == 255);
        assert!(inked, "the fed text did not reach the frame");
    }

    #[test]
    fn a_resize_command_moves_the_frame() {
        let (mut surface, _queue) = fixture(4, 2);
        let mut out = Vec::new();

        serve(
            br#"{"cmd":"resize","cols":8,"rows":3}
{"cmd":"render"}"#
                .as_slice(),
            &mut out,
            &mut surface,
        )
        .expect("serve");

        assert_eq!((width(&out), height(&out)), (64, 48));
    }

    #[test]
    fn an_idle_render_answers_with_a_zero_width_header() {
        let (mut surface, _queue) = fixture(4, 2);
        let mut painted = Vec::new();
        serve(br#"{"cmd":"render"}"#.as_slice(), &mut painted, &mut surface).expect("serve");
        assert_eq!(width(&painted), 32);

        let mut idle = Vec::new();
        serve(br#"{"cmd":"render"}"#.as_slice(), &mut idle, &mut surface).expect("serve");

        assert_eq!(idle.len(), FRAME_HEADER, "an idle answer is a header and nothing else");
        assert_eq!(kind(&idle), 0);
        assert_eq!(width(&idle), 0);
    }

    #[test]
    fn output_after_an_idle_answer_paints_again() {
        let (mut surface, queue) = fixture(4, 2);
        serve(br#"{"cmd":"render"}"#.as_slice(), &mut Vec::new(), &mut surface).expect("serve");

        queue.feed(b"hi");
        let mut out = Vec::new();
        serve(br#"{"cmd":"render"}"#.as_slice(), &mut out, &mut surface).expect("serve");

        assert_eq!(width(&out), 32);
    }

    #[test]
    fn a_copy_command_answers_with_the_word_that_was_double_clicked() {
        let (mut surface, queue) = fixture(20, 2);
        queue.feed(b"hello world");
        let mut out = Vec::new();

        serve(
            br#"{"cmd":"press","col":7,"row":0}
{"cmd":"press","col":7,"row":0}
{"cmd":"copy"}"#
                .as_slice(),
            &mut out,
            &mut surface,
        )
        .expect("serve");

        assert_eq!(text_of(&out), "world");
    }

    #[test]
    fn a_drag_extends_the_selection_a_press_started() {
        let (mut surface, queue) = fixture(20, 2);
        queue.feed(b"hello world");
        let mut out = Vec::new();

        serve(
            br#"{"cmd":"press","col":1,"row":0}
{"cmd":"move","col":4,"row":0,"button":0}
{"cmd":"copy"}"#
                .as_slice(),
            &mut out,
            &mut surface,
        )
        .expect("serve");

        assert_eq!(text_of(&out), "ello");
    }

    #[test]
    fn a_press_reaches_a_program_that_tracks_the_mouse() {
        let (mut surface, queue) = fixture(20, 2);
        // DECSET 1000 turns the pointer into X10 reports at the cell.
        queue.feed(b"\x1b[?1000h");
        let mut out = Vec::new();

        serve(br#"{"cmd":"press","col":1,"row":0}"#.as_slice(), &mut out, &mut surface).expect("serve");

        let written = queue.take_written();
        assert!(written.starts_with(b"\x1b[M"), "{written:?}");
        assert!(out.is_empty(), "a pointer event is not a frame");
    }

    #[test]
    fn shift_keeps_the_pointer_for_the_host_even_while_the_program_tracks_it() {
        let (mut surface, queue) = fixture(20, 2);
        queue.feed(b"\x1b[?1000h\x1b[?25lhello");
        let mut out = Vec::new();

        serve(
            br#"{"cmd":"press","col":1,"row":0,"shift":true}
{"cmd":"copy"}"#
                .as_slice(),
            &mut out,
            &mut surface,
        )
        .expect("serve");

        // The program was told nothing; the host selected the cell instead.
        assert!(queue.take_written().is_empty(), "the program heard the press");
        assert_eq!(text_of(&out), "e");
    }

    #[test]
    fn a_quit_command_ends_the_loop() {
        let (mut surface, _queue) = fixture(4, 2);
        let mut out = Vec::new();

        serve(
            br#"{"cmd":"quit"}
{"cmd":"render"}"#
                .as_slice(),
            &mut out,
            &mut surface,
        )
        .expect("serve");

        assert!(out.is_empty(), "nothing after a quit may be served");
    }
}
