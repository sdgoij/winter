//! A terminal a host embeds without a window.
//!
//! This is not a port. The pieces a terminal is made of are here already —
//! [`Control`] owns the parser and the buffer, [`crate::terminal_render`] copies
//! a frame into a renderer's shape, [`crate::view`] holds the viewport — but a
//! host that is not one of the shells has to compose them: feed bytes, translate
//! input, refresh the frame's data, paint, and read the pixels. Every host that
//! did that itself would re-derive the same four steps, and mis-derive them
//! slightly differently. [`Surface`] is the composition, once (ADR 082).
//!
//! # What a host owns
//!
//! - **The bytes.** [`Surface::feed`] is what the program wrote; [`Surface::take_written`]
//!   is what the terminal answered. Forwarding the second to a pty, a socket or a
//!   worker is the host's job, because only the host knows where the process is
//!   (ADR 015).
//! - **The font.** Measuring a face is the host's (ADR 029) and every shell does
//!   it its own way, so [`Setup`] carries the measured result rather than the
//!   path to a file.
//! - **The alerts.** [`Surface::take_events`] reports a bell, a desktop
//!   notification and an exit as data; whether a bell flashes, beeps or is
//!   ignored is a decision for the thing that draws (ADR 036).
//!
//! # What it owns
//!
//! The viewport, so [`Surface::scroll_by`] means something without the host
//! tracking a document position. Not the selection: a host that draws its own
//! chrome has its own pointer, and the selection is application state (ADR 019).

use std::cell::Ref;
use std::sync::{Arc, Mutex};
use std::time::Instant;

use renderer::renderer::Renderer;
use renderer::{
    AtlasEngine, FontInfo, GlyphRasterizer, RenderError, RenderSettings, Settings, ShapingBackend, SharedBackend,
    SoftwareBackend,
};
use term::input::MouseEvent;
use types::color_table::ColorTable;
use utils::InclusiveRect;

use crate::input::{self, Event, ModifierSides};
use crate::term_connection::Queue;
use crate::term_control::Control;
use crate::term_events::TerminalEvent;
use crate::terminal_render::TerminalData;
use crate::view::View;

/// What a host measures once and hands a [`Surface`].
///
/// The settings must already describe `columns` by `rows` cells: a host measures
/// the face *for* a grid, which is how every shell builds them, and the surface
/// does not guess a target size from a font it did not choose.
pub struct Setup {
    /// The face's metrics, as the host loaded it.
    pub font: FontInfo,
    /// The atlas settings: cell size, baseline, colours, target size.
    pub settings: Settings,
    /// Shapes a row's text into glyphs. [`renderer::CellShaping`] is the
    /// per-cell default; a host with a real shaper passes it here.
    pub shaper: Box<dyn ShapingBackend>,
    /// Rasterises one glyph into a coverage mask.
    pub rasterizer: Box<dyn GlyphRasterizer>,
    /// The grid's width in cells.
    pub columns: i32,
    /// The grid's height in cells.
    pub rows: i32,
}

/// A terminal and its frame, for a host that has no window toolkit.
pub struct Surface {
    control: Control,
    queue: Queue,
    view: View,
    renderer: Renderer<TerminalData>,
    shared: SharedBackend,
    /// The modifier sides the toolkit collapses but the encoder distinguishes.
    sides: ModifierSides,
    /// The cell size in pixels, which a resize needs and the host measured.
    cell: (u16, u16),
    size: (i32, i32),
}

impl Surface {
    /// Builds a surface over a queue the host feeds.
    pub fn new(setup: Setup) -> Self {
        let Setup {
            font,
            settings,
            shaper,
            rasterizer,
            columns,
            rows,
        } = setup;

        let cell = settings.font.cell_size;
        let queue = Queue::default();
        let control = Control::new(Box::new(queue.clone()), (columns, rows));

        let shared = SharedBackend::new(SoftwareBackend::with_rasterizer(rasterizer));
        let mut engine = AtlasEngine::with_settings(settings, Box::new(shared.clone()));
        engine.set_shaping(shaper);

        let mut renderer = Renderer::new(
            TerminalData::new(font, RenderSettings::new(), columns, rows),
            RenderSettings::new(),
        );
        renderer.add_engine(Box::new(engine));

        Self {
            control,
            queue,
            view: View::new(),
            renderer,
            shared,
            sides: ModifierSides::default(),
            cell,
            size: (columns, rows),
        }
    }

    /// Queues bytes the program wrote, for the next [`render`](Self::render).
    pub fn feed(&mut self, bytes: &[u8]) {
        self.queue.feed(bytes);
    }

