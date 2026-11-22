//! The egui terminal viewport, from `TermControl.cpp`.
//!
//! `Control` (`term_control.rs`) owns the terminal and its connection; this
//! module is the widget over it. It turns egui events into `term` vocabulary,
//! paints `grid` snapshots, and holds the viewport and the selection, which
//! ADR 019 keeps in the application rather than in the terminal (ADR 038).

use std::io;
use std::time::Instant;

use eframe::egui;
use server::pty::PtySize;
use term::input::{
    Key, KeyEvent, Modifiers as KeyModifiers, MouseButton, MouseButtons, MouseEvent, MouseEventKind, WHEEL_DELTA,
    WheelAxis,
};
use types::CursorType;
use types::TextAttribute;
use types::UnderlineStyle;
use types::color_table::{CURSOR_COLOR, ColorAlias, ColorTable, SELECTION_BACKGROUND as SELECTION_SLOT};
use types::text_color::{ColorType, TextColor};

use crate::app::{Faces, next_match};
use crate::layout::PaneId;
use crate::term_control::Control;
use crate::term_events::TerminalEvent;
use crate::term_settings::TerminalSettings;
use crate::{grid, scrollbar, session::Session, view::View};

/// The colour used when the palette has no default background.
/// How close to a scrollbar tick a press has to be, in pixels, to take it.
pub(crate) const MARK_GRAB: f32 = 4.0;

pub(crate) const FALLBACK_BACKGROUND: egui::Color32 = egui::Color32::from_rgb(12, 12, 12);
/// The colour used when the palette has no default foreground.
pub(crate) const FALLBACK_FOREGROUND: egui::Color32 = egui::Color32::from_rgb(204, 204, 204);
/// The fill behind selected cells when the table names no selection colour.
pub(crate) const FALLBACK_SELECTION: egui::Color32 = egui::Color32::from_rgb(38, 79, 120);

/// The find bar's state.
#[derive(Debug, Default)]
pub(crate) struct FindBar {
    /// Whether the bar is showing.
    pub(crate) open: bool,
    /// What the user is looking for.
    pub(crate) query: String,
    /// The query `matches` were computed for.
    pub(crate) searched: String,
    /// Every match in the document, in reading order.
    pub(crate) matches: Vec<buffer::TextMatch>,
    /// The match Enter moved to, if any.
    pub(crate) current: Option<usize>,
    /// The document height when the matches were computed.
    pub(crate) document_height: i32,
    /// Whether the field still needs the keyboard focus.
    pub(crate) focus: bool,
}

impl FindBar {
    pub(crate) fn show(&mut self) {
        self.open = true;
        self.focus = true;
    }

    pub(crate) fn hide(&mut self) {
        self.open = false;
        self.focus = false;
        self.current = None;
    }

    pub(crate) fn toggle(&mut self) {
        if self.open { self.hide() } else { self.show() }
    }
}

/// What every pane paints with: the cell, the faces, and the moment.
///
/// Bundled rather than passed one by one, because a pane, its tab and the
/// window all need the same four and the compiler stops counting at seven.
#[derive(Debug, Clone, Copy)]
pub(crate) struct GridPaint {
    pub(crate) cell: egui::Vec2,
    pub(crate) font_size: f32,
    pub(crate) faces: Faces,
    pub(crate) now: Instant,
}

/// One pane: the terminal a tab shows, its viewport and its overlays.
///
/// The terminal, its connection and what it reports live in a [`Control`]; the
/// viewport and the selection are this widget's, because ADR 019 keeps them in
/// the application.
pub(crate) struct Pane {
    pub(crate) id: PaneId,
    pub(crate) control: Control,
    pub(crate) view: View,
    /// The find bar's query and matches.
    pub(crate) find: FindBar,
    /// The scrollbar's drag state.
    pub(crate) scrollbar: scrollbar::Scrollbar,
    /// The cell the pointer was last forwarded from, for motion deduplication.
    pub(crate) last_mouse_cell: Option<(i32, i32)>,
}

impl Pane {
    pub(crate) fn new(ctx: &egui::Context, id: PaneId, settings: &TerminalSettings) -> io::Result<Self> {
        let size = PtySize::DEFAULT;
        let repaint = ctx.clone();
        let session = Session::spawn_with(&settings.command, size, move || repaint.request_repaint())?;
        let mut control = Control::new(Box::new(session), (i32::from(size.cols), i32::from(size.rows)));
        control.apply_scheme(settings.scheme.as_ref());

        Ok(Self {
            id,
            control,
            view: View::new(),
            find: FindBar::default(),
            scrollbar: scrollbar::Scrollbar::default(),
            last_mouse_cell: None,
        })
    }

