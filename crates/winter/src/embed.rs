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
//! - **The bytes.** [`Surface::new`] takes the connection the host owns — a pty
//!   session, or a [`Queue`] the host feeds — so the same surface drives a real
//!   shell and a test. Which bytes arrive, and where the terminal's answers go,
//!   is the connection's business (ADR 015).
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
use crate::term_connection::Connection;
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
    view: View,
    renderer: Renderer<TerminalData>,
    shared: SharedBackend,
    /// The modifier sides the toolkit collapses but the encoder distinguishes.
    sides: ModifierSides,
    /// The cell size in pixels, which a resize needs and the host measured.
    cell: (u16, u16),
    size: (i32, i32),
    /// Whether anything has changed since the last painted frame.
    dirty: bool,
    /// The cursor phase of the last painted frame.
    last_cursor: Option<bool>,
}

impl Surface {
    /// Builds a surface over a connection the host owns.
    ///
    /// A host that spawned a process passes its session; one that owns the bytes
    /// passes a [`Queue`](crate::term_connection::Queue) it keeps a clone of, so
    /// it can feed the program's output and take the terminal's answers.
    pub fn new(setup: Setup, connection: Box<dyn Connection>) -> Self {
        let Setup {
            font,
            settings,
            shaper,
            rasterizer,
            columns,
            rows,
        } = setup;

        let cell = settings.font.cell_size;
        let control = Control::new(connection, (columns, rows));

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
            view: View::new(),
            renderer,
            shared,
            sides: ModifierSides::default(),
            cell,
            size: (columns, rows),
            // The first frame has nothing to compare against.
            dirty: true,
            last_cursor: None,
        }
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

    /// Whether the program is tracking the mouse, so the pointer belongs to it.
    pub fn is_tracking_mouse(&self) -> bool {
        self.control.is_tracking_mouse()
    }

    /// Scrolls the view by `lines`, clamped to the history there is.
    pub fn scroll_by(&mut self, lines: i32) {
        let max = self.control.max_scroll();
        self.view.scroll_by(lines, max);
        self.dirty = true;
    }

    /// Starts a selection at a screen cell and returns how many clicks it was.
    ///
    /// The terminal has to have been pumped for this to name cells that exist:
    /// [`render`](Self::render) pumps, and a host that asks about the screen
    /// before painting calls [`pump`](Self::pump) itself.
    ///
    /// A double click selects the word under the cell and a triple click the
    /// whole row, which is the rule every shell applies (ADR 019). The host
    /// counts pixels; this decides what the click means.
    pub fn press(&mut self, cell: (i32, i32), now: Instant) -> usize {
        let (column, row) = cell;
        let document = self.document_row(row);
        let position = (column, document);
        let clicks = self.view.click(position, now);

        match clicks {
            2 => {
                let text = self.control.row_text(document);
                self.view.select_word(&text, position);
            }
            3 => self.view.select_line(document, self.size.0),
            _ => self.view.select(position, false),
        }
        self.dirty = true;
        clicks
    }

    /// Moves the selection's end to a screen cell, for a drag.
    pub fn drag(&mut self, cell: (i32, i32)) {
        let end = (cell.0, self.document_row(cell.1));
        self.view.extend(end);
        self.dirty = true;
    }

    /// Scrolls while a drag holds the pointer outside the pane.
    ///
    /// `outside` is negative above the grid, positive below it and zero
    /// inside; the rate is the view's bound, so a fast pointer cannot outrun
    /// the screen.
    pub fn drag_scroll(&mut self, outside: i32, now: Instant) {
        let max = self.control.max_scroll();
        self.view.drag_scroll(outside, max, now);
    }

    /// Drops the selection.
    pub fn clear_selection(&mut self) {
        self.view.clear_selection();
        self.dirty = true;
    }

    /// The selected text, copied through the buffer's own copy path.
    pub fn selection_text(&mut self) -> Option<String> {
        let selection = *self.view.selection()?;
        self.control.selection_text(&selection)
    }

    /// Moves whatever the connection has produced into the terminal.
    ///
    /// [`render`](Self::render) does this too; a host that answers a question
    /// about the screen before painting — a copy, a selection — pumps first.
    pub fn pump(&mut self) {
        if self.control.pump() {
            self.dirty = true;
        }
    }

    /// The document row a screen row shows, history included.
    fn document_row(&mut self, screen_row: i32) -> i32 {
        let history = self.control.max_scroll();
        input::screen_to_document(history, self.view.scroll(), screen_row)
    }

