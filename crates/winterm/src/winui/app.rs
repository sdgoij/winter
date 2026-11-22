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
use term::input::{Key, MouseButton, MouseEvent, MouseEventKind, WheelAxis};
use types::color_table::ColorAlias;
use utils::{InclusiveRect, Size};
use windows_reactor::*;

use crate::find::FindBar;
use crate::input;
use crate::session::Session;
use crate::tabs::Tabs;
use crate::term_control::Control;
use crate::term_events::TerminalEvent;
use crate::term_settings::TerminalSettings;
use crate::terminal_render::TerminalData;
use crate::view::View as ViewState;

use super::menu::{self, MenuEntry};
use super::settings::{SettingsCommand, SettingsMessage, SettingsWindow};
use super::surface::Surface;
use super::wheel::{self, Wheel};

/// The size the window opens at, before the panel reports its own.
const INITIAL_COLUMNS: i32 = 120;
const INITIAL_ROWS: i32 = 30;
/// The drop-down's width in DIPs, so its overlay can hang under it.
const MENU_WIDTH: f64 = 240.0;
/// The drop-down button's width in DIPs.
const DROP_DOWN_WIDTH: f64 = 32.0;
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
    /// The find bar's query, matches and current match.
    find: FindBar,
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
    /// The settings file in force, so a save can resolve its parts again.
    settings: settings::Settings,
    /// The settings dialog and its draft.
    settings_window: SettingsWindow,
    /// Whether the profile drop-down is showing.
    menu_open: bool,
    /// Whether the About dialog is showing.
    about_open: bool,
    /// The title bar header's measured width in DIPs, so the menu hangs under
    /// the drop-down button rather than at the pane's left edge.
    header_width: f64,
    /// The header element, wrapped in a Grid only so it can be measured.
    header: ElementRef<Grid>,
    _header_observation: ElementObservation,
    /// The file the settings were read from, polled for a save (ADR 025).
    settings_path: Option<PathBuf>,
    /// The file's stamp at the last read, so a save is noticed.
    settings_stamp: Option<settings::Stamp>,
    /// When the settings file was last polled, so the check is once a second.
    settings_at: Instant,
    renderer: Renderer<TerminalData>,
    shared: SharedBackend,
    panel: ElementRef<SwapChainPanel>,
    focus: ElementRef<Border>,
    find_field: ElementRef<TextBox>,
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
    /// The cell and block flag a left press started on, so a drag begins a
    /// selection: a click alone selects nothing (ADR 053).
    press: Option<((i32, i32), bool)>,
    /// The cell the pointer is over, so a wheel can be aimed at a program.
    pointer_cell: Option<(i32, i32)>,
    /// The message hook the wheel arrives through; kept alive for the session.
    _wheel: Option<Wheel>,
    face: PathBuf,
    /// The face size in DIPs, from the settings file.
    font_size: f32,
    /// The cell size in physical pixels at the current composition scale.
    cell: (u16, u16),
    /// The composition scale the face was rasterised for.
    scale: f32,
    /// The panel's size in DIPs, so a font change can re-fit the grid.
    panel_size: (f64, f64),
    /// The scale the panel reported, so a font change fits both axes.
    panel_scale: (f32, f32),
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
pub(super) enum Message {
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
    /// The shell's window, so the wheel hook can be installed.
    Window(isize),
    /// One edit in the settings dialog.
    Settings(SettingsMessage),
    /// The find bar's query changed.
    FindQuery(String),
    /// Moves to the next (true) or previous match.
    FindStep(bool),
    /// Closes the find bar.
    FindClose,
    /// Shows or hides the profile drop-down.
    MenuToggle,
    /// Hides the profile drop-down without acting on a row.
    MenuClose,
    /// Opens a tab on the named profile.
    MenuProfile(String),
    /// Opens the settings dialog from the drop-down.
    MenuSettings,
    /// Opens the About dialog from the drop-down.
    MenuAbout,
    /// The About dialog closed.
    AboutClose,
    /// The title bar header's width in DIPs changed.
    HeaderWidth(f64),
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
        let settings_path = loaded.path.clone();
        let settings_stamp = settings_path.as_deref().and_then(settings::stamp);
        let (template, bindings, font_size) = resolved_settings(&loaded.settings);
        let settings_window = SettingsWindow::new(&loaded);
        let settings = loaded.settings;

