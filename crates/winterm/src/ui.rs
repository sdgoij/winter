//! The egui shell — the only module that knows a GUI toolkit exists.
//!
//! ADR 018 records the boundary: this module turns egui events into `term`
//! vocabulary and `grid` snapshots into paint calls. Replacing egui with a
//! native backend means replacing this file, not the session or the grid.

use std::io;
use std::path::{Path, PathBuf};
use std::time::Instant;

use eframe::egui;
use server::pty::{PtyCommand, PtySize};
use settings::Settings;
use term::Terminal;
use term::input::{
    Key, KeyEvent, Modifiers as KeyModifiers, MouseButton, MouseButtons, MouseEvent, MouseEventKind, WHEEL_DELTA,
    WheelAxis,
};
use types::CursorType;
use types::TextAttribute;
use types::UnderlineStyle;
use types::color_table::{CURSOR_COLOR, ColorAlias, ColorTable, SELECTION_BACKGROUND as SELECTION_SLOT};
use types::text_color::{ColorType, TextColor};

use crate::layout::{self, Axis, Direction, Fraction, Layout, PaneId};
use crate::{blink::Blink, grid, session::Session, tabs::Tabs, view::View};

/// The colour used when the palette has no default background.
const FALLBACK_BACKGROUND: egui::Color32 = egui::Color32::from_rgb(12, 12, 12);
/// The colour used when the palette has no default foreground.
const FALLBACK_FOREGROUND: egui::Color32 = egui::Color32::from_rgb(204, 204, 204);
/// The fill behind selected cells when the table names no selection colour.
const FALLBACK_SELECTION: egui::Color32 = egui::Color32::from_rgb(38, 79, 120);
/// The line between panes.
const DIVIDER: egui::Color32 = egui::Color32::from_rgb(60, 60, 60);
/// How close to a divider a press has to be, in pixels, to grab it.
const DIVIDER_GRAB: f32 = 4.0;
/// The line around the focused pane.
const FOCUS_BORDER: egui::Color32 = egui::Color32::from_rgb(90, 140, 200);
/// The egui family the host's regular monospace face is registered under.
const REGULAR_FAMILY: &str = "winterm-mono";
/// The egui family the host's italic monospace face is registered under.
const ITALIC_FAMILY: &str = "winterm-mono-italic";

/// The faces the window could register from the host's fonts.
///
/// With no host family the shell keeps egui's embedded monospace face, which is
/// upright only: italic text then draws upright rather than in a second font
/// whose cell width would not match the grid (ADR 029).
#[derive(Debug, Clone, Copy, Default, PartialEq, Eq)]
struct Faces {
    regular: bool,
    italic: bool,
}

impl Faces {
    /// The font for a style, falling back to what is available.
    fn font(&self, size: f32, italic: bool) -> egui::FontId {
        let family = if italic && self.italic {
            Some(ITALIC_FAMILY)
        } else if self.regular {
            Some(REGULAR_FAMILY)
        } else {
            None
        };

        match family {
            Some(name) => egui::FontId::new(size, egui::FontFamily::Name(name.into())),
            None => egui::FontId::monospace(size),
        }
    }
}

/// Registers the host's monospace faces with egui, when there are any.
///
/// One family is chosen from the renderer's preference list and every style
/// comes from it, so the cell metrics stay consistent; a style the host does
/// not have keeps the family's upright face. Failure is silent on purpose: a
/// machine with no discoverable fonts runs on the embedded face.
fn register_fonts(ctx: &egui::Context) -> Faces {
    let directories = font_directories();
    if directories.is_empty() {
        return Faces::default();
    }

    let paths: Vec<&Path> = directories.iter().map(PathBuf::as_path).collect();
    let collection = renderer::FontCollection::from_directories(paths);

    // A family that has an italic is preferred over one that does not: the
    // first choice on a machine can be installed upright-only.
    let chosen = collection
        .default_monospace_with_italic(renderer::PREFERRED_MONOSPACE_FAMILIES)
        .map(|(regular, italic)| (regular, Some(italic)))
        .or_else(|| {
            collection
                .default_monospace(renderer::PREFERRED_MONOSPACE_FAMILIES)
                .map(|regular| (regular, None))
        });
    let Some((regular, italic)) = chosen else {
        return Faces::default();
    };
    let Some(regular_data) = font_data(regular) else {
        return Faces::default();
    };

    let mut definitions = egui::FontDefinitions::default();
    let fallbacks = definitions
        .families
        .get(&egui::FontFamily::Monospace)
        .cloned()
        .unwrap_or_default();

    definitions
        .font_data
        .insert(REGULAR_FAMILY.to_string(), std::sync::Arc::new(regular_data));
    let mut regular_chain = vec![REGULAR_FAMILY.to_string()];
    regular_chain.extend(fallbacks.iter().cloned());
    definitions
        .families
        .insert(egui::FontFamily::Name(REGULAR_FAMILY.into()), regular_chain);

    let mut faces = Faces {
        regular: true,
        italic: false,
    };

    if let Some(italic) = italic
        && let Some(italic_data) = font_data(italic)
    {
        definitions
            .font_data
            .insert(ITALIC_FAMILY.to_string(), std::sync::Arc::new(italic_data));
        let mut italic_chain = vec![ITALIC_FAMILY.to_string(), REGULAR_FAMILY.to_string()];
        italic_chain.extend(fallbacks);
        definitions
            .families
            .insert(egui::FontFamily::Name(ITALIC_FAMILY.into()), italic_chain);
        faces.italic = true;
    }

    ctx.set_fonts(definitions);
    faces
}

/// Reads a face's file and its index inside it.
fn font_data(face: &renderer::FontFace) -> Option<egui::FontData> {
    let bytes = std::fs::read(face.path()).ok()?;
    let mut data = egui::FontData::from_owned(bytes);
    data.index = face.index();
    Some(data)
}

/// Returns the directories this platform keeps fonts in, keeping the ones that
/// exist.
fn font_directories() -> Vec<PathBuf> {
    let mut directories = Vec::new();

    #[cfg(windows)]
    {
        if let Some(windir) = std::env::var_os("WINDIR") {
            directories.push(PathBuf::from(windir).join("Fonts"));
        }
        if let Some(local) = std::env::var_os("LOCALAPPDATA") {
            directories.push(PathBuf::from(local).join("Microsoft").join("Windows").join("Fonts"));
        }
    }
    #[cfg(target_os = "macos")]
    {
        directories.push(PathBuf::from("/System/Library/Fonts"));
        directories.push(PathBuf::from("/Library/Fonts"));
        if let Some(home) = std::env::var_os("HOME") {
            directories.push(PathBuf::from(home).join("Library").join("Fonts"));
        }
    }
    #[cfg(all(unix, not(target_os = "macos")))]
    {
        directories.push(PathBuf::from("/usr/share/fonts"));
        directories.push(PathBuf::from("/usr/local/share/fonts"));
        if let Some(home) = std::env::var_os("HOME") {
            let home = PathBuf::from(home);
            directories.push(home.join(".local").join("share").join("fonts"));
            directories.push(home.join(".fonts"));
        }
    }

    directories.retain(|directory| directory.is_dir());
    directories
}

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

    eframe::run_native(
        "winterm",
        options,
        Box::new(|cc| {
            let faces = register_fonts(&cc.egui_ctx);
            Ok(Box::new(App::new(&cc.egui_ctx, faces)?))
        }),
    )
}

/// One pane: a shell, its parser and its viewport.
struct Pane {
    id: PaneId,
    terminal: Terminal,
    session: Session,
    view: View,
    /// The title the shell set with OSC 0/2.
    osc_title: Option<String>,
    /// The grid size the terminal and pty were last told about.
    size: (i32, i32),
    /// The cell the pointer was last forwarded from, for motion deduplication.
    last_mouse_cell: Option<(i32, i32)>,
    /// The output stream ended.
    exited: bool,
}

impl Pane {
    fn new(
        ctx: &egui::Context,
        id: PaneId,
        command: PtyCommand,
        scheme: Option<&settings::Scheme>,
    ) -> io::Result<Self> {
        let size = PtySize::DEFAULT;
        let repaint = ctx.clone();
        let session = Session::spawn_with(&command, size, move || repaint.request_repaint())?;
        let mut terminal = Terminal::new();
        terminal.resize(i32::from(size.cols), i32::from(size.rows));

        let mut pane = Self {
            id,
            terminal,
            session,
            view: View::new(),
            osc_title: None,
            size: (i32::from(size.cols), i32::from(size.rows)),
            last_mouse_cell: None,
            exited: false,
        };
        pane.apply_scheme(scheme);

        Ok(pane)
    }

    /// Makes the pane's palette the built-in table plus `scheme`.
    ///
    /// At creation the scheme fills the table before the shell can write to it,
    /// so a program's OSC 10/11 arrives later and wins. On a reload the file is
    /// the later word, so the table is rebuilt and a program's OSC colours go.
    fn apply_scheme(&mut self, scheme: Option<&settings::Scheme>) {
        let handle = self.terminal.dispatch().color_table();
        let Ok(mut table) = handle.lock() else {
            return;
        };
        match scheme {
            Some(scheme) => scheme.apply(&mut table),
            None => table.reset(),
        }
    }

    /// Takes the bell count so the window can ask for attention once.
    fn take_bells(&mut self) -> usize {
        self.terminal.take_bell_requests()
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

        if let Some(title) = self.terminal.window_title()
            && self.osc_title.as_deref() != Some(title)
        {
            self.osc_title = Some(title.to_string());
        }
    }

