//! The WinUI 3 shell: a tab strip, a native window, the renderer's pixels.
//!
//! The component owns one terminal per tab, the renderer and the swap-chain
//! surface. A timer polls every pty on the UI thread (the connection is not
//! `Send`, so it cannot be moved to `spawn_background`), paints the active tab
//! and presents it through [`Surface`] (ADRs 006, 021, 040, 041).
//!
//! The panel reports its size in DIPs and its composition scale; the grid is
//! however many whole cells fit, and the face is rasterised at the scale so a
//! high-DPI monitor gets physical pixels rather than a stretched image.

use std::cell::RefCell;
use std::path::{Path, PathBuf};
use std::rc::Rc;
use std::time::{Duration, Instant};

use renderer::renderer::Renderer;
use renderer::{
    AtlasEngine, CellSizeInDIP, FontDueRasterizer, FontInfo, RenderSettings, Settings, SharedBackend, SoftwareBackend,
};
use server::pty::{PtyCommand, PtySize};
use term::input::{Key, MouseButton, MouseEvent, MouseEventKind};
use types::color_table::ColorAlias;
use utils::{InclusiveRect, Size};
use windows_reactor::*;

use crate::input;
use crate::session::Session;
use crate::tabs::Tabs;
use crate::term_control::Control;
use crate::term_events::TerminalEvent;
use crate::term_settings::TerminalSettings;
use crate::terminal_render::TerminalData;
use crate::view::View as ViewState;

use super::surface::Surface;

/// The size the window opens at, before the panel reports its own.
const INITIAL_COLUMNS: i32 = 120;
const INITIAL_ROWS: i32 = 30;
/// Far beyond any display, and it keeps a bad metric from asking the terminal
/// for an allocation it can never fill.
const MAX_CELLS: i32 = 1024;

/// Runs the WinUI 3 shell until its window closes.
pub fn run() -> windows_core::Result<()> {
    App::run_component::<Shell>(())
}

/// One tab: its own terminal, selection and title.
struct Tab {
    /// Stable across reorders, because the toolkit's close event names it.
    id: u64,
    control: Control,
    view: ViewState,
    title: String,
}

/// The application's state and its messages.
struct Shell {
    tabs: Tabs<Tab>,
    /// The id the next tab is given.
    next_tab: u64,
    /// The profile a new tab starts from.
    template: TerminalSettings,
    /// The window chords, matched before the terminal sees a key.
    bindings: Rc<[settings::Keybinding]>,
    renderer: Renderer<TerminalData>,
    shared: SharedBackend,
    panel: ElementRef<SwapChainPanel>,
    focus: ElementRef<Border>,
    surface: Rc<RefCell<Option<Surface>>>,
    _observation: ElementObservation,
    timer: Option<ComponentTimer>,
    sides: input::ModifierSides,
    blink_on: bool,
    blink_at: Instant,
    focused: bool,
    /// Whether the first focus request already succeeded, so a tab click is not
    /// immediately undone by the next tick.
    focus_attempted: bool,
    /// The cell a forwarded move last reported, so a still pointer is silent.
    last_mouse_cell: Option<(i32, i32)>,
    face: PathBuf,
    /// The face size in DIPs, from the settings file.
    font_size: f32,
    /// The cell size in physical pixels at the current composition scale.
    cell: (u16, u16),
    /// The composition scale the face was rasterised for.
    scale: f32,
    /// The grid in cells.
    grid: (i32, i32),
    /// The window's opening client size in DIPs. It is a constant, so the
    /// declaration only resizes the window once and never fights the user.
    initial_client: (f64, f64),
}

/// What the window sends back to the component.
///
/// The variants carry the toolkit's event records; the timer carries nothing,
/// which is what lets `set_timeout` stay `Send`.
enum Message {
    Tick,
    Key(KeyEventInfo),
    Char(CharacterEventInfo),
    Resize {
        width: f64,
        height: f64,
        scale_x: f32,
        scale_y: f32,
    },
    /// The tab strip selected `index`, or nothing.
    Select(Option<usize>),
    NewTab,
    /// The user asked for the tab with this id to close.
    CloseTab(u64),
    /// The tab strip's new order, by id.
    Reorder(Vec<u64>),
    Focused(bool),
    Pressed(PointerEventInfo),
    Moved(PointerEventInfo),
    Released(PointerEventInfo),
}

impl Component for Shell {
    type Input = ();
    type Message = Message;

