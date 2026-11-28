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

use std::cell::{Cell, RefCell};
use std::path::{Path, PathBuf};
use std::rc::Rc;
use std::sync::Arc;
use std::time::{Duration, Instant};

use renderer::renderer::Renderer;
use renderer::{
    AtlasEngine, CellSizeInDIP, FontDueRasterizer, FontInfo, IBackend, RenderSettings, Settings, SharedBackend,
    SoftwareBackend,
};
use server::pty::{PtyCommand, PtySize};
use term::input::{Key, MouseButton, MouseEvent, MouseEventKind, WheelAxis};
use types::text_color::ColorType;
use utils::{InclusiveRect, Size};
use windows_reactor::*;

use winter::about;
use winter::find::FindBar;
use winter::input;
use winter::layout::{self, Axis, Direction, Fraction, Layout, PaneId, SplitPath};
use winter::palette::{self, Palette};
use winter::scrollbar;
use winter::session::Session;
use winter::tabs::Tabs;
use winter::term_control::Control;
use winter::term_events::TerminalEvent;
use winter::term_settings::TerminalSettings;
use winter::terminal_render::TerminalData;
use winter::view::View as ViewState;

use super::command_palette;
use super::extension::{Extensions, Found};
use super::gpu::GpuBackend;
use super::icon;
use super::menu::{self, MenuEntry};
use super::profile_icon;
use super::scrollbar as bar;
use super::settings::{SettingsCommand, SettingsMessage, SettingsWindow};
use super::surface::Surface;
use super::wheel::{self, Wheel};
use super::wsl;
use winter::trace;

/// The size the window opens at, before the panel reports its own.
const INITIAL_COLUMNS: i32 = 120;
const INITIAL_ROWS: i32 = 30;
/// The drop-down's button width in DIPs, so its overlay can hang under it.
const MENU_WIDTH: f64 = 240.0;
/// The drop-down's half of the button group, in DIPs: the menu hangs under it.
const DROP_DOWN_WIDTH: f64 = 32.0;
/// The new-tab half of the button group, in DIPs.
const NEW_TAB_WIDTH: f64 = 32.0;
/// A profile's icon edge in DIPs.
const TAB_ICON: f64 = 16.0;
/// A tab's close button edge in DIPs.
const CLOSE_BUTTON: f64 = 20.0;
/// The gap between a tab's parts, in DIPs.
const TAB_GAP: f64 = 6.0;
/// A tab's padding: left, right.
const TAB_PADDING_LEFT: f64 = 12.0;
const TAB_PADDING_RIGHT: f64 = 4.0;
/// A tab's widest and narrowest, in DIPs.
///
/// The widest is upstream's cap; the narrowest is its icon, its close button and
/// the padding and gaps between them, so a tab that has been squeezed still
/// shows both and ellipsises its title rather than overlapping them.
const MAX_TAB_WIDTH: f64 = 240.0;
const MIN_TAB_WIDTH: f64 = TAB_PADDING_LEFT + TAB_ICON + 2.0 * TAB_GAP + CLOSE_BUTTON + TAB_PADDING_RIGHT;
/// The gap between the last tab and the new-tab group, in DIPs.
const TAB_ROW_GAP: f64 = 6.0;
/// What the window's own caption buttons take on the right, in DIPs, plus the
/// room the strip keeps clear of them. Three buttons at 46 DIPs is the Windows
/// 11 size at 100%; the strip stops short of the widest layout rather than
/// measuring a control the binding does not report.
const CAPTION_WIDTH: f64 = 138.0;
const CAPTION_GAP: f64 = 8.0;
/// The inset the terminal's text starts at, in DIPs: the first column off the
/// pane's left edge, and the last one short of the scrollbar.
const PANE_PADDING_LEFT: f64 = 8.0;
/// The height of the new-tab group, in DIPs.
///
/// A little taller than upstream's 24, so it fills most of the 28-DIP row it is
/// centred in.
const BUTTON_GROUP_HEIGHT: f64 = 26.0;
/// The tab row's height in DIPs.
///
/// Four DIPs of the title bar's slot stay above it, which is the gap a tab
/// leaves under the window's top edge.
const TAB_ROW_HEIGHT: f64 = 28.0;
/// How far a tab paints past its slot.
///
/// The title bar's header slot does not always end at the pane's top, and the
/// band between them is the one thing a tab strip that merges with its pane
/// cannot show. Upstream pads the strip by -1 for the same reason
/// (`TabRowControl.xaml`); this binding leaves more, and anything past the row
/// is painted over by the pane.
const TAB_OVERHANG: f64 = 12.0;
/// The face Windows Terminal draws its own glyphs from: upstream's
/// `SymbolThemeFontFamily`, which WinUI defines as this pair, so a Windows 10
/// machine that has no Fluent Icons file still finds every codepoint in MDL2.
const SYMBOL_FONT: &str = "Segoe Fluent Icons,Segoe MDL2 Assets";
/// A plus, the drop-down's chevron and a tab's close cross, as upstream names
/// them (`U+E710`, `U+E70D`, `U+E711`).
const PLUS_GLYPH: &str = "\u{e710}";
const CHEVRON_GLYPH: &str = "\u{e70d}";
const CLOSE_GLYPH: &str = "\u{e711}";
/// The drop-down's own glyphs: a gear for `Settings`, a puzzle piece for
/// `Extensions` and a question mark for `About`, `U+E713`, `U+E8B9` and
/// `U+E897` in the same two faces.
const SETTINGS_GLYPH: &str = "\u{e713}";
const EXTENSIONS_GLYPH: &str = "\u{e8b9}";
const ABOUT_GLYPH: &str = "\u{e897}";
/// The tab row's fill: behind an inactive tab, and between the last tab and
/// the caption buttons.
const TITLEBAR: Color = Color::rgb(32, 32, 36);
/// How far a hovered tab moves from the title bar towards the pane's colour.
const HOVER_MIX: f32 = 0.5;
/// How far an inactive tab's label is dimmed towards its own fill.
const LABEL_DIM: f32 = 0.35;
/// How far a hovered close button is tinted towards the tab's label colour.
const CLOSE_HOVER_MIX: f32 = 0.18;
/// Far beyond any display, and it keeps a bad metric from asking the terminal
/// for an allocation it can never fill.
const MAX_CELLS: i32 = 1024;

/// Runs the WinUI 3 shell until its window closes.
pub fn run() -> windows_core::Result<()> {
    App::run_component::<Shell>(())
}

/// One pane: its own terminal, selection, find bar and renderer.
pub(super) struct Pane {
    pub(super) id: PaneId,
    pub(super) control: Control,
    pub(super) view: ViewState,
    /// The program's OSC title while it names one.
    pub(super) title: Option<String>,
    /// The program the pane started, so the strip can show its icon.
    pub(super) program: String,
    /// The pane's own find bar, so switching panes keeps its query.
    find: FindBar,
    pub(super) renderer: Renderer<TerminalData>,
    /// The software fallback's pixels, when the CPU path paints.
    shared: Option<SharedBackend>,
    /// Where the pane's grid sits in the swap chain, in device pixels.
    offset: Rc<Cell<(i32, i32)>>,
    /// The pane's grid in cells.
    pub(super) grid: (i32, i32),
}

/// One tab: a pane tree, its focused pane and a title.
pub(super) struct Tab {
    /// Stable across reorders, because the toolkit's close event names it.
    id: u64,
    pub(super) panes: Vec<Pane>,
    layout: Layout,
    focused: PaneId,
    /// The terminal settings a split starts from.
    terminal: TerminalSettings,
    /// The title to show until the focused pane's shell sets one.
    title: String,
}

impl Tab {
    fn pane(&self, id: PaneId) -> Option<&Pane> {
        self.panes.iter().find(|pane| pane.id == id)
    }

    fn pane_mut(&mut self, id: PaneId) -> Option<&mut Pane> {
        self.panes.iter_mut().find(|pane| pane.id == id)
    }

    pub(super) fn focused_pane(&self) -> Option<&Pane> {
        self.pane(self.focused)
    }

    pub(super) fn focused_pane_mut(&mut self) -> Option<&mut Pane> {
        self.pane_mut(self.focused)
    }

    /// The title the strip shows: the focused pane's OSC title, else the tab's.
    pub(super) fn label(&self) -> &str {
        self.focused_pane()
            .and_then(|pane| pane.title.as_deref())
            .unwrap_or(&self.title)
    }

    /// The program the focused pane runs, for the tab's icon.
    fn program(&self) -> &str {
        self.focused_pane().map_or("", |pane| pane.program.as_str())
    }
}

/// A key press waiting for the character it produced.
struct PendingKey {
    key: Key,
    repeat: bool,
    modifiers: input::Modifiers,
    virtual_key: u16,
    scan_code: u16,
    repeat_count: u16,
}

/// A pane's rectangle in the pane's DIPs.
#[derive(Clone, Copy, Debug, PartialEq)]
struct PaneRect {
    x: f64,
    y: f64,
    width: f64,
    height: f64,
}

impl PaneRect {
    fn contains(self, point: (f64, f64)) -> bool {
        point.0 >= self.x && point.0 < self.x + self.width && point.1 >= self.y && point.1 < self.y + self.height
    }
}

/// The part of `area` a pane's fraction covers.
fn fraction_rect(area: PaneRect, fraction: Fraction) -> PaneRect {
    PaneRect {
        x: area.x + area.width * f64::from(fraction.x),
        y: area.y + area.height * f64::from(fraction.y),
        width: area.width * f64::from(fraction.width),
        height: area.height * f64::from(fraction.height),
    }
}

/// The half a split of `rect` gives its new, second pane.
fn split_second(rect: PaneRect, axis: Axis) -> PaneRect {
    match axis {
        Axis::Horizontal => PaneRect {
            x: rect.x + rect.width / 2.0,
            y: rect.y,
            width: rect.width / 2.0,
            height: rect.height,
        },
        Axis::Vertical => PaneRect {
            x: rect.x,
            y: rect.y + rect.height / 2.0,
            width: rect.width,
            height: rect.height / 2.0,
        },
    }
}

/// The split whose divider is within `grab` points of `point`, if any.
fn divider_at(
    area: PaneRect,
    splits: &[(SplitPath, Axis, Fraction, f32)],
    point: (f64, f64),
    grab: f64,
) -> Option<(SplitPath, Axis, PaneRect)> {
    for (path, axis, fraction, ratio) in splits.iter().rev() {
        let split = fraction_rect(area, *fraction);
        match axis {
            Axis::Horizontal => {
                let x = split.x + split.width * f64::from(*ratio);
                if (point.0 - x).abs() <= grab && point.1 >= split.y && point.1 <= split.y + split.height {
                    return Some((path.clone(), *axis, split));
                }
            }
            Axis::Vertical => {
                let y = split.y + split.height * f64::from(*ratio);
                if (point.1 - y).abs() <= grab && point.0 >= split.x && point.0 <= split.x + split.width {
                    return Some((path.clone(), *axis, split));
                }
            }
        }
    }
    None
}

/// The grid a pane of `rect` DIPs holds at the panel's scale.
fn pane_grid(rect: PaneRect, cell: (u16, u16), scale_x: f32, scale_y: f32) -> (i32, i32) {
    let width = (rect.width - bar::WIDTH).max(0.0);
    let columns = width * f64::from(finite_scale(scale_x)) / f64::from(cell.0.max(1));
    let rows = rect.height.max(0.0) * f64::from(finite_scale(scale_y)) / f64::from(cell.1.max(1));
    (cells(columns), cells(rows))
}