    /// Everything the terminal answered since the last call, taking it.
    ///
    /// These are the program's due: a device-attributes reply, a cursor-position
    /// report, an OSC colour query. The host forwards them to whatever it spawned.
    pub fn take_written(&self) -> Vec<u8> {
        self.queue.take_written()
    }

    /// Sends text to the program, as if the user had typed it.
    pub fn write(&mut self, text: &str) {
        self.control.write(text);
    }

    /// Pastes text through the terminal's bracketed-paste filter.
    pub fn paste(&mut self, text: &str) {
        self.control.paste(text);
    }

    /// Translates one toolkit event and sends what it encodes.
    ///
    /// The rules — a printable key arriving twice, modifier sides, AltGr — are
    /// [`crate::input`]'s, so a host does not re-derive them (ADR 009, 040).
    pub fn key(&mut self, event: &Event) {
        let win32 = self.control.input_modes().win32_input;
        let outcome = input::translate(event, &mut self.sides, win32);

        // Typing returns the view to the live screen, as every shell does.
        if outcome.activity {
            self.view.scroll_to_bottom();
        }
        if let Some(text) = outcome.paste {
            self.control.paste(&text);
        }
        for key in outcome.keys {
            self.control.send_key(&key);
        }
    }

    /// Sends a mouse event to a program that tracks the mouse.
    ///
    /// Returns whether it was taken; a terminal that is not tracking owns the
    /// wheel and the clicks itself, so the host scrolls or selects instead.
    pub fn mouse(&mut self, event: &MouseEvent) -> bool {
        self.control.send_mouse(event)
    }

    /// Scrolls the view by `lines`, clamped to the history there is.
    pub fn scroll_by(&mut self, lines: i32) {
        let max = self.control.max_scroll();
        self.view.scroll_by(lines, max);
    }

    /// Resizes the terminal to `columns` by `rows` cells.
    ///
    /// The frame is repainted at the new size on the next [`render`](Self::render).
    pub fn resize(&mut self, columns: i32, rows: i32) -> Result<(), RenderError> {
        self.control.resize(columns, rows);
        self.size = (columns, rows);
        self.renderer.resize(
            target_size(self.cell, columns, rows),
            InclusiveRect::new(0, 0, columns.max(1) - 1, rows.max(1) - 1),
        )
    }

    /// Moves the fed bytes into the terminal and paints one frame.
    ///
    /// The frame is read with [`pixels`](Self::pixels) afterwards. Returning the
    /// pixels from here is not possible without keeping the surface mutably
    /// borrowed for as long as they are held, because the buffer they borrow is
    /// a field of the surface; the pair is one call apart instead.
    pub fn render(&mut self, cursor_on: bool) -> Result<(), RenderError> {
        self.control.pump();
        self.renderer
            .data_mut()
            .refresh(&mut self.control, &self.view, cursor_on, None);

        // A program's OSC 10/11 can change the palette, so the engine is handed
        // the table every frame, exactly as the shells do (ADR 040).
        if let Ok(table) = self.control.color_table().lock() {
            self.renderer.settings_mut().set_color_table(table.clone());
        }

        // The atlas paints only its dirty area; a host that presents the whole
        // buffer every frame invalidates first (ADR 006).
        self.renderer.invalidate_all();
        self.renderer.paint_frame()
    }

    /// The last painted frame's pixels, row-major `0xAARRGGBB`.
    pub fn pixels(&self) -> Ref<'_, [u32]> {
        self.shared.pixels()
    }

    /// The last painted frame's size in pixels.
    pub fn frame_size(&self) -> (u32, u32) {
        (self.shared.width(), self.shared.height())
    }

    /// What the terminal did since the last call: a bell, a notification, an exit.
    pub fn take_events(&mut self, now: Instant) -> Vec<TerminalEvent> {
        self.control.take_events(now)
    }

    /// The palette the frame is painted from.
    ///
    /// A host sets its colours here and the next paint uses them; a program's
    /// OSC 10/11 still wins, because the engine is handed this table every
    /// frame (ADR 040). This is the settings-free way to theme an embedding.
    pub fn color_table(&self) -> Arc<Mutex<ColorTable>> {
        self.control.color_table()
    }

    /// The grid size in cells.
    pub fn size(&self) -> (i32, i32) {
        self.size
    }

    /// The title the program set, if it set one.
    pub fn title(&self) -> Option<&str> {
        self.control.title()
    }
}