    fn create(_input: &(), context: &ComponentContext<Self>) -> Self {
        let grid = (INITIAL_COLUMNS, INITIAL_ROWS);
        let size = PtySize::new(INITIAL_COLUMNS as u16, INITIAL_ROWS as u16);

        // A fresh installation is given a file to edit, as the egui shell does
        // (ADR 034). A broken file still yields the defaults.
        settings::ensure_file();
        let loaded = settings::load_with_report();
        let template = TerminalSettings::resolve(&loaded.settings);
        let font_size = loaded.settings.font.size;

        let session = spawn_shell(&template.command, size).expect("a shell");
        let mut control = Control::new(Box::new(session), grid);
        control.apply_scheme(template.scheme.as_ref());
        let tab = Tab {
            id: 0,
            control,
            view: ViewState::new(),
            title: template.title.clone(),
        };

        let face = face_path();
        let fonts = font_setup(&face, font_size, 1.0, grid);
        let cell = fonts.settings.font.cell_size;
        let (renderer, shared) = renderer_for(fonts, grid);

        let panel = ElementRef::<SwapChainPanel>::new();
        let surface = Rc::new(RefCell::new(None::<Surface>));
        let observation = {
            let bound = panel.clone();
            let surface = Rc::clone(&surface);
            let sender = context.sender();
            panel.observe_surface_metrics(move |event| {
                let SwapChainPanelEvent::Metrics {
                    width,
                    height,
                    scale_x,
                    scale_y,
                    ..
                } = event
                else {
                    return;
                };
                // The panel reports zero before its first layout pass.
                if width < 1.0 || height < 1.0 {
                    return;
                }

                let target = physical_size(width, height, scale_x, scale_y);
                {
                    let mut slot = surface.borrow_mut();
                    if let Some(surface) = slot.as_mut() {
                        let _ = surface.resize(target.0, target.1);
                        let _ = surface.set_scale(scale_x, scale_y);
                    } else if let Ok(mut created) = Surface::new(target.0, target.1) {
                        let _ = created.set_scale(scale_x, scale_y);
                        let _ = bound.request_set_swap_chain(created.swap_chain(), |_| {});
                        *slot = Some(created);
                    }
                }

                let _ = sender.send(Message::Resize {
                    width,
                    height,
                    scale_x,
                    scale_y,
                });
            })
        };

        let timer = Some(context.set_timeout(Duration::from_millis(16), Message::Tick));

        Self {
            tabs: Tabs::new(tab),
            next_tab: 1,
            template,
            bindings: loaded.settings.keybindings.clone().into(),
            renderer,
            shared,
            panel,
            focus: ElementRef::<Border>::new(),
            surface,
            _observation: observation,
            timer,
            sides: input::ModifierSides::default(),
            blink_on: true,
            blink_at: Instant::now(),
            focused: false,
            focus_attempted: false,
            last_mouse_cell: None,
            face,
            font_size,
            cell,
            scale: 1.0,
            grid,
            initial_client: (
                f64::from(cell.0) * f64::from(INITIAL_COLUMNS as u16),
                f64::from(cell.1) * f64::from(INITIAL_ROWS as u16),
            ),
        }
    }

    fn view(&self, _input: &(), context: &mut ViewContext<Self>) -> View {
        let keys = context.sender();
        let characters = keys.clone();
        let add = keys.clone();
        let select = keys.clone();
        let close = keys.clone();
        let reorder = keys.clone();
        let got = keys.clone();
        let lost = keys.clone();
        let pressed = keys.clone();
        let moved = keys.clone();
        let released = keys.clone();
        let bindings = Rc::clone(&self.bindings);

        let items: Vec<KeyedView> = (0..self.tabs.len())
            .filter_map(|index| self.tabs.get(index))
            .map(|tab| {
                KeyedView::new(
                    tab_key(tab.id),
                    TabViewItem::new().header(tab.title.clone()).is_closable(true),
                )
            })
            .collect();

        let strip = TabView::new()
            .is_add_tab_button_visible(true)
            .can_reorder_tabs(true)
            .selected_index(Some(self.tabs.index()))
            .keyed_tab_items(items)
            .on_add_tab_button_click(move || {
                let _ = add.send(Message::NewTab);
            })
            .on_selection_changed(move |index| {
                let _ = select.send(Message::Select(index));
            })
            .on_close_requested(move |key: windows_reactor::Key| {
                if let Some(id) = key.as_str().and_then(tab_id) {
                    let _ = close.send(Message::CloseTab(id));
                }
            })
            .on_reordered(move |keys: Vec<windows_reactor::Key>| {
                let ids = keys.iter().filter_map(|key| key.as_str().and_then(tab_id)).collect();
                let _ = reorder.send(Message::Reorder(ids));
            });

        let pane = Border::new()
            .is_tab_stop(true)
            .capture_pointer_on_press(true)
            .focus_on_pointer_release(true)
            .element_ref(&self.focus)
            .background(Color::rgb(12, 12, 12))
            .content(SwapChainPanel::new().element_ref(&self.panel))
            .on_preview_key_down(RoutedCallback::new(move |info| {
                let _ = keys.send(Message::Key(info));
                // A bound chord is the window's, so the toolkit must not also
                // type it. A printable key with no binding still has to raise
                // `CharacterReceived`, which is the text path; the keys the
                // terminal encodes itself are marked handled.
                let bound = key_of(info.key)
                    .and_then(|key| chord_action(&bindings, key, info.modifiers))
                    .is_some();
                bound || !matches!(key_of(info.key), Some(Key::Char(_) | Key::Space))
            }))
            .on_character_received(RoutedCallback::new(move |info| {
                let _ = characters.send(Message::Char(info));
                true
            }))
            .on_pointer_pressed(move |info: PointerEventInfo| {
                let _ = pressed.send(Message::Pressed(info));
            })
            .on_pointer_moved(move |info: PointerEventInfo| {
                let _ = moved.send(Message::Moved(info));
            })
            .on_pointer_released(move |info: PointerEventInfo| {
                let _ = released.send(Message::Released(info));
            })
            .on_got_focus(move |_| {
                let _ = got.send(Message::Focused(true));
            })
            .on_lost_focus(move |_| {
                let _ = lost.send(Message::Focused(false));
            });

        context.window_title(self.title());
        context.window_visuals(
            WindowVisuals::new()
                .theme(WindowTheme::Dark)
                .backdrop(WindowBackdrop::Mica)
                .client_size(self.initial_client.0, self.initial_client.1),
        );

        // The tab strip sizes to its own strip, and the pane takes the rest; the
        // panel stays mounted, so switching tabs never rebuilds the surface.
        Grid::new()
            .rows([GridLength::Auto, GridLength::STAR])
            .children((strip, pane.grid_row(1)))
            .into()
    }