    /// Resizes the terminal to `columns` by `rows` cells.
    ///
    /// The frame is repainted at the new size on the next [`render`](Self::render).
    pub fn resize(&mut self, columns: i32, rows: i32) -> Result<(), RenderError> {
        self.control.resize(columns, rows);
        self.size = (columns, rows);
        self.dirty = true;
        self.renderer.resize(
            target_size(self.cell, columns, rows),
            InclusiveRect::new(0, 0, columns.max(1) - 1, rows.max(1) - 1),
        )
    }

    /// Moves the fed bytes into the terminal and paints one frame.
    ///
    /// Returns whether a frame was painted. Nothing new to draw — no bytes, no
    /// resize, no scroll, no selection, and the same cursor phase — is the usual
    /// case while a program sits idle, and a host should keep the frame it has
    /// rather than ship the same pixels again (ADR 083).
    ///
    /// The frame is read with [`pixels`](Self::pixels) afterwards. Returning the
    /// pixels from here is not possible without keeping the surface mutably
    /// borrowed for as long as they are held, because the buffer they borrow is
    /// a field of the surface; the pair is one call apart instead.
    pub fn render(&mut self, cursor_on: bool) -> Result<bool, RenderError> {
        self.pump();
        if !self.dirty && self.last_cursor == Some(cursor_on) {
            return Ok(false);
        }

        // Refreshing copies the screen into the data, and the table into the
        // data's settings; the engine reads the renderer's, so both get it —
        // which is what the shells do (ADR 040).
        self.renderer
            .data_mut()
            .refresh(&mut self.control, &self.view, cursor_on, None);
        if let Ok(table) = self.control.color_table().lock() {
            self.renderer.settings_mut().set_color_table(table.clone());
        }

        // The atlas paints only its dirty area; a host that presents the whole
        // buffer every frame invalidates first (ADR 006).
        self.renderer.invalidate_all();
        self.renderer.paint_frame()?;

        self.dirty = false;
        self.last_cursor = Some(cursor_on);
        Ok(true)
    }