/// The pixel size of a `columns` by `rows` grid of `cell`-sized cells.
///
/// The shells compute this for themselves when they measure their fonts; the
/// surface has to recompute it on a resize, because the host hands it a grid and
/// not a pixel target.
fn target_size(cell: (u16, u16), columns: i32, rows: i32) -> (u16, u16) {
    let side = |cells: i32, cell: u16| {
        cell.saturating_mul(u16::try_from(cells.max(1)).unwrap_or(u16::MAX))
            .max(1)
    };
    (side(columns, cell.0), side(rows, cell.1))
}

#[cfg(test)]
mod tests {
    use renderer::{CellShaping, CellSizeInDIP, FontId, GlyphBitmap};
    use utils::Size;

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

    fn font() -> FontInfo {
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
        font
    }

    fn settings(columns: u16, rows: u16) -> Settings {
        Settings {
            font: renderer::FontSettings {
                cell_size: (8, 16),
                font_size: 16.0,
                advance_width: 8,
                baseline: 12,
                descender: 4,
                thin_line_width: 1,
                ..renderer::FontSettings::default()
            },
            misc: renderer::MiscellaneousSettings {
                background_color: 0x00_0000,
                foreground_color: 0xFF_FFFF,
                ..renderer::MiscellaneousSettings::default()
            },
            target_size: (columns * 8, rows * 16),
            viewport_cell_count: (columns, rows),
            ..Settings::default()
        }
    }

    fn surface(columns: i32, rows: i32) -> Surface {
        Surface::new(Setup {
            font: font(),
            // A grid this small always fits a `u16`, so the test can cast.
            settings: settings(columns as u16, rows as u16),
            shaper: Box::new(CellShaping),
            rasterizer: Box::new(SolidRasterizer),
            columns,
            rows,
        })
    }

    #[test]
    fn a_frame_of_fed_bytes_reaches_the_pixels() {
        let mut surface = surface(4, 2);
        // The text is on the second row and the cursor is hidden, so ink there
        // can only have come from the fed bytes.
        surface.feed(b"\x1b[?25l\r\nhi");

        surface.render(true).expect("a frame");

        assert_eq!(surface.frame_size(), (32, 32));
        let pixels = surface.pixels();
        assert_eq!(pixels.len(), 32 * 32);
        let second_row = &pixels[16 * 32..];
        assert!(
            second_row.iter().any(|&pixel| pixel & 0x00FF_FFFF > 0x0080_8080),
            "the fed text was not painted"
        );
    }

    #[test]
    fn a_key_and_a_paste_reach_the_program_in_order() {
        let mut surface = surface(20, 2);

        surface.key(&Event::Text("a".to_string()));
        surface.paste("two words");

        assert_eq!(surface.take_written(), b"atwo words");
        // Taking it leaves nothing for the next forward.
        assert!(surface.take_written().is_empty());
    }

    #[test]
    fn a_resize_moves_the_terminal_the_grid_and_the_frame() {
        let mut surface = surface(4, 2);

        surface.resize(8, 3).expect("resize");
        surface.render(true).expect("a frame");

        assert_eq!(surface.size(), (8, 3));
        assert_eq!(surface.frame_size(), (64, 48));
    }

    #[test]
    fn a_host_can_set_the_palette_the_frame_paints() {
        use types::color_table::ColorAlias;
        use types::text_color::TextColor;

        let mut surface = surface(2, 1);
        // The cursor is a filled box over the first cell, so the second cell's
        // background is what this reads; hiding it keeps the sample stable.
        surface.feed(b"\x1b[?25l");
        surface.render(true).expect("a frame");
        let before = surface.pixels()[8];

        let palette = surface.color_table();
        {
            let mut table = palette.lock().expect("lock");
            let index = table.alias_index(ColorAlias::DefaultBackground);
            table.set(index, TextColor::from_colorref(0x0000_00FF));
        }

        surface.render(true).expect("a frame");
        assert_ne!(surface.pixels()[8], before, "the new background was not painted");
    }

    #[test]
    fn the_title_the_program_sets_is_the_one_reported() {
        let mut surface = surface(20, 2);
        surface.feed(b"\x1b]0;a title\x07");

        surface.render(true).expect("a frame");

        assert_eq!(surface.title(), Some("a title"));
    }

    #[test]
    fn the_programs_answer_is_written_and_a_bell_is_reported_once() {
        let now = Instant::now();
        let mut surface = surface(20, 2);
        // DA1: the terminal answers this itself, with no input from the host.
        surface.feed(b"\x1b[c\x07");

        surface.render(true).expect("a frame");

        let written = surface.take_written();
        assert!(written.starts_with(b"\x1b["), "{written:?}");
        assert!(written.ends_with(b"c"), "{written:?}");
        assert_eq!(surface.take_events(now), [TerminalEvent::Bell]);
        assert!(surface.take_events(now).is_empty());
    }
}