    fn update(&mut self, message: Message, context: &ComponentContext<Self>) {
        match message {
            Message::Tick => self.tick(context),
            Message::Key(info) => self.key(info, context),
            Message::Char(info) => self.character(info),
            Message::Resize {
                width,
                height,
                scale_x,
                scale_y,
            } => self.resize(width, height, scale_x, scale_y),
            Message::Select(index) => self.select(index),
            Message::NewTab => self.open_tab(),
            Message::CloseTab(id) => self.close_tab(id, context),
            Message::Reorder(ids) => self.reorder(&ids),
            Message::Focused(focused) => self.focused = focused,
            Message::Pressed(info) => self.pointer_pressed(info),
            Message::Moved(info) => self.pointer_moved(info),
            Message::Released(info) => self.pointer_released(info),
        }
    }
}

impl Shell {
    /// Pumps every pty, paints the active tab and re-arms the timer.
    fn tick(&mut self, context: &ComponentContext<Self>) {
        let now = Instant::now();
        let mut exited = Vec::new();
        for index in 0..self.tabs.len() {
            let Some(tab) = self.tabs.get_mut(index) else {
                continue;
            };
            tab.control.pump();

            let mut done = false;
            for event in tab.control.take_events(now) {
                if event == TerminalEvent::Exited {
                    done = true;
                }
            }

            if let Some(title) = tab.control.title().map(str::to_string)
                && tab.title != title
            {
                tab.title = title;
            }

            if done {
                exited.push(index);
            }
        }

        // Closing from the end keeps the earlier indices valid.
        for index in exited.into_iter().rev() {
            if !self.tabs.close(index) {
                let _ = context.close_window();
                return;
            }
        }

        if now.duration_since(self.blink_at) >= Duration::from_millis(500) {
            self.blink_on = !self.blink_on;
            self.blink_at = now;
        }

        // One attempt is enough: after that, the pane takes focus on a click and
        // a tab selection asks for it back, so this cannot fight the strip.
        if !self.focused && !self.focus_attempted {
            self.focused = self.focus.request_focus();
            self.focus_attempted = self.focused;
        }

        let blink_on = self.blink_on;
        let (painted, background) = {
            let Shell { renderer, tabs, .. } = self;
            let Some(tab) = tabs.active_mut() else {
                return;
            };

            renderer.data_mut().refresh(&mut tab.control, &tab.view, blink_on);

            // The engine resolves a cell's colours through the renderer's table,
            // not the data's copy, so the shell hands it the pane's palette
            // (`RenderSettings::set_color_table`), and the pane's own colours
            // become the engine's defaults.
            let (background, foreground) = default_colors(&tab.control);
            let _ = renderer.set_default_colors(background, foreground);
            sync_palette(renderer, &tab.control);

            renderer.invalidate_all();
            (renderer.paint_frame().is_ok(), background)
        };

        if painted {
            let width = self.shared.width();
            let height = self.shared.height();
            let surface = Rc::clone(&self.surface);
            self.shared.with_pixels(|pixels| {
                if let Some(surface) = surface.borrow_mut().as_mut() {
                    let _ = surface.present(pixels, width, height, background);
                }
            });
        }

        self.timer = Some(context.set_timeout(Duration::from_millis(16), Message::Tick));
    }

    /// Fits the grid to the panel and rasterises the face for its scale.
    fn resize(&mut self, width: f64, height: f64, scale_x: f32, scale_y: f32) {
        let scale_x = finite_scale(scale_x);
        let scale_y = finite_scale(scale_y);

        let rescaled = (scale_x - self.scale).abs() > f32::EPSILON;
        if rescaled {
            self.scale = scale_x;
            self.rebuild_face(scale_x);
        }

        let grid = grid_for(width, height, self.cell, scale_x, scale_y);
        if rescaled || grid != self.grid {
            self.apply_grid(grid);
        }
    }

    /// Replaces the renderer with one whose face is `scale` times the DIP size.
    ///
    /// The pane's colours are re-applied on the next tick, from the active tab's
    /// palette, so a reloaded face never falls back to the built-in table.
    fn rebuild_face(&mut self, scale: f32) {
        let fonts = font_setup(&self.face, self.font_size, scale, self.grid);
        self.cell = fonts.settings.font.cell_size;
        let (renderer, shared) = renderer_for(fonts, self.grid);
        self.renderer = renderer;
        self.shared = shared;
    }

    /// Tells every tab's terminal and the renderer how large the grid is.
    fn apply_grid(&mut self, (columns, rows): (i32, i32)) {
        self.grid = (columns, rows);
        for index in 0..self.tabs.len() {
            if let Some(tab) = self.tabs.get_mut(index) {
                tab.control.resize(columns, rows);
            }
        }

        let target = target_size(self.cell, self.grid);
        let viewport = InclusiveRect::new(0, 0, columns.max(1) - 1, rows.max(1) - 1);
        let _ = self.renderer.resize(target, viewport);
    }

    /// The title the window shows: the active tab's.
    fn title(&self) -> &str {
        self.tabs.active().map_or("winterm", |tab| tab.title.as_str())
    }

    /// Opens a tab on the same profile and activates it.
    fn open_tab(&mut self) {
        let size = pty_size(self.grid);
        let Ok(session) = spawn_shell(&self.template.command, size) else {
            return;
        };

        let mut control = Control::new(Box::new(session), self.grid);
        control.apply_scheme(self.template.scheme.as_ref());

        let id = self.next_tab;
        self.next_tab += 1;
        self.tabs.add(Tab {
            id,
            control,
            view: ViewState::new(),
            title: self.template.title.clone(),
        });
        let _ = self.focus.request_focus();
    }