    /// The last painted frame's pixels, row-major `0xAARRGGBB`.
    pub fn pixels(&self) -> Ref<'_, [u32]> {
        self.shared.pixels()
    }

    /// The last painted frame's size in pixels.
    pub fn frame_size(&self) -> (u32, u32) {
        (self.shared.width(), self.shared.height())
    }

    /// One cell's size in pixels, which a host needs to turn a pane's pixel size
    /// back into a grid.
    pub fn cell_size(&self) -> (u16, u16) {
        self.cell
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
    ///
    /// Taking the table marks the surface for a repaint: nothing tells it when
    /// the host writes to it.
    pub fn color_table(&mut self) -> Arc<Mutex<ColorTable>> {
        self.dirty = true;
        self.control.color_table()
    }

    /// Whether the cursor is drawn in the pane at all.
    ///
    /// A host uses this to pass a constant cursor phase while it is hidden, so
    /// blinking does not ask for frames nobody can see.
    pub fn cursor_visible(&self) -> bool {
        self.renderer.data().cursor_visible()
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
    use std::time::Duration;

    use renderer::{CellShaping, CellSizeInDIP, FontId, GlyphBitmap};
    use utils::Size;

    use crate::term_connection::Queue;

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

    fn surface(columns: i32, rows: i32) -> (Surface, Queue) {
        let queue = Queue::default();
        let surface = Surface::new(
            Setup {
                font: font(),
                // A grid this small always fits a `u16`, so the test can cast.
                settings: settings(columns as u16, rows as u16),
                shaper: Box::new(CellShaping),
                rasterizer: Box::new(SolidRasterizer),
                columns,
                rows,
            },
            Box::new(queue.clone()),
        );
        (surface, queue)
    }

    #[test]
    fn a_frame_of_fed_bytes_reaches_the_pixels() {
        let (mut surface, queue) = surface(4, 2);
        // The text is on the second row and the cursor is hidden, so ink there
        // can only have come from the bytes the queue carried.
        queue.feed(b"\x1b[?25l\r\nhi");

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
        let (mut surface, queue) = surface(20, 2);

        surface.key(&Event::Text("a".to_string()));
        surface.paste("two words");

        assert_eq!(queue.take_written(), b"atwo words");
        // Taking it leaves nothing for the next forward.
        assert!(queue.take_written().is_empty());
    }

    #[test]
    fn a_resize_moves_the_terminal_the_grid_and_the_frame() {
        let (mut surface, _queue) = surface(4, 2);

        surface.resize(8, 3).expect("resize");
        surface.render(true).expect("a frame");

        assert_eq!(surface.size(), (8, 3));
        assert_eq!(surface.frame_size(), (64, 48));
    }

    #[test]
    fn a_host_can_set_the_palette_the_frame_paints() {
        use types::color_table::ColorAlias;
        use types::text_color::TextColor;

        let (mut surface, queue) = surface(2, 1);
        // The cursor is a filled box over the first cell, so the second cell's
        // background is what this reads; hiding it keeps the sample stable.
        queue.feed(b"\x1b[?25l");
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
        let (mut surface, queue) = surface(20, 2);
        queue.feed(b"\x1b]0;a title\x07");

        surface.render(true).expect("a frame");

        assert_eq!(surface.title(), Some("a title"));
    }

    #[test]
    fn the_programs_answer_is_written_and_a_bell_is_reported_once() {
        let now = Instant::now();
        let (mut surface, queue) = surface(20, 2);
        // DA1: the terminal answers this itself, with no input from the host.
        queue.feed(b"\x1b[c\x07");

        surface.render(true).expect("a frame");

        let written = queue.take_written();
        assert!(written.starts_with(b"\x1b["), "{written:?}");
        assert!(written.ends_with(b"c"), "{written:?}");
        assert_eq!(surface.take_events(now), [TerminalEvent::Bell]);
        assert!(surface.take_events(now).is_empty());
    }

    #[test]
    fn a_drag_selects_the_cells_between_the_press_and_the_pointer() {
        let (mut surface, queue) = surface(20, 2);
        queue.feed(b"hello world");
        surface.pump();
        let now = Instant::now();

        surface.press((1, 0), now);
        surface.drag((4, 0));

        assert_eq!(surface.selection_text().as_deref(), Some("ello"));
    }

    #[test]
    fn a_double_click_selects_the_word_under_the_cell() {
        let (mut surface, queue) = surface(20, 2);
        queue.feed(b"hello world");
        surface.pump();
        let now = Instant::now();

        surface.press((7, 0), now);
        let clicks = surface.press((7, 0), now + Duration::from_millis(1));

        assert_eq!(clicks, 2);
        assert_eq!(surface.selection_text().as_deref(), Some("world"));
    }

    #[test]
    fn a_triple_click_selects_the_row() {
        let (mut surface, queue) = surface(20, 2);
        queue.feed(b"hello world");
        surface.pump();
        let now = Instant::now();

        surface.press((7, 0), now);
        surface.press((7, 0), now);
        assert_eq!(surface.press((7, 0), now), 3);

        let text = surface.selection_text().expect("a selection");
        assert!(text.starts_with("hello world"), "{text:?}");
    }

    #[test]
    fn clearing_the_selection_leaves_nothing_to_copy() {
        let (mut surface, queue) = surface(20, 2);
        queue.feed(b"hello");
        surface.pump();

        surface.press((1, 0), Instant::now());
        assert!(surface.selection_text().is_some());

        surface.clear_selection();
        assert!(surface.selection_text().is_none());
    }

    #[test]
    fn a_second_render_with_nothing_new_paints_nothing() {
        let (mut surface, queue) = surface(4, 2);

        assert!(surface.render(true).expect("a frame"));
        assert!(!surface.render(true).expect("nothing changed"));

        queue.feed(b"hi");
        assert!(surface.render(true).expect("new output is a frame"));
    }

    #[test]
    fn a_cursor_phase_change_paints_again() {
        let (mut surface, _queue) = surface(4, 2);

        assert!(surface.render(true).expect("a frame"));
        assert!(surface.render(false).expect("a phase change is a frame"));
        assert!(!surface.render(false).expect("nothing changed"));
    }

    #[test]
    fn a_selection_repaints_the_frame() {
        let (mut surface, queue) = surface(20, 2);
        queue.feed(b"hello");

        assert!(surface.render(true).expect("a frame"));
        assert!(!surface.render(true).expect("nothing changed"));

        surface.press((1, 0), Instant::now());
        assert!(surface.render(true).expect("a selection is a frame"));
    }

    #[test]
    fn the_cursor_is_reported_as_drawn_only_when_it_is_visible() {
        let (mut visible, _queue) = surface(4, 2);
        assert!(visible.render(true).expect("a frame"));
        assert!(visible.cursor_visible());

        let (mut hidden, queue) = surface(4, 2);
        queue.feed(b"\x1b[?25l");
        assert!(hidden.render(true).expect("a frame"));
        assert!(!hidden.cursor_visible());
    }
}
