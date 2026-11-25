//! A host for [`winter::embed::Surface`] that speaks a line protocol.
//!
//! The editor demo. A process owns the shell and the pixels; an editor talks to
//! it over stdio. Commands arrive as one JSON object per line, and a frame goes
//! back as a fixed header and RGBA bytes (ADR 083).
//!
//! This is not a port, and it is deliberately not a terminal: everything that
//! decides what the screen says is `winter`. What lives here is the smallest
//! host that proves a host can exist outside the shells — the font measurement a
//! shell would do (ADR 029) and the frame transport an editor needs.

use std::io::{self, BufRead, Write};
use std::path::Path;
use std::time::Instant;

use renderer::{CellSizeInDIP, FontDueRasterizer, FontInfo, Settings};
use utils::Size;
use winter::embed::{Setup, Surface};
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
    /// Paint and send a frame.
    Render {
        /// Whether the cursor is in its "on" blink phase.
        cursor: bool,
    },
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
    match value.get("cmd")?.as_str()? {
        "write" => Some(Command::Write(value.get("text")?.as_str()?.to_string())),
        "resize" => Some(Command::Resize {
            columns: i32::try_from(value.get("cols")?.as_i64()?).ok()?,
            rows: i32::try_from(value.get("rows")?.as_i64()?).ok()?,
        }),
        "render" => Some(Command::Render {
            cursor: value.get("cursor").and_then(serde_json::Value::as_bool).unwrap_or(true),
        }),
        "quit" => Some(Command::Quit),
        _ => None,
    }
}

/// Serves commands from `input`, writing frames to `output`.
///
/// Each frame is five `u32`s — width, height, cell width, cell height and the
/// byte length — then that many RGBA bytes. The cell size is in the header so a
/// host can turn a pane's pixel size into a grid to ask for: without it a host
/// would have to guess the font's metrics.
pub fn serve(input: impl BufRead, output: &mut impl Write, surface: &mut Surface) -> io::Result<()> {
    for line in input.lines() {
        let line = line?;
        if line.trim().is_empty() {
            continue;
        }
        let Some(command) = parse(&line) else {
            continue;
        };
        match command {
            Command::Write(text) => surface.write(&text),
            Command::Resize { columns, rows } => surface.resize(columns, rows).map_err(io::Error::other)?,
            Command::Render { cursor } => {
                write_frame(surface, output, cursor)?;
                output.flush()?;
                if surface.take_events(Instant::now()).contains(&TerminalEvent::Exited) {
                    break;
                }
            }
            Command::Quit => break,
        }
    }
    Ok(())
}

/// Paints a frame and writes its header and pixels.
pub fn write_frame(surface: &mut Surface, output: &mut impl Write, cursor: bool) -> io::Result<()> {
    surface.render(cursor).map_err(io::Error::other)?;
    let (width, height) = surface.frame_size();
    let (cell_width, cell_height) = surface.cell_size();
    let pixels = surface.pixels();

    let mut rgba = Vec::with_capacity(pixels.len() * 4);
    for pixel in pixels.iter() {
        let [_, red, green, blue] = pixel.to_be_bytes();
        // The pane is opaque: a canvas with `alpha = 0` would show through.
        rgba.extend_from_slice(&[red, green, blue, 255]);
    }

    output.write_all(&width.to_be_bytes())?;
    output.write_all(&height.to_be_bytes())?;
    output.write_all(&u32::from(cell_width).to_be_bytes())?;
    output.write_all(&u32::from(cell_height).to_be_bytes())?;
    output.write_all(&(rgba.len() as u32).to_be_bytes())?;
    output.write_all(&rgba)
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

    fn width(frame: &[u8]) -> u32 {
        u32::from_be_bytes(frame[0..4].try_into().expect("four bytes"))
    }

    fn height(frame: &[u8]) -> u32 {
        u32::from_be_bytes(frame[4..8].try_into().expect("four bytes"))
    }

    #[test]
    fn a_render_command_asks_for_a_frame_and_names_the_cursor() {
        assert_eq!(parse(r#"{"cmd":"render"}"#), Some(Command::Render { cursor: true }));
        assert_eq!(
            parse(r#"{"cmd":"render","cursor":false}"#),
            Some(Command::Render { cursor: false })
        );
    }

    #[test]
    fn a_line_that_names_nothing_this_server_knows_is_ignored() {
        assert_eq!(parse("hello"), None);
        assert_eq!(parse(r#"{"cmd":"nope"}"#), None);
        // A resize missing a side cannot be honoured.
        assert_eq!(parse(r#"{"cmd":"resize","cols":1}"#), None);
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
    fn a_render_command_writes_a_header_and_rgba_pixels() {
        let (mut surface, queue) = fixture(4, 2);
        // Ink on the second row, with the cursor hidden so it can only be text.
        queue.feed(b"\x1b[?25l\r\nhi");
        let mut out = Vec::new();

        serve(br#"{"cmd":"render","cursor":true}"#.as_slice(), &mut out, &mut surface).expect("serve");

        assert_eq!((width(&out), height(&out)), (32, 32));
        assert_eq!(u32::from_be_bytes(out[8..12].try_into().expect("four bytes")), 8);
        assert_eq!(u32::from_be_bytes(out[12..16].try_into().expect("four bytes")), 16);
        let length = u32::from_be_bytes(out[16..20].try_into().expect("four bytes")) as usize;
        assert_eq!(length, 32 * 32 * 4);
        assert_eq!(out.len(), 20 + length);

        let rgba = &out[20..];
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