    /// Makes the pane's palette the built-in table plus `scheme`.
    pub(crate) fn apply_scheme(&mut self, scheme: Option<&settings::Scheme>) {
        self.control.apply_scheme(scheme);
    }

    /// Takes what the terminal did since the last frame.
    pub(crate) fn take_events(&mut self, now: Instant) -> Vec<TerminalEvent> {
        self.control.take_events(now)
    }

    /// Takes the DECPS notes the program asked for.
    pub(crate) fn take_sounds(&mut self) -> Vec<audio::Note> {
        self.control.take_sounds()
    }

    /// Moves bytes from the pty into the parser and answers from the adapter.
    pub(crate) fn pump_output(&mut self) {
        self.control.pump();
    }

    pub(crate) fn handle_events(&mut self, events: Vec<egui::Event>, sides: &mut ModifierSides) {
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

    pub(crate) fn send_key(&mut self, event: &KeyEvent) {
        self.control.send_key(event);
    }

    pub(crate) fn resize(&mut self, columns: i32, rows: i32) {
        self.control.resize(columns, rows);
    }

    /// Handles the pointer: wheel, buttons, motion and selection.
    ///
    /// A tracking application owns the mouse; Shift is the user's override and
    /// starts a selection instead.
    pub(crate) fn handle_pointer(
        &mut self,
        ctx: &egui::Context,
        area: egui::Rect,
        cell: egui::Vec2,
        sides: ModifierSides,
    ) {
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

        // The scrollbar owns presses and drags on its strip; the wheel still
        // reaches the viewport below.
        if (input.primary_pressed || input.primary_released || self.scrollbar.dragging())
            && self.handle_scrollbar(&input, area)
        {
            return;
        }

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
            let max = self.control.max_scroll();
            self.view.drag_scroll(outside, max, Instant::now());

            let (columns, rows) = self.control.size();
            let rows = rows.max(1);
            let row = if outside < 0 { 0 } else { rows - 1 };
            let column = (((position.x - area.min.x) / cell.x).floor() as i32).clamp(0, columns.max(1) - 1);
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
        let history = self.control.max_scroll();
        let document_row = screen_to_document(history, self.view.scroll(), cell_position.1);
        let document_position = (cell_position.0, document_row);

        let forwards = forwards_to_application(self.control.is_tracking_mouse(), input.modifiers.shift);

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
            let sent = if input.modifiers.shift {
                false
            } else {
                self.control.send_mouse(&wheel)
            };
            if !sent {
                let max = self.control.max_scroll();
                self.view.scroll_by(lines, max);
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
                self.control.send_mouse(&event);
            }
        } else {
            self.last_mouse_cell = None;

            if input.primary_pressed {
                let count = self.view.click(document_position, Instant::now());

                if count >= 3 {
                    let width = self.control.screen_width();
                    self.view.select_line(document_row, width);
                } else if count == 2 {
                    let text = self.control.row_text(document_row);
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
    pub(crate) fn send_mouse_buttons(
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
            self.control.send_mouse(&event);
        }
    }

    /// Maps a window position to a screen cell, if it is inside the grid.
    pub(crate) fn cell_at(&self, area: egui::Rect, cell: egui::Vec2, position: egui::Pos2) -> Option<(i32, i32)> {
        let column = ((position.x - area.min.x) / cell.x).floor() as i32;
        let row = ((position.y - area.min.y) / cell.y).floor() as i32;
        let (columns, rows) = self.control.size();
        if column < 0 || row < 0 || column >= columns || row >= rows {
            return None;
        }
        Some((column, row))
    }

    /// Returns the selected cells, using the buffer's copy path.
    pub(crate) fn selection_text(&mut self) -> Option<String> {
        let selection = self.view.selection()?;
        self.control.selection_text(selection)
    }

    /// Pastes text as the terminal would, through the bracketed-paste filter.
    pub(crate) fn paste_text(&mut self, text: &str) {
        self.control.paste(text);
    }

    pub(crate) fn snapshot(&mut self) -> grid::Screen {
        self.control.snapshot(&self.view)
    }

    /// Shows or hides this pane's find bar.
    pub(crate) fn toggle_find(&mut self) {
        self.find.toggle();
    }

    /// Recomputes the matches after the query changed and scrolls to the first.
    pub(crate) fn refresh_find(&mut self) {
        let (matches, document_height) = self.control.find_matches(&self.find.query);

        self.find.matches = matches;
        self.find.document_height = document_height;
        self.find.searched = self.find.query.clone();
        self.find.current = (!self.find.matches.is_empty()).then_some(0);
        self.scroll_to_find();
    }

    /// Moves to the next (or previous) match, wrapping at both ends.
    pub(crate) fn step_find(&mut self, forward: bool) {
        self.find.current = next_match(self.find.current, self.find.matches.len(), forward);
        self.scroll_to_find();
    }

    /// Brings the current match into the middle of the view.
    pub(crate) fn scroll_to_find(&mut self) {
        let Some(found) = self.find.current.and_then(|index| self.find.matches.get(index)) else {
            return;
        };
        let (row, height, total) = (found.start_row, self.control.size().1, self.find.document_height);
        self.view.scroll_to_row(row, height, total);
    }

    /// Draws the find bar and takes its keys.
    pub(crate) fn find_bar(&mut self, ctx: &egui::Context) {
        if !self.find.open {
            return;
        }

        // Read the keys before the field draws: Enter and Escape belong to the
        // bar, not to the text the field would otherwise consume.
        let (enter, shift, escape) = ctx.input(|input| {
            (
                input.key_pressed(egui::Key::Enter),
                input.modifiers.shift,
                input.key_pressed(egui::Key::Escape),
            )
        });
        if escape {
            self.find.hide();
            return;
        }

        let mut step = None;
        egui::Window::new("Find")
            .anchor(egui::Align2::RIGHT_TOP, egui::vec2(-16.0, 64.0))
            .collapsible(false)
            .resizable(false)
            .show(ctx, |ui| {
                ui.horizontal(|ui| {
                    let field = ui.add(
                        egui::TextEdit::singleline(&mut self.find.query)
                            .hint_text("Find")
                            .desired_width(180.0),
                    );
                    if self.find.focus {
                        field.request_focus();
                        self.find.focus = false;
                    }
                    if ui.small_button("↑").on_hover_text("Previous").clicked() {
                        step = Some(false);
                    }
                    if ui.small_button("↓").on_hover_text("Next").clicked() {
                        step = Some(true);
                    }
                });

                if self.find.searched != self.find.query {
                    self.refresh_find();
                }

                let count = self.find.matches.len();
                let shown = self.find.current.map_or(0, |index| index + 1);
                let status = if count == 0 && !self.find.query.is_empty() {
                    "No matches".to_string()
                } else {
                    format!("{shown} of {count}")
                };
                ui.label(egui::RichText::new(status).weak());
            });

        if enter {
            step = Some(!shift);
        }
        if let Some(forward) = step {
            // A query typed this frame has matches only after the refresh.
            if self.find.searched != self.find.query {
                self.refresh_find();
            }
            self.step_find(forward);
        }
    }

    /// Handles a press or drag on the scrollbar, returning whether it took one.
    pub(crate) fn handle_scrollbar(&mut self, input: &PointerInput, area: egui::Rect) -> bool {
        let (document, marks) = self.control.scroll_facts();
        let visible = self.control.size().1;
        if document <= visible {
            self.scrollbar.release();
            return false;
        }

        let track = scrollbar_track(area);
        if track.height() <= 0.0 {
            return false;
        }
        let fraction = |position: egui::Pos2| (position.y - track.min.y) / track.height();

        let mut handled = false;
        if input.primary_pressed
            && let Some(hover) = input.hover
            && track.contains(hover)
        {
            let pressed = fraction(hover).clamp(0.0, 1.0);
            let positions: Vec<f32> = marks
                .iter()
                .map(|mark| scrollbar::position(mark.row, document))
                .collect();

            if let Some(index) = scrollbar::tick_at(&positions, pressed, MARK_GRAB / track.height()) {
                // A prompt tick is a precise target: it brings the prompt into
                // view rather than centring the clicked fraction.
                self.view.scroll_to_row(marks[index].row, visible, document);
            } else {
                let press = self.scrollbar.press(pressed, self.view.scroll(), document, visible);
                if let scrollbar::Press::Jump(scroll) = press {
                    self.view.set_scroll(scroll, document - visible);
                }
            }
            handled = true;
        } else if self.scrollbar.dragging() && input.primary_down {
            if let Some(hover) = input.hover
                && let Some(scroll) = self.scrollbar.drag(fraction(hover), document, visible)
            {
                self.view.set_scroll(scroll, document - visible);
            }
            handled = true;
        }

        if input.primary_released {
            let was_dragging = self.scrollbar.dragging();
            self.scrollbar.release();
            handled |= was_dragging;
        }

        handled
    }

    /// Draws the bar over the pane's right edge.
    pub(crate) fn paint_scrollbar(&self, painter: &egui::Painter, area: egui::Rect, screen: &grid::Screen) {
        let track = scrollbar_track(area);
        if track.height() <= 0.0 {
            return;
        }

        painter.rect_filled(track, 0, scrollbar_track_color());

        for mark in &screen.marks {
            let y = track.min.y + scrollbar::position(mark.row, screen.document_height) * track.height();
            let color = if mark.error {
                scrollbar_error_color()
            } else {
                scrollbar_mark_color()
            };
            painter.line_segment(
                [egui::pos2(track.min.x, y), egui::pos2(track.max.x, y)],
                egui::Stroke::new(1.5, color),
            );
        }

        let (top, height) = scrollbar::thumb(self.view.scroll(), screen.document_height, screen.height);
        let height = (height * track.height()).max(scrollbar::MIN_THUMB).min(track.height());
        let top = (top * track.height()).min(track.height() - height);
        let thumb = egui::Rect::from_min_size(
            egui::pos2(track.min.x + 1.0, track.min.y + top),
            egui::vec2(track.width() - 2.0, height),
        );
        painter.rect_filled(thumb, 2, scrollbar_thumb_color());
    }

    /// Adds the pane's grid size to the pane's own rect.
    pub(crate) fn fit(&mut self, rect: egui::Rect, cell: egui::Vec2) {
        let columns = ((rect.width() / cell.x).floor() as i32).max(1);
        let rows = ((rect.height() / cell.y).floor() as i32).max(1);
        if (columns, rows) != self.control.size() {
            self.resize(columns, rows);
        }
    }

    pub(crate) fn paint(&mut self, ui: &egui::Ui, area: egui::Rect, cursor_on: bool, paint: GridPaint) {
        let GridPaint {
            cell,
            font_size,
            faces,
            now,
        } = paint;
        let screen = self.control.snapshot(&self.view);
        let painter = ui.painter().with_clip_rect(area);

        if cell.x <= 0.0 || cell.y <= 0.0 {
            return;
        }

        let table = self.control.color_table();
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

        if self.find.open && !self.find.matches.is_empty() {
            let fill = table
                .get(SELECTION_SLOT)
                .and_then(egui_color)
                .unwrap_or(FALLBACK_SELECTION);
            let dim = egui::Color32::from_rgba_unmultiplied(fill.r(), fill.g(), fill.b(), 90);
            for (index, found) in self.find.matches.iter().enumerate() {
                // Matches are in document coordinates, like the selection; one
                // that runs through an autowrap covers more than one row.
                let first = (found.start_row - screen.top).max(0);
                let last = (found.end_row - screen.top).min(screen.height - 1);
                for screen_row in first..=last {
                    let Some((column, cells)) = found.row_span(screen.top + screen_row, screen.width) else {
                        continue;
                    };
                    let column = column.clamp(0, (screen.width - 1).max(0));
                    let cells = cells.min(screen.width - column).max(1);
                    let rect = egui::Rect::from_min_size(
                        egui::pos2(
                            area.min.x + column as f32 * cell.x,
                            area.min.y + screen_row as f32 * cell.y,
                        ),
                        egui::vec2(cells as f32 * cell.x, cell.y),
                    );
                    let color = if self.find.current == Some(index) { fill } else { dim };
                    painter.rect_filled(rect, 0, color);
                }
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

        if screen.document_height > screen.height {
            self.paint_scrollbar(&painter, area, &screen);
        }

        // The bell's flash covers the pane, text and all, and is drawn last so
        // nothing paints over it.
        if let Some(alpha) = self.control.bell_alpha(now) {
            let value = (alpha.clamp(0.0, 1.0) * 255.0).round() as u8;
            painter.rect_filled(area, 0, egui::Color32::from_rgba_unmultiplied(255, 255, 255, value));
        }
    }
}

/// The strip the scrollbar occupies on the pane's right edge.
pub(crate) fn scrollbar_track(area: egui::Rect) -> egui::Rect {
    egui::Rect::from_min_max(egui::pos2(area.max.x - scrollbar::WIDTH, area.min.y), area.max)
}

pub(crate) fn scrollbar_track_color() -> egui::Color32 {
    egui::Color32::from_rgba_unmultiplied(0, 0, 0, 90)
}

pub(crate) fn scrollbar_thumb_color() -> egui::Color32 {
    egui::Color32::from_rgba_unmultiplied(160, 160, 160, 200)
}

pub(crate) fn scrollbar_mark_color() -> egui::Color32 {
    egui::Color32::from_rgba_unmultiplied(110, 170, 255, 220)
}

pub(crate) fn scrollbar_error_color() -> egui::Color32 {
    egui::Color32::from_rgba_unmultiplied(235, 90, 90, 235)
}

/// The modifier sides egui collapses but the encoder distinguishes.
#[derive(Debug, Clone, Copy, Default)]
pub(crate) struct ModifierSides {
    pub(crate) right_alt: bool,
    pub(crate) right_ctrl: bool,
}

/// Records which side of a modifier key an event touched.
pub(crate) fn track_modifier(sides: &mut ModifierSides, key: egui::Key, pressed: bool) {
    match key {
        egui::Key::AltRight => sides.right_alt = pressed,
        egui::Key::ControlRight => sides.right_ctrl = pressed,
        _ => {}
    }
}

/// Resolves a cell's foreground and background the way the renderer does.
pub(crate) fn attribute_colors(attribute: &TextAttribute, table: &ColorTable) -> (egui::Color32, egui::Color32) {
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
pub(crate) struct PointerInput {
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
pub(crate) const fn screen_to_document(history: i32, scroll: i32, screen_row: i32) -> i32 {
    history - scroll + screen_row
}

/// Returns whether the application, rather than the selection, gets the mouse.
///
/// Shift is the user's override: it selects even while an application tracks
/// the mouse, which is how a person copies text out of a full-screen program.
pub(crate) const fn forwards_to_application(tracking: bool, shift: bool) -> bool {
    tracking && !shift
}

/// Maps the pointer's buttons to the protocol's set of held buttons.
pub(crate) const fn mouse_buttons(primary: bool, middle: bool, secondary: bool) -> MouseButtons {
    MouseButtons {
        left: primary,
        middle,
        right: secondary,
    }
}

/// Draws the cursor in the shape the terminal asked for.
pub(crate) fn paint_cursor(
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

pub(crate) fn paint_cursor_bar(
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

/// Resolves a terminal colour through the adapter's palette.
pub(crate) fn resolve(color: TextColor, table: &ColorTable, alias: ColorAlias) -> Option<egui::Color32> {
    let entry = match color.get_type() {
        ColorType::Rgb { .. } => color,
        ColorType::Index16(_) | ColorType::Index256(_) => table.get(usize::from(color.get_index()))?,
        ColorType::Default => table.get(table.alias_index(alias))?,
    };

    egui_color(entry)
}

/// The egui colour an RGB terminal colour names.
pub(crate) fn egui_color(color: TextColor) -> Option<egui::Color32> {
    match color.get_type() {
        ColorType::Rgb { r, g, b } => Some(egui::Color32::from_rgb(r, g, b)),
        _ => None,
    }
}

/// Black or white, whichever reads on `fill`.
///
/// A theme names a background and no foreground, so the tab label has to pick
/// a side. The weights are the usual luminance ones; the cut-off is where a
/// mid grey stops reading as dark.
pub(crate) fn contrasting_text(fill: egui::Color32) -> egui::Color32 {
    let luminance = 0.299 * f32::from(fill.r()) + 0.587 * f32::from(fill.g()) + 0.114 * f32::from(fill.b());
    if luminance > 140.0 {
        egui::Color32::from_gray(20)
    } else {
        egui::Color32::from_gray(230)
    }
}

/// Maps an egui key to the key's identity in `term`.
///
/// Punctuation is named by the character it produces, which differs from the
/// US-layout base key `term` documents; the difference only matters for a
/// Ctrl chord on punctuation, where the text event is suppressed.
pub(crate) fn keymap(key: egui::Key) -> Option<Key> {
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
pub(crate) fn key_modifiers(modifiers: egui::Modifiers, right_alt: bool, right_ctrl: bool) -> KeyModifiers {
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