    /// Activates the tab the strip selected.
    fn select(&mut self, index: Option<usize>) {
        let Some(index) = index else {
            return;
        };
        if self.tabs.select(index) {
            let _ = self.focus.request_focus();
        }
    }

    /// Closes the tab `id`, or the window when it was the last one.
    fn close_tab(&mut self, id: u64, context: &ComponentContext<Self>) {
        let Some(index) = (0..self.tabs.len()).find(|&index| self.tabs.get(index).is_some_and(|tab| tab.id == id))
        else {
            return;
        };

        if !self.tabs.close(index) {
            let _ = context.close_window();
        } else {
            let _ = self.focus.request_focus();
        }
    }

    /// Applies the strip's new order, which names tabs by id.
    fn reorder(&mut self, ids: &[u64]) {
        let order: Vec<usize> = ids
            .iter()
            .filter_map(|id| (0..self.tabs.len()).find(|&index| self.tabs.get(index).is_some_and(|tab| tab.id == *id)))
            .collect();
        if order.len() == self.tabs.len() {
            let _ = self.tabs.reorder(&order);
        }
    }

    /// Runs a bound chord, or turns the key into the active terminal's encoding.
    fn key(&mut self, info: KeyEventInfo, context: &ComponentContext<Self>) {
        let Some(key) = key_of(info.key) else {
            return;
        };
        let pressed = !info.status.is_released;

        // A bound chord is the window's: the shell consumes it before the tab
        // sees the event (ADR 023).
        if pressed && let Some(action) = chord_action(&self.bindings, key, info.modifiers) {
            self.perform(action, context);
            return;
        }

        let event = input::Event::Key {
            key,
            pressed,
            repeat: info.status.was_down && pressed,
            modifiers: modifiers(info.modifiers),
        };

        let outcome = input::translate(&event, &mut self.sides);
        let Some(tab) = self.tabs.active_mut() else {
            return;
        };
        for key in outcome.keys {
            tab.control.send_key(&key);
        }
    }

    /// Hands composed text to the active terminal's text path.
    fn character(&mut self, info: CharacterEventInfo) {
        let Some(character) = char::from_u32(u32::from(info.character)) else {
            return;
        };
        if character.is_control() {
            return;
        }

        let outcome = input::translate(&input::Event::Text(character.to_string()), &mut self.sides);
        let Some(tab) = self.tabs.active_mut() else {
            return;
        };
        for key in outcome.keys {
            tab.control.send_key(&key);
        }
    }

    /// Runs the window action a chord asked for.
    fn perform(&mut self, action: settings::Action, context: &ComponentContext<Self>) {
        match action {
            settings::Action::NewTab => self.open_tab(),
            settings::Action::CloseTab => {
                if let Some(id) = self.tabs.active().map(|tab| tab.id) {
                    self.close_tab(id, context);
                }
            }
            settings::Action::NextTab => {
                self.tabs.step(1);
                let _ = self.focus.request_focus();
            }
            settings::Action::PrevTab => {
                self.tabs.step(-1);
                let _ = self.focus.request_focus();
            }
            settings::Action::Copy => {
                let _ = self.copy_selection();
            }
            settings::Action::Paste => self.paste_clipboard(),
            // Panes, find, the palette and the settings window are not this
            // shell's yet, so their chords fall through.
            _ => {}
        }
    }

    /// Puts the selection on the clipboard.
    fn copy_selection(&mut self) -> bool {
        let Some(tab) = self.tabs.active_mut() else {
            return false;
        };
        let Some(selection) = tab.view.selection().copied() else {
            return false;
        };
        let Some(text) = tab.control.selection_text(&selection) else {
            return false;
        };

        arboard::Clipboard::new()
            .and_then(|mut clipboard| clipboard.set_text(text))
            .is_ok()
    }

    /// Sends the clipboard through the terminal's bracketed-paste filter.
    fn paste_clipboard(&mut self) {
        let Ok(mut clipboard) = arboard::Clipboard::new() else {
            return;
        };
        let Ok(text) = clipboard.get_text() else {
            return;
        };
        if let Some(tab) = self.tabs.active_mut() {
            tab.control.paste(&text);
        }
    }

    /// Whether the active terminal tracks the mouse.
    fn active_tracking(&self) -> bool {
        self.tabs.active().is_some_and(|tab| tab.control.is_tracking_mouse())
    }

    /// Starts a selection, or forwards the press to a tracking application.
    fn pointer_pressed(&mut self, info: PointerEventInfo) {
        let shift = info.modifiers.contains(InputModifiers::SHIFT);
        let forward = input::forwards_to_application(self.active_tracking(), shift);
        let cell = pointer_cell(info.x, info.y, self.grid, self.cell, self.scale);

        let Some(tab) = self.tabs.active_mut() else {
            return;
        };

        if forward {
            if let Some(position) = cell {
                let buttons = input::mouse_buttons(
                    info.is_left_button_pressed,
                    info.is_middle_button_pressed,
                    info.is_right_button_pressed,
                );
                let chord =
                    input::key_modifiers(modifiers(info.modifiers), self.sides.right_alt, self.sides.right_ctrl);
                let event = MouseEvent {
                    position,
                    kind: MouseEventKind::Press(MouseButton::Left),
                    buttons,
                    modifiers: chord,
                    delta: 0,
                };
                tab.control.send_mouse(&event);
                self.last_mouse_cell = Some(position);
            }
            return;
        }

        let Some((column, row)) = cell else {
            return;
        };

        // Selection is in document coordinates: history first, then the screen.
        let history = tab.control.max_scroll();
        let position = (column, input::screen_to_document(history, tab.view.scroll(), row));

        let count = tab.view.click(position, Instant::now());
        if count >= 3 {
            let width = tab.control.screen_width();
            tab.view.select_line(position.1, width);
        } else if count == 2 {
            let text = tab.control.row_text(position.1);
            tab.view.select_word(&text, position);
        } else {
            tab.view.select(position, modifiers(info.modifiers).alt);
        }
    }