        let session = spawn_shell(&template.command, size).expect("a shell");
        let mut control = Control::new(Box::new(session), grid);
        control.apply_scheme(template.scheme.as_ref());
        let tab = Tab {
            id: 0,
            control,
            view: ViewState::new(),
            title: template.title.clone(),
            find: FindBar::default(),
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
        let _ = context.run_window(|handle| Message::Window(handle.as_raw() as isize));

        // The header's width decides where the drop-down's menu hangs. The
        // binding reports no control position, but a Grid reports its own
        // size, which is the header's left offset in this layout.
        let header = ElementRef::<Grid>::new();
        let header_observation = {
            let sender = context.sender();
            header.observe_composition_host(move |event| {
                let width = match event {
                    CompositionHostEvent::Ready { width, .. } | CompositionHostEvent::Metrics { width, .. } => width,
                };
                let _ = sender.send(Message::HeaderWidth(width));
            })
        };

        let initial_client = (
            f64::from(cell.0) * f64::from(INITIAL_COLUMNS as u16),
            f64::from(cell.1) * f64::from(INITIAL_ROWS as u16),
        );

        Self {
            tabs: Tabs::new(tab),
            next_tab: 1,
            template,
            bindings,
            settings,
            settings_window,
            menu_open: false,
            about_open: false,
            header_width: 0.0,
            header,
            _header_observation: header_observation,
            settings_path,
            settings_stamp,
            settings_at: Instant::now(),
            renderer,
            shared,
            panel,
            focus: ElementRef::<Border>::new(),
            find_field: ElementRef::<TextBox>::new(),
            surface,
            _observation: observation,
            timer,
            sides: input::ModifierSides::default(),
            blink_on: true,
            blink_at: Instant::now(),
            focused: false,
            focus_attempted: false,
            last_mouse_cell: None,
            press: None,
            pointer_cell: None,
            _wheel: None,
            face,
            font_size,
            cell,
            scale: 1.0,
            panel_size: initial_client,
            panel_scale: (1.0, 1.0),
            grid,
            initial_client,
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
        let dialog = keys.clone();
        let menu_sender = keys.clone();
        let escape = keys.clone();
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

        let find_open = self.tabs.active().is_some_and(|tab| tab.find.open);
        let menu_open = self.menu_open;

        let mut pane_children: Vec<View> = vec![SwapChainPanel::new().element_ref(&self.panel).into()];
        if let Some(tab) = self.tabs.active()
            && tab.find.open
        {
            pane_children.push(find_bar(tab, &dialog, &self.find_field));
        }
        if menu_open {
            pane_children.push(profile_menu(&self.settings, &dialog, menu_left(self)));
        }
        let pane_content = Grid::new().children(pane_children);

        let pane = Border::new()
            .is_tab_stop(true)
            .capture_pointer_on_press(true)
            .focus_on_pointer_release(true)
            .element_ref(&self.focus)
            .background(Color::rgb(12, 12, 12))
            .content(pane_content)
            .on_preview_key_down(RoutedCallback::new(move |info: KeyEventInfo| {
                let pressed = !info.status.is_released;
                let bound = key_of(info.key)
                    .and_then(|key| chord_action(&bindings, key, info.modifiers))
                    .is_some();

                if find_open {
                    // A bound chord still reaches the window; Enter and Escape
                    // move the bar; every other key belongs to its text field.
                    if pressed && bound {
                        let _ = keys.send(Message::Key(info));
                        return true;
                    }
                    if pressed {
                        match key_of(info.key) {
                            Some(Key::Escape) => {
                                let _ = keys.send(Message::FindClose);
                                return true;
                            }
                            Some(Key::Enter) => {
                                let forward = !info.modifiers.contains(InputModifiers::SHIFT);
                                let _ = keys.send(Message::FindStep(forward));
                                return true;
                            }
                            _ => {}
                        }
                    }
                    return false;
                }

                if menu_open {
                    // The drop-down owns the keyboard: a bound chord still
                    // reaches the window, Escape dismisses it, and nothing is
                    // typed into the terminal behind it (ADR 052).
                    if !pressed {
                        return true;
                    }
                    if bound {
                        let _ = keys.send(Message::Key(info));
                    } else if matches!(key_of(info.key), Some(Key::Escape)) {
                        let _ = keys.send(Message::MenuClose);
                    }
                    return true;
                }

                // A bound chord is the window's, so the toolkit must not also
                // type it. A printable key with no binding still has to raise
                // `CharacterReceived`, which is the text path; the keys the
                // terminal encodes itself are marked handled.
                let _ = keys.send(Message::Key(info));
                bound || !matches!(key_of(info.key), Some(Key::Char(_) | Key::Space))
            }))
            .on_character_received(RoutedCallback::new(move |info: CharacterEventInfo| {
                if find_open || menu_open {
                    return false;
                }
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

        // The drop-down sits immediately after the strip's `+`, as in Windows
        // Terminal: the two are the same height and share one header. The Grid
        // wraps them only so its width can be measured, which is how the menu
        // finds the button's left edge (ADR 052).
        let drop_down = Button::new()
            .content("\u{25be}")
            .style(ButtonStyle::Subtle)
            .width(DROP_DOWN_WIDTH)
            .height(24.0)
            // A Button's stock style has a 32-DIP minimum; the tab strip's own
            // button is 24, so the default would make the two uneven.
            .min_height(0.0)
            // The tab strip's own button sits 13 DIP below the header's top (a
            // 32-DIP tab item with a 24-DIP button, and the header's slot starts
            // 4 DIP above the title bar's content); match it, or the two buttons
            // are visibly uneven.
            .vertical_alignment(VerticalAlignment::Top)
            .margin(Thickness::new(0.0, 13.0, 0.0, 0.0))
            .on_click(move || {
                let _ = menu_sender.send(Message::MenuToggle);
            });
        let header_panel: View = StackPanel::new()
            .orientation(Orientation::Horizontal)
            .children((strip, drop_down))
            .into();
        let header = Grid::new().element_ref(&self.header).children(vec![header_panel]);

        // The tab strip is the title bar, as in Windows Terminal: the TitleBar
        // control extends the client area and hands the system the drag region,
        // leaving room for the caption buttons on its right.
        let title_bar = Border::new()
            .background(Color::rgb(32, 32, 36))
            // This snapshot leaves an 8-DIP strip of the system caption title
            // above the control's content; pulling the row up covers it
            // (ADR 051).
            .margin(Thickness::new(0.0, -8.0, 0.0, 0.0))
            .content(
                TitleBar::new()
                    .is_back_button_visible(false)
                    .is_pane_toggle_button_visible(false)
                    .left_header(header),
            );

        // The pane takes the rest; the panel stays mounted, so switching tabs
        // never rebuilds the surface.
        //
        // The root is a Border only for its window-level preview key handler:
        // the drop-down's buttons live in the title bar, which is not under the
        // pane's preview route, so this is the one ancestor both share (ADR
        // 052).
        let root: View = Border::new()
            .on_preview_key_down(RoutedCallback::new(move |info: KeyEventInfo| {
                if !menu_open || info.status.is_released {
                    return false;
                }
                if matches!(key_of(info.key), Some(Key::Escape)) {
                    let _ = escape.send(Message::MenuClose);
                    return true;
                }
                false
            }))
            .content(
                Grid::new()
                    .rows([GridLength::Auto, GridLength::STAR])
                    .children((title_bar, pane.grid_row(1))),
            )
            .into();

        if self.settings_window.is_open() {
            root.content_dialog(
                self.settings_window
                    .view(&self.settings, self.settings_path.as_deref(), &dialog),
            )
        } else if self.about_open {
            root.content_dialog(about_dialog(&dialog))
        } else {
            root
        }
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
            Message::Window(handle) => self._wheel = wheel::install(handle),
            Message::Settings(message) => self.settings_message(message),
            Message::FindQuery(text) => self.find_query(text),
            Message::FindStep(forward) => self.find_step(forward),
            Message::FindClose => self.close_find(),
            Message::MenuToggle => self.menu_open = !self.menu_open,
            Message::MenuClose => self.close_menu(),
            Message::MenuProfile(name) => self.open_profile_tab(&name),
            Message::MenuSettings => {
                self.close_menu();
                if !self.settings_window.is_open() {
                    self.toggle_settings();
                }
            }
            Message::MenuAbout => {
                self.close_menu();
                self.about_open = true;
            }
            Message::AboutClose => {
                self.about_open = false;
                let _ = self.focus.request_focus();
            }
            Message::HeaderWidth(width) => self.header_width = width,
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

        // The find field mounts a frame after the bar opens, so the request is
        // retried until the element exists.
        let find_focus = self.tabs.active().is_some_and(|tab| tab.find.open && tab.find.focus);
        if find_focus
            && self.find_field.request_focus()
            && let Some(tab) = self.tabs.active_mut()
        {
            tab.find.focus = false;
        }

        // The binding has no wheel event, so the message hook collected it.
        // A modal dialog owns the wheel while it is open; our copy is dropped so
        // the dialog's own scroller moves and the terminal behind it stays put.
        let notches = wheel::take();
        if notches != 0 && !self.settings_window.is_open() {
            self.wheel(notches);
        }

        // The poll is the watcher (ADR 025): one metadata call a second, and a
        // changed stamp means the file was saved and is read again.
        if now.duration_since(self.settings_at) >= Duration::from_secs(1) {
            self.settings_at = now;
            self.poll_settings();
        }

        let blink_on = self.blink_on;
        let (painted, background) = {
            let Shell { renderer, tabs, .. } = self;
            let Some(tab) = tabs.active_mut() else {
                return;
            };

            renderer
                .data_mut()
                .refresh(&mut tab.control, &tab.view, blink_on, Some(&tab.find));

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

        // Kept so a font change can re-fit the grid without waiting for the
        // panel to report its metrics again.
        self.panel_size = (width, height);
        self.panel_scale = (scale_x, scale_y);

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

    /// Reloads the settings file when a save changed its stamp.
    ///
    /// A dirty draft is not overwritten: the dialog says the file changed and
    /// lets Save win or Revert load it (ADR 025).
    fn poll_settings(&mut self) {
        let Some(path) = &self.settings_path else {
            return;
        };
        if settings::stamp(path) == self.settings_stamp {
            return;
        }

        let dirty = self
            .settings_window
            .draft()
            .is_some_and(|draft| draft.differs_from(&self.settings));
        if dirty {
            self.settings_window.set_conflict();
        } else {
            self.reload_settings();
        }
    }

    /// Re-reads the settings file and applies what changed.
    ///
    /// Chords and the font size take effect at once, and a scheme repaints
    /// every tab. A changed profile affects the next tab, because a running
    /// shell keeps the command it started with.
    fn reload_settings(&mut self) {
        let loaded = settings::load_with_report();
        self.settings_stamp = loaded.path.as_deref().and_then(settings::stamp);
        let warnings: Vec<String> = loaded.warnings.iter().map(ToString::to_string).collect();
        let migrated_from = loaded.migrated_from.clone();
        if let Some(error) = loaded.error {
            // A half-written save must not replace the settings a running
            // session is using; the next valid save is read again.
            self.settings_window.set_error(error.to_string());
            return;
        }

        self.settings = loaded.settings;
        self.settings_path = loaded.path;
        self.settings_window.set_report(warnings, migrated_from);
        self.settings_window.clear_error();
        self.settings_window.clear_conflict();
        self.settings_window.sync_draft(&self.settings);

        let (template, bindings, font_size) = resolved_settings(&self.settings);
        self.template = template;
        self.bindings = bindings;

        if font_size != self.font_size {
            self.font_size = font_size;
            self.rebuild_face(self.panel_scale.0);
            let grid = grid_for(
                self.panel_size.0,
                self.panel_size.1,
                self.cell,
                self.panel_scale.0,
                self.panel_scale.1,
            );
            self.apply_grid(grid);
        }

        let scheme = self.template.scheme.clone();
        for index in 0..self.tabs.len() {
            if let Some(tab) = self.tabs.get_mut(index) {
                tab.control.apply_scheme(scheme.as_ref());
            }
        }
    }

    /// Shows or hides the settings dialog; closing discards unsaved edits.
    fn toggle_settings(&mut self) {
        if self.settings_window.is_open() {
            self.settings_window.close();
            let _ = self.focus.request_focus();
        } else {
            self.settings_window.open(&self.settings);
        }
    }

    /// Applies one settings-dialog edit and runs the command it may ask for.
    fn settings_message(&mut self, message: SettingsMessage) {
        match self.settings_window.apply(message) {
            SettingsCommand::None => {}
            SettingsCommand::Save => self.save_settings(),
            SettingsCommand::Revert => self.reload_settings(),
            SettingsCommand::Close => {
                let _ = self.focus.request_focus();
            }
        }
    }

    /// Writes the draft through the same path startup reads.
    fn save_settings(&mut self) {
        let Some(document) = self.settings_window.document(&self.settings) else {
            return;
        };

        match settings::save(&document) {
            Ok(()) => {
                self.settings_window.clear_error();
                self.reload_settings();
            }
            Err(error) => self.settings_window.set_error(error.to_string()),
        }
    }

    /// Shows or hides the active tab's find bar.
    fn toggle_find(&mut self) {
        let Some(tab) = self.tabs.active_mut() else {
            return;
        };
        tab.find.toggle();
        if tab.find.open {
            tab.find.refresh(&mut tab.control);
            tab.find.scroll_to(&tab.control, &mut tab.view);
        } else {
            let _ = self.focus.request_focus();
        }
    }

    /// Recomputes the matches and scrolls to the first.
    fn find_query(&mut self, text: String) {
        let Some(tab) = self.tabs.active_mut() else {
            return;
        };
        tab.find.query = text;
        tab.find.refresh(&mut tab.control);
        tab.find.scroll_to(&tab.control, &mut tab.view);
    }

    /// Moves to the next (true) or previous match.
    fn find_step(&mut self, forward: bool) {
        let Some(tab) = self.tabs.active_mut() else {
            return;
        };
        if tab.find.searched != tab.find.query {
            tab.find.refresh(&mut tab.control);
        }
        tab.find.step(forward);
        tab.find.scroll_to(&tab.control, &mut tab.view);
    }

    /// Closes the find bar and returns focus to the terminal.
    fn close_find(&mut self) {
        if let Some(tab) = self.tabs.active_mut() {
            tab.find.hide();
        }
        let _ = self.focus.request_focus();
    }

    /// The title the window shows: the active tab's.
    fn title(&self) -> &str {
        self.tabs.active().map_or("winterm", |tab| tab.title.as_str())
    }

    /// Opens a tab on the same profile and activates it.
    fn open_tab(&mut self) {
        self.add_tab(self.template.clone());
    }

    /// Opens a tab on the profile the drop-down named and activates it.
    fn open_profile_tab(&mut self, name: &str) {
        self.close_menu();
        let Some(profile) = self.settings.profile_named(name) else {
            return;
        };
        let terminal = TerminalSettings::for_profile(&self.settings, Some(profile));
        self.add_tab(terminal);
    }

    /// Hides the drop-down, returning the pointer to the pane.
    fn close_menu(&mut self) {
        self.menu_open = false;
    }

    /// Starts a terminal and adds it as the active tab.
    fn add_tab(&mut self, terminal: TerminalSettings) {
        let size = pty_size(self.grid);
        let Ok(session) = spawn_shell(&terminal.command, size) else {
            return;
        };

        let mut control = Control::new(Box::new(session), self.grid);
        control.apply_scheme(terminal.scheme.as_ref());

        let id = self.next_tab;
        self.next_tab += 1;
        self.tabs.add(Tab {
            id,
            control,
            view: ViewState::new(),
            title: terminal.title,
            find: FindBar::default(),
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
        // sees the event (ADR 023). Copy is the exception: with no selection
        // it declines the key so ctrl+c still interrupts a program (ADR 053).
        if pressed
            && let Some(action) = chord_action(&self.bindings, key, info.modifiers)
            && self.perform(action, context)
        {
            return;
        }

        // The find field owns the keyboard while its bar is open, and the
        // drop-down owns it while it shows; only a bound chord, handled above,
        // leaves either.
        if self.menu_open || self.tabs.active().is_some_and(|tab| tab.find.open) {
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
        // Typing or pressing a key returns the view to the live screen.
        if outcome.activity {
            tab.view.scroll_to_bottom();
        }
        for key in outcome.keys {
            tab.control.send_key(&key);
        }
    }

    /// Hands composed text to the active terminal's text path.
    fn character(&mut self, info: CharacterEventInfo) {
        if self.menu_open || self.tabs.active().is_some_and(|tab| tab.find.open) {
            return;
        }

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
        if outcome.activity {
            tab.view.scroll_to_bottom();
        }
        for key in outcome.keys {
            tab.control.send_key(&key);
        }
    }

    /// Runs the window action a chord asked for, reporting whether it consumed
    /// the key.
    ///
    /// Every action consumes its chord except copy, which declines when there
    /// is nothing selected so the key reaches the program (ADR 053).
    fn perform(&mut self, action: settings::Action, context: &ComponentContext<Self>) -> bool {
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
            settings::Action::Copy => return self.copy_selection(),
            settings::Action::Paste => self.paste_clipboard(),
            settings::Action::ScrollUp => self.scroll_active(1),
            settings::Action::ScrollDown => self.scroll_active(-1),
            settings::Action::ScrollUpPage => self.scroll_page_active(1),
            settings::Action::ScrollDownPage => self.scroll_page_active(-1),
            settings::Action::ScrollToTop => self.scroll_to_active(false),
            settings::Action::ScrollToBottom => self.scroll_to_active(true),
            settings::Action::OpenSettings => self.toggle_settings(),
            settings::Action::Find => self.toggle_find(),
            // Panes and the palette are not this shell's yet, so their chords
            // fall through.
            _ => {}
        }
        true
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

    /// Scrolls the active viewport by `rows`; positive is towards the history.
    fn scroll_active(&mut self, rows: i32) {
        let Some(tab) = self.tabs.active_mut() else {
            return;
        };
        let max = tab.control.max_scroll();
        tab.view.scroll_by(rows, max);
    }

    /// Sends a wheel to a tracking program, or scrolls the viewport.
    ///
    /// Shift keeps the wheel for the viewport, so a person can always scroll
    /// back out of a full-screen program (ADR 019).
    fn wheel(&mut self, notches: i32) {
        let Some(tab) = self.tabs.active_mut() else {
            return;
        };

        let shift = wheel::shift_held();
        let forward = input::forwards_to_application(tab.control.is_tracking_mouse(), shift);
        if forward && let Some(position) = self.pointer_cell {
            let event = MouseEvent::wheel(position, WheelAxis::Vertical, notches * 120);
            if tab.control.send_mouse(&event) {
                return;
            }
        }

        let max = tab.control.max_scroll();
        tab.view.scroll_by(notches, max);
    }

    /// Scrolls the active viewport by `direction` pages.
    fn scroll_page_active(&mut self, direction: i32) {
        let Some(tab) = self.tabs.active_mut() else {
            return;
        };
        let rows = (tab.control.size().1 - 1).max(1) * direction.signum();
        let max = tab.control.max_scroll();
        tab.view.scroll_by(rows, max);
    }

    /// Jumps the active viewport to the live screen (`true`) or the oldest row.
    fn scroll_to_active(&mut self, bottom: bool) {
        let Some(tab) = self.tabs.active_mut() else {
            return;
        };
        if bottom {
            tab.view.scroll_to_bottom();
        } else {
            let max = tab.control.max_scroll();
            tab.view.set_scroll(max, max);
        }
    }

    /// Starts a selection, or forwards the press to a tracking application.
    fn pointer_pressed(&mut self, info: PointerEventInfo) {
        // A press outside the menu dismisses it, as a flyout does; the press
        // itself is not passed on (ADR 052).
        if self.menu_open {
            self.close_menu();
            return;
        }

        let shift = info.modifiers.contains(InputModifiers::SHIFT);
        let forward = input::forwards_to_application(self.active_tracking(), shift);
        let cell = pointer_cell(info.x, info.y, self.grid, self.cell, self.scale);
        self.pointer_cell = cell;

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
            self.press = None;
        } else if count == 2 {
            let text = tab.control.row_text(position.1);
            tab.view.select_word(&text, position);
            self.press = None;
        } else {
            // A single click selects nothing: it clears what was selected and
            // leaves the anchor a drag starts from, as upstream's
            // `LeftClickOnTerminal` does. A one-cell selection would otherwise
            // make ctrl+c copy a blank instead of interrupting (ADR 053).
            tab.view.clear_selection();
            self.press = Some((position, modifiers(info.modifiers).alt));
        }
    }

    /// Extends a selection, scrolls a drag past the pane, or forwards the move.
    fn pointer_moved(&mut self, info: PointerEventInfo) {
        let (columns, rows) = self.grid;
        let cell = pointer_cell(info.x, info.y, self.grid, self.cell, self.scale);
        self.pointer_cell = cell;
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

        if !info.is_left_button_pressed {
            return;
        }

        let position = if outside != 0 {
            // A drag that leaves the pane claims the edge row and scrolls one
            // row towards the pointer, so the selection reaches into history.
            let max = tab.control.max_scroll();
            tab.view.drag_scroll(outside, max, Instant::now());
            let row = if outside < 0 { 0 } else { rows.max(1) - 1 };
            let column = column.clamp(0, columns.max(1) - 1);
            Some((column, input::screen_to_document(max, tab.view.scroll(), row)))
        } else {
            cell.map(|(column, row)| {
                let history = tab.control.max_scroll();
                (column, input::screen_to_document(history, tab.view.scroll(), row))
            })
        };

        let Some(position) = position else {
            return;
        };
        match self.press.take() {
            // The first move away from the press cell starts the selection.
            Some((anchor, block)) if position != anchor => {
                tab.view.select(anchor, block);
                tab.view.extend(position);
            }
            Some(press) => self.press = Some(press),
            None => tab.view.extend(position),
        }
    }

    /// Finishes a forwarded press.
    fn pointer_released(&mut self, info: PointerEventInfo) {
        let shift = info.modifiers.contains(InputModifiers::SHIFT);
        if !input::forwards_to_application(self.active_tracking(), shift) {
            self.last_mouse_cell = None;
            self.press = None;
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

/// The parts of a saved document a running shell applies at once.
///
/// The template is what the next tab starts from, the bindings are matched
/// before the terminal sees a key, and the font size decides the grid.
fn resolved_settings(document: &settings::Settings) -> (TerminalSettings, Rc<[settings::Keybinding]>, f32) {
    (
        TerminalSettings::resolve(document),
        document.keybindings.clone().into(),
        document.font.size,
    )
}

/// The find bar: a query field, previous and next, and the match count.
fn find_bar(tab: &Tab, sender: &LocalSender<Message>, field: &ElementRef<TextBox>) -> View {
    let count = tab.find.matches.len();
    let shown = tab.find.current.map_or(0, |index| index + 1);
    let status = if count == 0 && !tab.find.query.is_empty() {
        "No matches".to_string()
    } else {
        format!("{shown} of {count}")
    };

    let query = {
        let sender = sender.clone();
        TextBox::new()
            .text(&tab.find.query)
            .placeholder_text("Find")
            .width(180.0)
            .element_ref(field)
            .on_text_changed(move |text: Rc<str>| {
                let _ = sender.send(Message::FindQuery(text.to_string()));
            })
    };
    let previous = {
        let sender = sender.clone();
        Button::new().content("↑").on_click(move || {
            let _ = sender.send(Message::FindStep(false));
        })
    };
    let next = {
        let sender = sender.clone();
        Button::new().content("↓").on_click(move || {
            let _ = sender.send(Message::FindStep(true));
        })
    };

    Border::new()
        .background(Color::rgb(32, 32, 36))
        .padding(Thickness::uniform(6.0))
        .horizontal_alignment(HorizontalAlignment::Right)
        .vertical_alignment(VerticalAlignment::Top)
        .margin(Thickness::uniform(12.0))
        .content(
            StackPanel::new()
                .orientation(Orientation::Horizontal)
                .spacing(6.0)
                .children((query, previous, next, TextBlock::new().text(status))),
        )
        .into()
}

/// The drop-down next to the tab strip: one row per profile and the window
/// commands.
///
/// The pinned binding has no flyout, so this is an overlay anchored to the
/// pane's top-left, under the title bar (ADR 052).
fn profile_menu(settings: &settings::Settings, sender: &LocalSender<Message>, left: f64) -> View {
    let rows: Vec<View> = menu::entries(settings)
        .into_iter()
        .map(|entry| menu_row(entry, sender))
        .collect();

    Border::new()
        .background(Color::rgb(44, 44, 48))
        .border_brush(Color::rgb(72, 72, 78))
        .border_thickness(Thickness::uniform(1.0))
        .corner_radius(CornerRadius::uniform(6.0))
        .padding(Thickness::uniform(4.0))
        .width(MENU_WIDTH)
        .horizontal_alignment(HorizontalAlignment::Left)
        .vertical_alignment(VerticalAlignment::Top)
        .margin(Thickness::new(left, 4.0, 0.0, 0.0))
        .content(StackPanel::new().spacing(1.0).children(rows))
        .into()
}

/// The drop-down's menu hangs under the button, kept inside the pane.
///
/// The header's measured width ends at the button's right edge, so its left
/// edge is the menu's anchor. A window too narrow for the menu pins it right.
fn menu_left(shell: &Shell) -> f64 {
    let rightmost = (shell.panel_size.0 - MENU_WIDTH - 8.0).max(8.0);
    (shell.header_width - DROP_DOWN_WIDTH).clamp(8.0, rightmost)
}

/// One clickable row of the drop-down.
fn menu_row(entry: MenuEntry, sender: &LocalSender<Message>) -> View {
    let button = match entry {
        MenuEntry::Profile(name) => {
            let sender = sender.clone();
            menu_button(name.clone()).on_click(move || {
                let _ = sender.send(Message::MenuProfile(name.clone()));
            })
        }
        MenuEntry::Settings => {
            let sender = sender.clone();
            menu_button("Settings".to_string()).on_click(move || {
                let _ = sender.send(Message::MenuSettings);
            })
        }
        MenuEntry::About => {
            let sender = sender.clone();
            menu_button("About winterm".to_string()).on_click(move || {
                let _ = sender.send(Message::MenuAbout);
            })
        }
    };
    button.into()
}

/// A flat, left-aligned row sized to the menu.
fn menu_button(label: String) -> Button {
    Button::new()
        .content(label)
        .style(ButtonStyle::Subtle)
        .horizontal_content_alignment(HorizontalAlignment::Left)
        .width(232.0)
}

/// The About dialog: the version this build was compiled as.
fn about_dialog(sender: &LocalSender<Message>) -> ContentDialog {
    let details = StackPanel::new().spacing(8.0).children((
        TextBlock::new().text(format!("winterm {}", env!("CARGO_PKG_VERSION"))),
        TextBlock::new().text("A cross-platform terminal emulator in Rust."),
        TextBlock::new().text("Ported from Microsoft's Windows Terminal."),
    ));

    ContentDialog::new()
        .title("About winterm")
        .close_button_text("Close")
        .is_open(true)
        .content(details)
        .on_closed({
            let sender = sender.clone();
            move |_| {
                let _ = sender.send(Message::AboutClose);
            }
        })
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
    fn a_reloaded_document_resolves_its_profile_chords_and_font() {
        let document = settings::parse(
            r#"
                default_profile = "two"

                [[profiles]]
                name = "one"
                command = "one.exe"

                [[profiles]]
                name = "two"
                command = "two.exe"

                [[keybindings]]
                keys = "ctrl+alt+t"
                action = "newTab"

                [font]
                size = 20
            "#,
        )
        .expect("settings");

        let (template, bindings, font_size) = resolved_settings(&document);
        assert_eq!(template.command.program(), "two.exe");
        assert_eq!(font_size, 20.0);
        assert!(
            bindings
                .iter()
                .any(|binding| binding.action == settings::Action::NewTab && binding.keys.to_string() == "ctrl+alt+t")
        );
    }

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
                keys = "ctrl+shift+q"
                action = "copy"
            "##,
        )
        .expect("settings");

        let mut ctrl_shift = InputModifiers::CONTROL;
        ctrl_shift |= InputModifiers::SHIFT;

        assert_eq!(
            chord_action(&file.keybindings, Key::Char('q'), ctrl_shift),
            Some(settings::Action::Copy)
        );
        // The same key without Shift is not this chord.
        assert_eq!(
            chord_action(&file.keybindings, Key::Char('q'), InputModifiers::CONTROL),
            None
        );
    }

    #[test]
    fn the_defaults_bind_ctrl_c_to_copy() {
        let bindings = settings::Settings::default().keybindings;

        assert_eq!(
            chord_action(&bindings, Key::Char('c'), InputModifiers::CONTROL),
            Some(settings::Action::Copy)
        );
        assert_eq!(
            chord_action(&bindings, Key::Char('v'), InputModifiers::CONTROL),
            Some(settings::Action::Paste)
        );
    }

    #[test]
    fn the_scroll_chords_are_bound_by_default() {
        let bindings = settings::Settings::default().keybindings;
        let shift = InputModifiers::SHIFT;

        assert_eq!(
            chord_action(&bindings, Key::PageUp, shift),
            Some(settings::Action::ScrollUpPage)
        );
        assert_eq!(
            chord_action(&bindings, Key::PageDown, shift),
            Some(settings::Action::ScrollDownPage)
        );
        // A page key without Shift is the program's.
        assert_eq!(chord_action(&bindings, Key::PageUp, InputModifiers::NONE), None);
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