/// The application's state and its messages.
pub(super) struct Shell {
    pub(super) tabs: Tabs<Tab>,
    /// The id the next tab is given.
    next_tab: u64,
    /// The profile a new tab starts from.
    template: TerminalSettings,
    /// The window chords, matched before the terminal sees a key.
    bindings: Rc<[settings::Keybinding]>,
    /// The settings file in force, so a save can resolve its parts again.
    pub(super) settings: settings::Settings,
    /// The profiles the window offers: the file's, then one per WSL distro the
    /// file does not already name. The distros are the machine's, so they are
    /// merged here and never written back with a save (ADR 065).
    profiles: Vec<settings::Profile>,
    /// The settings dialog and its draft.
    settings_window: SettingsWindow,
    /// Whether the profile drop-down is showing.
    menu_open: bool,
    /// The command palette's entries, filter and selection.
    commands: Palette,
    /// Whether the About dialog is showing.
    about_open: bool,
    /// The extension engine, when the settings or the environment asked for it.
    pub(super) extensions: Option<Extensions>,
    /// Whether the extensions dialog is showing.
    extensions_open: bool,
    /// The master switch while the dialog is open.
    extensions_enabled: bool,
    /// One switch per row, in `extensions_rows` order.
    extensions_checked: Vec<bool>,
    /// The rows the open dialog lists.
    extensions_rows: Vec<Found>,
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
    /// The id the next pane is given, unique across tabs.
    next_pane: u64,
    panel: ElementRef<SwapChainPanel>,
    pub(super) focus: ElementRef<Border>,
    find_field: ElementRef<TextBox>,
    palette_field: ElementRef<TextBox>,
    surface: Rc<RefCell<Option<Surface>>>,
    _observation: ElementObservation,
    timer: Option<ComponentTimer>,
    sides: input::ModifierSides,
    blink_on: bool,
    blink_at: Instant,
    /// When the trace last wrote a heartbeat, so a log that stops says the
    /// timer stopped with it.
    trace_at: Instant,
    focused: bool,
    /// Whether the first focus request already succeeded, so a tab click is not
    /// immediately undone by the next tick.
    focus_attempted: bool,
    /// The key press whose character has not arrived yet.
    ///
    /// A WinUI press is raised as `KeyDown` and then `CharacterReceived`; a
    /// Win32 input record carries both, so the press is held until its
    /// character arrives (ADR 056).
    pending: Option<PendingKey>,
    /// Whether the pane should take the keyboard back after XAML hands it to
    /// the window on a space, until the instant it stops trying.
    restore_focus_until: Option<Instant>,
    /// The scrollbar's drag state.
    scrollbar: scrollbar::Scrollbar,
    /// The tab the pointer is over, if any.
    hovered_tab: Option<u64>,
    /// The tab whose close button the pointer is over, if any.
    hovered_close: Option<u64>,
    /// The active terminal's background, so the selected tab matches it.
    active_background: Color,
    /// Whether the pressed pointer is on the scrollbar rather than the grid.
    scrollbar_press: bool,
    /// The cell a forwarded move last reported, so a still pointer is silent.
    last_mouse_cell: Option<(i32, i32)>,
    /// The cell and block flag a left press started on, so a drag begins a
    /// selection: a click alone selects nothing (ADR 053).
    press: Option<((i32, i32), bool)>,
    /// The pane the current press started in.
    press_pane: Option<PaneId>,
    /// The split whose divider the pointer is dragging.
    divider: Option<(SplitPath, Axis, PaneRect)>,
    /// The pane the pointer is over, so the wheel can be aimed at it.
    pointer_pane: Option<PaneId>,
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
    /// A key release; WinUI raises it as its own event, not as a press with a
    /// status flag.
    KeyUp(KeyEventInfo),
    Char(CharacterEventInfo),
    Resize {
        width: f64,
        height: f64,
        scale_x: f32,
        scale_y: f32,
    },
    /// The tab strip selected `id`, or nothing.
    Select(Option<u64>),
    /// The pointer entered (Some) or left (None) a tab.
    HoverTab(Option<u64>),
    /// The pointer entered (Some) or left (None) a tab's close button.
    HoverClose(Option<u64>),
    NewTab,
    /// The user asked for the tab with this id to close.
    CloseTab(u64),
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
    /// The command palette's filter changed.
    PaletteQuery(String),
    /// Moves the palette's selection.
    PaletteStep(bool),
    /// Runs the palette's selected row.
    PaletteRun,
    /// Runs the palette row at this filtered index.
    PaletteChoose(usize),
    /// Closes the palette.
    PaletteClose,
    /// Opens a tab on the named profile.
    MenuProfile(String),
    /// Opens the settings dialog from the drop-down.
    MenuSettings,
    /// Opens the extensions dialog from the drop-down.
    MenuExtensions,
    /// One switch in the extensions dialog, by row index.
    ExtensionsToggle(usize, bool),
    /// The extensions dialog's master switch.
    ExtensionsAll(bool),
    /// Saves the extensions dialog's switches, then closes it.
    ExtensionsSave,
    /// Closes the extensions dialog without saving.
    ExtensionsClose,
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

        // A fresh installation is given a file to edit, as the GTK shell does
        // (ADR 034). A broken file still yields the defaults.
        settings::ensure_file();
        let loaded = settings::load_with_report();
        let settings_path = loaded.path.clone();
        let settings_stamp = settings_path.as_deref().and_then(settings::stamp);
        let (template, bindings, font_size) = resolved_settings(&loaded.settings);
        let settings_window = SettingsWindow::new(&loaded);
        let settings = loaded.settings;
        let profiles = offered_profiles(&settings);

        let face = face_path();
        let fonts = font_setup(&face, font_size, 1.0, grid);
        let cell = fonts.settings.font.cell_size;

        let panel = ElementRef::<SwapChainPanel>::new();
        let surface = Rc::new(RefCell::new(None::<Surface>));
        let first_pane = spawn_pane(PaneId(0), &template, grid, font_size, 1.0, &face, &surface).expect("a shell");
        let tab = Tab {
            id: 0,
            panes: vec![first_pane],
            layout: Layout::leaf(PaneId(0)),
            focused: PaneId(0),
            terminal: template.clone(),
            title: template.title.clone(),
        };
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

        let mut shell = Self {
            tabs: Tabs::new(tab),
            next_tab: 1,
            template,
            bindings,
            settings,
            profiles,
            settings_window,
            menu_open: false,
            commands: Palette::default(),
            about_open: false,
            extensions: None,
            extensions_open: false,
            extensions_enabled: false,
            extensions_checked: Vec::new(),
            extensions_rows: Vec::new(),
            header_width: 0.0,
            header,
            _header_observation: header_observation,
            settings_path,
            settings_stamp,
            settings_at: Instant::now(),
            next_pane: 1,
            panel,
            focus: ElementRef::<Border>::new(),
            find_field: ElementRef::<TextBox>::new(),
            palette_field: ElementRef::<TextBox>::new(),
            surface,
            _observation: observation,
            timer,
            sides: input::ModifierSides::default(),
            pending: None,
            restore_focus_until: None,
            scrollbar: scrollbar::Scrollbar::default(),
            hovered_tab: None,
            hovered_close: None,
            active_background: Color::rgb(12, 12, 12),
            scrollbar_press: false,
            blink_on: true,
            blink_at: Instant::now(),
            trace_at: Instant::now(),
            focused: false,
            focus_attempted: false,
            last_mouse_cell: None,
            press: None,
            press_pane: None,
            divider: None,
            pointer_pane: None,
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
        };