    /// Extends a selection, scrolls a drag past the pane, or forwards the move.
    fn pointer_moved(&mut self, info: PointerEventInfo) {
        let (columns, rows) = self.grid;
        let cell = pointer_cell(info.x, info.y, self.grid, self.cell, self.scale);
        let shift = info.modifiers.contains(InputModifiers::SHIFT);
        let forward = input::forwards_to_application(self.active_tracking(), shift);

        let scale = f64::from(finite_scale(self.scale));
        let width = f64::from(self.cell.0) / scale;
        let height = f64::from(self.cell.1) / scale;
        let outside = if info.y < 0.0 {
            -1
        } else if height > 0.0 && info.y >= f64::from(rows.max(0)) * height {
            1
        } else {
            0
        };
        let column = if width > 0.0 {
            (info.x / width).floor() as i32
        } else {
            0
        };

        let Some(tab) = self.tabs.active_mut() else {
            return;
        };

        if forward {
            if let Some(position) = cell {
                let buttons = input::mouse_buttons(
                    info.is_left_button_pressed,
                    info.is_middle_button_pressed,
                    info.is_right_button_pressed,
                );
                let dragging = buttons.left || buttons.middle || buttons.right;
                if self.last_mouse_cell != Some(position) || dragging {
                    let chord =
                        input::key_modifiers(modifiers(info.modifiers), self.sides.right_alt, self.sides.right_ctrl);
                    let event = MouseEvent {
                        position,
                        kind: MouseEventKind::Move,
                        buttons,
                        modifiers: chord,
                        delta: 0,
                    };
                    tab.control.send_mouse(&event);
                    self.last_mouse_cell = Some(position);
                }
            } else {
                self.last_mouse_cell = None;
            }
            return;
        }

        if !info.is_left_button_pressed || tab.view.selection().is_none() {
            return;
        }

        if outside != 0 {
            // A drag that leaves the pane claims the edge row and scrolls one
            // row towards the pointer, so the selection reaches into history.
            let max = tab.control.max_scroll();
            tab.view.drag_scroll(outside, max, Instant::now());
            let row = if outside < 0 { 0 } else { rows.max(1) - 1 };
            let column = column.clamp(0, columns.max(1) - 1);
            let document = input::screen_to_document(max, tab.view.scroll(), row);
            tab.view.extend((column, document));
        } else if let Some((column, row)) = cell {
            let history = tab.control.max_scroll();
            let document = input::screen_to_document(history, tab.view.scroll(), row);
            tab.view.extend((column, document));
        }
    }

    /// Finishes a forwarded press.
    fn pointer_released(&mut self, info: PointerEventInfo) {
        let shift = info.modifiers.contains(InputModifiers::SHIFT);
        if !input::forwards_to_application(self.active_tracking(), shift) {
            self.last_mouse_cell = None;
            return;
        }

        let cell = pointer_cell(info.x, info.y, self.grid, self.cell, self.scale);
        let Some(tab) = self.tabs.active_mut() else {
            return;
        };

        if let Some(position) = cell {
            let buttons = input::mouse_buttons(
                info.is_left_button_pressed,
                info.is_middle_button_pressed,
                info.is_right_button_pressed,
            );
            let chord = input::key_modifiers(modifiers(info.modifiers), self.sides.right_alt, self.sides.right_ctrl);
            let event = MouseEvent {
                position,
                kind: MouseEventKind::Release(MouseButton::Left),
                buttons,
                modifiers: chord,
                delta: 0,
            };
            tab.control.send_mouse(&event);
        }
        self.last_mouse_cell = None;
    }
}

/// Starts a shell, falling back to what every Windows install has.
fn spawn_shell(command: &PtyCommand, size: PtySize) -> std::io::Result<Session> {
    Session::spawn(command, size)
        .or_else(|_| Session::spawn(&PtyCommand::new("powershell.exe"), size))
        .or_else(|_| Session::spawn(&PtyCommand::new("cmd.exe"), size))
}

/// The pty size a `grid` asks for.
fn pty_size(grid: (i32, i32)) -> PtySize {
    let side = |cells: i32| u16::try_from(cells.max(1)).unwrap_or(u16::MAX);
    PtySize::new(side(grid.0), side(grid.1))
}

/// The key a tab item is declared with.
fn tab_key(id: u64) -> String {
    format!("tab-{id}")
}

/// The tab id a key names, if it names one.
fn tab_id(key: &str) -> Option<u64> {
    key.strip_prefix("tab-")?.parse().ok()
}

/// The settings vocabulary key for a terminal key, if it has one.
fn settings_key(key: Key) -> Option<settings::Key> {
    Some(match key {
        Key::Char(character) => settings::Key::Char(character.to_ascii_lowercase()),
        Key::Tab => settings::Key::Tab,
        Key::Enter => settings::Key::Enter,
        Key::Escape => settings::Key::Escape,
        Key::Space => settings::Key::Space,
        Key::Backspace => settings::Key::Backspace,
        Key::Insert => settings::Key::Insert,
        Key::Delete => settings::Key::Delete,
        Key::Home => settings::Key::Home,
        Key::End => settings::Key::End,
        Key::PageUp => settings::Key::PageUp,
        Key::PageDown => settings::Key::PageDown,
        Key::Left => settings::Key::Left,
        Key::Right => settings::Key::Right,
        Key::Up => settings::Key::Up,
        Key::Down => settings::Key::Down,
        Key::Function(number) => settings::Key::Function(number),
        _ => return None,
    })
}