    fn handle_events(&mut self, events: Vec<egui::Event>, sides: &mut ModifierSides) {
        for event in events {
            // Typing returns to the live screen, as upstream's TrySnapOnInput.
            if self.view.scroll() != 0
                && matches!(&event, egui::Event::Text(_) | egui::Event::Key { pressed: true, .. })
            {
                self.view.scroll_to_bottom();
            }

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
                    self.paste_text(&text);
                }
                egui::Event::Key {
                    key,
                    pressed,
                    repeat,
                    modifiers,
                    ..
                } => {
                    track_modifier(sides, key, pressed);

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
                        modifiers: key_modifiers(modifiers, sides.right_alt, sides.right_ctrl),
                        pressed,
                        repeat,
                        ..KeyEvent::down(key)
                    });
                }
                _ => {}
            }
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
    fn handle_pointer(&mut self, ctx: &egui::Context, area: egui::Rect, cell: egui::Vec2, sides: ModifierSides) {
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

        let Some(position) = input.hover else {
            self.last_mouse_cell = None;
            return;
        };

        // A selection drag that leaves the pane keeps extending: the pointer
        // claims the edge row and the view scrolls one row towards it, so the
        // selection reaches into the history or back to the live screen.
        let outside = if position.y < area.min.y {
            -1
        } else if position.y > area.max.y {
            1
        } else {
            0
        };
        if outside != 0 && input.primary_down && self.view.selection().is_some() {
            let max = self.max_scroll();
            self.view.drag_scroll(outside, max, Instant::now());

            let rows = self.size.1.max(1);
            let row = if outside < 0 { 0 } else { rows - 1 };
            let column = (((position.x - area.min.x) / cell.x).floor() as i32).clamp(0, self.size.0.max(1) - 1);
            let document = screen_to_document(max, self.view.scroll(), row);
            self.view.extend((column, document));

            self.last_mouse_cell = None;
            ctx.request_repaint_after(std::time::Duration::from_millis(25));
            return;
        }

        if !area.contains(position) {
            self.last_mouse_cell = None;
            return;
        }
        let Some(cell_position) = self.cell_at(area, cell, position) else {
            return;
        };

        // Selection is in document coordinates: history first, then the
        // screen. The pointer is clamped to the grid, so the row is in range.
        let history = self.max_scroll();
        let document_row = screen_to_document(history, self.view.scroll(), cell_position.1);
        let document_position = (cell_position.0, document_row);

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
            let modifiers = key_modifiers(input.modifiers, sides.right_alt, sides.right_ctrl);

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
                let count = self.view.click(document_position, Instant::now());

                if count >= 3 {
                    let width = self.screen_width();
                    self.view.select_line(document_row, width);
                } else if count == 2 {
                    let text = self.row_text(document_row);
                    self.view.select_word(&text, document_position);
                } else {
                    self.view.select(document_position, input.modifiers.alt);
                }
            } else if input.primary_down && self.view.selection().is_some() {
                self.view.extend(document_position);
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

    fn row_text(&mut self, document_row: i32) -> String {
        let buffer = self.terminal.page_manager_mut().active_page().buffer();
        match buffer.read() {
            Ok(buffer) => buffer
                .get_row_by_document_line(document_row)
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

    /// Returns the selected cells, using the buffer's copy path.
    fn selection_text(&mut self) -> Option<String> {
        let buffer = self.terminal.page_manager_mut().active_page().buffer();
        let buffer = buffer.read().ok()?;
        let selection = self.view.selection()?;
        let text = selection.text(&buffer);
        (!text.is_empty()).then_some(text)
    }

    /// Pastes text as the terminal would, through the bracketed-paste filter.
    fn paste_text(&mut self, text: &str) {
        let bytes = self.terminal.paste(text);
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

    /// Adds the pane's grid size to the pane's own rect.
    fn fit(&mut self, rect: egui::Rect, cell: egui::Vec2) {
        let columns = ((rect.width() / cell.x).floor() as i32).max(1);
        let rows = ((rect.height() / cell.y).floor() as i32).max(1);
        if (columns, rows) != self.size {
            self.resize(columns, rows);
        }
    }

    fn paint(
        &mut self,
        ui: &egui::Ui,
        area: egui::Rect,
        cell: egui::Vec2,
        cursor_on: bool,
        font_size: f32,
        faces: Faces,
    ) {
        let screen = self.snapshot();
        let painter = ui.painter().with_clip_rect(area);

        if cell.x <= 0.0 || cell.y <= 0.0 {
            return;
        }

        let table = self.terminal.dispatch().color_table();
        let Ok(table) = table.lock() else {
            return;
        };
        let pane_background =
            resolve(TextColor::new(), &table, ColorAlias::DefaultBackground).unwrap_or(FALLBACK_BACKGROUND);
        painter.rect_filled(area, 0, pane_background);

        // Backgrounds first, so the selection can cover them and the text can
        // be drawn over both.
        for (index, row) in screen.rows.iter().enumerate() {
            let top = area.min.y + index as f32 * cell.y;
            for run in &row.runs {
                let (_, run_background) = attribute_colors(&run.attribute, &table);
                if run_background == pane_background {
                    continue;
                }
                let left = area.min.x + run.column as f32 * cell.x;
                let rect =
                    egui::Rect::from_min_size(egui::pos2(left, top), egui::vec2(cell.x * run.cells as f32, cell.y));
                painter.rect_filled(rect, 0, run_background);
            }
        }

        if let Some(selection) = self.view.selection() {
            let fill = table
                .get(SELECTION_SLOT)
                .and_then(egui_color)
                .unwrap_or(FALLBACK_SELECTION);
            for row in 0..screen.height {
                // The selection is in document coordinates; the snapshot says
                // which document row is painted on screen row zero.
                let Some((start, end)) = selection.row_span(screen.top + row) else {
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
                painter.rect_filled(rect, 0, fill);
            }
        }

        for (index, row) in screen.rows.iter().enumerate() {
            let top = area.min.y + index as f32 * cell.y;
            for run in &row.runs {
                if run.text.is_empty() {
                    continue;
                }

                let attribute = run.attribute;
                if attribute.is_invisible() {
                    continue;
                }

                let left = area.min.x + run.column as f32 * cell.x;
                let width = cell.x * run.cells as f32;
                let (foreground, _) = attribute_colors(&attribute, &table);

                painter.text(
                    egui::pos2(left, top),
                    egui::Align2::LEFT_TOP,
                    &run.text,
                    faces.font(font_size, attribute.is_italic()),
                    foreground,
                );

                // Decorations follow the text, in the text's colour unless the
                // underline asks for its own.
                let underline = if attribute.get_underline_color().is_default() {
                    foreground
                } else {
                    resolve(attribute.get_underline_color(), &table, ColorAlias::DefaultForeground)
                        .unwrap_or(foreground)
                };
                let line = |y: f32, color: egui::Color32| {
                    painter.line_segment(
                        [egui::pos2(left, y), egui::pos2(left + width, y)],
                        egui::Stroke::new(1.0, color),
                    );
                };

                if attribute.is_top_horizontal_displayed() {
                    line(top, foreground);
                }
                if attribute.is_crossed_out() {
                    line(top + cell.y * 0.5, foreground);
                }
                match attribute.get_underline_style() {
                    UnderlineStyle::NoUnderline => {}
                    UnderlineStyle::DoublyUnderlined => {
                        line(top + cell.y - 2.0, underline);
                        line(top + cell.y - 4.0, underline);
                    }
                    // Curly, dotted and dashed are drawn straight: the shape
                    // of the line is a font detail this shell does not stage.
                    _ => line(top + cell.y - 1.0, underline),
                }
            }
        }

        if screen.cursor_visible && cursor_on {
            let column = screen.cursor.0.clamp(0, screen.width.max(1) - 1);
            let row = screen.cursor.1.clamp(0, screen.height.max(1) - 1);
            let origin = egui::pos2(area.min.x + column as f32 * cell.x, area.min.y + row as f32 * cell.y);
            let color = table
                .get(CURSOR_COLOR)
                .and_then(egui_color)
                .map(|color| egui::Color32::from_rgba_unmultiplied(color.r(), color.g(), color.b(), 160))
                .unwrap_or_else(|| egui::Color32::from_rgba_unmultiplied(230, 230, 230, 160));
            paint_cursor(&painter, origin, cell, screen.cursor_shape, color);
        }
    }
}

/// A divider the pointer is dragging.
struct DividerDrag {
    path: layout::SplitPath,
    axis: Axis,
    /// The split's area when the drag began, for turning pixels into a ratio.
    rect: egui::Rect,
}

/// One tab: a pane tree, its focused pane and a title.
struct Tab {
    panes: Vec<Pane>,
    layout: Layout,
    focused: PaneId,
    /// The divider under the pointer, while it is being dragged.
    divider: Option<DividerDrag>,
    /// The title to show until a pane's shell sets one.
    fallback_title: String,
    next_id: u64,
}

impl Tab {
    fn new(
        ctx: &egui::Context,
        command: PtyCommand,
        fallback_title: String,
        scheme: Option<&settings::Scheme>,
    ) -> io::Result<Self> {
        let id = PaneId(0);
        Ok(Self {
            panes: vec![Pane::new(ctx, id, command, scheme)?],
            layout: Layout::leaf(id),
            focused: id,
            divider: None,
            fallback_title,
            next_id: 1,
        })
    }

    fn focused_pane(&self) -> Option<&Pane> {
        self.panes.iter().find(|pane| pane.id == self.focused)
    }

    fn focused_pane_mut(&mut self) -> Option<&mut Pane> {
        let focused = self.focused;
        self.panes.iter_mut().find(|pane| pane.id == focused)
    }

    /// The title the tab strip shows: the focused pane's OSC title, else the
    /// shell's name.
    fn display_title(&self) -> &str {
        self.focused_pane()
            .and_then(|pane| pane.osc_title.as_deref())
            .unwrap_or(&self.fallback_title)
    }

    /// Splits the focused pane, spawning a shell for the new half.
    fn split(
        &mut self,
        axis: Axis,
        ctx: &egui::Context,
        command: PtyCommand,
        scheme: Option<&settings::Scheme>,
    ) -> bool {
        let id = PaneId(self.next_id);
        let Ok(pane) = Pane::new(ctx, id, command, scheme) else {
            return false;
        };

        if !self.layout.split(self.focused, axis, id) {
            return false;
        }

        self.next_id += 1;
        self.panes.push(pane);
        self.focused = id;
        true
    }

    /// Closes the focused pane. Returns whether a pane remains.
    fn close_focused(&mut self) -> bool {
        if self.layout.is_single() {
            return false;
        }
        if !self.layout.remove(self.focused) {
            return false;
        }

        let closed = self.focused;
        self.panes.retain(|pane| pane.id != closed);
        if let Some(id) = self.layout.leaves().first().copied() {
            self.focused = id;
        }
        true
    }

    /// Moves focus to the nearest pane in `direction`.
    fn focus(&mut self, direction: Direction) -> bool {
        let Some(next) = layout::neighbour(&self.layout.rects(), self.focused, direction) else {
            return false;
        };
        self.focused = next;
        true
    }

    /// Grows the focused pane in `direction`.
    fn resize(&mut self, direction: Direction) -> bool {
        self.layout.resize(self.focused, direction, 0.05)
    }

    fn pump_output(&mut self) {
        for pane in &mut self.panes {
            pane.pump_output();
        }
    }

    fn take_bells(&mut self) -> usize {
        self.panes.iter_mut().map(Pane::take_bells).sum()
    }

    fn handle_events(&mut self, events: Vec<egui::Event>, sides: &mut ModifierSides) {
        if let Some(pane) = self.focused_pane_mut() {
            pane.handle_events(events, sides);
        }
    }

    /// Routes the pointer to the pane under it, focusing that pane on a press.
    ///
    /// A divider under the pointer takes precedence: it is grabbed on a press,
    /// dragged while the button is down, and consumes the pointer so a resize
    /// never starts a selection in the pane beneath it.
    fn handle_pointer(&mut self, ctx: &egui::Context, area: egui::Rect, cell: egui::Vec2, sides: ModifierSides) {
        if self.divider.is_some() && !ctx.input(|input| input.pointer.primary_down()) {
            self.divider = None;
        }

        let Some(position) = ctx.input(|input| input.pointer.hover_pos()) else {
            return;
        };

        if let Some(drag) = &self.divider {
            let ratio = match drag.axis {
                Axis::Horizontal => (position.x - drag.rect.min.x) / drag.rect.width(),
                Axis::Vertical => (position.y - drag.rect.min.y) / drag.rect.height(),
            };
            self.layout.set_ratio(&drag.path, ratio);
            ctx.set_cursor_icon(divider_cursor(drag.axis));
            return;
        }

        if let Some((path, axis, rect)) = divider_at(&self.layout, position, area) {
            ctx.set_cursor_icon(divider_cursor(axis));
            if ctx.input(|input| input.pointer.primary_pressed()) {
                self.divider = Some(DividerDrag { path, axis, rect });
            }
            return;
        }

        let pressed = ctx.input(|input| input.pointer.primary_pressed());

        let hit = self.layout.rects().into_iter().find_map(|(id, fraction)| {
            let rect = fraction_rect(fraction, area);
            rect.contains(position).then_some((id, rect))
        });
        let Some((id, rect)) = hit else {
            return;
        };

        if pressed {
            self.focused = id;
        }

        if let Some(pane) = self.panes.iter_mut().find(|pane| pane.id == id) {
            pane.handle_pointer(ctx, rect, cell, sides);
        }
    }

    /// Rebuilds every pane's palette from the scheme.
    fn apply_scheme(&mut self, scheme: Option<&settings::Scheme>) {
        for pane in &mut self.panes {
            pane.apply_scheme(scheme);
        }
    }

    /// Sizes every pane's grid to the rectangle it occupies.
    fn fit_panes(&mut self, area: egui::Rect, cell: egui::Vec2) {
        for (id, fraction) in self.layout.rects() {
            let rect = fraction_rect(fraction, area);
            if let Some(pane) = self.panes.iter_mut().find(|pane| pane.id == id) {
                pane.fit(rect, cell);
            }
        }
    }

    fn focused_snapshot(&mut self) -> grid::Screen {
        self.focused_pane_mut().map(Pane::snapshot).unwrap_or_default()
    }

    fn paint(
        &mut self,
        ui: &egui::Ui,
        area: egui::Rect,
        cell: egui::Vec2,
        cursor_on: bool,
        font_size: f32,
        faces: Faces,
    ) {
        let focused = self.focused;
        let multiple = self.panes.len() > 1;

        for (id, fraction) in self.layout.rects() {
            let rect = fraction_rect(fraction, area);
            if let Some(pane) = self.panes.iter_mut().find(|pane| pane.id == id) {
                pane.paint(ui, rect, cell, cursor_on && id == focused, font_size, faces);
            }

            // With more than one pane the borders double as the focus cue.
            if multiple {
                let color = if id == focused { FOCUS_BORDER } else { DIVIDER };
                ui.painter()
                    .rect_stroke(rect, 0, egui::Stroke::new(1.0, color), egui::StrokeKind::Inside);
            }
        }
    }

    fn selection_text(&mut self) -> Option<String> {
        self.focused_pane_mut().and_then(Pane::selection_text)
    }

    fn paste_text(&mut self, text: &str) {
        if let Some(pane) = self.focused_pane_mut() {
            pane.paste_text(text);
        }
    }
}

/// Maps a pane's fractional rectangle onto the window.
fn fraction_rect(fraction: Fraction, area: egui::Rect) -> egui::Rect {
    egui::Rect::from_min_size(
        egui::pos2(
            area.min.x + fraction.x * area.width(),
            area.min.y + fraction.y * area.height(),
        ),
        egui::vec2(fraction.width * area.width(), fraction.height * area.height()),
    )
}

/// The split whose divider is under `position`, deepest first.
///
/// A divider is the edge two children share; a point within `DIVIDER_GRAB`
/// pixels of it, inside the split's own area, grabs that split. Walking
/// backwards prefers a nested divider over an ancestor's collinear one.
fn divider_at(
    layout: &Layout,
    position: egui::Pos2,
    area: egui::Rect,
) -> Option<(layout::SplitPath, Axis, egui::Rect)> {
    layout
        .splits()
        .into_iter()
        .rev()
        .find_map(|(path, axis, fraction, ratio)| {
            let rect = fraction_rect(fraction, area);
            let (distance, on_span) = match axis {
                Axis::Horizontal => (
                    (position.x - (rect.min.x + rect.width() * ratio)).abs(),
                    rect.min.y <= position.y && position.y <= rect.max.y,
                ),
                Axis::Vertical => (
                    (position.y - (rect.min.y + rect.height() * ratio)).abs(),
                    rect.min.x <= position.x && position.x <= rect.max.x,
                ),
            };

            (on_span && distance <= DIVIDER_GRAB).then_some((path, axis, rect))
        })
}

/// The pointer shape that says a divider can be dragged.
const fn divider_cursor(axis: Axis) -> egui::CursorIcon {
    match axis {
        Axis::Horizontal => egui::CursorIcon::ResizeHorizontal,
        Axis::Vertical => egui::CursorIcon::ResizeVertical,
    }
}

/// The colour of a conflict notice in the settings window.
const CONFLICT: egui::Color32 = egui::Color32::from_rgb(220, 170, 60);

/// The settings the window can edit, held apart from the settings in force.
///
/// Whether the draft is dirty is computed by comparing it with the settings in
/// force, so no widget has to remember to set a flag (ADRs 028, 030).
struct Draft {
    font_size: f32,
    default_profile: Option<String>,
    default_scheme: Option<String>,
    profiles: Vec<ProfileDraft>,
    keybindings: Vec<BindingDraft>,
}

/// One profile as the window edits it.
///
/// The arguments are text while they are being edited: splitting them on every
/// keystroke would make a space impossible to type. They are split on
/// whitespace when the draft is applied, and the file stays the place for an
/// argument that contains a space (ADR 030).
struct ProfileDraft {
    name: String,
    command: String,
    args: String,
    title: String,
}

impl ProfileDraft {
    fn from_profile(profile: &settings::Profile) -> Self {
        Self {
            name: profile.name.clone(),
            command: profile.command.clone(),
            args: profile.args.join(" "),
            title: profile.title.clone().unwrap_or_default(),
        }
    }

    fn to_profile(&self) -> settings::Profile {
        settings::Profile {
            name: self.name.clone(),
            command: self.command.clone(),
            args: self.args.split_whitespace().map(str::to_string).collect(),
            title: (!self.title.is_empty()).then(|| self.title.clone()),
        }
    }
}

/// One chord as the window edits it.
///
/// The chord stays text until the document is built, so a half-typed chord is
/// kept. A row whose text does not parse keeps the window from saving and is
/// named in the window (ADR 031).
struct BindingDraft {
    keys: String,
    action: settings::Action,
}

impl BindingDraft {
    fn from_binding(binding: &settings::Keybinding) -> Self {
        Self {
            keys: binding.keys.to_string(),
            action: binding.action,
        }
    }

    fn to_binding(&self) -> Option<settings::Keybinding> {
        Some(settings::Keybinding {
            keys: settings::Chord::parse(&self.keys).ok()?,
            action: self.action,
        })
    }
}

impl Draft {
    fn from_settings(settings: &Settings) -> Self {
        Self {
            font_size: settings.font.size,
            default_profile: settings.default_profile.clone(),
            default_scheme: settings.default_scheme.clone(),
            profiles: settings.profiles.iter().map(ProfileDraft::from_profile).collect(),
            keybindings: settings.keybindings.iter().map(BindingDraft::from_binding).collect(),
        }
    }

    /// Whether the draft has edits that a save would write.
    ///
    /// An invalid chord counts as an edit: the window must not look clean while
    /// a row cannot be saved.
    fn differs_from(&self, settings: &Settings) -> bool {
        self.document(settings).is_none_or(|document| document != *settings)
    }

    /// The document a save writes, or None when a chord does not parse.
    fn document(&self, settings: &Settings) -> Option<Settings> {
        let mut document = settings.clone();
        document.font.size = self.font_size;
        document.default_profile = self.default_profile.clone();
        document.default_scheme = self.default_scheme.clone();
        document.profiles = self.profiles.iter().map(ProfileDraft::to_profile).collect();
        document.keybindings = self
            .keybindings
            .iter()
            .map(BindingDraft::to_binding)
            .collect::<Option<Vec<_>>>()?;
        Some(document)
    }
}

/// The window: a tab strip and the active tab.
struct App {
    tabs: Tabs<Tab>,
    settings: Settings,
    /// The faces egui renders the grid with.
    faces: Faces,
    /// The scheme every pane starts with, if the file names one.
    scheme: Option<settings::Scheme>,
    /// The grid's point size, from the settings file.
    font_size: f32,
    /// The path the settings were read from, when the platform has one.
    settings_path: Option<PathBuf>,
    /// Why the file was not used, if it was not.
    settings_error: Option<String>,
    /// What normalization had to decide for the file, in the window's words.
    settings_warnings: Vec<String>,
    /// The editable copy the settings window shows, while it is open.
    draft: Option<Draft>,
    /// The file changed on disk while the draft had unsaved edits.
    settings_conflict: bool,
    /// Whether the settings window is showing.
    settings_open: bool,
    /// The settings file's stamp when it was last read, so a save is noticed.
    settings_stamp: Option<settings::Stamp>,
    /// The sides egui collapses but the encoder distinguishes.
    sides: ModifierSides,
    clipboard: Option<arboard::Clipboard>,
    blink: Blink,
    /// Whether the cursor is drawn this frame.
    cursor_on: bool,
    /// The title last sent to the window.
    window_title: Option<String>,
}

/// What the tab strip or a chord asked for.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
enum TabAction {
    Select(usize),
    Close(usize),
    CloseActive,
    New,
    Next,
    Previous,
    Copy,
    Paste,
    Split(Axis),
    Focus(Direction),
    Resize(Direction),
    ToggleSettings,
}

impl App {
    fn new(ctx: &egui::Context, faces: Faces) -> Result<Self, Box<dyn std::error::Error + Send + Sync>> {
        let loaded = settings::load_with_report();
        let settings_stamp = loaded.path.as_deref().and_then(settings::stamp);
        let settings_warnings = loaded.warnings.iter().map(ToString::to_string).collect();
        let settings = loaded.settings;
        let scheme = settings.scheme().cloned();
        let font_size = settings.font.size;
        let (command, title) = profile_command(settings.profile());
        let tab = Tab::new(ctx, command, title, scheme.as_ref())?;

        Ok(Self {
            tabs: Tabs::new(tab),
            settings,
            faces,
            scheme,
            font_size,
            settings_path: loaded.path,
            settings_error: loaded.error.map(|error| error.to_string()),
            settings_warnings,
            draft: None,
            settings_conflict: false,
            settings_open: false,
            settings_stamp,
            sides: ModifierSides::default(),
            clipboard: arboard::Clipboard::new().ok(),
            blink: Blink::new(Instant::now()),
            cursor_on: true,
            window_title: None,
        })
    }

    /// Opens another shell in a new tab.
    fn open_tab(&mut self, ctx: &egui::Context) {
        let (command, title) = profile_command(self.settings.profile());
        if let Ok(tab) = Tab::new(ctx, command, title, self.scheme.as_ref()) {
            self.tabs.add(tab);
        }
    }

    fn apply(&mut self, action: TabAction, ctx: &egui::Context) {
        match action {
            TabAction::Select(index) => {
                self.tabs.select(index);
            }
            TabAction::Close(index) => self.close_tab(index, ctx),
            TabAction::CloseActive => {
                let closes_tab = match self.tabs.active_mut() {
                    Some(tab) => !tab.close_focused(),
                    None => false,
                };
                if closes_tab {
                    self.close_tab(self.tabs.index(), ctx);
                }
            }
            TabAction::New => self.open_tab(ctx),
            TabAction::Next => self.tabs.step(1),
            TabAction::Previous => self.tabs.step(-1),
            TabAction::Split(axis) => {
                let (command, _) = profile_command(self.settings.profile());
                if let Some(tab) = self.tabs.active_mut() {
                    tab.split(axis, ctx, command, self.scheme.as_ref());
                }
            }
            TabAction::Focus(direction) => {
                if let Some(tab) = self.tabs.active_mut() {
                    tab.focus(direction);
                }
            }
            TabAction::Resize(direction) => {
                if let Some(tab) = self.tabs.active_mut() {
                    tab.resize(direction);
                }
            }
            TabAction::Copy => {
                if let Some(text) = self.tabs.active_mut().and_then(Tab::selection_text)
                    && let Some(clipboard) = self.clipboard.as_mut()
                {
                    let _ = clipboard.set_text(text);
                }
            }
            TabAction::Paste => {
                if let Some(clipboard) = self.clipboard.as_mut()
                    && let Ok(text) = clipboard.get_text()
                    && let Some(tab) = self.tabs.active_mut()
                {
                    tab.paste_text(&text);
                }
            }
            TabAction::ToggleSettings => self.toggle_settings(),
        }
    }

    /// Shows or hides the settings window, rebuilding the draft when it opens.
    ///
    /// Closing discards unsaved edits, which is what the window says while the
    /// draft is dirty.
    fn toggle_settings(&mut self) {
        self.settings_open = !self.settings_open;
        self.settings_conflict = false;
        self.draft = if self.settings_open {
            Some(Draft::from_settings(&self.settings))
        } else {
            None
        };
    }

    fn close_tab(&mut self, index: usize, ctx: &egui::Context) {
        if !self.tabs.close(index) {
            ctx.send_viewport_cmd(egui::ViewportCommand::Close);
        }
    }

    /// Draws what the settings file said, whether it was used, and the fields
    /// the window can change.
    fn settings_window(&mut self, ctx: &egui::Context) {
        let mut open = self.settings_open;
        let mut save = false;
        let mut revert = false;
        let rows = settings_rows(
            &self.settings,
            self.settings_path.as_deref(),
            self.settings_error.as_deref(),
        );

        egui::Window::new("Settings")
            .open(&mut open)
            .default_width(520.0)
            .resizable(true)
            .show(ctx, |ui| {
                egui::Grid::new("winterm-settings-rows")
                    .num_columns(2)
                    .spacing([16.0, 4.0])
                    .show(ui, |ui| {
                        for (label, value) in &rows {
                            ui.strong(*label);
                            ui.label(value);
                            ui.end_row();
                        }
                    });

                ui.separator();
                if let Some(draft) = self.draft.as_mut() {
                    draft_editor(
                        ui,
                        draft,
                        &self.settings,
                        self.settings_conflict,
                        &mut save,
                        &mut revert,
                    );
                }

                ui.separator();
                ui.label("The file is also reloaded when another program saves it.");

                if !self.settings_warnings.is_empty() {
                    ui.separator();
                    ui.strong("Adjusted on load");
                    for warning in &self.settings_warnings {
                        ui.label(warning);
                    }
                }
            });

        self.settings_open = open;
        if !self.settings_open {
            self.draft = None;
            self.settings_conflict = false;
        }

        if save {
            self.save_draft();
        }
        if revert {
            self.reload_settings();
        }
    }

    /// Writes the draft and applies what was written.
    ///
    /// The document is read back through the same path as startup, so what runs
    /// is always the file's own settings, normalized.
    fn save_draft(&mut self) {
        let Some(draft) = &self.draft else {
            return;
        };
        let Some(document) = draft.document(&self.settings) else {
            return;
        };

        match settings::save(&document) {
            Ok(()) => {
                self.settings_conflict = false;
                self.settings_error = None;
                self.reload_settings();
                if self.settings_open {
                    self.draft = Some(Draft::from_settings(&self.settings));
                }
            }
            Err(error) => self.settings_error = Some(error.to_string()),
        }
    }

    /// Draws the tab strip and returns what the user clicked.
    fn tab_bar(&self, ui: &mut egui::Ui) -> Vec<TabAction> {
        let mut actions = Vec::new();
        let active = self.tabs.index();

        egui::Panel::top("winterm-tabs").show(ui, |ui| {
            ui.horizontal(|ui| {
                for index in 0..self.tabs.len() {
                    let Some(tab) = self.tabs.get(index) else {
                        continue;
                    };
                    if ui.selectable_label(index == active, tab.display_title()).clicked() {
                        actions.push(TabAction::Select(index));
                    }
                    if ui.small_button("×").clicked() {
                        actions.push(TabAction::Close(index));
                    }
                }

                if ui.button("+").clicked() {
                    actions.push(TabAction::New);
                }

                if ui.button("⚙").on_hover_text("Settings").clicked() {
                    actions.push(TabAction::ToggleSettings);
                }
            });
        });

        actions
    }

    /// Re-reads the settings file and applies what changed.
    ///
    /// Chords and the font size take effect at once, a scheme repaints every
    /// pane, and a changed profile affects the next tab: a running shell keeps
    /// the command it started with.
    fn reload_settings(&mut self) {
        let loaded = settings::load_with_report();
        self.settings_stamp = loaded.path.as_deref().and_then(settings::stamp);
        if let Some(error) = loaded.error {
            // A half-written save must not wipe a running session: the previous
            // settings stay in force and the window reports the error.
            self.settings_error = Some(error.to_string());
            return;
        }

        self.settings = loaded.settings;
        self.settings_path = loaded.path;
        self.settings_error = None;
        self.settings_warnings = loaded.warnings.iter().map(ToString::to_string).collect();
        self.font_size = self.settings.font.size;
        self.scheme = self.settings.scheme().cloned();

        let scheme = self.scheme.clone();
        for index in 0..self.tabs.len() {
            if let Some(tab) = self.tabs.get_mut(index) {
                tab.apply_scheme(scheme.as_ref());
            }
        }

        if self.settings_open {
            self.draft = Some(Draft::from_settings(&self.settings));
        }
    }
}

impl eframe::App for App {
    fn logic(&mut self, ctx: &egui::Context, _frame: &mut eframe::Frame) {
        // Every tab keeps draining: an inactive shell must not block on a full
        // pty buffer while its tab is in the background.
        for index in 0..self.tabs.len() {
            if let Some(tab) = self.tabs.get_mut(index) {
                tab.pump_output();
            }
        }

        if let Some(tab) = self.tabs.active() {
            let title = tab.display_title().to_string();
            if self.window_title.as_deref() != Some(title.as_str()) {
                self.window_title = Some(title.clone());
                ctx.send_viewport_cmd(egui::ViewportCommand::Title(title));
            }
        }

        let mut bells = 0;
        for index in 0..self.tabs.len() {
            if let Some(tab) = self.tabs.get_mut(index) {
                bells += tab.take_bells();
            }
        }
        if bells > 0 {
            ctx.send_viewport_cmd(egui::ViewportCommand::RequestUserAttention(
                egui::UserAttentionType::Informational,
            ));
        }

        // Poll the settings file: a changed stamp means a save, and the reload
        // applies the new chords, font, scheme and profile. The poll is the
        // watcher (ADR 025), so the window wakes once a second while a settings
        // path exists.
        if let Some(path) = &self.settings_path {
            let stamp = settings::stamp(path);
            if stamp != self.settings_stamp {
                let dirty = self
                    .draft
                    .as_ref()
                    .is_some_and(|draft| draft.differs_from(&self.settings));
                if dirty {
                    // Loading would discard the edit in progress; the window
                    // asks the user to choose between saving and reverting.
                    self.settings_conflict = true;
                } else {
                    self.reload_settings();
                }
            }
            ctx.request_repaint_after(std::time::Duration::from_secs(1));
        }
    }

    fn ui(&mut self, ui: &mut egui::Ui, _frame: &mut eframe::Frame) {
        let ctx = ui.ctx().clone();

        for action in self.tab_bar(ui) {
            self.apply(action, &ctx);
        }

        // Tab chords belong to the window; everything else goes to the shell.
        let events = ctx.input(|input| input.events.clone());
        let mut forwarded = Vec::with_capacity(events.len());
        for event in events {
            if let Some(action) = command_for(&self.settings.keybindings, &event) {
                self.apply(action, &ctx);
            } else {
                forwarded.push(event);
            }
        }

        if self.tabs.is_empty() {
            return;
        }

        let focused = ctx.input(|input| input.focused);
        let font = self.faces.font(self.font_size, false);
        let cell = ctx.fonts_mut(|fonts| egui::vec2(fonts.glyph_width(&font, 'M'), fonts.row_height(&font)));

        let area = ui.available_rect_before_wrap();

        let sides = self.sides;
        let Some(tab) = self.tabs.active_mut() else {
            return;
        };

        tab.fit_panes(area, cell);
        // An egui area — the settings window or the tab strip — takes the
        // pointer while it is over it, so a click never selects text behind it.
        if !ctx.is_pointer_over_egui() {
            tab.handle_pointer(&ctx, area, cell, sides);
            // A drag moved a divider; size the panes to the new rectangles now
            // rather than a frame later.
            tab.fit_panes(area, cell);
        }
        tab.handle_events(forwarded, &mut self.sides);

        let screen = tab.focused_snapshot();
        let now = Instant::now();
        self.cursor_on = self.blink.update(screen.cursor_blinking, focused, now);
        if let Some(delay) = self.blink.next_toggle(screen.cursor_blinking, focused, now) {
            ctx.request_repaint_after(delay);
        }
        tab.paint(ui, area, cell, self.cursor_on, self.font_size, self.faces);

        if self.settings_open {
            self.settings_window(&ctx);
        }
    }
}

/// The modifier sides egui collapses but the encoder distinguishes.
#[derive(Debug, Clone, Copy, Default)]
struct ModifierSides {
    right_alt: bool,
    right_ctrl: bool,
}

/// Records which side of a modifier key an event touched.
fn track_modifier(sides: &mut ModifierSides, key: egui::Key, pressed: bool) {
    match key {
        egui::Key::AltRight => sides.right_alt = pressed,
        egui::Key::ControlRight => sides.right_ctrl = pressed,
        _ => {}
    }
}

/// The window command an event carries, according to the settings' bindings.
///
/// A bound chord never reaches the shell: the window consumes it before the tab
/// sees the event. A chord the file unbound is no longer in the list, so the key
/// falls through to the shell, as it does when nothing binds it.
fn command_for(bindings: &[settings::Keybinding], event: &egui::Event) -> Option<TabAction> {
    let egui::Event::Key {
        key,
        pressed: true,
        modifiers,
        ..
    } = event
    else {
        return None;
    };

    let key = chord_key(*key)?;
    let modifiers = settings::Modifiers {
        ctrl: modifiers.ctrl,
        shift: modifiers.shift,
        alt: modifiers.alt,
    };
    let binding = bindings.iter().find(|binding| binding.keys.matches(key, modifiers))?;

    action_for(binding.action)
}

/// The window action a settings action names.
fn action_for(action: settings::Action) -> Option<TabAction> {
    Some(match action {
        settings::Action::NewTab => TabAction::New,
        settings::Action::CloseTab => TabAction::CloseActive,
        settings::Action::NextTab => TabAction::Next,
        settings::Action::PrevTab => TabAction::Previous,
        settings::Action::SplitHorizontal => TabAction::Split(Axis::Horizontal),
        settings::Action::SplitVertical => TabAction::Split(Axis::Vertical),
        settings::Action::FocusLeft => TabAction::Focus(Direction::Left),
        settings::Action::FocusRight => TabAction::Focus(Direction::Right),
        settings::Action::FocusUp => TabAction::Focus(Direction::Up),
        settings::Action::FocusDown => TabAction::Focus(Direction::Down),
        settings::Action::ResizeLeft => TabAction::Resize(Direction::Left),
        settings::Action::ResizeRight => TabAction::Resize(Direction::Right),
        settings::Action::ResizeUp => TabAction::Resize(Direction::Up),
        settings::Action::ResizeDown => TabAction::Resize(Direction::Down),
        settings::Action::Copy => TabAction::Copy,
        settings::Action::Paste => TabAction::Paste,
        settings::Action::OpenSettings => TabAction::ToggleSettings,
        // `unbound` is resolved away when the file loads.
        settings::Action::Unbound => return None,
    })
}

/// Maps an egui key to the key a chord names.
///
/// This is the vocabulary `settings::Key` documents, not the encoder's: a chord
/// key is what a key event reports, so punctuation is the character the key
/// produces on the user's layout.
fn chord_key(key: egui::Key) -> Option<settings::Key> {
    use egui::Key as E;
    use settings::Key as C;

    Some(match key {
        E::A => C::Char('a'),
        E::B => C::Char('b'),
        E::C => C::Char('c'),
        E::D => C::Char('d'),
        E::E => C::Char('e'),
        E::F => C::Char('f'),
        E::G => C::Char('g'),
        E::H => C::Char('h'),
        E::I => C::Char('i'),
        E::J => C::Char('j'),
        E::K => C::Char('k'),
        E::L => C::Char('l'),
        E::M => C::Char('m'),
        E::N => C::Char('n'),
        E::O => C::Char('o'),
        E::P => C::Char('p'),
        E::Q => C::Char('q'),
        E::R => C::Char('r'),
        E::S => C::Char('s'),
        E::T => C::Char('t'),
        E::U => C::Char('u'),
        E::V => C::Char('v'),
        E::W => C::Char('w'),
        E::X => C::Char('x'),
        E::Y => C::Char('y'),
        E::Z => C::Char('z'),
        E::Num0 => C::Char('0'),
        E::Num1 => C::Char('1'),
        E::Num2 => C::Char('2'),
        E::Num3 => C::Char('3'),
        E::Num4 => C::Char('4'),
        E::Num5 => C::Char('5'),
        E::Num6 => C::Char('6'),
        E::Num7 => C::Char('7'),
        E::Num8 => C::Char('8'),
        E::Num9 => C::Char('9'),
        E::Colon => C::Char(':'),
        E::Comma => C::Char(','),
        E::Backslash => C::Char('\\'),
        E::Slash => C::Char('/'),
        E::Pipe => C::Char('|'),
        E::Questionmark => C::Char('?'),
        E::Exclamationmark => C::Char('!'),
        E::OpenBracket => C::Char('['),
        E::CloseBracket => C::Char(']'),
        E::OpenCurlyBracket => C::Char('{'),
        E::CloseCurlyBracket => C::Char('}'),
        E::Backtick => C::Char('`'),
        E::Minus => C::Char('-'),
        E::Period => C::Char('.'),
        E::Plus => C::Char('+'),
        E::Equals => C::Char('='),
        E::Semicolon => C::Char(';'),
        E::Quote => C::Char('\''),
        E::Tab => C::Tab,
        E::Enter => C::Enter,
        E::Escape => C::Escape,
        E::Space => C::Space,
        E::Backspace => C::Backspace,
        E::Insert => C::Insert,
        E::Delete => C::Delete,
        E::Home => C::Home,
        E::End => C::End,
        E::PageUp => C::PageUp,
        E::PageDown => C::PageDown,
        E::ArrowLeft => C::Left,
        E::ArrowRight => C::Right,
        E::ArrowUp => C::Up,
        E::ArrowDown => C::Down,
        E::F1 => C::Function(1),
        E::F2 => C::Function(2),
        E::F3 => C::Function(3),
        E::F4 => C::Function(4),
        E::F5 => C::Function(5),
        E::F6 => C::Function(6),
        E::F7 => C::Function(7),
        E::F8 => C::Function(8),
        E::F9 => C::Function(9),
        E::F10 => C::Function(10),
        E::F11 => C::Function(11),
        E::F12 => C::Function(12),
        E::F13 => C::Function(13),
        E::F14 => C::Function(14),
        E::F15 => C::Function(15),
        E::F16 => C::Function(16),
        E::F17 => C::Function(17),
        E::F18 => C::Function(18),
        E::F19 => C::Function(19),
        E::F20 => C::Function(20),
        E::F21 => C::Function(21),
        E::F22 => C::Function(22),
        E::F23 => C::Function(23),
        E::F24 => C::Function(24),
        _ => return None,
    })
}

/// The name a tab shows until its shell sets a title.
fn command_title(command: &PtyCommand) -> String {
    Path::new(command.program())
        .file_stem()
        .and_then(std::ffi::OsStr::to_str)
        .unwrap_or("shell")
        .to_string()
}

/// Resolves a cell's foreground and background the way the renderer does.
fn attribute_colors(attribute: &TextAttribute, table: &ColorTable) -> (egui::Color32, egui::Color32) {
    // Mirrors `RenderSettings::get_attribute_colors` (`render_settings.rs`):
    // intense legacy colours take their bright entry (`intenseIsBright`), faint
    // halves each component, reverse video swaps the pair, and invisible text
    // takes the background. The shell paints with egui, so the rule runs
    // against the adapter's live table instead of the renderer's copy.
    let default_fg = table.alias_index(ColorAlias::DefaultForeground);
    let default_bg = table.alias_index(ColorAlias::DefaultBackground);

    let mut foreground = TextColor::from_colorref(attribute.get_foreground().get_color(
        table,
        default_fg,
        attribute.is_intense(),
    ));
    let mut background = TextColor::from_colorref(attribute.get_background().get_color(table, default_bg, false));

    if attribute.is_faint() {
        foreground = TextColor::from_colorref((foreground.get_colorref() >> 1) & 0x007F_7F7F);
    }
    if attribute.is_reverse_video() {
        std::mem::swap(&mut foreground, &mut background);
    }
    if attribute.is_invisible() {
        foreground = background;
    }

    (
        egui_color(foreground).unwrap_or(FALLBACK_FOREGROUND),
        egui_color(background).unwrap_or(FALLBACK_BACKGROUND),
    )
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

/// Maps a screen row to a document row for the current scroll.
///
/// Document row zero is the oldest row still held; the screen's first row is
/// `history - scroll`.
const fn screen_to_document(history: i32, scroll: i32, screen_row: i32) -> i32 {
    history - scroll + screen_row
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

/// Draws the cursor in the shape the terminal asked for.
fn paint_cursor(
    painter: &egui::Painter,
    origin: egui::Pos2,
    cell: egui::Vec2,
    shape: CursorType,
    color: egui::Color32,
) {
    let rect = egui::Rect::from_min_size(origin, cell);
    let bar = (cell.x / 8.0).max(1.0);

    match shape {
        CursorType::Legacy | CursorType::FullBox => {
            painter.rect_filled(rect, 0, color);
        }
        CursorType::EmptyBox => {
            painter.rect_stroke(rect, 0, egui::Stroke::new(1.0, color), egui::StrokeKind::Inside);
        }
        CursorType::VerticalBar => {
            painter.rect_filled(egui::Rect::from_min_size(origin, egui::vec2(bar, cell.y)), 0, color);
        }
        CursorType::Underscore => {
            paint_cursor_bar(painter, origin, cell, bar, bar, color);
        }
        CursorType::DoubleUnderscore => {
            paint_cursor_bar(painter, origin, cell, bar, bar, color);
            paint_cursor_bar(painter, origin, cell, bar * 3.0, bar, color);
        }
    }
}

fn paint_cursor_bar(
    painter: &egui::Painter,
    origin: egui::Pos2,
    cell: egui::Vec2,
    from_bottom: f32,
    height: f32,
    color: egui::Color32,
) {
    let top = origin.y + cell.y - from_bottom;
    painter.rect_filled(
        egui::Rect::from_min_size(egui::pos2(origin.x, top), egui::vec2(cell.x, height)),
        0,
        color,
    );
}

/// The rows the settings window shows.
///
/// Kept out of the window so the strings are testable without an egui context;
/// the window adds only layout.
fn settings_rows(settings: &Settings, path: Option<&Path>, error: Option<&str>) -> Vec<(&'static str, String)> {
    let mut rows = Vec::with_capacity(6);

    rows.push((
        "File",
        path.map_or_else(
            || "(none on this platform)".to_string(),
            |path| path.display().to_string(),
        ),
    ));
    rows.push(("Status", error.unwrap_or("loaded").to_string()));

    let profile = settings.profile();
    rows.push((
        "Profile",
        profile.map_or_else(
            || "(none)".to_string(),
            |profile| format!("{} ({})", profile.name, profile.command),
        ),
    ));

    rows.push(("Keybindings", settings.keybindings.len().to_string()));
    rows.push((
        "Scheme",
        settings
            .scheme()
            .map_or_else(|| "(built-in)".to_string(), |scheme| scheme.name.clone()),
    ));
    rows.push(("Font", format!("{} pt", settings.font.size)));

    rows
}

/// Draws the editable fields and the Save/Revert buttons.
fn draft_editor(
    ui: &mut egui::Ui,
    draft: &mut Draft,
    settings: &Settings,
    conflict: bool,
    save: &mut bool,
    revert: &mut bool,
) {
    egui::Grid::new("winterm-settings-editor")
        .num_columns(2)
        .spacing([16.0, 4.0])
        .show(ui, |ui| {
            ui.strong("Font size");
            ui.add(egui::DragValue::new(&mut draft.font_size).range(4.0..=100.0).speed(0.5));
            ui.end_row();

            ui.strong("Default profile");
            let names: Vec<String> = draft.profiles.iter().map(|profile| profile.name.clone()).collect();
            let selected = draft.default_profile.clone().unwrap_or_default();
            let selected = if selected.is_empty() {
                "(none)".to_string()
            } else {
                selected
            };
            egui::ComboBox::from_id_salt("winterm-settings-profile")
                .selected_text(selected)
                .show_ui(ui, |ui| {
                    for name in names {
                        ui.selectable_value(&mut draft.default_profile, Some(name.clone()), name);
                    }
                });
            ui.end_row();

            ui.strong("Scheme");
            let selected = draft.default_scheme.clone().unwrap_or_else(|| "built-in".to_string());
            egui::ComboBox::from_id_salt("winterm-settings-scheme")
                .selected_text(selected)
                .show_ui(ui, |ui| {
                    ui.selectable_value(&mut draft.default_scheme, None, "built-in");
                    for scheme in &settings.schemes {
                        ui.selectable_value(&mut draft.default_scheme, Some(scheme.name.clone()), &scheme.name);
                    }
                });
            ui.end_row();
        });

    ui.separator();
    ui.strong("Profiles");
    ui.label("Arguments are split on spaces; quote an argument in the file.");

    let mut remove = None;
    egui::ScrollArea::vertical()
        .id_salt("winterm-profiles")
        .max_height(220.0)
        .show(ui, |ui| {
            for (index, profile) in draft.profiles.iter_mut().enumerate() {
                egui::Grid::new(("winterm-profile", index))
                    .num_columns(2)
                    .spacing([16.0, 4.0])
                    .show(ui, |ui| {
                        ui.label("Name");
                        ui.text_edit_singleline(&mut profile.name);
                        ui.end_row();

                        ui.label("Command");
                        ui.text_edit_singleline(&mut profile.command);
                        ui.end_row();

                        ui.label("Arguments");
                        ui.text_edit_singleline(&mut profile.args);
                        ui.end_row();

                        ui.label("Title");
                        ui.text_edit_singleline(&mut profile.title);
                        ui.end_row();

                        ui.label("");
                        if ui.small_button("Remove").clicked() {
                            remove = Some(index);
                        }
                        ui.end_row();
                    });
            }
        });
    if let Some(index) = remove {
        draft.profiles.remove(index);
    }
    if ui.button("Add profile").clicked() {
        let mut profile = ProfileDraft::from_profile(&settings::Profile::default());
        profile.name = format!("profile {}", draft.profiles.len() + 1);
        draft.profiles.push(profile);
    }

    ui.separator();
    ui.strong("Keybindings");
    ui.label("A chord is [ctrl+][alt+][shift+]<key>. The file has the full chord syntax.");

    let mut remove = None;
    egui::ScrollArea::vertical()
        .id_salt("winterm-bindings")
        .max_height(200.0)
        .show(ui, |ui| {
            for (index, binding) in draft.keybindings.iter_mut().enumerate() {
                ui.horizontal(|ui| {
                    ui.add(egui::TextEdit::singleline(&mut binding.keys).desired_width(150.0));
                    egui::ComboBox::from_id_salt(("winterm-binding-action", index))
                        .selected_text(binding.action.name())
                        .show_ui(ui, |ui| {
                            for action in settings::Action::ALL {
                                ui.selectable_value(&mut binding.action, *action, action.name());
                            }
                        });
                    if ui.small_button("Remove").clicked() {
                        remove = Some(index);
                    }
                });
            }
        });
    if let Some(index) = remove {
        draft.keybindings.remove(index);
    }
    if ui.button("Add binding").clicked() {
        draft.keybindings.push(BindingDraft {
            keys: String::new(),
            action: settings::Action::NewTab,
        });
    }

    let invalid: Vec<&str> = draft
        .keybindings
        .iter()
        .filter(|binding| binding.to_binding().is_none())
        .map(|binding| binding.keys.as_str())
        .collect();
    if !invalid.is_empty() {
        ui.colored_label(CONFLICT, format!("These chords do not parse: {}", invalid.join(", ")));
    }

    ui.separator();
    let valid = draft.document(settings).is_some();
    let dirty = draft.differs_from(settings);
    ui.horizontal(|ui| {
        if ui.add_enabled(dirty && valid, egui::Button::new("Save")).clicked() {
            *save = true;
        }
        if ui.add_enabled(dirty, egui::Button::new("Revert")).clicked() {
            *revert = true;
        }
    });

    if conflict {
        ui.colored_label(
            CONFLICT,
            "The file changed on disk. Save overwrites it; Revert loads it.",
        );
    }
    if dirty {
        ui.label("Unsaved changes are discarded when this window closes.");
    }
}

/// Turns a profile into the command a tab starts and the title to show.
fn profile_command(profile: Option<&settings::Profile>) -> (PtyCommand, String) {
    let profile = profile.cloned().unwrap_or_default();
    let command = PtyCommand::new(&profile.command).args(profile.args.iter().cloned());
    let title = profile.title.clone().unwrap_or_else(|| command_title(&command));
    (command, title)
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

    egui_color(entry)
}

/// The egui colour an RGB terminal colour names.
fn egui_color(color: TextColor) -> Option<egui::Color32> {
    match color.get_type() {
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
    fn a_face_is_chosen_by_style_and_falls_back_to_the_embedded_one() {
        let embedded = Faces::default();
        assert_eq!(embedded.font(16.0, false).family, egui::FontFamily::Monospace);
        assert_eq!(embedded.font(16.0, true).family, egui::FontFamily::Monospace);

        let host = Faces {
            regular: true,
            italic: false,
        };
        assert_eq!(
            host.font(16.0, false).family,
            egui::FontFamily::Name(REGULAR_FAMILY.into())
        );
        // Without an italic face, italic text keeps the upright one.
        assert_eq!(
            host.font(16.0, true).family,
            egui::FontFamily::Name(REGULAR_FAMILY.into())
        );

        let with_italic = Faces {
            regular: true,
            italic: true,
        };
        assert_eq!(
            with_italic.font(16.0, true).family,
            egui::FontFamily::Name(ITALIC_FAMILY.into())
        );
        assert_eq!(with_italic.font(21.0, true).size, 21.0);
    }

    #[test]
    #[ignore = "reads the host's installed fonts; run with `cargo test -p winterm -- --ignored`"]
    fn the_host_faces_register_with_egui() {
        let ctx = egui::Context::default();
        let faces = register_fonts(&ctx);
        assert!(faces.regular, "no host monospace family was found");
        eprintln!("registered faces: {faces:?}");

        // The atlas is built by the first pass; its texture delta is dropped
        // with the output, so mark it handled first.
        let mut widths = (0.0, 0.0);
        let mut output = ctx.run_ui(egui::RawInput::default(), |ui| {
            widths.0 = ui
                .ctx()
                .fonts_mut(|fonts| fonts.glyph_width(&faces.font(16.0, false), 'M'));
            if faces.italic {
                widths.1 = ui
                    .ctx()
                    .fonts_mut(|fonts| fonts.glyph_width(&faces.font(16.0, true), 'M'));
            }
        });
        output.textures_delta.clear();

        assert!(widths.0 > 0.0, "the regular face has no glyphs");
        if faces.italic {
            assert!(widths.1 > 0.0, "the italic face has no glyphs");
        }
    }

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
    fn a_scheme_colours_resolve_through_the_table() {
        let settings = settings::parse(
            r##"
                default_scheme = "x"

                [[schemes]]
                name = "x"
                foreground = "#010203"
                background = "#101112"
                blue = "#0000FF"
            "##,
        )
        .expect("settings");

        let mut table = ColorTable::new();
        settings.scheme().expect("scheme").apply(&mut table);

        assert_eq!(
            resolve(TextColor::new(), &table, ColorAlias::DefaultForeground),
            Some(egui::Color32::from_rgb(1, 2, 3))
        );
        assert_eq!(
            resolve(TextColor::new(), &table, ColorAlias::DefaultBackground),
            Some(egui::Color32::from_rgb(0x10, 0x11, 0x12))
        );
        assert_eq!(
            resolve(TextColor::index16(4), &table, ColorAlias::DefaultForeground),
            Some(egui::Color32::from_rgb(0, 0, 0xFF))
        );
    }

    #[test]
    fn reverse_video_swaps_the_attribute_colours() {
        let table = ColorTable::new();
        let mut attribute = TextAttribute::new();
        attribute.set_foreground(TextColor::rgb(1, 2, 3));
        attribute.set_background(TextColor::rgb(4, 5, 6));
        attribute.set_reverse_video(true);

        assert_eq!(
            attribute_colors(&attribute, &table),
            (egui::Color32::from_rgb(4, 5, 6), egui::Color32::from_rgb(1, 2, 3))
        );
    }

    #[test]
    fn intense_brightens_a_legacy_colour() {
        let table = ColorTable::new();
        let mut attribute = TextAttribute::new();
        attribute.set_indexed_foreground(1);
        let normal = attribute_colors(&attribute, &table).0;
        attribute.set_intense(true);

        assert_eq!(normal, egui::Color32::from_rgb(0xC5, 0x0F, 0x1F));
        assert_eq!(
            attribute_colors(&attribute, &table).0,
            egui::Color32::from_rgb(0xE7, 0x48, 0x56)
        );
    }

    #[test]
    fn faint_halves_the_foreground() {
        let table = ColorTable::new();
        let mut attribute = TextAttribute::new();
        attribute.set_foreground(TextColor::rgb(200, 100, 50));
        attribute.set_faint(true);

        assert_eq!(
            attribute_colors(&attribute, &table).0,
            egui::Color32::from_rgb(100, 50, 25)
        );
    }

    #[test]
    fn a_profile_becomes_a_command_and_a_title() {
        let profile = settings::Profile {
            name: "x".to_string(),
            command: "/bin/sh".to_string(),
            args: vec!["-l".to_string()],
            title: None,
        };

        let (command, title) = profile_command(Some(&profile));
        assert_eq!(command.program(), std::ffi::OsStr::new("/bin/sh"));
        assert_eq!(
            command.arguments().collect::<Vec<_>>(),
            vec![std::ffi::OsStr::new("-l")]
        );
        assert_eq!(title, "sh");

        let (_, title) = profile_command(Some(&settings::Profile {
            title: Some("Named".to_string()),
            ..profile
        }));
        assert_eq!(title, "Named");
    }

    #[test]
    fn a_command_title_is_the_program_stem() {
        assert_eq!(command_title(&PtyCommand::new("cmd.exe")), "cmd");
        assert_eq!(command_title(&PtyCommand::new("/bin/sh")), "sh");
        assert_eq!(command_title(&PtyCommand::new("")), "shell");
    }

    #[test]
    fn tab_chords_are_the_windows_and_not_the_shells() {
        let key = |key, ctrl, shift| egui::Event::Key {
            key,
            physical_key: None,
            pressed: true,
            repeat: false,
            modifiers: egui::Modifiers {
                ctrl,
                shift,
                ..egui::Modifiers::default()
            },
        };
        let bindings = settings::DEFAULT_KEYBINDINGS;

        assert_eq!(
            command_for(bindings, &key(egui::Key::T, true, true)),
            Some(TabAction::New)
        );
        assert_eq!(
            command_for(bindings, &key(egui::Key::W, true, true)),
            Some(TabAction::CloseActive)
        );
        assert_eq!(
            command_for(bindings, &key(egui::Key::Tab, true, false)),
            Some(TabAction::Next)
        );
        assert_eq!(
            command_for(bindings, &key(egui::Key::Tab, true, true)),
            Some(TabAction::Previous)
        );
        assert_eq!(
            command_for(bindings, &key(egui::Key::C, true, true)),
            Some(TabAction::Copy)
        );
        assert_eq!(
            command_for(bindings, &key(egui::Key::V, true, true)),
            Some(TabAction::Paste)
        );
        assert_eq!(
            command_for(bindings, &key(egui::Key::D, true, true)),
            Some(TabAction::Split(Axis::Horizontal))
        );
        assert_eq!(
            command_for(bindings, &key(egui::Key::E, true, true)),
            Some(TabAction::Split(Axis::Vertical))
        );

        // Without both modifiers, the key belongs to the shell.
        assert_eq!(command_for(bindings, &key(egui::Key::T, true, false)), None);
        assert_eq!(command_for(bindings, &key(egui::Key::Tab, false, false)), None);
        assert_eq!(command_for(bindings, &egui::Event::Text("t".into())), None);
    }

    #[test]
    fn alt_arrows_focus_and_alt_shift_arrows_resize() {
        let key = |key, ctrl, shift, alt| egui::Event::Key {
            key,
            physical_key: None,
            pressed: true,
            repeat: false,
            modifiers: egui::Modifiers {
                ctrl,
                shift,
                alt,
                ..egui::Modifiers::default()
            },
        };
        let bindings = settings::DEFAULT_KEYBINDINGS;

        assert_eq!(
            command_for(bindings, &key(egui::Key::ArrowLeft, false, false, true)),
            Some(TabAction::Focus(Direction::Left))
        );
        assert_eq!(
            command_for(bindings, &key(egui::Key::ArrowDown, false, false, true)),
            Some(TabAction::Focus(Direction::Down))
        );
        assert_eq!(
            command_for(bindings, &key(egui::Key::ArrowUp, false, true, true)),
            Some(TabAction::Resize(Direction::Up))
        );

        // Ctrl+Alt+Arrow is the shell's, and a bare arrow is a key.
        assert_eq!(
            command_for(bindings, &key(egui::Key::ArrowLeft, true, false, true)),
            None
        );
        assert_eq!(
            command_for(bindings, &key(egui::Key::ArrowLeft, false, false, false)),
            None
        );
    }

    #[test]
    fn the_settings_decide_which_chords_the_window_claims() {
        let key = |key, ctrl, shift, alt| egui::Event::Key {
            key,
            physical_key: None,
            pressed: true,
            repeat: false,
            modifiers: egui::Modifiers {
                ctrl,
                shift,
                alt,
                ..egui::Modifiers::default()
            },
        };
        let settings = settings::parse(
            r#"
                [[keybindings]]
                keys = "ctrl+alt+t"
                action = "newTab"

                [[keybindings]]
                keys = "ctrl+shift+t"
                action = "paste"

                [[keybindings]]
                keys = "ctrl+shift+c"
                action = "unbound"
            "#,
        )
        .expect("settings");

        // A chord the file adds runs its action.
        assert_eq!(
            command_for(&settings.keybindings, &key(egui::Key::T, true, false, true)),
            Some(TabAction::New)
        );
        // A user chord takes the default's place instead of running beside it.
        assert_eq!(
            command_for(&settings.keybindings, &key(egui::Key::T, true, true, false)),
            Some(TabAction::Paste)
        );
        // An unbound default reaches the shell.
        assert_eq!(
            command_for(&settings.keybindings, &key(egui::Key::C, true, true, false)),
            None
        );
        // A default the file does not name is still the window's.
        assert_eq!(
            command_for(&settings.keybindings, &key(egui::Key::W, true, true, false)),
            Some(TabAction::CloseActive)
        );
    }

    #[test]
    fn a_draft_knows_its_edits_and_builds_the_document_a_save_writes() {
        let settings = settings::parse(
            r#"
                default_profile = "a"
                default_scheme = "s"

                [[profiles]]
                name = "a"
                command = "sh"

                [[profiles]]
                name = "b"
                command = "sh"

                [[schemes]]
                name = "s"

                [font]
                size = 18
            "#,
        )
        .expect("settings");

        let mut draft = Draft::from_settings(&settings);
        assert!(!draft.differs_from(&settings));

        draft.font_size = 22.0;
        assert!(draft.differs_from(&settings));
        let document = draft.document(&settings).expect("document");
        assert_eq!(document.font.size, 22.0);
        assert_eq!(document.default_profile.as_deref(), Some("a"));

        draft.default_profile = Some("b".to_string());
        assert_eq!(
            draft.document(&settings).expect("document").default_profile.as_deref(),
            Some("b")
        );

        // Choosing the built-in palette writes no scheme name.
        draft.default_scheme = None;
        assert_eq!(draft.document(&settings).expect("document").default_scheme, None);
        assert!(draft.differs_from(&settings));
    }

    #[test]
    fn a_profile_round_trips_through_the_draft_text() {
        let profile = settings::Profile {
            name: "pwsh".to_string(),
            command: "pwsh.exe".to_string(),
            args: vec!["-NoLogo".to_string(), "-NoProfile".to_string()],
            title: Some("PowerShell".to_string()),
        };

        let draft = ProfileDraft::from_profile(&profile);
        assert_eq!(draft.args, "-NoLogo -NoProfile");
        assert_eq!(draft.to_profile(), profile);

        // Blank text comes back as absent.
        let bare = ProfileDraft {
            name: "sh".to_string(),
            command: "/bin/sh".to_string(),
            args: "   ".to_string(),
            title: String::new(),
        };
        let profile = bare.to_profile();
        assert_eq!(profile.args, Vec::<String>::new());
        assert_eq!(profile.title, None);
    }

    #[test]
    fn the_draft_edits_the_profile_list() {
        let settings = settings::parse(
            r#"
                [[profiles]]
                name = "a"
                command = "sh"

                [[profiles]]
                name = "b"
                command = "zsh"
            "#,
        )
        .expect("settings");

        let mut draft = Draft::from_settings(&settings);
        assert!(!draft.differs_from(&settings));

        draft.profiles[1].command = "bash".to_string();
        draft
            .profiles
            .push(ProfileDraft::from_profile(&settings::Profile::default()));
        assert!(draft.differs_from(&settings));

        let document = draft.document(&settings).expect("document");
        assert_eq!(document.profiles.len(), 3);
        assert_eq!(document.profiles[1].command, "bash");
        assert_eq!(document.profiles[2].name, "default");
    }

    #[test]
    fn a_binding_round_trips_through_the_draft_and_rejects_a_bad_chord() {
        let binding = settings::Keybinding {
            keys: settings::Chord::parse("ctrl+alt+t").expect("chord"),
            action: settings::Action::NewTab,
        };

        let draft = BindingDraft::from_binding(&binding);
        assert_eq!(draft.keys, "ctrl+alt+t");
        assert_eq!(draft.to_binding(), Some(binding));

        let bad = BindingDraft {
            keys: "ctrl+a+b".to_string(),
            action: settings::Action::Copy,
        };
        assert_eq!(bad.to_binding(), None);
    }

    #[test]
    fn the_draft_edits_the_keybindings_and_blocks_a_bad_chord() {
        let settings =
            settings::parse("[[keybindings]]\nkeys = \"ctrl+alt+t\"\naction = \"newTab\"").expect("settings");
        let mut draft = Draft::from_settings(&settings);
        assert!(!draft.differs_from(&settings));

        // The list is the effective one, defaults included.
        assert_eq!(draft.keybindings.len(), settings.keybindings.len());

        draft.keybindings[0].action = settings::Action::Copy;
        assert!(draft.differs_from(&settings));
        let document = draft.document(&settings).expect("document");
        assert_eq!(document.keybindings[0].action, settings::Action::Copy);

        // A chord that does not parse keeps the document from being built.
        draft.keybindings[0].keys = "ctrl+a+b".to_string();
        assert!(draft.document(&settings).is_none());
        assert!(draft.differs_from(&settings));
    }

    #[test]
    fn the_settings_rows_report_what_the_file_asked_for() {
        let settings = settings::parse(
            r#"
                default_profile = "zsh"
                default_scheme = "muted"

                [[profiles]]
                name = "zsh"
                command = "/bin/zsh"

                [[schemes]]
                name = "muted"

                [font]
                size = 18
            "#,
        )
        .expect("settings");

        let rows = settings_rows(&settings, Some(Path::new("/cfg/settings.toml")), None);
        let value = |label: &str| {
            rows.iter()
                .find(|(name, _)| *name == label)
                .map(|(_, value)| value.as_str())
                .unwrap_or("")
        };

        assert_eq!(value("File"), "/cfg/settings.toml");
        assert_eq!(value("Status"), "loaded");
        assert_eq!(value("Profile"), "zsh (/bin/zsh)");
        assert_eq!(value("Keybindings"), "17");
        assert_eq!(value("Scheme"), "muted");
        assert_eq!(value("Font"), "18 pt");

        // A file that was not used still reports what is running and why.
        let rows = settings_rows(&Settings::default(), None, Some("bad file"));
        let value = |label: &str| {
            rows.iter()
                .find(|(name, _)| *name == label)
                .map(|(_, value)| value.as_str())
                .unwrap_or("")
        };

        assert_eq!(value("File"), "(none on this platform)");
        assert_eq!(value("Status"), "bad file");
        assert_eq!(value("Scheme"), "(built-in)");
        assert_eq!(value("Font"), "16 pt");
    }

    #[test]
    fn chord_keys_name_the_egui_keys_the_settings_can_use() {
        assert_eq!(chord_key(egui::Key::T), Some(settings::Key::Char('t')));
        assert_eq!(chord_key(egui::Key::Num7), Some(settings::Key::Char('7')));
        assert_eq!(chord_key(egui::Key::Semicolon), Some(settings::Key::Char(';')));
        assert_eq!(chord_key(egui::Key::F5), Some(settings::Key::Function(5)));
        assert_eq!(chord_key(egui::Key::PageDown), Some(settings::Key::PageDown));
        assert_eq!(chord_key(egui::Key::BrowserBack), None);
    }

    #[test]
    fn the_settings_chord_toggles_the_window() {
        let key = egui::Event::Key {
            key: egui::Key::Comma,
            physical_key: None,
            pressed: true,
            repeat: false,
            modifiers: egui::Modifiers {
                ctrl: true,
                ..egui::Modifiers::default()
            },
        };

        assert_eq!(
            command_for(settings::DEFAULT_KEYBINDINGS, &key),
            Some(TabAction::ToggleSettings)
        );
    }

    #[test]
    fn a_pointer_near_a_divider_names_that_split() {
        let mut layout = Layout::leaf(PaneId(0));
        assert!(layout.split(PaneId(0), Axis::Horizontal, PaneId(1)));
        assert!(layout.split(PaneId(1), Axis::Vertical, PaneId(2)));
        let area = egui::Rect::from_min_size(egui::pos2(0.0, 0.0), egui::vec2(200.0, 100.0));

        // The root's divider is the vertical line at x = 100.
        let (path, axis, _) = divider_at(&layout, egui::pos2(102.0, 20.0), area).expect("divider");
        assert_eq!(path, layout::SplitPath::new());
        assert_eq!(axis, Axis::Horizontal);

        // The nested divider is the horizontal line at y = 50, on the right.
        let (path, axis, _) = divider_at(&layout, egui::pos2(150.0, 52.0), area).expect("divider");
        assert_eq!(path, vec![1]);
        assert_eq!(axis, Axis::Vertical);

        // Away from both, the point belongs to a pane; a point past the grab
        // distance is a pane's too, and so is one beside a nested divider's
        // span.
        assert_eq!(divider_at(&layout, egui::pos2(40.0, 20.0), area), None);
        assert_eq!(divider_at(&layout, egui::pos2(110.0, 20.0), area), None);
        assert_eq!(divider_at(&layout, egui::pos2(50.0, 51.0), area), None);
    }

    #[test]
    fn where_a_divider_meets_another_the_deeper_one_wins() {
        let mut layout = Layout::leaf(PaneId(0));
        assert!(layout.split(PaneId(0), Axis::Horizontal, PaneId(1)));
        assert!(layout.split(PaneId(1), Axis::Vertical, PaneId(2)));
        let area = egui::Rect::from_min_size(egui::pos2(0.0, 0.0), egui::vec2(200.0, 100.0));

        // A pixel from the T-junction: both the root's vertical divider and the
        // nested horizontal one are close.
        let (path, axis, _) = divider_at(&layout, egui::pos2(101.0, 51.0), area).expect("divider");
        assert_eq!(path, vec![1]);
        assert_eq!(axis, Axis::Vertical);
    }

    #[test]
    fn a_cell_count_is_clamped_for_the_pty() {
        assert_eq!(to_cells(0), 1);
        assert_eq!(to_cells(120), 120);
        assert_eq!(to_cells(i32::MAX), u16::MAX);
    }

    #[test]
    fn a_screen_row_maps_into_the_document() {
        assert_eq!(screen_to_document(5, 2, 0), 3);
        assert_eq!(screen_to_document(5, 0, 4), 9);
        assert_eq!(screen_to_document(0, 0, 0), 0);
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