        if std::env::var_os("WINTER_OPEN_PALETTE").is_some() {
            shell.open_palette();
        }
        shell.start_extensions();
        if std::env::var_os("WINTER_OPEN_EXTENSIONS").is_some() {
            shell.open_extensions();
        }
        if std::env::var_os("WINTER_SPLIT").is_some() {
            shell.split_pane(Axis::Horizontal);
        }
        shell
    }

    fn view(&self, _input: &(), context: &mut ViewContext<Self>) -> View {
        let keys = context.sender();
        let characters = keys.clone();
        let add = keys.clone();
        let got = keys.clone();
        let lost = keys.clone();
        let pressed = keys.clone();
        let moved = keys.clone();
        let released = keys.clone();
        let dialog = keys.clone();
        let menu_sender = keys.clone();
        let escape = keys.clone();
        let releases = keys.clone();
        let bindings = Rc::clone(&self.bindings);

        // The strip is ours, not a `TabView`: only then can a tab carry the
        // terminal's own background, a hover fill and (later) an icon (ADR
        // 064). A tab also has to fill the row's height -- a 34-DIP tab in a
        // 40-DIP row leaves a band the pane cannot cover -- so it is left
        // without a height and stretches, and the selected tab ends flush with
        // the pane instead of floating above it.
        let titlebar = TITLEBAR;
        let active_background = self.active_background;
        let hovered_tab = self.hovered_tab;
        let hovered_close = self.hovered_close;
        let selected = self.tabs.index();

        // A theme may name the strip's fills. A fill it leaves out keeps the
        // one the window would have chosen anyway, so the default window is the
        // terminal's colour from the selected tab down to the pane, and a theme
        // that asks for something else gets it (ADRs 035, 064).
        let theme = self.settings.theme();
        let focused = self.focused;
        let row = theme
            .and_then(|theme| theme.tab_row_background(focused))
            .and_then(theme_color)
            .unwrap_or(titlebar);
        let active_fill = theme
            .and_then(|theme| theme.tab_background(true))
            .and_then(theme_color)
            .unwrap_or(active_background);
        let idle_fill = theme
            .and_then(|theme| theme.tab_background(false))
            .and_then(theme_color)
            .unwrap_or(row);

        let mut strip_children: Vec<KeyedView> = Vec::new();
        // Equal widths, as upstream's `TabView` uses: a tab that does not fit the
        // window shrinks its title rather than pushing the new-tab group under
        // the caption buttons (ADR 068).
        let room = tab_room(pane_width(self.panel_size.0));
        let count = self.tabs.len().max(1);
        let gaps = count.saturating_sub(1) as f64;
        let tab_width = ((room - gaps) / count as f64).clamp(MIN_TAB_WIDTH, MAX_TAB_WIDTH);
        for index in 0..self.tabs.len() {
            let Some(tab) = self.tabs.get(index) else {
                continue;
            };
            let id = tab.id;
            let is_active = selected == index;
            let background = if is_active {
                active_fill
            } else if hovered_tab == Some(id) {
                blend(idle_fill, active_fill, HOVER_MIX)
            } else {
                idle_fill
            };
            // The fill names no foreground, so the label picks a side and an
            // inactive one is dimmed towards it -- the same rule the GTK and macOS strips
            // applies, and what keeps a light scheme readable (ADR 035).
            let foreground = contrasting_text(background);
            let label = if is_active {
                foreground
            } else {
                blend(foreground, background, LABEL_DIM)
            };

            let select = keys.clone();
            let close = keys.clone();
            let enter_close = keys.clone();
            let leave_close = keys.clone();
            let enter = keys.clone();
            let leave = keys.clone();

            // A `Border`, not a `Button`: the stock button style's padding
            // leaves a 22-DIP box a few DIPs of content, and a glyph centred in
            // what is left is not a cross. The press also has to reach the
            // strip, which is why the strip selects by id (ADR 067).
            let close_button = Border::new()
                .width(CLOSE_BUTTON)
                .height(CLOSE_BUTTON)
                // A transparent brush, not the default null one: a `Border`
                // with no background is not hit-testable, and the press would
                // fall through to the tab.
                .background(if hovered_close == Some(id) {
                    blend(background, label, CLOSE_HOVER_MIX)
                } else {
                    Color::transparent()
                })
                .corner_radius(CornerRadius::uniform(4.0))
                .vertical_alignment(VerticalAlignment::Center)
                .content(
                    TextBlock::new()
                        .text(CLOSE_GLYPH)
                        .font_family(SYMBOL_FONT)
                        .font_size(10.0)
                        .foreground(label)
                        .horizontal_alignment(HorizontalAlignment::Center)
                        .vertical_alignment(VerticalAlignment::Center),
                )
                .on_pointer_entered(move |_| {
                    let _ = enter_close.send(Message::HoverClose(Some(id)));
                })
                .on_pointer_exited(move |_| {
                    let _ = leave_close.send(Message::HoverClose(None));
                })
                .on_pointer_pressed(move |info: PointerEventInfo| {
                    if !info.is_left_button_pressed {
                        return;
                    }
                    let _ = close.send(Message::CloseTab(id));
                });

            // The shell's own icon, as Windows Terminal shows it (ADR 066).
            let icon = profile_icon::png_for(tab.program());
            let title = TextBlock::new()
                .text(tab.label())
                .font_size(12.0)
                .foreground(label)
                .vertical_alignment(VerticalAlignment::Center)
                .text_trimming(TextTrimming::CharacterEllipsis)
                .max_lines(1)
                .max_width(title_room(tab_width, icon.is_some()));

            let mut body: Vec<View> = Vec::new();
            if let Some(icon) = icon {
                body.push(
                    Image::new()
                        .source_data(EncodedImage::new(icon))
                        .width(TAB_ICON)
                        .height(TAB_ICON)
                        .stretch(Stretch::Uniform)
                        .vertical_alignment(VerticalAlignment::Center)
                        .grid_column(0)
                        .into(),
                );
            }
            body.push(title.grid_column(1).into());
            body.push(close_button.grid_column(2).into());

            // A Grid, not a stack: the title's column is the star, so a short
            // title still leaves the close button at the tab's right edge.
            let body = Grid::new()
                .columns([GridLength::Auto, GridLength::STAR, GridLength::Auto])
                .column_spacing(TAB_GAP)
                .children(body);

            let item = Border::new()
                .background(background)
                .width(tab_width)
                .corner_radius(CornerRadius::new(6.0, 6.0, 0.0, 0.0))
                // The bottom padding and the matching negative margin cancel out
                // for the content and not for the fill: the label stays centred
                // in the row, and the tab's colour runs past the slot to the
                // pane. Anything past the row is behind the pane.
                .padding(Thickness::new(TAB_PADDING_LEFT, 0.0, TAB_PADDING_RIGHT, TAB_OVERHANG))
                .margin(Thickness::new(0.0, 0.0, 0.0, -TAB_OVERHANG))
                .content(body)
                .on_pointer_pressed(move |info: PointerEventInfo| {
                    if !info.is_left_button_pressed {
                        return;
                    }
                    let _ = select.send(Message::Select(Some(id)));
                })
                .on_pointer_entered(move |_| {
                    let _ = enter.send(Message::HoverTab(Some(id)));
                })
                .on_pointer_exited(move |_| {
                    let _ = leave.send(Message::HoverTab(None));
                });
            // Keyed by the tab's id, so closing the middle tab updates the
            // elements that remain instead of re-binding their handlers to a
            // shifted index.
            strip_children.push(KeyedView::new(tab_key(id), item));
        }

        let strip = StackPanel::new()
            .orientation(Orientation::Horizontal)
            .spacing(1.0)
            .height(TAB_ROW_HEIGHT)
            // A cap, not a clip: if the tabs still do not fit, the overflow stays
            // inside this slot so the new-tab group never moves (ADR 068).
            .max_width(room)
            .vertical_alignment(VerticalAlignment::Bottom)
            .keyed_children(strip_children);

        let menu_open = self.menu_open;
        let palette_open = self.commands.open;
        let find_open = self
            .tabs
            .active()
            .and_then(|tab| tab.focused_pane())
            .is_some_and(|pane| pane.find.open);

        let mut pane_children: Vec<View> = vec![SwapChainPanel::new().element_ref(&self.panel).into()];
        if let Some(tab) = self.tabs.active() {
            let area = PaneRect {
                x: 0.0,
                y: 0.0,
                width: self.panel_size.0,
                height: self.panel_size.1,
            };
            let mut bars: Vec<View> = Vec::new();
            let mut fills: Vec<View> = Vec::new();
            let (cell_width, cell_height) = self.dip_cell();
            for (id, fraction) in tab.layout.rects() {
                let Some(pane) = tab.pane(id) else {
                    continue;
                };
                let rect = fraction_rect(area, fraction);

                // Each pane's gutter and sub-cell remainder keep that pane's
                // own background over the shared swap chain. They are added
                // before the scrollbars.
                let (background, _) = pane.control.default_colors();
                let fill = color_of(background);
                let grid_width = cell_width * f64::from(pane.grid.0);
                let grid_height = cell_height * f64::from(pane.grid.1);
                let content_width = (rect.width - bar::WIDTH).max(0.0);
                fills.push(
                    Rectangle::new()
                        .fill(fill)
                        .width(bar::WIDTH)
                        .height(rect.height)
                        .canvas_left(rect.x + rect.width - bar::WIDTH)
                        .canvas_top(rect.y)
                        .into(),
                );
                if grid_height < rect.height {
                    fills.push(
                        Rectangle::new()
                            .fill(fill)
                            .width(content_width)
                            .height(rect.height - grid_height)
                            .canvas_left(rect.x)
                            .canvas_top(rect.y + grid_height)
                            .into(),
                    );
                }
                if grid_width < content_width {
                    fills.push(
                        Rectangle::new()
                            .fill(fill)
                            .width(content_width - grid_width)
                            .height(grid_height)
                            .canvas_left(rect.x + grid_width)
                            .canvas_top(rect.y)
                            .into(),
                    );
                }

                if let Some(track) = bar::overlay(
                    pane.view.scroll(),
                    pane.renderer.data(),
                    pane.control.size().1,
                    rect.height,
                ) {
                    bars.push(
                        Grid::new()
                            .width(rect.width)
                            .height(rect.height)
                            .canvas_left(rect.x)
                            .canvas_top(rect.y)
                            .children(vec![track])
                            .into(),
                    );
                }
            }
            if !fills.is_empty() {
                pane_children.push(Canvas::new().children(fills).into());
            }
            if !bars.is_empty() {
                pane_children.push(Canvas::new().children(bars).into());
            }

            if tab.panes.len() > 1 {
                let mut chrome: Vec<View> = Vec::new();
                for (_path, axis, fraction, ratio) in tab.layout.splits() {
                    let split = fraction_rect(area, fraction);
                    let line = match axis {
                        Axis::Horizontal => Rectangle::new()
                            .fill(Color::argb(90, 128, 128, 128))
                            .width(1.0)
                            .height(split.height)
                            .canvas_left(split.x + split.width * f64::from(ratio) - 0.5)
                            .canvas_top(split.y),
                        Axis::Vertical => Rectangle::new()
                            .fill(Color::argb(90, 128, 128, 128))
                            .width(split.width)
                            .height(1.0)
                            .canvas_left(split.x)
                            .canvas_top(split.y + split.height * f64::from(ratio) - 0.5),
                    };
                    chrome.push(line.into());
                }
                if let Some(rect) = self.pane_rect(tab.focused) {
                    chrome.push(
                        Border::new()
                            .background(Color::transparent())
                            .border_brush(blend(self.active_background, Color::rgb(255, 255, 255), 0.45))
                            .border_thickness(Thickness::uniform(1.0))
                            .width((rect.width - 1.0).max(1.0))
                            .height((rect.height - 1.0).max(1.0))
                            .canvas_left(rect.x + 0.5)
                            .canvas_top(rect.y + 0.5)
                            .into(),
                    );
                }
                pane_children.push(Canvas::new().children(chrome).into());
            }

            if let Some(pane) = tab.focused_pane()
                && pane.find.open
                && let Some(rect) = self.pane_rect(tab.focused)
            {
                pane_children.push(
                    Grid::new()
                        .width(rect.width)
                        .height(rect.height)
                        .canvas_left(rect.x)
                        .canvas_top(rect.y)
                        .children(vec![find_bar(pane, &dialog, &self.find_field)])
                        .into(),
                );
            }
        }
        if menu_open {
            pane_children.push(profile_menu(&self.profiles, &dialog, menu_left(self)));
        }
        if palette_open {
            let rows = self.palette_rows();
            pane_children.push(command_palette::card(
                &self.commands.filter,
                &rows,
                self.commands.selected,
                &self.palette_field,
                &dialog,
            ));
        }
        let pane_content = Grid::new().children(pane_children);

        let pane = Border::new()
            .is_tab_stop(true)
            .capture_pointer_on_press(true)
            .focus_on_pointer_release(true)
            .element_ref(&self.focus)
            .background(self.active_background)
            // The text starts in from the pane's left edge (ADR 068). The panel
            // reports its own width, which is this much narrower than the pane;
            // `pane_width` and the pointer mapping undo the difference.
            .padding(Thickness::new(PANE_PADDING_LEFT, 0.0, 0.0, 0.0))
            .content(pane_content)
            .on_key_up(RoutedCallback::new(move |info: KeyEventInfo| {
                if palette_open {
                    return true;
                }
                if find_open || menu_open {
                    return false;
                }
                let _ = releases.send(Message::KeyUp(info));
                // XAML's invocation runs on the space key-up, so the key-up a
                // terminal consumed must be marked handled too or the pane
                // loses focus.
                true
            }))
            .on_preview_key_down(RoutedCallback::new(move |info: KeyEventInfo| {
                let pressed = !info.status.is_released;
                let bound = key_of(info.key)
                    .and_then(|key| input::chord_action(&bindings, key, modifiers(info.modifiers)))
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

                if palette_open {
                    // The palette's field owns the keyboard. Escape closes it
                    // from the pane, because the pane's preview runs before the
                    // card's; every other key reaches the field so it can type.
                    if pressed && matches!(key_of(info.key), Some(Key::Escape)) {
                        let _ = keys.send(Message::PaletteClose);
                        return true;
                    }
                    return false;
                }

                // A bound chord is the window's, so the toolkit must not also
                // type it. A printable key with no binding still has to raise
                // `CharacterReceived`, which is the text path; the keys the
                // terminal encodes itself are marked handled.
                let _ = keys.send(Message::Key(info));
                // Space is handled here: XAML treats an unhandled space as an
                // invocation and moves focus off the pane. Printing keys stay
                // unhandled so their character event still arrives.
                bound || !matches!(key_of(info.key), Some(Key::Char(_)))
            }))
            .on_character_received(RoutedCallback::new(move |info: CharacterEventInfo| {
                if find_open || menu_open || palette_open {
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

        // The `+` and the drop-down are one split control, as in Windows
        // Terminal's `SplitButton` (`TabRowControl.xaml`): a single 24-DIP pill
        // with a hairline between the halves, each half its own press. The
        // binding has no `SplitButton`, so the pill is a `Border` around two
        // flat buttons.
        let glyph = |text: &str| {
            TextBlock::new()
                .text(text)
                .font_family(SYMBOL_FONT)
                .font_size(10.0)
                .horizontal_alignment(HorizontalAlignment::Center)
                .vertical_alignment(VerticalAlignment::Center)
        };
        let new_tab = Button::new()
            .content(glyph(PLUS_GLYPH))
            .style(ButtonStyle::Subtle)
            .width(NEW_TAB_WIDTH)
            .height(BUTTON_GROUP_HEIGHT)
            .min_height(0.0)
            .on_click(move || {
                let _ = add.send(Message::NewTab);
            });
        let drop_down = Button::new()
            .content(glyph(CHEVRON_GLYPH))
            .style(ButtonStyle::Subtle)
            .width(DROP_DOWN_WIDTH)
            .height(BUTTON_GROUP_HEIGHT)
            .min_height(0.0)
            .on_click(move || {
                let _ = menu_sender.send(Message::MenuToggle);
            });
        let divider = Border::new()
            .width(1.0)
            .height(16.0)
            .background(blend(row, Color::rgb(255, 255, 255), 0.18))
            .vertical_alignment(VerticalAlignment::Center);
        let buttons = Border::new()
            .background(blend(row, Color::rgb(255, 255, 255), 0.06))
            .corner_radius(CornerRadius::uniform(4.0))
            .height(BUTTON_GROUP_HEIGHT)
            // Centred in the row, as upstream's 32-DIP row with `Margin="0,4"`
            // centres its 24-DIP button.
            .vertical_alignment(VerticalAlignment::Center)
            .content(
                StackPanel::new()
                    .orientation(Orientation::Horizontal)
                    .spacing(0.0)
                    .children((new_tab, divider, drop_down)),
            );
        let header_panel: View = StackPanel::new()
            .orientation(Orientation::Horizontal)
            .spacing(TAB_ROW_GAP)
            .children((strip, buttons))
            .into();
        // The Grid wrapping the header is only there so its width can be
        // measured, which is how the menu finds the buttons' left edge (ADR
        // 052), and so the row can sit at the bottom of the title bar's slot
        // whatever height that turns out to be.
        let header = Grid::new()
            .element_ref(&self.header)
            .vertical_alignment(VerticalAlignment::Bottom)
            .children(vec![header_panel]);

        // The tab strip is the title bar, as in Windows Terminal: the TitleBar
        // control extends the client area and hands the system the drag region,
        // leaving room for the caption buttons on its right.
        let title_bar = Border::new()
            .background(row)
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
        } else if self.extensions_open {
            root.content_dialog(extensions_dialog(
                &self.extensions_rows,
                self.extensions_enabled,
                &self.extensions_checked,
                &dialog,
            ))
        } else if self.about_open {
            root.content_dialog(about_dialog(&dialog))
        } else {
            root
        }
    }

    fn update(&mut self, message: Message, context: &ComponentContext<Self>) {
        match message {
            Message::Tick => self.tick(context),
            Message::Key(info) => self.key(info, true, context),
            Message::KeyUp(info) => self.key(info, false, context),
            Message::Char(info) => self.character(info),
            Message::Resize {
                width,
                height,
                scale_x,
                scale_y,
            } => self.resize(width, height, scale_x, scale_y),
            Message::Select(id) => self.select(id),
            Message::HoverTab(id) => self.hovered_tab = id,
            Message::HoverClose(id) => self.hovered_close = id,
            Message::NewTab => self.open_tab(),
            Message::CloseTab(id) => self.close_tab(id, context),
            Message::Focused(focused) => {
                self.focused = focused;
                if focused {
                    self.restore_focus_until = None;
                } else if self.restore_focus_until.is_some_and(|until| Instant::now() <= until) {
                    // XAML gave the keyboard to the window after the terminal
                    // consumed a space; it belongs to the pane.
                    self.restore_focus_until = None;
                    let _ = self.focus.request_focus();
                }
            }
            Message::Pressed(info) => self.pointer_pressed(info),
            Message::Moved(info) => self.pointer_moved(info),
            Message::Released(info) => self.pointer_released(info),
            Message::Window(handle) => {
                self._wheel = wheel::install(handle);
                icon::apply(handle);
            }
            Message::Settings(message) => self.settings_message(message),
            Message::FindQuery(text) => self.find_query(text),
            Message::FindStep(forward) => self.find_step(forward),
            Message::FindClose => self.close_find(),
            Message::MenuToggle => self.menu_open = !self.menu_open,
            Message::MenuClose => self.close_menu(),
            Message::PaletteQuery(text) => self.palette_query(text),
            Message::PaletteStep(forward) => self.palette_step(forward),
            Message::PaletteRun => self.run_selected_palette(context),
            Message::PaletteChoose(index) => self.choose_palette(index, context),
            Message::PaletteClose => self.close_palette(),
            Message::MenuProfile(name) => self.open_profile_tab(&name),
            Message::MenuSettings => {
                self.close_menu();
                if !self.settings_window.is_open() {
                    self.toggle_settings();
                }
            }
            Message::MenuExtensions => {
                self.close_menu();
                self.open_extensions();
            }
            Message::ExtensionsToggle(index, on) => {
                if let Some(checked) = self.extensions_checked.get_mut(index) {
                    *checked = on;
                }
            }
            Message::ExtensionsAll(on) => self.extensions_enabled = on,
            Message::ExtensionsSave => self.save_extensions_dialog(),
            Message::ExtensionsClose => self.close_extensions(),
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
        // What the extensions observe: the pty events this tick saw, in order.
        let mut observed = Vec::new();
        let mut settings_reloaded = false;
        for index in 0..self.tabs.len() {
            let Some(tab) = self.tabs.get_mut(index) else {
                continue;
            };
            for pane in &mut tab.panes {
                pane.control.pump();

                let mut done = false;
                for event in pane.control.take_events(now) {
                    match event {
                        TerminalEvent::Exited => {
                            observed.push(extensions::Event::PaneExited {
                                tab: index,
                                pane: pane.id.0.to_string(),
                            });
                            done = true;
                        }
                        TerminalEvent::Notification { title, body } => {
                            observed.push(extensions::Event::Notification {
                                title: title.clone(),
                                body: body.clone(),
                            });
                        }
                        TerminalEvent::Bell => observed.push(extensions::Event::Bell),
                    }
                }

                if let Some(title) = pane.control.title().map(str::to_string)
                    && pane.title.as_deref() != Some(title.as_str())
                {
                    pane.title = Some(title);
                }

                if done {
                    exited.push((index, pane.id));
                }
            }
        }

        // Closing from the end keeps the earlier indices valid.
        for (index, pane) in exited.into_iter().rev() {
            if self.tabs.get(index).is_some_and(|tab| tab.layout.is_single()) {
                if !self.tabs.close(index) {
                    let _ = context.close_window();
                    return;
                }
            } else {
                self.remove_pane(index, pane);
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
        // retried until the element exists. The palette's field is the same.
        let palette_focus = self.commands.open && self.commands.focus;
        if palette_focus && self.palette_field.request_focus() && self.commands.open {
            self.commands.focus = false;
        }
        let find_focus = self
            .tabs
            .active()
            .and_then(|tab| tab.focused_pane())
            .is_some_and(|pane| pane.find.open && pane.find.focus);
        if find_focus
            && self.find_field.request_focus()
            && let Some(pane) = self.tabs.active_mut().and_then(|tab| tab.focused_pane_mut())
        {
            pane.find.focus = false;
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
            settings_reloaded = self.poll_settings();
            self.watch_extensions();
        }

        // Extensions observe the window after it has acted: the marks this
        // tick left on the focused pane, and whatever the poll found.
        let marks = self
            .tabs
            .active_mut()
            .and_then(|tab| tab.focused_pane_mut())
            .map(|pane| (pane.id, pane.control.scroll_facts().1));
        self.pump_extensions(
            marks.as_ref().map(|(id, marks)| (*id, marks.as_slice())),
            observed,
            settings_reloaded,
        );

        let (painted, background) = self.paint_panes();

        // A heartbeat, so a log that stops says the timer did too.
        if now.duration_since(self.trace_at) >= Duration::from_secs(1) {
            self.trace_at = now;
            trace::line(format_args!("tick grid={:?} painted={painted}", self.grid));
        }

        // The selected tab is painted in the pane's own colour, so the strip has
        // to learn it from the palette the frame was painted with -- a scheme
        // change or an OSC 11 reply moves both together.
        self.active_background = color_of(background);

        self.timer = Some(context.set_timeout(Duration::from_millis(16), Message::Tick));
    }

    /// Refreshes every pane and presents the active tab as one frame.
    fn paint_panes(&mut self) -> (bool, u32) {
        let failed = (false, 0x000C_0C0C);
        let Some(tab) = self.tabs.active() else {
            return failed;
        };
        let (background, _) = tab
            .focused_pane()
            .map_or((0x000C_0C0C, 0x00CC_CCCC), |pane| pane.control.default_colors());
        let surface = Rc::clone(&self.surface);
        {
            let Ok(mut slot) = surface.try_borrow_mut() else {
                return (false, background);
            };
            let Some(surface) = slot.as_mut() else {
                return (false, background);
            };
            if surface.begin_composite(background).is_err() {
                return (false, background);
            }
        }

        let blink_on = self.blink_on;
        let software = self
            .tabs
            .active()
            .and_then(|tab| tab.panes.first())
            .is_some_and(|pane| pane.shared.is_some());
        let (scale_x, scale_y) = (
            f64::from(finite_scale(self.panel_scale.0)),
            f64::from(finite_scale(self.panel_scale.1)),
        );
        let area = PaneRect {
            x: 0.0,
            y: 0.0,
            width: self.panel_size.0,
            height: self.panel_size.1,
        };
        let mut painted = true;

        {
            let Shell { tabs, .. } = &mut *self;
            let Some(tab) = tabs.active_mut() else {
                if let Ok(mut slot) = surface.try_borrow_mut()
                    && let Some(surface) = slot.as_mut()
                {
                    let _ = surface.end_composite();
                }
                return (false, background);
            };
            let rects = tab.layout.rects();
            for (id, fraction) in rects {
                let Some(pane) = tab.pane_mut(id) else {
                    continue;
                };
                let rect = fraction_rect(area, fraction);
                pane.renderer
                    .data_mut()
                    .refresh(&mut pane.control, &pane.view, blink_on, Some(&pane.find));

                // The engine resolves a cell's colours through the renderer's
                // table, not the data's copy, so the shell hands it the pane's
                // palette (`RenderSettings::set_color_table`).
                let (cell_background, foreground) = pane.control.default_colors();
                let _ = pane.renderer.set_default_colors(cell_background, foreground);
                sync_palette(&mut pane.renderer, &pane.control);
                pane.renderer.invalidate_all();

                let offset = ((rect.x * scale_x).round() as i32, (rect.y * scale_y).round() as i32);
                pane.offset.set(offset);

                if pane.renderer.paint_frame().is_err() {
                    painted = false;
                    trace::line(format_args!("frame error on pane {}", pane.id.0));
                    continue;
                }
                if software && let Some(shared) = &pane.shared {
                    let width = shared.width();
                    let height = shared.height();
                    // The surface is borrowed only for the blit: a pane's own
                    // backend borrows the same cell while it paints.
                    if let Ok(mut slot) = surface.try_borrow_mut()
                        && let Some(surface) = slot.as_mut()
                    {
                        shared.with_pixels(|pixels| {
                            let _ = surface.blit_at(pixels, width, height, offset.0, offset.1);
                        });
                    }
                }
            }
        }

        if let Ok(mut slot) = surface.try_borrow_mut()
            && let Some(surface) = slot.as_mut()
            && surface.end_composite().is_err()
        {
            painted = false;
        }
        (painted, background)
    }

    /// Fits every pane to the panel and rasterises the face for its scale.
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

        self.grid = grid_for(grid_width(width), height, self.cell, scale_x, scale_y);
        self.apply_layout();
    }

    /// Replaces every pane's renderer with one whose face is `scale` times the
    /// DIP size.
    ///
    /// The pane's colours are re-applied on the next tick, from the active tab's
    /// palette, so a reloaded face never falls back to the built-in table.
    fn rebuild_face(&mut self, scale: f32) {
        for index in 0..self.tabs.len() {
            let Some(tab) = self.tabs.get_mut(index) else {
                continue;
            };
            for pane in &mut tab.panes {
                let fonts = font_setup(&self.face, self.font_size, scale, pane.grid);
                self.cell = fonts.settings.font.cell_size;
                let (renderer, shared) = renderer_for(
                    fonts,
                    pane.grid,
                    Rc::clone(&self.surface),
                    &self.face,
                    Rc::clone(&pane.offset),
                );
                pane.renderer = renderer;
                pane.shared = shared;
            }
        }
    }

    /// Tells every pane's terminal and renderer how large its grid is.
    fn apply_layout(&mut self) {
        let area = PaneRect {
            x: 0.0,
            y: 0.0,
            width: self.panel_size.0,
            height: self.panel_size.1,
        };
        let (cell, scale_x, scale_y) = (self.cell, self.panel_scale.0, self.panel_scale.1);
        for index in 0..self.tabs.len() {
            let Some(tab) = self.tabs.get_mut(index) else {
                continue;
            };
            for (id, fraction) in tab.layout.rects() {
                let Some(pane) = tab.pane_mut(id) else {
                    continue;
                };
                let cells = pane_grid(fraction_rect(area, fraction), cell, scale_x, scale_y);
                if cells == pane.grid {
                    continue;
                }
                pane.grid = cells;
                pane.control.resize(cells.0, cells.1);
                let target = target_size(cell, cells);
                let viewport = InclusiveRect::new(0, 0, cells.0.max(1) - 1, cells.1.max(1) - 1);
                let _ = pane.renderer.resize(target, viewport);
            }
        }
    }

    /// Reloads the settings file when a save changed its stamp.
    ///
    /// A dirty draft is not overwritten: the dialog says the file changed and
    /// lets Save win or Revert load it (ADR 025).
    fn poll_settings(&mut self) -> bool {
        let Some(path) = &self.settings_path else {
            return false;
        };
        if settings::stamp(path) == self.settings_stamp {
            return false;
        }

        let dirty = self
            .settings_window
            .draft()
            .is_some_and(|draft| draft.differs_from(&self.settings));
        if dirty {
            self.settings_window.set_conflict();
            false
        } else {
            self.reload_settings();
            true
        }
    }

    /// Re-reads the settings file and applies what changed.
    ///
    /// Chords and the font size take effect at once, and a scheme repaints
    /// every tab. A changed profile affects the next tab, because a running
    /// shell keeps the command it started with.
    pub(super) fn reload_settings(&mut self) {
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

        let previous = self.settings.extensions.clone();
        self.settings = loaded.settings;
        self.profiles = offered_profiles(&self.settings);
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
                grid_width(self.panel_size.0),
                self.panel_size.1,
                self.cell,
                self.panel_scale.0,
                self.panel_scale.1,
            );
            self.grid = grid;
            self.apply_layout();
        }

        let scheme = self.template.scheme.clone();
        for index in 0..self.tabs.len() {
            if let Some(tab) = self.tabs.get_mut(index) {
                for pane in &mut tab.panes {
                    pane.control.apply_scheme(scheme.as_ref());
                }
            }
        }
        // A changed `[extensions]` table rebuilds the engine without a restart
        // (ADR 088). A refused table never reaches here, so a half-written file
        // leaves the running engine alone.
        if self.settings.extensions != previous {
            self.reload_extensions();
        }
    }

    /// Shows or hides the settings dialog; closing discards unsaved edits.
    fn toggle_settings(&mut self) {
        if self.settings_window.is_open() {
            self.settings_window.close();
            let _ = self.focus.request_focus();
        } else {
            if self.extensions_open {
                self.close_extensions();
            }
            self.settings_window.open(&self.settings);
        }
    }

    /// Opens the extensions dialog on the current switches.
    fn open_extensions(&mut self) {
        if self.extensions_open || self.settings_window.is_open() || self.about_open {
            return;
        }
        self.extensions_rows = self.installed_extensions();
        self.extensions_enabled = self.settings.extensions.enabled;
        self.extensions_checked = self.extensions_rows.iter().map(|row| row.enabled).collect();
        self.extensions_open = true;
    }

    /// Writes the switches the dialog holds and closes it.
    ///
    /// Loading changes on the next start, not now: dropping a live realm is a
    /// different operation from re-reading a document.
    fn save_extensions_dialog(&mut self) {
        let disabled: Vec<String> = self
            .extensions_rows
            .iter()
            .enumerate()
            .filter(|(index, row)| row.switchable && !self.extensions_checked.get(*index).copied().unwrap_or(true))
            .map(|(_, row)| row.id.clone())
            .collect();
        if let Err(error) = self.save_extensions(self.extensions_enabled, disabled) {
            eprintln!("winter: cannot save extensions: {error}");
        }
        self.close_extensions();
    }

    /// Closes the extensions dialog without saving.
    fn close_extensions(&mut self) {
        self.extensions_open = false;
        let _ = self.focus.request_focus();
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

    /// Shows or hides the focused pane's find bar.
    fn toggle_find(&mut self) {
        let Some(pane) = self.tabs.active_mut().and_then(|tab| tab.focused_pane_mut()) else {
            return;
        };
        pane.find.toggle();
        if pane.find.open {
            pane.find.refresh(&mut pane.control);
            pane.find.scroll_to(&pane.control, &mut pane.view);
        } else {
            let _ = self.focus.request_focus();
        }
    }

    /// Recomputes the matches and scrolls to the first.
    fn find_query(&mut self, text: String) {
        let Some(pane) = self.tabs.active_mut().and_then(|tab| tab.focused_pane_mut()) else {
            return;
        };
        pane.find.query = text;
        pane.find.refresh(&mut pane.control);
        pane.find.scroll_to(&pane.control, &mut pane.view);
    }

    /// Moves to the next (true) or previous match.
    fn find_step(&mut self, forward: bool) {
        let Some(pane) = self.tabs.active_mut().and_then(|tab| tab.focused_pane_mut()) else {
            return;
        };
        if pane.find.searched != pane.find.query {
            pane.find.refresh(&mut pane.control);
        }
        pane.find.step(forward);
        pane.find.scroll_to(&pane.control, &mut pane.view);
    }

    /// Closes the find bar and returns focus to the terminal.
    fn close_find(&mut self) {
        if let Some(pane) = self.tabs.active_mut().and_then(|tab| tab.focused_pane_mut()) {
            pane.find.hide();
        }
        let _ = self.focus.request_focus();
    }

    /// Shows or hides the command palette.
    fn toggle_palette(&mut self) {
        if self.commands.open {
            self.close_palette();
        } else {
            self.open_palette();
        }
    }

    /// Opens the palette with an empty filter and the first row selected.
    fn open_palette(&mut self) {
        // Only one overlay owns the keyboard at a time.
        self.menu_open = false;
        if let Some(pane) = self.tabs.active_mut().and_then(|tab| tab.focused_pane_mut()) {
            pane.find.hide();
        }
        self.commands.show();
    }

    /// Closes the palette and gives the keyboard back to the pane.
    fn close_palette(&mut self) {
        self.commands.hide();
        let _ = self.focus.request_focus();
    }

    /// Narrows the palette to what the field holds.
    fn palette_query(&mut self, text: String) {
        self.commands.filter = text;
        self.commands.selected = 0;
        let rows = self.palette_rows().len();
        self.commands.clamp(rows);
    }

    /// Moves the palette's selection one row, wrapping at both ends.
    fn palette_step(&mut self, forward: bool) {
        let rows = self.palette_rows().len();
        self.commands.step(rows, forward);
    }

    /// Runs the filtered row a pointer pressed.
    fn choose_palette(&mut self, index: usize, context: &ComponentContext<Self>) {
        self.commands.selected = index;
        self.run_selected_palette(context);
    }

    /// Closes the palette and runs the row Enter chose.
    fn run_selected_palette(&mut self, context: &ComponentContext<Self>) {
        let rows = self.palette_rows();
        let Some(entry) = rows.get(self.commands.selected).cloned() else {
            return;
        };
        self.close_palette();
        self.run_palette_command(entry.command, context);
    }

    /// Runs one palette row after the card has closed.
    fn run_palette_command(&mut self, command: palette::Command, context: &ComponentContext<Self>) {
        match command {
            palette::Command::Action(action) => {
                self.perform(action, context);
            }
            palette::Command::Profile(name) => self.open_profile_tab(&name),
            palette::Command::Extension(key) => self.run_extension(&key),
        }
    }

    /// The rows the current filter leaves, with the window's merged profiles.
    fn palette_rows(&self) -> Vec<palette::Entry> {
        let mut settings = self.settings.clone();
        settings.profiles = self.profiles.clone();
        let mut entries = palette::entries(&settings, palette_supports);
        if let Some(extensions) = &self.extensions {
            entries.extend(palette::extension_entries(
                extensions
                    .commands()
                    .into_iter()
                    .map(|command| (command.key, command.label)),
            ));
        }
        self.commands.rows(&entries).into_iter().cloned().collect()
    }

    /// The title the window shows: the focused pane's.
    fn title(&self) -> &str {
        self.tabs.active().map_or("Winter", Tab::label)
    }

    /// Opens a tab on the same profile and activates it.
    pub(super) fn open_tab(&mut self) {
        self.add_tab(self.template.clone());
    }

    /// Opens a tab on the profile the drop-down named and activates it.
    fn open_profile_tab(&mut self, name: &str) {
        self.close_menu();
        let Some(profile) = self.profiles.iter().find(|profile| profile.name == name).cloned() else {
            return;
        };
        let terminal = TerminalSettings::for_profile(&self.settings, Some(&profile));
        self.add_tab(terminal);
    }

    /// Hides the drop-down, returning the pointer to the pane.
    fn close_menu(&mut self) {
        self.menu_open = false;
    }

    /// Starts a terminal and adds it as the active tab.
    fn add_tab(&mut self, terminal: TerminalSettings) {
        let title = terminal.title.clone();
        let id = PaneId(self.next_pane);
        let Some(pane) = spawn_pane(
            id,
            &terminal,
            self.grid,
            self.font_size,
            self.scale,
            &self.face,
            &self.surface,
        ) else {
            return;
        };
        self.next_pane += 1;

        let tab_id = self.next_tab;
        self.next_tab += 1;
        self.tabs.add(Tab {
            id: tab_id,
            panes: vec![pane],
            layout: Layout::leaf(id),
            focused: id,
            terminal,
            title,
        });
        let index = self.tabs.index();
        self.resize_tab(index);
        let _ = self.focus.request_focus();
        self.report_extension(extensions::Event::TabOpened { index });
    }

    /// Splits the focused pane, starting the new shell at the size it will hold.
    fn split_pane(&mut self, axis: Axis) {
        let (focused, terminal, cells) = {
            let Some(tab) = self.tabs.active() else {
                return;
            };
            let focused = tab.focused;
            let cells = self.pane_rect(focused).map_or(self.grid, |rect| {
                pane_grid(
                    split_second(rect, axis),
                    self.cell,
                    self.panel_scale.0,
                    self.panel_scale.1,
                )
            });
            (focused, tab.terminal.clone(), cells)
        };

        let id = PaneId(self.next_pane);
        let Some(pane) = spawn_pane(
            id,
            &terminal,
            cells,
            self.font_size,
            self.scale,
            &self.face,
            &self.surface,
        ) else {
            return;
        };
        let Some(tab) = self.tabs.active_mut() else {
            return;
        };
        if !tab.layout.split(focused, axis, id) {
            return;
        }
        self.next_pane += 1;
        tab.panes.push(pane);
        tab.focused = id;

        let index = self.tabs.index();
        self.resize_tab(index);
        let _ = self.focus.request_focus();
        self.report_extension(extensions::Event::PaneSplit {
            tab: index,
            pane: id.0.to_string(),
        });
    }

    /// Closes the focused pane, or the tab when it is the last one.
    fn close_pane(&mut self, context: &ComponentContext<Self>) {
        let index = self.tabs.index();
        let Some(focused) = self.tabs.active().map(|tab| tab.focused) else {
            return;
        };
        if self.tabs.get(index).is_some_and(|tab| tab.layout.is_single()) {
            if !self.tabs.close(index) {
                let _ = context.close_window();
            } else {
                let _ = self.focus.request_focus();
                self.report_extension(extensions::Event::TabClosed { index });
            }
        } else {
            self.remove_pane(index, focused);
        }
    }

    /// Removes a pane, closing the tab when it was its only one.
    fn remove_pane(&mut self, index: usize, pane: PaneId) {
        if self.tabs.get(index).is_some_and(|tab| tab.layout.is_single()) {
            let _ = self.tabs.close(index);
            return;
        }
        {
            let Some(tab) = self.tabs.get_mut(index) else {
                return;
            };
            if !tab.layout.remove(pane) {
                return;
            }
            tab.panes.retain(|candidate| candidate.id != pane);
            if tab.focused == pane
                && let Some(next) = tab.layout.leaves().first().copied()
            {
                tab.focused = next;
            }
        }
        self.resize_tab(index);
    }

    /// Moves the focus to the nearest pane in `direction`.
    fn focus_pane(&mut self, direction: Direction) {
        let Some(tab) = self.tabs.active_mut() else {
            return;
        };
        let Some(next) = layout::neighbour(&tab.layout.rects(), tab.focused, direction) else {
            return;
        };
        tab.focused = next;
    }

    /// Grows the focused pane one step in `direction`.
    fn resize_pane(&mut self, direction: Direction) {
        let index = self.tabs.index();
        let Some(tab) = self.tabs.active_mut() else {
            return;
        };
        let focused = tab.focused;
        if !tab.layout.resize(focused, direction, 0.05) {
            return;
        }
        self.resize_tab(index);
    }

    /// Fits every pane of one tab to its part of the panel.
    fn resize_tab(&mut self, index: usize) {
        let area = PaneRect {
            x: 0.0,
            y: 0.0,
            width: self.panel_size.0,
            height: self.panel_size.1,
        };
        let (cell, scale_x, scale_y) = (self.cell, self.panel_scale.0, self.panel_scale.1);
        let Some(tab) = self.tabs.get_mut(index) else {
            return;
        };
        for (id, fraction) in tab.layout.rects() {
            let Some(pane) = tab.pane_mut(id) else {
                continue;
            };
            let cells = pane_grid(fraction_rect(area, fraction), cell, scale_x, scale_y);
            if cells == pane.grid {
                continue;
            }
            pane.grid = cells;
            pane.control.resize(cells.0, cells.1);
            let target = target_size(cell, cells);
            let viewport = InclusiveRect::new(0, 0, cells.0.max(1) - 1, cells.1.max(1) - 1);
            let _ = pane.renderer.resize(target, viewport);
        }
    }

    /// A pane's rectangle on the active tab.
    fn pane_rect(&self, id: PaneId) -> Option<PaneRect> {
        let tab = self.tabs.active()?;
        let area = PaneRect {
            x: 0.0,
            y: 0.0,
            width: self.panel_size.0,
            height: self.panel_size.1,
        };
        tab.layout
            .rects()
            .into_iter()
            .find(|(pane, _)| *pane == id)
            .map(|(_, fraction)| fraction_rect(area, fraction))
    }

    /// The active tab's pane under `point`, with its rectangle.
    fn pane_at(&self, point: (f64, f64)) -> Option<(PaneId, PaneRect)> {
        let tab = self.tabs.active()?;
        let area = PaneRect {
            x: 0.0,
            y: 0.0,
            width: self.panel_size.0,
            height: self.panel_size.1,
        };
        tab.layout.rects().into_iter().find_map(|(id, fraction)| {
            let rect = fraction_rect(area, fraction);
            rect.contains(point).then_some((id, rect))
        })
    }

    /// The cell a point in `rect` names, inside `pane`'s grid.
    fn cell_in(&self, pane: &Pane, rect: PaneRect, point: (f64, f64)) -> Option<(i32, i32)> {
        let (width, height) = self.dip_cell();
        let column = ((point.0 - rect.x) / width).floor() as i32;
        let row = ((point.1 - rect.y) / height).floor() as i32;
        let (columns, rows) = pane.grid;
        (column >= 0 && row >= 0 && column < columns && row < rows).then_some((column, row))
    }

    /// The cell a point names, or the nearest in-range cell of `pane`.
    fn clamped_cell_in(&self, pane: &Pane, rect: PaneRect, point: (f64, f64)) -> Option<(i32, i32)> {
        let (width, height) = self.dip_cell();
        let column = ((point.0 - rect.x) / width).floor() as i32;
        let row = ((point.1 - rect.y) / height).floor() as i32;
        let (columns, rows) = pane.grid;
        Some((column.clamp(0, columns.max(1) - 1), row.clamp(0, rows.max(1) - 1)))
    }

    /// The cell size in the panel's DIPs.
    fn dip_cell(&self) -> (f64, f64) {
        (
            f64::from(self.cell.0) / f64::from(finite_scale(self.panel_scale.0)),
            f64::from(self.cell.1) / f64::from(finite_scale(self.panel_scale.1)),
        )
    }

    /// The active tab's split tree, for the chrome's dividers.
    fn splits(&self) -> Vec<(SplitPath, Axis, Fraction, f32)> {
        self.tabs.active().map_or_else(Vec::new, |tab| tab.layout.splits())
    }

    /// Activates the tab `id`.
    ///
    /// The id, not the index: a press on a tab's close button bubbles to the
    /// tab after the close has run, and an index would then name whatever slid
    /// into that place.
    fn select(&mut self, id: Option<u64>) {
        let Some(index) = id.and_then(|id| tab_index(&self.tabs, id)) else {
            return;
        };
        if self.tabs.select(index) {
            let _ = self.focus.request_focus();
        }
    }

    /// Closes the tab `id`, or the window when it was the last one.
    fn close_tab(&mut self, id: u64, context: &ComponentContext<Self>) {
        let Some(index) = tab_index(&self.tabs, id) else {
            return;
        };

        if !self.tabs.close(index) {
            let _ = context.close_window();
        } else {
            let _ = self.focus.request_focus();
            self.report_extension(extensions::Event::TabClosed { index });
        }
    }

    /// Runs a bound chord, or turns the key into the active terminal's encoding.
    fn key(&mut self, info: KeyEventInfo, pressed: bool, context: &ComponentContext<Self>) {
        let Some(key) = key_of(info.key) else {
            return;
        };
        let modifiers = modifiers(info.modifiers);

        // A bound chord is the window's: the shell consumes it before the tab
        // sees the event (ADR 023). Copy is the exception: with no selection
        // it declines the key so ctrl+c still interrupts a program (ADR 053).
        if pressed
            && let Some(action) = input::chord_action(&self.bindings, key, modifiers)
            && self.perform(action, context)
        {
            return;
        }

        if pressed && let Some(command) = input::extension_chord(&self.settings.extensions.commands, key, modifiers) {
            let command = command.to_string();
            self.run_extension(&command);
            return;
        }

        // The find field owns the keyboard while its bar is open, and the
        // drop-down owns it while it shows; only a bound chord, handled above,
        // leaves either.
        if self.menu_open
            || self.commands.open
            || self
                .tabs
                .active()
                .and_then(|tab| tab.focused_pane())
                .is_some_and(|pane| pane.find.open)
        {
            return;
        }

        // Space is handled by the pane (so it keeps focus), which means no
        // character event follows. Its character is layout-independent, so the
        // key carries it instead (ADR 056).
        if pressed && key == Key::Space {
            self.flush_pending();
            let printable = !modifiers.ctrl && !modifiers.alt && !modifiers.command;
            self.send_input(&input::Event::Key {
                key,
                pressed: true,
                repeat: info.status.was_down,
                modifiers,
                virtual_key: virtual_key(info.key),
                scan_code: scan_code(info.status.scan_code),
                repeat_count: repeat_count(info.status.repeat_count),
                codepoint: printable.then_some(' '),
            });
            self.restore_focus_until = Some(Instant::now() + Duration::from_millis(500));
            return;
        }

        // A printable key's character arrives in its own event. Hold the press
        // so the Win32 record can carry it; otherwise the console would
        // synthesise a press and a release together (ADR 056).
        if pressed && input::produces_text(key, modifiers) {
            self.flush_pending();
            self.pending = Some(PendingKey {
                key,
                repeat: info.status.was_down,
                modifiers,
                virtual_key: virtual_key(info.key),
                scan_code: scan_code(info.status.scan_code),
                repeat_count: repeat_count(info.status.repeat_count),
            });
            return;
        }

        self.flush_pending();
        self.send_input(&input::Event::Key {
            key,
            pressed,
            repeat: info.status.was_down && pressed,
            modifiers,
            virtual_key: virtual_key(info.key),
            scan_code: scan_code(info.status.scan_code),
            repeat_count: repeat_count(info.status.repeat_count),
            // Enter, Tab, Backspace and Escape never produce a character event,
            // so the character they type is spelled out here: a Win32 input
            // record with a zero there is a key that typed nothing (ADR 072).
            codepoint: input::key_character(key, modifiers),
        });

        // The keyboard may be handed to the window on the space release too.
        if !pressed && key == Key::Space {
            self.restore_focus_until = Some(Instant::now() + Duration::from_millis(500));
        }
    }

    /// Hands composed text to the active terminal's text path.
    fn character(&mut self, info: CharacterEventInfo) {
        if self.menu_open
            || self.commands.open
            || self
                .tabs
                .active()
                .and_then(|tab| tab.focused_pane())
                .is_some_and(|pane| pane.find.open)
        {
            return;
        }

        let Some(character) = char::from_u32(u32::from(info.character)) else {
            return;
        };
        if character.is_control() {
            return;
        }

        if let Some(pending) = self.pending.take() {
            self.send_input(&input::Event::Key {
                key: pending.key,
                pressed: true,
                repeat: pending.repeat,
                modifiers: pending.modifiers,
                virtual_key: pending.virtual_key,
                scan_code: pending.scan_code,
                repeat_count: pending.repeat_count,
                codepoint: Some(character),
            });
        } else {
            self.send_input(&input::Event::Text(character.to_string()));
        }
    }

    /// Sends a held press without a character, so a dead key or a key whose
    /// character never arrived is not lost.
    fn flush_pending(&mut self) {
        let Some(pending) = self.pending.take() else {
            return;
        };
        self.send_input(&input::Event::Key {
            key: pending.key,
            pressed: true,
            repeat: pending.repeat,
            modifiers: pending.modifiers,
            virtual_key: pending.virtual_key,
            scan_code: pending.scan_code,
            repeat_count: pending.repeat_count,
            codepoint: None,
        });
    }

    /// Translates an event with the active terminal's modes and sends it.
    fn send_input(&mut self, event: &input::Event) {
        // A printable key travels as the character it typed, unless the program
        // asked for every key: then the key and its release travel instead.
        let modes = self
            .tabs
            .active()
            .and_then(|tab| tab.focused_pane())
            .map(|pane| pane.control.input_modes());
        let win32 = modes.is_some_and(|modes| modes.win32_input);
        let keys = modes.is_some_and(|modes| modes.reports_all_keys());
        let outcome = input::translate(event, &mut self.sides, win32, keys);
        let Some(pane) = self.tabs.active_mut().and_then(|tab| tab.focused_pane_mut()) else {
            return;
        };
        // Typing or pressing a key returns the view to the live screen.
        if outcome.activity {
            pane.view.scroll_to_bottom();
        }
        for key in outcome.keys {
            pane.control.send_key(&key);
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
            settings::Action::CloseTab => self.close_pane(context),
            settings::Action::NextTab => {
                self.tabs.step(1);
                let _ = self.focus.request_focus();
            }
            settings::Action::PrevTab => {
                self.tabs.step(-1);
                let _ = self.focus.request_focus();
            }
            settings::Action::SplitHorizontal => self.split_pane(Axis::Horizontal),
            settings::Action::SplitVertical => self.split_pane(Axis::Vertical),
            settings::Action::FocusLeft => self.focus_pane(Direction::Left),
            settings::Action::FocusRight => self.focus_pane(Direction::Right),
            settings::Action::FocusUp => self.focus_pane(Direction::Up),
            settings::Action::FocusDown => self.focus_pane(Direction::Down),
            settings::Action::ResizeLeft => self.resize_pane(Direction::Left),
            settings::Action::ResizeRight => self.resize_pane(Direction::Right),
            settings::Action::ResizeUp => self.resize_pane(Direction::Up),
            settings::Action::ResizeDown => self.resize_pane(Direction::Down),
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
            settings::Action::CommandPalette => self.toggle_palette(),
            _ => {}
        }
        true
    }

    /// Puts the focused pane's selection on the clipboard.
    fn copy_selection(&mut self) -> bool {
        let Some(pane) = self.tabs.active_mut().and_then(|tab| tab.focused_pane_mut()) else {
            return false;
        };
        let Some(selection) = pane.view.selection().copied() else {
            return false;
        };
        let Some(text) = pane.control.selection_text(&selection) else {
            return false;
        };

        let copied = arboard::Clipboard::new()
            .and_then(|mut clipboard| clipboard.set_text(text))
            .is_ok();
        if copied {
            // A copy ends the selection, as it does upstream (ADR 053).
            pane.view.clear_selection();
        }
        copied
    }

    /// Sends the clipboard through the terminal's bracketed-paste filter.
    fn paste_clipboard(&mut self) {
        let Ok(mut clipboard) = arboard::Clipboard::new() else {
            return;
        };
        let Ok(text) = clipboard.get_text() else {
            return;
        };
        if let Some(pane) = self.tabs.active_mut().and_then(|tab| tab.focused_pane_mut()) {
            pane.control.paste(&text);
        }
    }

    /// Scrolls the focused viewport by `rows`; positive is towards the history.
    fn scroll_active(&mut self, rows: i32) {
        let Some(pane) = self.tabs.active_mut().and_then(|tab| tab.focused_pane_mut()) else {
            return;
        };
        let max = pane.control.max_scroll();
        pane.view.scroll_by(rows, max);
    }

    /// Sends a wheel to a tracking program, or scrolls the viewport.
    ///
    /// Shift keeps the wheel for the viewport, so a person can always scroll
    /// back out of a full-screen program (ADR 019).
    fn wheel(&mut self, notches: i32) {
        let target = self.pointer_pane.or_else(|| self.tabs.active().map(|tab| tab.focused));
        let Some(pane_id) = target else {
            return;
        };
        let Some(tab) = self.tabs.active_mut() else {
            return;
        };
        let Some(pane) = tab.pane_mut(pane_id) else {
            return;
        };

        let shift = wheel::shift_held();
        let forward = input::forwards_to_application(pane.control.is_tracking_mouse(), shift);
        if forward && let Some(position) = self.pointer_cell {
            let event = MouseEvent::wheel(position, WheelAxis::Vertical, notches * 120);
            if pane.control.send_mouse(&event) {
                return;
            }
        }

        let max = pane.control.max_scroll();
        pane.view.scroll_by(notches, max);
    }

    /// Scrolls the focused viewport by `direction` pages.
    fn scroll_page_active(&mut self, direction: i32) {
        let Some(pane) = self.tabs.active_mut().and_then(|tab| tab.focused_pane_mut()) else {
            return;
        };
        let rows = (pane.control.size().1 - 1).max(1) * direction.signum();
        let max = pane.control.max_scroll();
        pane.view.scroll_by(rows, max);
    }

    /// Jumps the focused viewport to the live screen (`true`) or the oldest row.
    fn scroll_to_active(&mut self, bottom: bool) {
        let Some(pane) = self.tabs.active_mut().and_then(|tab| tab.focused_pane_mut()) else {
            return;
        };
        if bottom {
            pane.view.scroll_to_bottom();
        } else {
            let max = pane.control.max_scroll();
            pane.view.set_scroll(max, max);
        }
    }

    /// Takes a press that landed in a pane's scrollbar strip.
    fn scrollbar_pressed(&mut self, pane_id: PaneId, y: f64) -> bool {
        let Some(rect) = self.pane_rect(pane_id) else {
            return false;
        };
        let track = rect.height.max(0.0);
        if track <= 0.0 {
            return false;
        }

        let Some(tab) = self.tabs.active_mut() else {
            return false;
        };
        let Some(pane) = tab.pane_mut(pane_id) else {
            return false;
        };
        let (document, marks) = pane.control.scroll_facts();
        let visible = pane.control.size().1;
        if document <= visible {
            self.scrollbar.release();
            return false;
        }

        let fraction = ((y - rect.y) / track).clamp(0.0, 1.0) as f32;
        let positions: Vec<f32> = marks
            .iter()
            .map(|mark| scrollbar::position(mark.row, document))
            .collect();

        if let Some(index) = scrollbar::tick_at(&positions, fraction, scrollbar::MARK_GRAB / track as f32) {
            // A prompt tick is a precise target: it brings the prompt into view
            // rather than centring the clicked fraction.
            pane.view.scroll_to_row(marks[index].row, visible, document);
        } else if let scrollbar::Press::Jump(scroll) =
            self.scrollbar.press(fraction, pane.view.scroll(), document, visible)
        {
            pane.view.set_scroll(scroll, document - visible);
        }

        self.scrollbar_press = true;
        self.pointer_pane = Some(pane_id);
        true
    }

    /// Continues a held scrollbar drag.
    fn scrollbar_dragged(&mut self, pane_id: PaneId, y: f64) {
        let Some(rect) = self.pane_rect(pane_id) else {
            return;
        };
        let track = rect.height.max(0.0);
        if track <= 0.0 {
            return;
        }
        let Some(tab) = self.tabs.active_mut() else {
            return;
        };
        let Some(pane) = tab.pane_mut(pane_id) else {
            return;
        };
        let visible = pane.control.size().1;
        let document = pane.control.max_scroll() + visible;
        let fraction = ((y - rect.y) / track).clamp(0.0, 1.0) as f32;
        if let Some(scroll) = self.scrollbar.drag(fraction, document, visible) {
            pane.view.set_scroll(scroll, document - visible);
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
        // A press outside the palette's card dismisses it; a press on a row has
        // already sent its command by the time this bubbles here.
        if self.commands.open {
            self.close_palette();
            return;
        }

        let point = (info.x - PANE_PADDING_LEFT, info.y);

        // A divider is grabbed before the panes it separates.
        if info.is_left_button_pressed {
            let area = PaneRect {
                x: 0.0,
                y: 0.0,
                width: self.panel_size.0,
                height: self.panel_size.1,
            };
            if let Some((path, axis, split)) = divider_at(area, &self.splits(), point, 3.0) {
                self.divider = Some((path, axis, split));
                return;
            }
        }

        let Some((pane_id, rect)) = self.pane_at(point) else {
            return;
        };
        self.pointer_pane = Some(pane_id);
        if let Some(tab) = self.tabs.active_mut() {
            tab.focused = pane_id;
        }

        if info.is_left_button_pressed
            && bar::contains(rect.width, point.0 - rect.x)
            && self.scrollbar_pressed(pane_id, info.y)
        {
            return;
        }

        let shift = info.modifiers.contains(InputModifiers::SHIFT);
        let (tracking, cell) = {
            let Some(tab) = self.tabs.active() else {
                return;
            };
            let Some(pane) = tab.pane(pane_id) else {
                return;
            };
            (pane.control.is_tracking_mouse(), self.cell_in(pane, rect, point))
        };
        let forward = input::forwards_to_application(tracking, shift);
        self.pointer_cell = cell;

        // A right press pastes, unless the program asked to see the mouse.
        if info.is_right_button_pressed && !forward {
            self.paste_clipboard();
            return;
        }

        let Some(tab) = self.tabs.active_mut() else {
            return;
        };
        let Some(pane) = tab.pane_mut(pane_id) else {
            return;
        };

        if forward {
            if let Some(position) = cell {
                let buttons = input::mouse_buttons(
                    info.is_left_button_pressed,
                    info.is_middle_button_pressed,
                    info.is_right_button_pressed,
                );
                let button = if info.is_right_button_pressed {
                    MouseButton::Right
                } else if info.is_middle_button_pressed {
                    MouseButton::Middle
                } else {
                    MouseButton::Left
                };
                let chord =
                    input::key_modifiers(modifiers(info.modifiers), self.sides.right_alt, self.sides.right_ctrl);
                let event = MouseEvent {
                    position,
                    kind: MouseEventKind::Press(button),
                    buttons,
                    modifiers: chord,
                    delta: 0,
                };
                pane.control.send_mouse(&event);
                self.last_mouse_cell = Some(position);
            }
            return;
        }

        let Some((column, row)) = cell else {
            return;
        };

        // Selection is in document coordinates: history first, then the screen.
        let history = pane.control.max_scroll();
        let position = (column, input::screen_to_document(history, pane.view.scroll(), row));

        let count = pane.view.click(position, Instant::now());
        if count >= 3 {
            let width = pane.control.screen_width();
            pane.view.select_line(position.1, width);
            self.press = None;
            self.press_pane = None;
        } else if count == 2 {
            let text = pane.control.row_text(position.1);
            pane.view.select_word(&text, position);
            self.press = None;
            self.press_pane = None;
        } else {
            // A single click selects nothing: it clears what was selected and
            // leaves the anchor a drag starts from, as upstream's
            // `LeftClickOnTerminal` does. A one-cell selection would otherwise
            // make ctrl+c copy a blank instead of interrupting (ADR 053).
            pane.view.clear_selection();
            self.press = Some((position, modifiers(info.modifiers).alt));
            self.press_pane = Some(pane_id);
        }
    }

    /// Extends a selection, scrolls a drag past the pane, or forwards the move.
    fn pointer_moved(&mut self, info: PointerEventInfo) {
        if let Some((path, axis, split)) = self.divider.clone() {
            if !info.is_left_button_pressed {
                self.divider = None;
                return;
            }
            let point = (info.x - PANE_PADDING_LEFT, info.y);
            let (origin, extent) = match axis {
                Axis::Horizontal => (point.0 - split.x, split.width),
                Axis::Vertical => (point.1 - split.y, split.height),
            };
            if extent > 0.0 {
                let ratio = (origin / extent).clamp(0.0, 1.0) as f32;
                let index = self.tabs.index();
                if let Some(tab) = self.tabs.active_mut() {
                    tab.layout.set_ratio(&path, ratio);
                }
                self.resize_tab(index);
            }
            return;
        }

        if self.scrollbar_press {
            let pane = self.pointer_pane;
            if info.is_left_button_pressed {
                if let Some(pane) = pane {
                    self.scrollbar_dragged(pane, info.y);
                }
            } else {
                // A release outside the pane may never be reported; the button
                // state on the move is the fallback.
                self.scrollbar_press = false;
                self.scrollbar.release();
            }
            return;
        }

        let point = (info.x - PANE_PADDING_LEFT, info.y);
        let target = self.press_pane.or_else(|| self.pane_at(point).map(|(id, _)| id));
        let Some(pane_id) = target else {
            self.pointer_pane = None;
            self.last_mouse_cell = None;
            return;
        };
        let Some(rect) = self.pane_rect(pane_id) else {
            return;
        };
        let (cell, tracking, columns, rows) = {
            let Some(tab) = self.tabs.active() else {
                return;
            };
            let Some(pane) = tab.pane(pane_id) else {
                return;
            };
            (
                self.cell_in(pane, rect, point),
                pane.control.is_tracking_mouse(),
                pane.grid.0,
                pane.grid.1,
            )
        };
        self.pointer_cell = cell;
        self.pointer_pane = Some(pane_id);
        let shift = info.modifiers.contains(InputModifiers::SHIFT);
        let forward = input::forwards_to_application(tracking, shift);
        let (cell_width, cell_height) = self.dip_cell();

        let Some(tab) = self.tabs.active_mut() else {
            return;
        };
        let Some(pane) = tab.pane_mut(pane_id) else {
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
                    pane.control.send_mouse(&event);
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

        let outside = if point.1 < rect.y {
            -1
        } else if cell_height > 0.0 && point.1 >= rect.y + f64::from(rows.max(0)) * cell_height {
            1
        } else {
            0
        };
        let column = if cell_width > 0.0 {
            ((point.0 - rect.x) / cell_width).floor() as i32
        } else {
            0
        };

        let position = if outside != 0 {
            // A drag that leaves the pane claims the edge row and scrolls one
            // row towards the pointer, so the selection reaches into history.
            let max = pane.control.max_scroll();
            pane.view.drag_scroll(outside, max, Instant::now());
            let row = if outside < 0 { 0 } else { rows.max(1) - 1 };
            let column = column.clamp(0, columns.max(1) - 1);
            Some((column, input::screen_to_document(max, pane.view.scroll(), row)))
        } else {
            cell.map(|(column, row)| {
                let history = pane.control.max_scroll();
                (column, input::screen_to_document(history, pane.view.scroll(), row))
            })
        };

        let Some(position) = position else {
            return;
        };
        match self.press.take() {
            // The first move away from the press cell starts the selection.
            Some((anchor, block)) if position != anchor => {
                pane.view.select(anchor, block);
                pane.view.extend(position);
            }
            Some(press) => self.press = Some(press),
            None => pane.view.extend(position),
        }
    }

    /// Finishes a forwarded press.
    fn pointer_released(&mut self, info: PointerEventInfo) {
        if self.divider.take().is_some() {
            self.press = None;
            self.press_pane = None;
            return;
        }

        if self.scrollbar_press {
            self.scrollbar_press = false;
            self.scrollbar.release();
            self.press = None;
            self.press_pane = None;
            self.last_mouse_cell = None;
            return;
        }

        let point = (info.x - PANE_PADDING_LEFT, info.y);
        let target = self.press_pane.take().or_else(|| self.pane_at(point).map(|(id, _)| id));
        let Some(pane_id) = target else {
            self.last_mouse_cell = None;
            self.press = None;
            return;
        };
        let Some(rect) = self.pane_rect(pane_id) else {
            self.last_mouse_cell = None;
            self.press = None;
            return;
        };
        let shift = info.modifiers.contains(InputModifiers::SHIFT);
        let release = {
            let Some(tab) = self.tabs.active() else {
                return;
            };
            let Some(pane) = tab.pane(pane_id) else {
                return;
            };
            input::forwards_to_application(pane.control.is_tracking_mouse(), shift)
                .then(|| self.clamped_cell_in(pane, rect, point))
                .flatten()
        };

        if let Some(position) = release {
            let buttons = input::mouse_buttons(
                info.is_left_button_pressed,
                info.is_middle_button_pressed,
                info.is_right_button_pressed,
            );
            let Some(tab) = self.tabs.active_mut() else {
                return;
            };
            let Some(pane) = tab.pane_mut(pane_id) else {
                return;
            };
            let chord = input::key_modifiers(modifiers(info.modifiers), self.sides.right_alt, self.sides.right_ctrl);
            let event = MouseEvent {
                position,
                kind: MouseEventKind::Release(MouseButton::Left),
                buttons,
                modifiers: chord,
                delta: 0,
            };
            pane.control.send_mouse(&event);
        }
        self.last_mouse_cell = None;
    }
}

/// The actions this shell performs, for the palette's row list.
///
/// `CommandPalette` is left out because `palette::entries` never offers the
/// palette itself.
fn palette_supports(action: settings::Action) -> bool {
    use settings::Action;

    matches!(
        action,
        Action::NewTab
            | Action::CloseTab
            | Action::NextTab
            | Action::PrevTab
            | Action::SplitHorizontal
            | Action::SplitVertical
            | Action::FocusLeft
            | Action::FocusRight
            | Action::FocusUp
            | Action::FocusDown
            | Action::ResizeLeft
            | Action::ResizeRight
            | Action::ResizeUp
            | Action::ResizeDown
            | Action::Copy
            | Action::Paste
            | Action::ScrollUp
            | Action::ScrollDown
            | Action::ScrollUpPage
            | Action::ScrollDownPage
            | Action::ScrollToTop
            | Action::ScrollToBottom
            | Action::Find
            | Action::OpenSettings
    )
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

/// The key a tab's element is declared with.
fn tab_key(id: u64) -> String {
    format!("tab-{id}")
}

/// The index of the open tab `id` names, if it is still open.
fn tab_index(tabs: &Tabs<Tab>, id: u64) -> Option<usize> {
    (0..tabs.len()).find(|&index| tabs.get(index).is_some_and(|tab| tab.id == id))
}

/// Black or white, whichever reads on `fill`.
///
/// A fill names no foreground, so a tab's label has to pick a side; a light
/// scheme would otherwise paint a white label on white. The weights and the
/// cut-off are shared with the GTK and macOS strips (ADR 035).
fn contrasting_text(fill: Color) -> Color {
    let luminance = 0.299 * f32::from(fill.r) + 0.587 * f32::from(fill.g) + 0.114 * f32::from(fill.b);
    if luminance > 140.0 {
        Color::rgb(20, 20, 20)
    } else {
        Color::rgb(230, 230, 230)
    }
}

/// A colour from the settings file as the toolkit's colour, when it names one.
fn theme_color(color: settings::Color) -> Option<Color> {
    match color.text_color().get_type() {
        ColorType::Rgb { r, g, b } => Some(Color::rgb(r, g, b)),
        _ => None,
    }
}

/// A `COLORREF` (`0x00BBGGRR`, which is what the terminal's palette uses) as
/// the toolkit's colour.
fn color_of(colorref: u32) -> Color {
    Color::rgb(
        (colorref & 0xFF) as u8,
        ((colorref >> 8) & 0xFF) as u8,
        ((colorref >> 16) & 0xFF) as u8,
    )
}

/// Mixes two opaque colours, fraction `t` of the way from `a` to `b`.
fn blend(a: Color, b: Color, t: f32) -> Color {
    let channel = |from: u8, to: u8| {
        let from = f32::from(from);
        let to = f32::from(to);
        (from + (to - from) * t).round().clamp(0.0, 255.0) as u8
    };
    Color::rgb(channel(a.r, b.r), channel(a.g, b.g), channel(a.b, b.b))
}

/// The settings vocabulary key for a terminal key, if it has one.
/// The grid cell a pane-relative position names, in DIPs.
/// The cell a pointer position names, kept for the pointer-mapping tests.
#[cfg(test)]
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

/// Starts a terminal and its renderer for one pane.
fn spawn_pane(
    id: PaneId,
    terminal: &TerminalSettings,
    cells: (i32, i32),
    font_size: f32,
    scale: f32,
    face: &Path,
    surface: &Rc<RefCell<Option<Surface>>>,
) -> Option<Pane> {
    let session = spawn_shell(&terminal.command, pty_size(cells)).ok()?;
    let mut control = Control::new(Box::new(session), cells);
    control.apply_scheme(terminal.scheme.as_ref());

    let fonts = font_setup(face, font_size, scale, cells);
    let offset = Rc::new(Cell::new((0, 0)));
    let (renderer, shared) = renderer_for(fonts, cells, Rc::clone(surface), face, Rc::clone(&offset));
    Some(Pane {
        id,
        control,
        view: ViewState::new(),
        title: None,
        program: terminal.command.program().to_string_lossy().into_owned(),
        find: FindBar::default(),
        renderer,
        shared,
        offset,
        grid: cells,
    })
}

/// Builds a renderer that paints a `grid`, on the GPU unless the CPU path is
/// forced.
fn renderer_for(
    fonts: Fonts,
    grid: (i32, i32),
    surface: Rc<RefCell<Option<Surface>>>,
    face: &Path,
    offset: Rc<Cell<(i32, i32)>>,
) -> (Renderer<TerminalData>, Option<SharedBackend>) {
    let Fonts {
        font,
        settings,
        shaper,
        rasterizer,
    } = fonts;

    let (backend, shared): (Box<dyn IBackend>, Option<SharedBackend>) = if software_renderer() {
        let shared = SharedBackend::new(SoftwareBackend::with_rasterizer(Box::new(rasterizer)));
        (Box::new(shared.clone()), Some(shared))
    } else {
        (Box::new(GpuBackend::new(surface, offset, face)), None)
    };

    let mut engine = AtlasEngine::with_settings(settings, backend);
    engine.set_shaping(Box::new(shaper));

    let mut renderer = Renderer::new(
        TerminalData::new(font, RenderSettings::new(), grid.0, grid.1),
        RenderSettings::new(),
    );
    renderer.add_engine(Box::new(engine));
    (renderer, shared)
}

/// Whether the CPU path is forced for this process.
///
/// `WINTER_SOFTWARE_RENDERER=1` is the escape hatch when a machine's Direct2D
/// behaves differently from the one the GPU backend was checked on.
fn software_renderer() -> bool {
    match std::env::var("WINTER_SOFTWARE_RENDERER") {
        Ok(value) => !value.is_empty() && value != "0",
        Err(_) => false,
    }
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

/// The pane's width in DIPs, from the swap chain's report.
///
/// The panel sits inside the pane's left padding, so its width is that much
/// narrower than the pane it is centred in.
fn pane_width(panel: f64) -> f64 {
    panel + PANE_PADDING_LEFT
}

/// The width the grid's cells may use: the panel less the scrollbar's strip, so
/// the last column stops short of the scrollbar instead of running under it
/// (ADR 068).
fn grid_width(panel: f64) -> f64 {
    (panel - bar::WIDTH).max(0.0)
}

/// The width the tabs may share, clear of the new-tab group and the window's
/// own buttons.
fn tab_room(pane: f64) -> f64 {
    (pane - CAPTION_WIDTH - CAPTION_GAP - NEW_TAB_WIDTH - DROP_DOWN_WIDTH - 1.0 - TAB_ROW_GAP).max(0.0)
}

/// The width a tab's title may use once its icon and close button are placed.
///
/// A tab's grid has three columns with a gap between each pair, whether or not
/// there is an icon, so the gaps are always counted twice.
fn title_room(tab: f64, has_icon: bool) -> f64 {
    let content = tab - TAB_PADDING_LEFT - TAB_PADDING_RIGHT;
    let fixed = CLOSE_BUTTON + 2.0 * TAB_GAP + if has_icon { TAB_ICON } else { 0.0 };
    (content - fixed).max(0.0)
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

/// The profiles the window offers: the file's, then the WSL distributions.
fn offered_profiles(settings: &settings::Settings) -> Vec<settings::Profile> {
    merged(settings.profiles.clone(), wsl::profiles())
}

/// `file` with every distro no profile already names appended to it.
///
/// A distribution that shares a name with a profile of the user's is dropped,
/// not renamed: the file is what the user wrote, and two menu rows with one name
/// would resolve to whichever came first.
fn merged(mut file: Vec<settings::Profile>, distros: Vec<settings::Profile>) -> Vec<settings::Profile> {
    for distro in distros {
        if !file.iter().any(|profile| profile.name == distro.name) {
            file.push(distro);
        }
    }
    file
}

/// The find bar: a query field, previous and next, and the match count.
fn find_bar(pane: &Pane, sender: &LocalSender<Message>, field: &ElementRef<TextBox>) -> View {
    let count = pane.find.matches.len();
    let shown = pane.find.current.map_or(0, |index| index + 1);
    let status = if count == 0 && !pane.find.query.is_empty() {
        "No matches".to_string()
    } else {
        format!("{shown} of {count}")
    };

    let query = {
        let sender = sender.clone();
        TextBox::new()
            .text(&pane.find.query)
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
fn profile_menu(profiles: &[settings::Profile], sender: &LocalSender<Message>, left: f64) -> View {
    let mut rows: Vec<View> = Vec::new();
    let mut separated = false;
    for entry in menu::entries(profiles) {
        // The window's commands are a group of their own, as upstream's flyout
        // separates them: one hairline between the last profile and `Settings`
        // (ADR 070).
        if !separated && !matches!(entry, MenuEntry::Profile(_)) {
            separated = true;
            if !rows.is_empty() {
                rows.push(menu_separator());
            }
        }

        // A profile's row carries the icon of the program it starts, and the two
        // commands carry their own glyph, so the menu reads like a list of
        // shells and actions rather than a list of names (ADRs 066, 070).
        let leading = match &entry {
            MenuEntry::Profile(name) => profiles
                .iter()
                .find(|profile| &profile.name == name)
                .and_then(|profile| profile_icon::png_for(&profile.command))
                .map(menu_image),
            MenuEntry::Settings => Some(menu_glyph(SETTINGS_GLYPH)),
            MenuEntry::Extensions => Some(menu_glyph(EXTENSIONS_GLYPH)),
            MenuEntry::About => Some(menu_glyph(ABOUT_GLYPH)),
        };
        rows.push(menu_row(entry, sender, leading));
    }

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

/// The hairline between the profile rows and the window's commands.
fn menu_separator() -> View {
    Border::new()
        .height(1.0)
        .margin(Thickness::new(6.0, 4.0, 6.0, 4.0))
        .background(Color::rgb(72, 72, 78))
        .into()
}

/// A row's program icon.
fn menu_image(icon: Arc<[u8]>) -> View {
    Image::new()
        .source_data(EncodedImage::new(icon))
        .width(TAB_ICON)
        .height(TAB_ICON)
        .stretch(Stretch::Uniform)
        .vertical_alignment(VerticalAlignment::Center)
        .into()
}

/// A row's symbol glyph, in the same square an icon takes so the labels line up.
fn menu_glyph(glyph: &str) -> View {
    TextBlock::new()
        .text(glyph)
        .font_family(SYMBOL_FONT)
        .font_size(14.0)
        .width(TAB_ICON)
        .text_alignment(TextAlignment::Center)
        .vertical_alignment(VerticalAlignment::Center)
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
fn menu_row(entry: MenuEntry, sender: &LocalSender<Message>, leading: Option<View>) -> View {
    let button = match entry {
        MenuEntry::Profile(name) => {
            let sender = sender.clone();
            let leading = leading.clone();
            menu_button(name.clone(), leading).on_click(move || {
                let _ = sender.send(Message::MenuProfile(name.clone()));
            })
        }
        MenuEntry::Settings => {
            let sender = sender.clone();
            menu_button("Settings".to_string(), leading).on_click(move || {
                let _ = sender.send(Message::MenuSettings);
            })
        }
        MenuEntry::Extensions => {
            let sender = sender.clone();
            menu_button("Extensions".to_string(), leading).on_click(move || {
                let _ = sender.send(Message::MenuExtensions);
            })
        }
        MenuEntry::About => {
            let sender = sender.clone();
            menu_button("About Winter".to_string(), leading).on_click(move || {
                let _ = sender.send(Message::MenuAbout);
            })
        }
    };
    button.into()
}

/// A flat, left-aligned row sized to the menu, with its icon or glyph.
fn menu_button(label: String, leading: Option<View>) -> Button {
    let content: View = match leading {
        Some(leading) => StackPanel::new()
            .orientation(Orientation::Horizontal)
            .spacing(8.0)
            .children((
                leading,
                TextBlock::new()
                    .text(label)
                    .vertical_alignment(VerticalAlignment::Center),
            ))
            .into(),
        None => TextBlock::new().text(label).into(),
    };

    Button::new()
        .content(content)
        .style(ButtonStyle::Subtle)
        .horizontal_content_alignment(HorizontalAlignment::Left)
        .width(232.0)
}

/// The extensions dialog: one switch per extension and one that loads the
/// feature at all.
///
/// The switches are the dialog's whole state; Save writes the settings file's
/// `[extensions]` table and nothing else, and says that loading changes on the
/// next start: a live engine is not reloaded.
fn extensions_dialog(rows: &[Found], enabled: bool, checked: &[bool], sender: &LocalSender<Message>) -> ContentDialog {
    let mut children: Vec<View> = vec![
        TextBlock::new()
            .text(
                "Anything under an `extensions` folder is listed here. A switch takes effect the next time Winter starts.",
            )
            .text_wrapping(TextWrapping::Wrap)
            .into(),
    ];

    let master_sender = sender.clone();
    children.push(
        ToggleSwitch::new()
            .header("Load extensions")
            .is_on(enabled)
            .on_toggled(move |on: bool| {
                let _ = master_sender.send(Message::ExtensionsAll(on));
            })
            .into(),
    );

    if rows.is_empty() {
        children.push(TextBlock::new().text("No extensions were found.").into());
    }
    for (index, row) in rows.iter().enumerate() {
        let on = checked.get(index).copied().unwrap_or(row.enabled);
        let row_sender = sender.clone();
        let toggle = ToggleSwitch::new()
            .header(format!("{}  ({})", row.name, row.id))
            .is_on(on)
            .is_enabled(row.switchable)
            .on_toggled(move |on: bool| {
                let _ = row_sender.send(Message::ExtensionsToggle(index, on));
            });

        // A row that could not be read says why in red, as the macOS window
        // does; a working one shows what it ships.
        let detail = if row.note.is_empty() {
            row.kind.to_string()
        } else {
            format!("{} · {}", row.kind, row.note)
        };
        let mut detail = TextBlock::new()
            .text(detail)
            .font_size(12.0)
            .text_wrapping(TextWrapping::Wrap);
        if !row.note.is_empty() {
            detail = detail.foreground(Color::rgb(220, 170, 60));
        }
        children.push(StackPanel::new().spacing(2.0).children((toggle, detail)).into());
    }

    let content = ScrollViewer::new()
        .horizontal_scroll_bar_visibility(ScrollBarVisibility::Disabled)
        .vertical_scroll_bar_visibility(ScrollBarVisibility::Auto)
        .max_height(420.0)
        .content(StackPanel::new().spacing(10.0).children(children));

    ContentDialog::new()
        .title("Extensions")
        .primary_button_text("Save")
        .close_button_text("Close")
        .is_open(true)
        .content(content)
        .on_closed({
            let sender = sender.clone();
            move |result: ContentDialogResult| {
                if matches!(result, ContentDialogResult::Primary) {
                    let _ = sender.send(Message::ExtensionsSave);
                } else {
                    let _ = sender.send(Message::ExtensionsClose);
                }
            }
        })
}

/// The About dialog: the version this build was compiled as.
fn about_dialog(sender: &LocalSender<Message>) -> ContentDialog {
    let mut details: Vec<View> = vec![TextBlock::new().text(about::title(env!("CARGO_PKG_VERSION"))).into()];
    for line in about::DESCRIPTION {
        details.push(TextBlock::new().text(line).into());
    }
    // The repository, where the issues are. A `HyperlinkButton` opens the
    // default browser itself, so the shell does not need `ShellExecute`.
    if let Ok(link) = HyperlinkButton::new()
        .navigate_uri(about::REPOSITORY)
        .map(|link| link.content(about::REPOSITORY))
    {
        details.push(link.into());
    }

    let details = StackPanel::new().spacing(8.0).children(details);

    ContentDialog::new()
        .title("About Winter")
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
pub(super) fn key_of(key: VirtualKey) -> Option<Key> {
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

/// The Win32 virtual-key code a record carries.
fn virtual_key(key: VirtualKey) -> u16 {
    u16::try_from(key.0).unwrap_or(0)
}

/// The Win32 scan code a record carries.
fn scan_code(code: u32) -> u16 {
    u16::try_from(code).unwrap_or(0)
}

/// The repeat count a record carries; a record's count is never zero.
fn repeat_count(count: u32) -> u16 {
    u16::try_from(count).unwrap_or(1).max(1)
}

#[cfg(test)]
mod tests {
    use super::*;
    use types::color_table::ColorAlias;

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
    fn a_pane_fraction_maps_to_its_part_of_the_area() {
        let area = PaneRect {
            x: 0.0,
            y: 0.0,
            width: 800.0,
            height: 600.0,
        };
        let right_half = fraction_rect(
            area,
            Fraction {
                x: 0.5,
                y: 0.0,
                width: 0.5,
                height: 1.0,
            },
        );

        assert_eq!(
            right_half,
            PaneRect {
                x: 400.0,
                y: 0.0,
                width: 400.0,
                height: 600.0
            }
        );
    }

    #[test]
    fn a_pane_computes_its_own_grid() {
        let pane = PaneRect {
            x: 400.0,
            y: 0.0,
            width: 400.0,
            height: 566.0,
        };

        // 400 - 10 = 390 points at scale 2 with an 8-pixel cell is 97 columns;
        // 566 points is 70 rows at 16 pixels.
        assert_eq!(pane_grid(pane, (8, 16), 2.0, 2.0), (97, 70));
    }

    #[test]
    fn a_split_gives_the_new_pane_the_second_half() {
        let rect = PaneRect {
            x: 100.0,
            y: 34.0,
            width: 400.0,
            height: 300.0,
        };

        let right = split_second(rect, Axis::Horizontal);
        assert_eq!(
            right,
            PaneRect {
                x: 300.0,
                y: 34.0,
                width: 200.0,
                height: 300.0
            }
        );

        let down = split_second(rect, Axis::Vertical);
        assert_eq!(
            down,
            PaneRect {
                x: 100.0,
                y: 184.0,
                width: 400.0,
                height: 150.0
            }
        );
    }

    #[test]
    fn a_divider_is_grabbed_within_its_width() {
        let area = PaneRect {
            x: 0.0,
            y: 0.0,
            width: 800.0,
            height: 600.0,
        };
        let mut layout = Layout::leaf(PaneId(0));
        assert!(layout.split(PaneId(0), Axis::Horizontal, PaneId(1)));
        let splits = layout.splits();

        let hit = divider_at(area, &splits, (401.0, 200.0), 3.0);
        assert_eq!(hit.map(|(_, axis, _)| axis), Some(Axis::Horizontal));
        assert!(divider_at(area, &splits, (420.0, 200.0), 3.0).is_none());
    }

    #[test]
    fn a_scheme_reaches_the_pane_and_survives_a_resize() {
        use winter::term_connection::Queue;

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

        let mut control = Control::new(Box::new(Queue::with_output(b"")), (20, 5));
        control.apply_scheme(terminal.scheme.as_ref());
        assert_eq!(control.default_colors(), (0x0020_1020, 0x00E0_E0E0));

        control.pump();
        assert_eq!(control.default_colors(), (0x0020_1020, 0x00E0_E0E0));

        control.resize(30, 10);
        assert_eq!(control.default_colors(), (0x0020_1020, 0x00E0_E0E0));
    }

    #[test]
    fn the_renderer_is_handed_the_panes_palette() {
        use winter::term_connection::Queue;

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

        let mut control = Control::new(Box::new(Queue::with_output(b"")), (20, 5));
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
    fn the_pty_size_follows_the_grid() {
        assert_eq!(pty_size((120, 30)), PtySize::new(120, 30));
        assert_eq!(pty_size((0, 0)), PtySize::new(1, 1));
    }

    #[test]
    fn the_tab_room_stops_short_of_the_buttons_and_the_caption() {
        let room = tab_room(1000.0);
        assert!(room > 0.0);
        assert_eq!(
            room,
            1000.0 - CAPTION_WIDTH - CAPTION_GAP - NEW_TAB_WIDTH - DROP_DOWN_WIDTH - 1.0 - TAB_ROW_GAP
        );

        // A window too narrow for the buttons has no room, not negative room.
        assert_eq!(tab_room(100.0), 0.0);
    }

    #[test]
    fn a_tab_at_its_floor_has_no_room_for_a_title() {
        assert_eq!(title_room(MIN_TAB_WIDTH, true), 0.0);
        assert!(title_room(MIN_TAB_WIDTH, false) > 0.0);
        assert!(title_room(MAX_TAB_WIDTH, true) < title_room(MAX_TAB_WIDTH, false));
        assert!(title_room(0.0, true) >= 0.0);
    }

    #[test]
    fn the_grid_stops_short_of_the_scrollbar() {
        assert_eq!(grid_width(800.0), 800.0 - bar::WIDTH);
        assert_eq!(grid_width(0.0), 0.0);
        assert_eq!(grid_width(-4.0), 0.0);
    }

    #[test]
    fn an_id_names_its_tab_and_nothing_once_it_closes() {
        fn tab(id: u64) -> Tab {
            Tab {
                id,
                panes: Vec::new(),
                layout: Layout::leaf(PaneId(id)),
                focused: PaneId(id),
                terminal: TerminalSettings {
                    command: PtyCommand::new("cmd.exe"),
                    title: format!("tab {id}"),
                    scheme: None,
                },
                title: format!("tab {id}"),
            }
        }

        let mut tabs = Tabs::new(tab(0));
        tabs.add(tab(1));
        tabs.add(tab(2));
        assert_eq!(tab_index(&tabs, 2), Some(2));

        // A press on a close button bubbles its tab's select after the close
        // has run. The id has to name nothing then, not whatever slid into its
        // place, which is what an index would do.
        assert!(tabs.close(2));
        assert_eq!(tab_index(&tabs, 2), None);
        assert_eq!(tab_index(&tabs, 0), Some(0));
        assert_eq!(tab_index(&tabs, 1), Some(1));
    }

    #[test]
    fn a_tab_key_is_unique_per_tab() {
        assert_eq!(tab_key(0), "tab-0");
        assert_ne!(tab_key(1), tab_key(11));
    }

    #[test]
    fn a_distro_named_like_a_profile_does_not_list_twice() {
        let file = vec![settings::Profile {
            name: "Ubuntu".to_string(),
            ..settings::Profile::default()
        }];
        let distros = vec![
            settings::Profile {
                name: "Ubuntu".to_string(),
                ..settings::Profile::default()
            },
            settings::Profile {
                name: "Debian".to_string(),
                ..settings::Profile::default()
            },
        ];

        let names: Vec<String> = merged(file, distros).into_iter().map(|profile| profile.name).collect();
        assert_eq!(names, vec!["Ubuntu", "Debian"]);
    }

    #[test]
    fn a_label_picks_the_side_that_reads() {
        assert_eq!(contrasting_text(Color::rgb(12, 12, 12)), Color::rgb(230, 230, 230));
        assert_eq!(contrasting_text(Color::rgb(250, 250, 250)), Color::rgb(20, 20, 20));
    }

    #[test]
    fn a_theme_colour_becomes_the_toolkit_color() {
        let colour = settings::Color::parse("#102030").expect("a hex colour");
        assert_eq!(theme_color(colour), Some(Color::rgb(0x10, 0x20, 0x30)));
    }

    #[test]
    fn a_colorref_becomes_the_toolkit_color() {
        // COLORREF is 0x00BBGGRR, the order the palette stores.
        assert_eq!(color_of(0x0020_1020), Color::rgb(0x20, 0x10, 0x20));
        assert_eq!(color_of(0x00C8_3010), Color::rgb(0x10, 0x30, 0xC8));
    }

    #[test]
    fn blend_picks_the_endpoints_and_the_middle() {
        let black = Color::rgb(0, 0, 0);
        let white = Color::rgb(255, 255, 255);
        assert_eq!(blend(black, white, 0.0), black);
        assert_eq!(blend(black, white, 1.0), white);
        assert_eq!(blend(black, white, 0.5), Color::rgb(128, 128, 128));
        // A tab between the title bar and the pane is neither of them.
        let titlebar = TITLEBAR;
        let pane = Color::rgb(12, 12, 12);
        let hover = blend(titlebar, pane, HOVER_MIX);
        assert_ne!(hover, titlebar);
        assert_ne!(hover, pane);
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
            input::chord_action(&file.keybindings, Key::Char('q'), modifiers(ctrl_shift)),
            Some(settings::Action::Copy)
        );
        // The same key without Shift is not this chord.
        assert_eq!(
            input::chord_action(&file.keybindings, Key::Char('q'), modifiers(InputModifiers::CONTROL)),
            None
        );
    }

    #[test]
    fn the_defaults_bind_ctrl_c_to_copy() {
        let bindings = settings::Settings::default().keybindings;

        assert_eq!(
            input::chord_action(&bindings, Key::Char('c'), modifiers(InputModifiers::CONTROL)),
            Some(settings::Action::Copy)
        );
        assert_eq!(
            input::chord_action(&bindings, Key::Char('v'), modifiers(InputModifiers::CONTROL)),
            Some(settings::Action::Paste)
        );
    }

    #[test]
    fn the_scroll_chords_are_bound_by_default() {
        let bindings = settings::Settings::default().keybindings;
        let shift = InputModifiers::SHIFT;

        assert_eq!(
            input::chord_action(&bindings, Key::PageUp, modifiers(shift)),
            Some(settings::Action::ScrollUpPage)
        );
        assert_eq!(
            input::chord_action(&bindings, Key::PageDown, modifiers(shift)),
            Some(settings::Action::ScrollDownPage)
        );
        // A page key without Shift is the program's.
        assert_eq!(
            input::chord_action(&bindings, Key::PageUp, modifiers(InputModifiers::NONE)),
            None
        );
    }

    #[test]
    fn a_key_without_a_settings_name_is_not_a_chord() {
        assert_eq!(input::settings_key(Key::LeftShift), None);
        assert_eq!(input::settings_key(Key::CapsLock), None);
        assert_eq!(input::settings_key(Key::Char('A')), Some(settings::Key::Char('a')));
        assert_eq!(input::settings_key(Key::Function(5)), Some(settings::Key::Function(5)));
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