/// The window action a chord names, if the file binds it.
fn chord_action(bindings: &[settings::Keybinding], key: Key, modifiers: InputModifiers) -> Option<settings::Action> {
    let key = settings_key(key)?;
    let chord = settings::Modifiers {
        ctrl: modifiers.contains(InputModifiers::CONTROL),
        shift: modifiers.contains(InputModifiers::SHIFT),
        alt: modifiers.contains(InputModifiers::ALT),
    };

    bindings
        .iter()
        .find(|binding| binding.keys.matches(key, chord))
        .map(|binding| binding.action)
}

/// The grid cell a pane-relative position names, in DIPs.
fn pointer_cell(x: f64, y: f64, grid: (i32, i32), cell: (u16, u16), scale: f32) -> Option<(i32, i32)> {
    let scale = f64::from(finite_scale(scale));
    let width = f64::from(cell.0) / scale;
    let height = f64::from(cell.1) / scale;
    if width <= 0.0 || height <= 0.0 {
        return None;
    }

    let column = (x / width).floor() as i32;
    let row = (y / height).floor() as i32;
    let (columns, rows) = grid;
    if column < 0 || row < 0 || column >= columns || row >= rows {
        return None;
    }
    Some((column, row))
}

/// The face the shell paints with and the settings its atlas needs.
struct Fonts {
    font: FontInfo,
    settings: Settings,
    shaper: FontDueRasterizer,
    rasterizer: FontDueRasterizer,
}

/// Builds a renderer that paints a `grid` through a fresh software backend.
fn renderer_for(fonts: Fonts, grid: (i32, i32)) -> (Renderer<TerminalData>, SharedBackend) {
    let shared = SharedBackend::new(SoftwareBackend::with_rasterizer(Box::new(fonts.rasterizer)));
    let mut engine = AtlasEngine::with_settings(fonts.settings, Box::new(shared.clone()));
    engine.set_shaping(Box::new(fonts.shaper));

    let mut renderer = Renderer::new(
        TerminalData::new(fonts.font, RenderSettings::new(), grid.0, grid.1),
        RenderSettings::new(),
    );
    renderer.add_engine(Box::new(engine));
    (renderer, shared)
}

/// Finds a monospace face and the metrics the atlas paints with at `scale`.
fn font_setup(face: &Path, base_size: f32, scale: f32, grid: (i32, i32)) -> Fonts {
    let scale = finite_scale(scale);
    let size = base_size * scale;

    let shaper = FontDueRasterizer::from_file(face, size).expect("Consolas is installed");
    let rasterizer = FontDueRasterizer::from_file(face, size).expect("Consolas is installed");

    let advance = shaper.advance_width('M');
    let cell_w = (advance.ceil() as u16).max(1);
    let cell_h = (size * 1.25).ceil() as u16;
    let descender = (size * 0.25).ceil().max(1.0) as u16;
    let baseline = cell_h.saturating_sub(descender);

    let mut font = FontInfo::default();
    font.set_from_engine(
        "Consolas".to_string(),
        0,
        400,
        65001,
        size * 0.75,
        CellSizeInDIP {
            width: f32::from(cell_w) / scale,
            height: f32::from(cell_h) / scale,
        },
        Size::new(i32::from(cell_w), i32::from(cell_h)),
    );

    let settings = Settings {
        font: renderer::FontSettings {
            font_name: "Consolas".to_string(),
            font_size: size,
            cell_size: (cell_w, cell_h),
            font_weight: 400,
            advance_width: cell_w,
            baseline,
            descender,
            thin_line_width: scale.round().max(1.0) as u16,
            dpi: (96.0 * scale).round() as u16,
            ..renderer::FontSettings::default()
        },
        misc: renderer::MiscellaneousSettings {
            background_color: 0x000C_0C0C,
            foreground_color: 0x00CC_CCCC,
            ..renderer::MiscellaneousSettings::default()
        },
        target_size: target_size((cell_w, cell_h), grid),
        viewport_cell_count: (
            u16::try_from(grid.0.max(1)).unwrap_or(u16::MAX),
            u16::try_from(grid.1.max(1)).unwrap_or(u16::MAX),
        ),
        ..Settings::default()
    };

    Fonts {
        font,
        settings,
        shaper,
        rasterizer,
    }
}

/// The installed face the shell paints with.
fn face_path() -> PathBuf {
    PathBuf::from(std::env::var_os("WINDIR").unwrap_or_else(|| "C:\\Windows".into()))
        .join("Fonts")
        .join("consola.ttf")
}

/// Hands the renderer the palette the pane paints with.
fn sync_palette(renderer: &mut Renderer<TerminalData>, control: &Control) {
    if let Ok(table) = control.color_table().lock() {
        renderer.settings_mut().set_color_table(table.clone());
    }
}

/// The palette's default background and foreground, as `COLORREF`s.
///
/// The scheme was already applied to the control's table, so reading the table
/// is the one place both shells agree on what the colours are.
fn default_colors(control: &Control) -> (u32, u32) {
    let handle = control.color_table();
    let Ok(table) = handle.lock() else {
        return (0x000C_0C0C, 0x00CC_CCCC);
    };
    let entry = |alias: ColorAlias| {
        table
            .get(table.alias_index(alias))
            .map_or(0, |color| color.get_colorref())
    };
    (
        entry(ColorAlias::DefaultBackground),
        entry(ColorAlias::DefaultForeground),
    )
}

/// A scale the panel reported, or 1.0 when it reported nonsense.
fn finite_scale(scale: f32) -> f32 {
    if scale.is_finite() && scale > 0.0 { scale } else { 1.0 }
}

/// The panel's size in physical pixels, rounded up to at least one.
fn physical_size(width: f64, height: f64, scale_x: f32, scale_y: f32) -> (u32, u32) {
    let width = (width * f64::from(finite_scale(scale_x))).round();
    let height = (height * f64::from(finite_scale(scale_y))).round();
    let clamp = |value: f64| {
        if value.is_finite() {
            // A float-to-integer cast saturates, so a huge panel keeps its
            // scale instead of wrapping.
            (value.max(1.0) as u32).max(1)
        } else {
            1
        }
    };
    (clamp(width), clamp(height))
}

/// How many whole cells of `cell` fit in a panel of `width` by `height` DIPs.
fn grid_for(width: f64, height: f64, cell: (u16, u16), scale_x: f32, scale_y: f32) -> (i32, i32) {
    let columns = width * f64::from(finite_scale(scale_x)) / f64::from(cell.0.max(1));
    let rows = height * f64::from(finite_scale(scale_y)) / f64::from(cell.1.max(1));
    (cells(columns), cells(rows))
}

/// Floors a cell count and keeps it inside what a terminal can hold.
fn cells(count: f64) -> i32 {
    if !count.is_finite() {
        return 1;
    }
    (count.floor() as i64).clamp(1, i64::from(MAX_CELLS)) as i32
}

/// The pixel size of a `grid` of `cell`s, saturated to the settings' `u16`.
fn target_size(cell: (u16, u16), grid: (i32, i32)) -> (u16, u16) {
    let side = |cell: u16, cells: i32| {
        let pixels = u32::from(cell) * u32::try_from(cells.max(1)).unwrap_or(1);
        u16::try_from(pixels).unwrap_or(u16::MAX)
    };
    (side(cell.0, grid.0), side(cell.1, grid.1))
}

/// Maps a Windows virtual-key code to the terminal's key identity.
fn key_of(key: VirtualKey) -> Option<Key> {
    Some(match key.0 {
        0x41..=0x5A => Key::Char((b'a' + (key.0 - 0x41) as u8) as char),
        0x30..=0x39 => Key::Char((b'0' + (key.0 - 0x30) as u8) as char),
        0x08 => Key::Backspace,
        0x09 => Key::Tab,
        0x0D => Key::Enter,
        0x1B => Key::Escape,
        0x20 => Key::Space,
        0x21 => Key::PageUp,
        0x22 => Key::PageDown,
        0x23 => Key::End,
        0x24 => Key::Home,
        0x25 => Key::Left,
        0x26 => Key::Up,
        0x27 => Key::Right,
        0x28 => Key::Down,
        0x2D => Key::Insert,
        0x2E => Key::Delete,
        0x10 | 0xA0 => Key::LeftShift,
        0xA1 => Key::RightShift,
        0x11 | 0xA2 => Key::LeftControl,
        0xA3 => Key::RightControl,
        0x12 | 0xA4 => Key::LeftAlt,
        0xA5 => Key::RightAlt,
        0x5B => Key::LeftSuper,
        0x5C => Key::RightSuper,
        0x14 => Key::CapsLock,
        0x90 => Key::NumLock,
        0x91 => Key::ScrollLock,
        0x2C => Key::PrintScreen,
        0x13 => Key::Pause,
        0x5D => Key::Menu,
        0x70..=0x87 => Key::Function((key.0 - 0x6F) as u8),
        0xBA => Key::Char(';'),
        0xBB => Key::Char('='),
        0xBC => Key::Char(','),
        0xBD => Key::Char('-'),
        0xBE => Key::Char('.'),
        0xBF => Key::Char('/'),
        0xC0 => Key::Char('`'),
        0xDB => Key::Char('['),
        0xDC | 0xE2 => Key::Char('\\'),
        0xDD => Key::Char(']'),
        0xDE => Key::Char('\''),
        _ => return None,
    })
}

/// The toolkit's collapsed modifiers in the shell's vocabulary.
fn modifiers(modifiers: InputModifiers) -> input::Modifiers {
    input::Modifiers {
        shift: modifiers.contains(InputModifiers::SHIFT),
        ctrl: modifiers.contains(InputModifiers::CONTROL),
        alt: modifiers.contains(InputModifiers::ALT),
        command: modifiers.contains(InputModifiers::WINDOWS),
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn a_scheme_reaches_the_pane_and_survives_a_resize() {
        use crate::term_connection::tests::Buffer;

        let text = r##"
            version = 1
            default_scheme = "probe"

            [[profiles]]
            name = "probe"
            command = "cmd.exe"

            [[schemes]]
            name = "probe"
            background = "#201020"
            foreground = "#E0E0E0"
        "##;
        let file = settings::parse(text).expect("settings");
        let terminal = TerminalSettings::resolve(&file);

        let mut control = Control::new(Box::new(Buffer::with_output(b"")), (20, 5));
        control.apply_scheme(terminal.scheme.as_ref());
        assert_eq!(default_colors(&control), (0x0020_1020, 0x00E0_E0E0));

        control.pump();
        assert_eq!(default_colors(&control), (0x0020_1020, 0x00E0_E0E0));

        control.resize(30, 10);
        assert_eq!(default_colors(&control), (0x0020_1020, 0x00E0_E0E0));
    }

    #[test]
    fn the_renderer_is_handed_the_panes_palette() {
        use crate::term_connection::tests::Buffer;

        let file = settings::parse(
            r##"
                version = 1
                default_scheme = "probe"

                [[schemes]]
                name = "probe"
                background = "#201020"
                foreground = "#E0E0E0"
            "##,
        )
        .expect("settings");
        let terminal = TerminalSettings::resolve(&file);

        let mut control = Control::new(Box::new(Buffer::with_output(b"")), (20, 5));
        control.apply_scheme(terminal.scheme.as_ref());

        let mut renderer = Renderer::new(
            TerminalData::new(FontInfo::default(), RenderSettings::new(), 20, 5),
            RenderSettings::new(),
        );
        sync_palette(&mut renderer, &control);

        let table = renderer.settings().color_table();
        let index = table.alias_index(ColorAlias::DefaultBackground);
        assert_eq!(table.get(index).map(|color| color.get_colorref()), Some(0x0020_1020));
    }

    #[test]
    fn a_tab_key_round_trips_its_id() {
        assert_eq!(tab_id(&tab_key(7)), Some(7));
        assert_eq!(tab_id("tab-0"), Some(0));
    }

    #[test]
    fn a_key_that_is_not_a_tab_id_is_ignored() {
        assert_eq!(tab_id("tab-"), None);
        assert_eq!(tab_id("tab-x"), None);
        assert_eq!(tab_id("other-1"), None);
        assert_eq!(tab_id(""), None);
    }

    #[test]
    fn the_pty_size_follows_the_grid() {
        assert_eq!(pty_size((120, 30)), PtySize::new(120, 30));
        assert_eq!(pty_size((0, 0)), PtySize::new(1, 1));
    }

    #[test]
    fn a_pointer_position_maps_to_a_cell() {
        assert_eq!(pointer_cell(0.0, 0.0, (80, 24), (9, 20), 1.0), Some((0, 0)));
        assert_eq!(pointer_cell(9.0, 20.0, (80, 24), (9, 20), 1.0), Some((1, 1)));
        assert_eq!(pointer_cell(-1.0, 0.0, (80, 24), (9, 20), 1.0), None);
        assert_eq!(pointer_cell(720.0, 0.0, (80, 24), (9, 20), 1.0), None);
    }

    #[test]
    fn a_scaled_pointer_position_names_the_same_cell() {
        // At 200% the physical cell is twice the DIP cell, and the pointer
        // position is in DIPs.
        assert_eq!(pointer_cell(9.0, 20.0, (80, 24), (18, 40), 2.0), Some((1, 1)));
    }

    #[test]
    fn a_bound_chord_is_matched_before_the_terminal_sees_it() {
        let file = settings::parse(
            r##"
                version = 1

                [[keybindings]]
                keys = "ctrl+shift+c"
                action = "copy"
            "##,
        )
        .expect("settings");

        let mut ctrl_shift = InputModifiers::CONTROL;
        ctrl_shift |= InputModifiers::SHIFT;

        assert_eq!(
            chord_action(&file.keybindings, Key::Char('c'), ctrl_shift),
            Some(settings::Action::Copy)
        );
        // The same key without Shift is not this chord.
        assert_eq!(
            chord_action(&file.keybindings, Key::Char('c'), InputModifiers::CONTROL),
            None
        );
    }

    #[test]
    fn a_key_without_a_settings_name_is_not_a_chord() {
        assert_eq!(settings_key(Key::LeftShift), None);
        assert_eq!(settings_key(Key::CapsLock), None);
        assert_eq!(settings_key(Key::Char('A')), Some(settings::Key::Char('a')));
        assert_eq!(settings_key(Key::Function(5)), Some(settings::Key::Function(5)));
    }

    #[test]
    fn a_panel_of_dips_fits_whole_cells() {
        // 800 device pixels at 100% scale with an 8 by 16 cell: 100 by 50.
        assert_eq!(grid_for(800.0, 800.0, (8, 16), 1.0, 1.0), (100, 50));
        // The same DIPs at 150% need the bigger physical cell to divide them.
        assert_eq!(grid_for(800.0, 800.0, (12, 24), 1.5, 1.5), (100, 50));
    }

    #[test]
    fn a_sub_cell_remainder_is_dropped() {
        assert_eq!(grid_for(805.0, 803.0, (8, 16), 1.0, 1.0), (100, 50));
    }

    #[test]
    fn a_panel_too_small_for_a_cell_still_has_one() {
        assert_eq!(grid_for(0.0, 0.0, (8, 16), 1.0, 1.0), (1, 1));
        assert_eq!(grid_for(-5.0, 3.0, (8, 16), 1.0, 1.0), (1, 1));
    }

    #[test]
    fn nonsense_metrics_do_not_produce_a_grid() {
        assert_eq!(grid_for(f64::NAN, 800.0, (8, 16), 1.0, 1.0), (1, 50));
        assert_eq!(grid_for(800.0, 800.0, (8, 16), 0.0, -2.0), (100, 50));
    }

    #[test]
    fn the_grid_is_bounded() {
        assert_eq!(grid_for(1.0e9, 1.0e9, (1, 1), 1.0, 1.0), (MAX_CELLS, MAX_CELLS));
    }

    #[test]
    fn the_target_is_the_grid_in_physical_pixels() {
        assert_eq!(target_size((8, 16), (100, 50)), (800, 800));
    }

    #[test]
    fn the_target_saturates_rather_than_wrapping() {
        assert_eq!(target_size((u16::MAX, u16::MAX), (2, 2)), (u16::MAX, u16::MAX));
    }

    #[test]
    fn a_physical_size_is_the_dips_times_the_scale() {
        assert_eq!(physical_size(800.0, 600.0, 1.0, 1.0), (800, 600));
        assert_eq!(physical_size(800.0, 600.0, 1.5, 1.5), (1200, 900));
        assert_eq!(physical_size(0.0, 0.0, 1.0, 1.0), (1, 1));
    }
}
