//! The window's state: tabs, the renderer, chords and the pointer rules.
//!
//! Nothing here names AppKit except the [`Frame`] the pixels live in; the view
//! translates events into these methods, and the controller turns the requests
//! this module raises (a profile menu, the settings window, closing the window)
//! into AppKit work. That keeps the rules the WinUI and GTK shells share in the
//! same shape, and leaves a headless test able to exercise the geometry.

use std::path::{Path, PathBuf};
use std::rc::Rc;
use std::time::{Duration, Instant};

use objc2_foundation::{NSPoint, NSRect, NSSize};
use renderer::renderer::Renderer;
use renderer::{
    AtlasEngine, CellSizeInDIP, Composition, FontDueRasterizer, FontFace, FontInfo, IBackend, RenderSettings,
    RenderingPayload, Settings, SharedBackend, SoftwareBackend,
};
use server::pty::{PtyCommand, PtySize};
use term::input::{Key, MouseButton, MouseButtons, MouseEvent, MouseEventKind, WHEEL_DELTA, WheelAxis};
use types::text_color::ColorType;
use utils::Size;

use winter::find::FindBar;
use winter::input;
use winter::layout::{self, Axis, Direction, Fraction, Layout, PaneId, SplitPath};
use winter::palette::{self, Palette};
use winter::scrollbar::{self, Press, Scrollbar};
use winter::session::Session;
use winter::sound::Sound;
use winter::tabs::Tabs;
use winter::term_control::Control;
use winter::term_events::TerminalEvent;
use winter::term_settings::TerminalSettings;
use winter::terminal_render::TerminalData;
use winter::view::View as ViewState;

use super::chrome::{self, STRIP_HEIGHT, Strip, StripColors, StripHit};
use super::extension::Extensions;
use super::frame::Frame;
use super::metal;

/// The grid the window opens with, before its bounds are known.
pub(crate) const INITIAL_GRID: (i32, i32) = (100, 28);
/// How often the pty is polled, matching the WinUI shell (ADR 006).
pub(crate) const TICK: Duration = Duration::from_millis(16);
/// The cursor's blink half-period, matching the GTK and WinUI shells.
const BLINK: Duration = Duration::from_millis(500);

/// One pane: its own terminal, viewport, find bar and renderer.
pub(crate) struct Pane {
    pub(crate) id: PaneId,
    pub(crate) control: Control,
    pub(crate) view: ViewState,
    pub(crate) find: FindBar,
    /// The title the pane's program asked for.
    pub(crate) title: Option<String>,
    /// The program the pane runs, so the strip can find the tab's icon.
    pub(crate) program: String,
    pub(crate) renderer: Renderer<TerminalData>,
    pub(crate) content: Content,
    pub(crate) frame: Frame,
    /// The pane's grid in cells.
    pub(crate) grid: (i32, i32),
}

impl Pane {
    /// Starts a terminal and its renderer for `terminal`.
    fn new(
        id: PaneId,
        terminal: &TerminalSettings,
        grid: (i32, i32),
        fonts: Fonts,
        mtm: objc2::MainThreadMarker,
    ) -> Result<Self, String> {
        let session = spawn_shell(&terminal.command, pty_size(grid))?;
        let mut control = Control::new(Box::new(session), grid);
        control.apply_scheme(terminal.scheme.as_ref());
        let (renderer, content) = renderer_for(fonts, grid);
        let frame = Frame::new(1, 1, mtm).ok_or("cannot allocate the terminal's pixel buffer")?;

        Ok(Self {
            id,
            control,
            view: ViewState::new(),
            find: FindBar::default(),
            title: None,
            program: terminal.command.program().to_string_lossy().into_owned(),
            renderer,
            content,
            frame,
            grid,
        })
    }
}

/// One tab: a pane tree, its focused pane and a title.
pub(crate) struct Tab {
    pub(crate) panes: Vec<Pane>,
    pub(crate) layout: Layout,
    pub(crate) focused: PaneId,
    /// The terminal settings a split starts from.
    pub(crate) terminal: TerminalSettings,
    /// The title to show until the focused pane's shell sets one.
    pub(crate) title: String,
}

impl Tab {
    fn pane(&self, id: PaneId) -> Option<&Pane> {
        self.panes.iter().find(|pane| pane.id == id)
    }

    fn pane_mut(&mut self, id: PaneId) -> Option<&mut Pane> {
        self.panes.iter_mut().find(|pane| pane.id == id)
    }

    pub(crate) fn focused_pane(&self) -> Option<&Pane> {
        self.pane(self.focused)
    }

    pub(crate) fn focused_pane_mut(&mut self) -> Option<&mut Pane> {
        self.pane_mut(self.focused)
    }

    /// The title the strip shows: the focused pane's OSC title, else the tab's.
    pub(crate) fn label(&self) -> &str {
        self.focused_pane()
            .and_then(|pane| pane.title.as_deref())
            .unwrap_or(&self.title)
    }

    /// The program the focused pane runs, for the tab's icon.
    pub(crate) fn program(&self) -> &str {
        self.focused_pane().map_or("", |pane| pane.program.as_str())
    }
}

/// What one tick asks the controller to do.
#[derive(Debug, Default)]
pub(crate) struct Tick {
    /// The window's title changed.
    pub(crate) title: Option<String>,
    /// The last tab ended, so the window should close.
    pub(crate) close_window: bool,
    /// A program rang the bell.
    pub(crate) bell: bool,
    /// The notifications a program asked for.
    pub(crate) notifications: Vec<(String, String)>,
    /// Something was drawn, so the view needs a new frame.
    pub(crate) redraw: bool,
}

/// The shell's state.
pub(crate) struct Shell {
    pub(crate) tabs: Tabs<Tab>,
    template: TerminalSettings,
    bindings: Rc<[settings::Keybinding]>,
    settings: settings::Settings,
    /// The profiles the window offers. On macOS these are the file's profiles;
    /// the shell has no WSL-like source to merge.
    profiles: Vec<settings::Profile>,
    settings_path: Option<PathBuf>,
    settings_stamp: Option<settings::Stamp>,
    settings_at: Instant,
    warnings: Vec<String>,
    migrated_from: Option<PathBuf>,
    settings_error: Option<String>,
    /// The next pane identity, unique across tabs so a pane's layer can be
    /// tracked for the life of the window.
    next_pane: u64,
    face: FontFace,
    font_size: f32,
    /// The cell in physical pixels.
    cell: (u16, u16),
    /// The backing scale the face was rasterised for.
    scale: f32,
    /// The view's bounds in points.
    size: (f64, f64),
    /// The whole content area's grid, the size a new pane opens at.
    grid: (i32, i32),
    /// Whether the window is in full screen, where AppKit hides the traffic
    /// lights and the strip starts at the window's edge.
    fullscreen: bool,
    sides: input::ModifierSides,
    blink_on: bool,
    blink_at: Instant,
    focused: bool,
    scrollbar: Scrollbar,
    hover: Option<f64>,
    hovered_tab: Option<usize>,
    hovered_close: Option<usize>,
    /// The cell a forwarded move last reported, so a still pointer is silent.
    last_mouse_cell: Option<(i32, i32)>,
    /// The pane the current press started in.
    press_pane: Option<PaneId>,
    /// The cell a press started on, so the first move starts a selection.
    press: Option<((i32, i32), bool)>,
    /// The tab a press grabbed, until the drag ends.
    tab_drag: Option<usize>,
    /// The split whose divider the pointer is dragging.
    divider: Option<(SplitPath, Axis, NSRect)>,
    /// The pane whose scrollbar is being dragged.
    scrollbar_pane: Option<PaneId>,
    /// The cell the pointer is over, so a wheel can be aimed at a program.
    pointer_cell: Option<(i32, i32)>,
    /// Fractional trackpad scrolling waiting to make a row.
    wheel_pixels: f64,
    /// The menu the view asked for, until the controller takes it.
    menu_requested: bool,
    /// The settings window the controller should show, or hide.
    settings_requested: bool,
    /// The find field needs the keyboard.
    find_focus: bool,
    /// The command palette's entries, filter and selection.
    palette: Palette,
    /// The window should close once the last tab is gone.
    close_requested: bool,
    /// The active tab changed, so the window title must follow it.
    title_dirty: bool,
    /// The input method's marked text, drawn at the cursor until it commits.
    composition: Composition,
    /// The DECPS tones every tab asked for, played on a thread of their own.
    sound: Sound,
    /// Extensions, when `WINTER_EXT` asked for them.
    pub(crate) extensions: Option<Extensions>,
}

/// One pane as the controller lays its content layer.
pub(crate) struct PaneView {
    pub(crate) id: PaneId,
    /// The pane's area in view points.
    pub(crate) rect: NSRect,
    /// The grid's rectangle in view points.
    pub(crate) grid_rect: NSRect,
    /// The Metal backend, when the pane presents through a layer.
    pub(crate) metal: Option<metal::Metal>,
}

impl Shell {
    /// Starts the shell from the settings file and a host face.
    ///
    /// `scale` is the backing scale the window will have, so the face, the cell
    /// and the opening window's size agree. A face built at the wrong scale is
    /// replaced and the grid recomputed on the first resize, and a pty that
    /// changes size after the shell started lays its first prompt out for the
    /// old width (a zsh `PROMPT_EOL_MARK` is the visible symptom).
    pub(crate) fn new(face: FontFace, scale: f32, mtm: objc2::MainThreadMarker) -> Result<Self, String> {
        let scale = finite_scale(scale);
        settings::ensure_file();
        let loaded = settings::load_with_report();
        let settings_path = loaded.path.clone();
        let settings_stamp = settings_path.as_deref().and_then(settings::stamp);
        let (template, bindings, font_size) = resolved_settings(&loaded.settings);
        let profiles = loaded.settings.profiles.clone();
        let warnings: Vec<String> = loaded.warnings.iter().map(ToString::to_string).collect();

        let fonts = font_setup(&face, font_size, scale, INITIAL_GRID)?;
        let cell = fonts.settings.font.cell_size;
        let pane = Pane::new(PaneId(0), &template, INITIAL_GRID, fonts, mtm)?;

        let mut shell = Self {
            tabs: Tabs::new(Tab {
                panes: vec![pane],
                layout: Layout::leaf(PaneId(0)),
                focused: PaneId(0),
                terminal: template.clone(),
                title: template.title.clone(),
            }),
            template,
            bindings,
            settings: loaded.settings,
            profiles,
            settings_path,
            settings_stamp,
            settings_at: Instant::now(),
            warnings,
            migrated_from: loaded.migrated_from,
            settings_error: loaded.error.map(|error| error.to_string()),
            next_pane: 1,
            face,
            font_size,
            cell,
            scale,
            size: (0.0, 0.0),
            grid: INITIAL_GRID,
            fullscreen: false,
            sides: input::ModifierSides::default(),
            blink_on: true,
            blink_at: Instant::now(),
            focused: true,
            scrollbar: Scrollbar::default(),
            hover: None,
            hovered_tab: None,
            hovered_close: None,
            last_mouse_cell: None,
            press_pane: None,
            press: None,
            tab_drag: None,
            divider: None,
            scrollbar_pane: None,
            pointer_cell: None,
            wheel_pixels: 0.0,
            menu_requested: false,
            settings_requested: false,
            find_focus: false,
            palette: Palette::default(),
            close_requested: false,
            title_dirty: false,
            composition: Composition::default(),
            sound: Sound::new(),
            extensions: None,
        };
        shell.start_extensions();
        Ok(shell)
    }

    /// The active tab's title, or the application's name.
    pub(crate) fn title(&self) -> &str {
        self.tabs.active().map_or("Winter", |tab| tab.label())
    }

    pub(crate) fn titles(&self) -> Vec<String> {
        (0..self.tabs.len())
            .filter_map(|index| self.tabs.get(index).map(|tab| tab.label().to_string()))
            .collect()
    }

    /// The program each tab runs, in strip order.
    pub(crate) fn programs(&self) -> Vec<String> {
        (0..self.tabs.len())
            .filter_map(|index| self.tabs.get(index).map(|tab| tab.program().to_string()))
            .collect()
    }

    pub(crate) fn active_index(&self) -> usize {
        self.tabs.index()
    }

    pub(crate) fn strip(&self) -> Strip {
        // A full-screen window has no traffic lights to clear.
        let inset = if self.fullscreen { 0.0 } else { chrome::LEFT_INSET };
        chrome::strip(self.size.0, self.tabs.len(), inset)
    }

    /// Tells the strip whether the window is full screen.
    pub(crate) fn set_fullscreen(&mut self, fullscreen: bool) {
        self.fullscreen = fullscreen;
    }

    pub(crate) fn cell(&self) -> (u16, u16) {
        self.cell
    }

    pub(crate) fn scale(&self) -> f32 {
        self.scale
    }

    /// One pane as the controller lays its content layer.
    pub(crate) fn pane_views(&self) -> Vec<PaneView> {
        let Some(tab) = self.tabs.active() else {
            return Vec::new();
        };
        let area = self.pane_area();
        tab.layout
            .rects()
            .into_iter()
            .filter_map(|(id, fraction)| {
                let pane = tab.pane(id)?;
                let rect = chrome::fraction_rect(area, fraction);
                let grid_rect = chrome::pane_grid_rect(rect, f64::from(finite_scale(self.scale)), self.cell, pane.grid);
                Some(PaneView {
                    id,
                    rect,
                    grid_rect,
                    metal: match &pane.content {
                        Content::Metal(backend) => Some(backend.clone()),
                        Content::Software(_) => None,
                    },
                })
            })
            .collect()
    }

    /// The active tab's focused pane rectangle, in view points.
    pub(crate) fn focused_pane_rect(&self) -> Option<NSRect> {
        self.pane_views()
            .into_iter()
            .find(|view| self.tabs.active().is_some_and(|tab| tab.focused == view.id))
            .map(|view| view.rect)
    }

    /// Every pane in every tab, so the controller can drop a closed pane's layer.
    pub(crate) fn pane_ids(&self) -> Vec<PaneId> {
        (0..self.tabs.len())
            .filter_map(|index| self.tabs.get(index))
            .flat_map(|tab| tab.panes.iter().map(|pane| pane.id))
            .collect()
    }

    /// The active tab's split tree, for the chrome's dividers.
    pub(crate) fn splits(&self) -> Vec<(SplitPath, Axis, Fraction, f32)> {
        self.tabs.active().map_or_else(Vec::new, |tab| tab.layout.splits())
    }

    /// The panes as the view paints them, with their frames borrowed.
    pub(crate) fn pane_paints(&self, now: Instant, layered: bool, capturing: bool) -> Vec<chrome::PanePaint<'_>> {
        let Some(tab) = self.tabs.active() else {
            return Vec::new();
        };
        let area = self.pane_area();
        tab.layout
            .rects()
            .into_iter()
            .filter_map(|(id, fraction)| {
                let pane = tab.pane(id)?;
                let rect = chrome::fraction_rect(area, fraction);
                let grid_rect = chrome::pane_grid_rect(rect, f64::from(finite_scale(self.scale)), self.cell, pane.grid);
                Some(chrome::PanePaint {
                    rect,
                    grid_rect,
                    fill: chrome::rgb(pane.control.default_colors().0),
                    frame: if layered && pane.content.layer_presented() && !capturing {
                        None
                    } else {
                        Some(&pane.frame)
                    },
                    scroll: (
                        pane.view.scroll(),
                        pane.renderer.data().document_height(),
                        pane.control.size().1,
                    ),
                    marks: pane.renderer.data().marks(),
                    bell: pane.control.bell_alpha(now),
                    find_open: pane.id == tab.focused && pane.find.open,
                    focused: pane.id == tab.focused,
                })
            })
            .collect()
    }

    /// The window's content area, below the strip.
    fn pane_area(&self) -> NSRect {
        chrome::pane_rect(NSRect::new(
            NSPoint::new(0.0, 0.0),
            NSSize::new(self.size.0, self.size.1),
        ))
    }

    /// The focused pane's terminal pixels, when the view blits them.
    pub(crate) fn focused_pane(&self) -> Option<&Pane> {
        self.tabs.active().and_then(|tab| tab.focused_pane())
    }

    /// Whether the GPU path paints any pane.
    pub(crate) fn uses_metal(&self) -> bool {
        self.tabs
            .active()
            .is_some_and(|tab| tab.panes.iter().any(|pane| matches!(pane.content, Content::Metal(_))))
    }

    /// Shows or clears the input method's marked text.
    pub(crate) fn set_composition(&mut self, text: &str, selected_utf16: usize) {
        self.composition = winter::terminal_render::composition(text, selected_utf16);
    }

    fn clear_composition(&mut self) {
        self.composition = Composition::default();
    }

    /// The caret's box in view points, for anchoring the candidate window.
    pub(crate) fn caret_rect(&self) -> (f64, f64, f64, f64) {
        let Some(pane) = self.focused_pane() else {
            return (0.0, 0.0, 1.0, 1.0);
        };
        let origin = self
            .focused_pane_rect()
            .map_or(NSPoint::new(0.0, STRIP_HEIGHT), |rect| rect.origin);
        let point = pane.renderer.data().cursor_screen();
        let scale = f64::from(finite_scale(self.scale));
        let width = f64::from(self.cell.0) / scale;
        let height = f64::from(self.cell.1) / scale;
        (
            origin.x + chrome::PANE_LEFT + f64::from(point.x()) * width,
            origin.y + f64::from(point.y()) * height,
            width.max(1.0),
            height.max(1.0),
        )
    }

    pub(crate) fn set_focused(&mut self, focused: bool) {
        self.focused = focused;
    }

    /// The visible screen as plain text, for the accessibility client.
    pub(crate) fn accessible_text(&self) -> String {
        self.focused_pane()
            .map_or_else(String::new, |pane| pane.renderer.data().visible_text())
    }

    pub(crate) fn menu_requested(&mut self) -> bool {
        std::mem::take(&mut self.menu_requested)
    }

    pub(crate) fn settings_requested(&mut self) -> bool {
        std::mem::take(&mut self.settings_requested)
    }

    pub(crate) fn take_find_focus(&mut self) -> bool {
        std::mem::take(&mut self.find_focus)
    }

    pub(crate) fn settings(&self) -> &settings::Settings {
        &self.settings
    }

    pub(crate) fn settings_path(&self) -> Option<&Path> {
        self.settings_path.as_deref()
    }

    pub(crate) fn warnings(&self) -> &[String] {
        &self.warnings
    }

    pub(crate) fn migrated_from(&self) -> Option<&Path> {
        self.migrated_from.as_deref()
    }

    pub(crate) fn settings_error(&self) -> Option<&str> {
        self.settings_error.as_deref()
    }

    pub(crate) fn profiles(&self) -> &[settings::Profile] {
        &self.profiles
    }

    /// The strip's fills, from the pane's palette and the theme.
    pub(crate) fn strip_colors(&self) -> StripColors {
        let (background, foreground) = self.active_colors();
        let theme = self.settings.theme();
        let row = theme
            .and_then(|theme| theme.tab_row_background(self.focused))
            .and_then(theme_color);
        let active = theme.and_then(|theme| theme.tab_background(true)).and_then(theme_color);
        let inactive = theme
            .and_then(|theme| theme.tab_background(false))
            .and_then(theme_color);
        chrome::strip_colors(background, foreground, row, active, inactive)
    }

    /// The focused pane's background and foreground, as `COLORREF`s.
    pub(crate) fn active_colors(&self) -> (u32, u32) {
        self.focused_pane()
            .map_or((0x000C_0C0C, 0x00CC_CCCC), |pane| pane.control.default_colors())
    }

    /// Whether AppKit's dark appearance reads best on the active pane.
    pub(crate) fn prefers_dark(&self) -> bool {
        let (background, _) = self.active_colors();
        chrome::dark_background(background)
    }

    /// The bell's fade at `now`, while the focused pane flashes.
    pub(crate) fn bell_alpha(&self, now: Instant) -> Option<f32> {
        self.focused_pane().and_then(|pane| pane.control.bell_alpha(now))
    }

    /// The find bar's query, current match and match count.
    pub(crate) fn find_state(&self) -> (bool, String, usize, usize) {
        let Some(pane) = self.focused_pane() else {
            return (false, String::new(), 0, 0);
        };
        let current = pane.find.current.map_or(0, |index| index + 1);
        (
            pane.find.open,
            pane.find.query.clone(),
            current,
            pane.find.matches.len(),
        )
    }

    pub(crate) fn find_open(&self) -> bool {
        self.focused_pane().is_some_and(|pane| pane.find.open)
    }

    /// Fits the grids and the face to a new view size.
    pub(crate) fn resize(&mut self, size: (f64, f64), scale: f32) {
        let scale = finite_scale(scale);
        self.size = size;
        let rescaled = (scale - self.scale).abs() > f32::EPSILON;
        if rescaled {
            self.scale = scale;
            self.rebuild_face();
        }

        let bounds = NSRect::new(NSPoint::new(0.0, 0.0), NSSize::new(size.0, size.1));
        self.apply_grid(chrome::grid_for(bounds, f64::from(scale), self.cell));
    }

    /// Replaces every pane's renderer with one whose face is `scale` times the
    /// DIP size.
    fn rebuild_face(&mut self) {
        let face = &self.face;
        let (font_size, scale) = (self.font_size, self.scale);
        let mut cell = self.cell;

        for index in 0..self.tabs.len() {
            let Some(tab) = self.tabs.get_mut(index) else {
                continue;
            };
            for pane in &mut tab.panes {
                let Ok(fonts) = font_setup(face, font_size, scale, pane.grid) else {
                    continue;
                };
                cell = fonts.settings.font.cell_size;
                let (renderer, content) = renderer_for(fonts, pane.grid);
                pane.renderer = renderer;
                pane.content = content;
            }
        }

        self.cell = cell;
    }

    /// Fits every pane's terminal and renderer to its part of the window.
    fn apply_grid(&mut self, grid: (i32, i32)) {
        self.grid = grid;
        let area = self.pane_area();
        let (cell, scale) = (self.cell, f64::from(finite_scale(self.scale)));

        for index in 0..self.tabs.len() {
            let Some(tab) = self.tabs.get_mut(index) else {
                continue;
            };
            for (id, fraction) in tab.layout.rects() {
                let Some(pane) = tab.pane_mut(id) else {
                    continue;
                };
                let rect = chrome::fraction_rect(area, fraction);
                let cells = chrome::pane_grid(rect, scale, cell);
                if cells == pane.grid {
                    continue;
                }
                pane.grid = cells;
                pane.control.resize(cells.0, cells.1);
                let _ = pane.renderer.resize(target_size(cell, cells), chrome::viewport(cells));
            }
        }
    }

    /// Fits the panes of one tab after a split, a close or a divider drag.
    fn resize_tab(&mut self, index: usize) {
        let area = self.pane_area();
        let (cell, scale) = (self.cell, f64::from(finite_scale(self.scale)));
        let Some(tab) = self.tabs.get_mut(index) else {
            return;
        };
        for (id, fraction) in tab.layout.rects() {
            let Some(pane) = tab.pane_mut(id) else {
                continue;
            };
            let rect = chrome::fraction_rect(area, fraction);
            let cells = chrome::pane_grid(rect, scale, cell);
            if cells == pane.grid {
                continue;
            }
            pane.grid = cells;
            pane.control.resize(cells.0, cells.1);
            let _ = pane.renderer.resize(target_size(cell, cells), chrome::viewport(cells));
        }
    }

    /// Pumps every pty, repaints the active tab and re-arms nothing: the
    /// controller's timer calls this again.
    pub(crate) fn tick(&mut self, now: Instant, mtm: objc2::MainThreadMarker) -> Tick {
        let mut tick = Tick::default();
        let mut exited: Vec<(usize, PaneId)> = Vec::new();
        let mut title_changed = false;
        let mut notes = Vec::new();

        if self.close_requested {
            tick.close_window = true;
            return tick;
        }

        for index in 0..self.tabs.len() {
            let Some(tab) = self.tabs.get_mut(index) else {
                continue;
            };
            for pane in &mut tab.panes {
                pane.control.pump();
                notes.extend(pane.control.take_sounds());
                for event in pane.control.take_events(now) {
                    match event {
                        TerminalEvent::Exited => exited.push((index, pane.id)),
                        TerminalEvent::Bell => tick.bell = true,
                        TerminalEvent::Notification { title, body } => tick.notifications.push((title, body)),
                    }
                }
                if let Some(title) = pane.control.title().map(str::to_string)
                    && pane.title.as_deref() != Some(title.as_str())
                {
                    pane.title = Some(title);
                    title_changed = true;
                }
            }
        }

        if !notes.is_empty() {
            self.sound.play(audio::midi::tones(&notes));
        }

        for (index, pane_id) in exited.into_iter().rev() {
            if self.tabs.get(index).is_none() {
                continue;
            }
            if self.tabs.get(index).is_some_and(|tab| tab.layout.is_single()) {
                if !self.tabs.close(index) {
                    self.close_requested = true;
                    tick.close_window = true;
                    return tick;
                }
            } else {
                self.remove_pane(index, pane_id);
            }
            self.title_dirty = true;
        }

        if title_changed || self.title_dirty {
            tick.title = Some(self.title().to_string());
            self.title_dirty = false;
        }

        if now.duration_since(self.blink_at) >= BLINK {
            self.blink_on = !self.blink_on;
            self.blink_at = now;
            tick.redraw = true;
        }

        let mut settings_reloaded = false;
        if now.duration_since(self.settings_at) >= Duration::from_secs(1) {
            self.settings_at = now;
            settings_reloaded = self.poll_settings();
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
            settings_reloaded,
        );

        self.paint(now, mtm);
        tick.redraw = true;
        tick
    }

    /// Removes a pane from a tab, closing the tab when it was its only one.
    fn remove_pane(&mut self, index: usize, pane_id: PaneId) {
        if self.tabs.get(index).is_some_and(|tab| tab.layout.is_single()) {
            if !self.tabs.close(index) {
                self.close_requested = true;
            }
            return;
        }

        {
            let Some(tab) = self.tabs.get_mut(index) else {
                return;
            };
            if !tab.layout.remove(pane_id) {
                return;
            }
            tab.panes.retain(|pane| pane.id != pane_id);
            if tab.focused == pane_id
                && let Some(next) = tab.layout.leaves().first().copied()
            {
                tab.focused = next;
            }
        }

        self.resize_tab(index);
    }

    /// Refreshes every pane's copy and paints the active tab.
    fn paint(&mut self, _now: Instant, mtm: objc2::MainThreadMarker) {
        let blink_on = self.blink_on;
        let Shell { tabs, composition, .. } = self;
        let Some(tab) = tabs.active_mut() else {
            return;
        };

        for pane in &mut tab.panes {
            pane.renderer
                .data_mut()
                .refresh(&mut pane.control, &pane.view, blink_on, Some(&pane.find));

            let (background, foreground) = pane.control.default_colors();
            let _ = pane.renderer.set_default_colors(background, foreground);
            if let Ok(table) = pane.control.color_table().lock() {
                pane.renderer.settings_mut().set_color_table(table.clone());
            }
            let marked = if pane.id == tab.focused {
                composition.clone()
            } else {
                Composition::default()
            };
            pane.renderer.data_mut().set_composition(marked);
            pane.renderer.invalidate_all();

            if pane.renderer.paint_frame().is_err() || pane.content.layer_presented() {
                continue;
            }
            // A presented drawable is not the frame's reader; only the fallback
            // and a capture copy the pixels into the bitmap.
            let width = pane.content.width();
            let height = pane.content.height();
            let frame = &mut pane.frame;
            pane.content.with_pixels(|pixels| {
                frame.copy(pixels, width, height, mtm);
            });
        }
    }

    /// Repaints every pane into its frame buffer for `WINTER_SCREENSHOT`.
    ///
    /// A layer's drawable never appears in the view's cached display, so the
    /// capture needs the offscreen pixels that a presenting frame skips.
    pub(crate) fn prepare_capture(&mut self, mtm: objc2::MainThreadMarker) {
        if let Some(tab) = self.tabs.active() {
            for pane in &tab.panes {
                pane.content.force_readback();
            }
        }
        self.paint(Instant::now(), mtm);
    }

    /// Reloads the settings file when a save changed its stamp.
    fn poll_settings(&mut self) -> bool {
        let Some(path) = &self.settings_path else {
            return false;
        };
        if settings::stamp(path) == self.settings_stamp {
            return false;
        }
        self.reload_settings();
        true
    }

    /// Re-reads the settings and applies what a running session can.
    pub(crate) fn reload_settings(&mut self) {
        let loaded = settings::load_with_report();
        self.settings_stamp = loaded.path.as_deref().and_then(settings::stamp);
        self.warnings = loaded.warnings.iter().map(ToString::to_string).collect();
        self.migrated_from = loaded.migrated_from.clone();
        if let Some(error) = loaded.error {
            self.settings_error = Some(error.to_string());
            return;
        }

        self.settings = loaded.settings;
        self.profiles = self.settings.profiles.clone();
        self.settings_path = loaded.path;
        self.settings_error = None;

        let (template, bindings, font_size) = resolved_settings(&self.settings);
        self.template = template;
        self.bindings = bindings;
        if font_size != self.font_size {
            self.font_size = font_size;
            self.rebuild_face();
            let bounds = NSRect::new(NSPoint::new(0.0, 0.0), NSSize::new(self.size.0, self.size.1));
            self.apply_grid(chrome::grid_for(bounds, f64::from(self.scale), self.cell));
        }

        let scheme = self.template.scheme.clone();
        for index in 0..self.tabs.len() {
            if let Some(tab) = self.tabs.get_mut(index) {
                for pane in &mut tab.panes {
                    pane.control.apply_scheme(scheme.as_ref());
                }
            }
        }
    }

    /// Writes a document and reloads it, returning the writer's error.
    pub(crate) fn save_document(&mut self, document: &settings::Settings) -> Result<(), String> {
        match settings::save(document) {
            Ok(()) => {
                self.reload_settings();
                Ok(())
            }
            Err(error) => {
                let message = error.to_string();
                self.settings_error = Some(message.clone());
                Err(message)
            }
        }
    }

    /// Opens a tab on the default profile and activates it.
    pub(crate) fn open_tab(&mut self) {
        self.add_tab(self.template.clone());
    }

    /// Opens a tab on the named profile.
    pub(crate) fn open_profile(&mut self, name: &str) {
        let Some(profile) = self.profiles.iter().find(|profile| profile.name == name).cloned() else {
            return;
        };
        let terminal = TerminalSettings::for_profile(&self.settings, Some(&profile));
        self.add_tab(terminal);
    }

    /// Starts a terminal and adds it as the active tab.
    pub(crate) fn add_tab(&mut self, terminal: TerminalSettings) {
        let Some(mtm) = objc2::MainThreadMarker::new() else {
            return;
        };
        self.clear_composition();
        let title = terminal.title.clone();
        let Ok(fonts) = font_setup(&self.face, self.font_size, self.scale, self.grid) else {
            return;
        };
        let id = PaneId(self.next_pane);
        let Ok(pane) = Pane::new(id, &terminal, self.grid, fonts, mtm) else {
            return;
        };
        self.next_pane += 1;

        self.tabs.add(Tab {
            panes: vec![pane],
            layout: Layout::leaf(id),
            focused: id,
            terminal,
            title,
        });
        let index = self.tabs.index();
        self.resize_tab(index);
        self.title_dirty = true;
        self.report_extension(extensions::Event::TabOpened { index });
    }
    pub(crate) fn split_pane(&mut self, axis: Axis) {
        let Some(mtm) = objc2::MainThreadMarker::new() else {
            return;
        };
        self.clear_composition();

        let (focused, terminal, grid) = {
            let Some(tab) = self.tabs.active() else {
                return;
            };
            let focused = tab.focused;
            let grid = tab.focused_pane().map_or(self.grid, |pane| {
                // Spawn the new half at the size it will fill, so its shell lays
                // its first prompt out for the width it will keep.
                let scale = f64::from(finite_scale(self.scale));
                self.pane_rect(focused)
                    .map(|rect| chrome::split_second(rect, axis))
                    .map_or(pane.grid, |rect| chrome::pane_grid(rect, scale, self.cell))
            });
            (focused, tab.terminal.clone(), grid)
        };
        let Ok(fonts) = font_setup(&self.face, self.font_size, self.scale, grid) else {
            return;
        };
        let id = PaneId(self.next_pane);
        let Ok(pane) = Pane::new(id, &terminal, grid, fonts, mtm) else {
            return;
        };
        self.next_pane += 1;

        let Some(tab) = self.tabs.active_mut() else {
            return;
        };
        if !tab.layout.split(focused, axis, id) {
            return;
        }
        tab.panes.push(pane);
        tab.focused = id;

        let index = self.tabs.index();
        self.resize_tab(index);
        self.title_dirty = true;
    }

    /// Closes the focused pane, or the tab when it is the last one.
    pub(crate) fn close_pane(&mut self) {
        self.clear_composition();
        let index = self.tabs.index();
        let Some(focused) = self.tabs.active().map(|tab| tab.focused) else {
            return;
        };
        if self.tabs.get(index).is_some_and(|tab| tab.layout.is_single()) {
            if !self.tabs.close(index) {
                self.close_requested = true;
            }
        } else {
            self.remove_pane(index, focused);
        }
        self.title_dirty = true;
    }

    /// Moves the focus to the nearest pane in `direction`.
    pub(crate) fn focus_pane(&mut self, direction: Direction) {
        let changed = {
            let Some(tab) = self.tabs.active_mut() else {
                return;
            };
            let Some(next) = layout::neighbour(&tab.layout.rects(), tab.focused, direction) else {
                return;
            };
            let changed = tab.focused != next;
            tab.focused = next;
            changed
        };
        if changed {
            self.clear_composition();
            self.title_dirty = true;
        }
    }

    /// Grows the focused pane one step in `direction`.
    pub(crate) fn resize_pane(&mut self, direction: Direction) {
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

    pub(crate) fn select_tab(&mut self, index: usize) {
        self.clear_composition();
        if self.tabs.select(index) {
            self.title_dirty = true;
        }
    }

    pub(crate) fn step_tab(&mut self, delta: i32) {
        self.clear_composition();
        self.tabs.step(delta);
        self.title_dirty = true;
    }

    /// Closes the active tab, or asks the window to close when it is the last.
    pub(crate) fn close_active_tab(&mut self) {
        self.clear_composition();
        let index = self.tabs.index();
        if !self.tabs.close(index) {
            self.close_requested = true;
        }
        self.title_dirty = true;
        self.report_extension(extensions::Event::TabClosed { index });
    }

    /// Closes a tab by its position in the strip.
    pub(crate) fn close_tab(&mut self, index: usize) {
        self.clear_composition();
        if !self.tabs.close(index) {
            self.close_requested = true;
        }
        self.title_dirty = true;
        self.report_extension(extensions::Event::TabClosed { index });
    }

    /// Whether the strip press at `point` was handled.
    ///
    /// `false` means the press was in the strip's empty space, which the view
    /// turns into a window drag.
    pub(crate) fn strip_pressed(&mut self, point: (f64, f64)) -> bool {
        if point.1 >= STRIP_HEIGHT {
            return false;
        }
        let layout = self.strip();
        match chrome::strip_hit(&layout, point.0) {
            Some(StripHit::Tab(index)) => {
                self.tabs.select(index);
                // A press on a tab body also arms a reorder; the close box, the
                // buttons and the empty strip do not, so a close never drags.
                self.tab_drag = Some(index);
                true
            }
            Some(StripHit::Close(index)) => {
                self.close_tab(index);
                true
            }
            Some(StripHit::New) => {
                self.open_tab();
                true
            }
            Some(StripHit::Menu) => {
                self.menu_requested = true;
                true
            }
            None => false,
        }
    }

    /// A key press, from the keyboard before the terminal sees it.
    ///
    /// The command key's shortcuts are the platform's; a chord from the settings
    /// file follows, and everything else becomes a `term` key or text.
    pub(crate) fn key_down(
        &mut self,
        key: Key,
        modifiers: input::Modifiers,
        repeat: bool,
        text: Option<String>,
    ) -> bool {
        if modifiers.command {
            return self.command_shortcut(key, modifiers);
        }

        // The palette's field owns the keyboard; a key that still reaches the
        // view must not reach the terminal.
        if self.palette.open {
            return true;
        }

        if let Some(action) = input::chord_action(&self.bindings, key, modifiers)
            && self.perform(action)
        {
            return true;
        }

        if self.find_open() {
            if key == Key::Escape {
                self.close_find();
            }
            return true;
        }

        // A printable key is its characters; a control or option chord is the
        // key itself, which is what encodes ^C and the Meta prefix.
        let printable = input::produces_text(key, modifiers);
        let text =
            text.filter(|text| printable && !text.is_empty() && !text.chars().any(|character| character.is_control()));
        if let Some(text) = text {
            self.send_input(&input::Event::Text(text));
        } else {
            self.send_input(&input::Event::Key {
                key,
                pressed: true,
                repeat,
                modifiers,
                virtual_key: 0,
                scan_code: 0,
                repeat_count: 1,
                codepoint: None,
            });
        }
        true
    }

    /// A key release, so a kitty-aware program sees it.
    pub(crate) fn key_up(&mut self, key: Key, modifiers: input::Modifiers) {
        if modifiers.command || self.palette.open {
            return;
        }
        // A printable key was typed through the text path, which carries no key
        // identity; a release for it would be a release without a press.
        if input::produces_text(key, modifiers) {
            return;
        }
        self.send_input(&input::Event::Key {
            key,
            pressed: false,
            repeat: false,
            modifiers,
            virtual_key: 0,
            scan_code: 0,
            repeat_count: 1,
            codepoint: None,
        });
    }

    /// A modifier's state changed.
    pub(crate) fn flags_changed(&mut self, key: Key, pressed: bool, modifiers: input::Modifiers) {
        input::track_modifier(&mut self.sides, key, pressed);
        self.send_input(&input::Event::Key {
            key,
            pressed,
            repeat: false,
            modifiers,
            virtual_key: 0,
            scan_code: 0,
            repeat_count: 1,
            codepoint: None,
        });
    }

    /// Sends composed text, from the keyboard or an input method.
    pub(crate) fn text_input(&mut self, text: String) {
        if !text.is_empty() {
            self.send_input(&input::Event::Text(text));
        }
    }

    /// Runs a command-key shortcut.
    fn command_shortcut(&mut self, key: Key, modifiers: input::Modifiers) -> bool {
        let shift = modifiers.shift;
        let alt = modifiers.alt;
        let ctrl = modifiers.ctrl;
        match key {
            Key::Char('c') if !shift => {
                self.copy_selection();
            }
            Key::Char('v') if !shift => self.paste_clipboard(),
            Key::Char('t') if !shift => self.open_tab(),
            Key::Char('w') if shift => self.close_pane(),
            Key::Char('w') => self.close_active_tab(),
            Key::Char('d') if shift => self.split_pane(Axis::Vertical),
            Key::Char('d') => self.split_pane(Axis::Horizontal),
            Key::Char('f') if !shift => self.toggle_find(),
            Key::Char(',') => self.settings_requested = true,
            Key::Left if alt && ctrl => self.resize_pane(Direction::Left),
            Key::Right if alt && ctrl => self.resize_pane(Direction::Right),
            Key::Up if alt && ctrl => self.resize_pane(Direction::Up),
            Key::Down if alt && ctrl => self.resize_pane(Direction::Down),
            Key::Left if alt => self.focus_pane(Direction::Left),
            Key::Right if alt => self.focus_pane(Direction::Right),
            Key::Up if alt => self.focus_pane(Direction::Up),
            Key::Down if alt => self.focus_pane(Direction::Down),
            Key::Char(character @ '1'..='9') => {
                let index = character as usize - '1' as usize;
                self.select_tab(index);
            }
            Key::Char(']') => self.step_tab(1),
            Key::Char('[') => self.step_tab(-1),
            _ => {}
        }
        true
    }

    /// Runs a window action, reporting whether it consumed the key.
    fn perform(&mut self, action: settings::Action) -> bool {
        match action {
            settings::Action::NewTab => self.open_tab(),
            settings::Action::CloseTab => self.close_active_tab(),
            settings::Action::NextTab => self.step_tab(1),
            settings::Action::PrevTab => self.step_tab(-1),
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
            settings::Action::ScrollUpPage => self.scroll_page(1),
            settings::Action::ScrollDownPage => self.scroll_page(-1),
            settings::Action::ScrollToTop => self.scroll_to(false),
            settings::Action::ScrollToBottom => self.scroll_to(true),
            settings::Action::OpenSettings => self.settings_requested = true,
            settings::Action::Find => self.toggle_find(),
            settings::Action::CommandPalette => self.toggle_palette(),
            _ => {}
        }
        true
    }

    /// Sends a portable event through the shared translation rules.
    fn send_input(&mut self, event: &input::Event) {
        // A printable key travels as the character it typed, unless the program
        // asked for every key: then the key and its release travel instead.
        let modes = self.focused_pane().map(|pane| pane.control.input_modes());
        let win32 = modes.is_some_and(|modes| modes.win32_input);
        let keys = modes.is_some_and(|modes| modes.reports_all_keys());
        let outcome = input::translate(event, &mut self.sides, win32, keys);
        let Some(pane) = self.tabs.active_mut().and_then(|tab| tab.focused_pane_mut()) else {
            return;
        };
        if outcome.activity {
            pane.view.scroll_to_bottom();
        }
        if let Some(text) = outcome.paste {
            pane.control.paste(&text);
        }
        for key in outcome.keys {
            pane.control.send_key(&key);
        }
    }

    /// Puts the focused pane's selection on the clipboard, clearing it like
    /// upstream.
    pub(crate) fn copy_selection(&mut self) -> bool {
        let Some(tab) = self.tabs.active_mut() else {
            return false;
        };
        let Some(pane) = tab.focused_pane_mut() else {
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
            pane.view.clear_selection();
        }
        copied
    }

    /// Sends the clipboard through the focused pane's bracketed-paste filter.
    pub(crate) fn paste_clipboard(&mut self) {
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

    /// Shows or hides the focused pane's find bar.
    pub(crate) fn toggle_find(&mut self) {
        let Some(tab) = self.tabs.active_mut() else {
            return;
        };
        let Some(pane) = tab.focused_pane_mut() else {
            return;
        };
        pane.find.toggle();
        if pane.find.open {
            pane.find.refresh(&mut pane.control);
            pane.find.scroll_to(&pane.control, &mut pane.view);
            self.find_focus = true;
            // The field takes the keyboard, so a half-composed terminal
            // preview is no longer where the user is typing.
            self.clear_composition();
        }
    }

    /// Sets the find query and recomputes the focused pane's matches.
    pub(crate) fn find_query(&mut self, query: String) {
        let Some(pane) = self.tabs.active_mut().and_then(|tab| tab.focused_pane_mut()) else {
            return;
        };
        pane.find.query = query;
        pane.find.refresh(&mut pane.control);
        pane.find.scroll_to(&pane.control, &mut pane.view);
    }

    /// Moves to the focused pane's next (true) or previous match.
    pub(crate) fn find_step(&mut self, forward: bool) {
        let Some(pane) = self.tabs.active_mut().and_then(|tab| tab.focused_pane_mut()) else {
            return;
        };
        if pane.find.searched != pane.find.query {
            pane.find.refresh(&mut pane.control);
        }
        pane.find.step(forward);
        pane.find.scroll_to(&pane.control, &mut pane.view);
    }

    /// Closes the focused pane's find bar.
    pub(crate) fn close_find(&mut self) {
        if let Some(pane) = self.tabs.active_mut().and_then(|tab| tab.focused_pane_mut()) {
            pane.find.hide();
        }
    }

    /// Whether the command palette is showing.
    pub(crate) fn palette_open(&self) -> bool {
        self.palette.open
    }

    /// The palette's selected row, for the card's paint.
    pub(crate) fn palette_selected(&self) -> usize {
        self.palette.selected
    }

    /// The rows the current filter leaves, in display order.
    pub(crate) fn palette_rows(&self) -> Vec<palette::Entry> {
        let mut entries = palette::entries(&self.settings, palette_supports);
        if let Some(extensions) = &self.extensions {
            entries.extend(palette::extension_entries(
                extensions
                    .commands()
                    .into_iter()
                    .map(|command| (command.key, command.label)),
            ));
        }
        self.palette.rows(&entries).into_iter().cloned().collect()
    }

    /// Shows or hides the palette.
    pub(crate) fn toggle_palette(&mut self) {
        if self.palette.open {
            self.close_palette();
        } else {
            self.open_palette();
        }
    }

    /// Opens the palette with an empty query and the first row selected.
    pub(crate) fn open_palette(&mut self) {
        // The two overlays never share the pane; the find bar's controls would
        // sit above the card.
        self.close_find();
        // The field takes the keyboard, so a half-composed preview is no longer
        // where the user is typing.
        self.clear_composition();
        self.palette.show();
    }

    /// Closes the palette and forgets the query.
    pub(crate) fn close_palette(&mut self) {
        self.palette.hide();
    }

    /// Narrows the rows to `filter` and selects the first one.
    pub(crate) fn palette_filter(&mut self, filter: String) {
        self.palette.filter = filter;
        self.palette.selected = 0;
        let len = self.palette_rows().len();
        self.palette.clamp(len);
    }

    /// Moves the selection one row, wrapping at both ends.
    pub(crate) fn palette_step(&mut self, forward: bool) {
        let len = self.palette_rows().len();
        self.palette.step(len, forward);
    }

    /// Runs the row under a press in the palette card.
    pub(crate) fn palette_clicked(&mut self, point: (f64, f64)) {
        let rows = self.palette_rows();
        let first = chrome::palette_first_visible(rows.len(), self.palette.selected);
        if let Some(index) = chrome::palette_row_at(point, rows.len(), first) {
            self.palette.selected = index;
            self.run_selected_palette();
        }
    }

    /// Runs the row Enter chose, if any.
    pub(crate) fn run_selected_palette(&mut self) {
        if !self.palette.open {
            return;
        }
        let rows = self.palette_rows();
        let Some(entry) = rows.get(self.palette.selected) else {
            return;
        };
        let command = entry.command.clone();
        self.close_palette();
        self.run_palette_command(command);
    }

    /// Runs one palette row after the palette has closed.
    fn run_palette_command(&mut self, command: palette::Command) {
        match command {
            palette::Command::Action(action) => {
                self.perform(action);
            }
            palette::Command::Profile(name) => self.open_profile(&name),
            palette::Command::Extension(key) => self.run_extension(&key),
        }
    }

    /// The palette's focus request, once.
    pub(crate) fn take_palette_focus(&mut self) -> bool {
        std::mem::take(&mut self.palette.focus)
    }

    fn scroll_active(&mut self, rows: i32) {
        let Some(pane) = self.tabs.active_mut().and_then(|tab| tab.focused_pane_mut()) else {
            return;
        };
        let max = pane.control.max_scroll();
        pane.view.scroll_by(rows, max);
    }

    fn scroll_page(&mut self, direction: i32) {
        let Some(pane) = self.tabs.active_mut().and_then(|tab| tab.focused_pane_mut()) else {
            return;
        };
        let rows = (pane.control.size().1 - 1).max(1) * direction.signum();
        let max = pane.control.max_scroll();
        pane.view.scroll_by(rows, max);
    }

    fn scroll_to(&mut self, bottom: bool) {
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

    /// The active tab's pane under `point`, with its area in view points.
    fn pane_at(&self, point: (f64, f64)) -> Option<(PaneId, NSRect)> {
        let tab = self.tabs.active()?;
        let area = self.pane_area();
        tab.layout.rects().into_iter().find_map(|(id, fraction)| {
            let rect = chrome::fraction_rect(area, fraction);
            let inside = point.0 >= rect.origin.x
                && point.0 < rect.origin.x + rect.size.width
                && point.1 >= rect.origin.y
                && point.1 < rect.origin.y + rect.size.height;
            inside.then_some((id, rect))
        })
    }

    /// A pane's area in view points.
    fn pane_rect(&self, pane_id: PaneId) -> Option<NSRect> {
        let tab = self.tabs.active()?;
        let area = self.pane_area();
        tab.layout
            .rects()
            .into_iter()
            .find(|(id, _)| *id == pane_id)
            .map(|(_, fraction)| chrome::fraction_rect(area, fraction))
    }

    /// The cell a point in `rect` names, inside `pane`'s grid.
    fn cell_in(&self, pane: &Pane, rect: NSRect, point: (f64, f64)) -> Option<(i32, i32)> {
        let scale = f64::from(finite_scale(self.scale));
        let width = f64::from(self.cell.0) / scale;
        let height = f64::from(self.cell.1) / scale;
        if width <= 0.0 || height <= 0.0 {
            return None;
        }
        let column = ((point.0 - rect.origin.x - chrome::PANE_LEFT) / width).floor() as i32;
        let row = ((point.1 - rect.origin.y) / height).floor() as i32;
        let (columns, rows) = pane.grid;
        (column >= 0 && row >= 0 && column < columns && row < rows).then_some((column, row))
    }

    /// The cell a point names, or the nearest in-range cell of `pane`.
    fn clamped_cell_in(&self, pane: &Pane, rect: NSRect, point: (f64, f64)) -> Option<(i32, i32)> {
        let scale = f64::from(finite_scale(self.scale));
        let width = f64::from(self.cell.0) / scale;
        let height = f64::from(self.cell.1) / scale;
        if width <= 0.0 || height <= 0.0 {
            return None;
        }
        let column = ((point.0 - rect.origin.x - chrome::PANE_LEFT) / width).floor() as i32;
        let row = ((point.1 - rect.origin.y) / height).floor() as i32;
        let (columns, rows) = pane.grid;
        Some((column.clamp(0, columns.max(1) - 1), row.clamp(0, rows.max(1) - 1)))
    }

    /// Focuses a pane of the active tab.
    fn focus_at(&mut self, pane_id: PaneId) {
        let changed = {
            let Some(tab) = self.tabs.active_mut() else {
                return;
            };
            let changed = tab.focused != pane_id;
            tab.focused = pane_id;
            changed
        };
        if changed {
            self.clear_composition();
            self.title_dirty = true;
        }
    }

    /// Handles a press, returning whether it was consumed.
    ///
    /// `false` from the strip means the view should drag the window; `false`
    /// from anywhere else is not returned.
    pub(crate) fn pointer_pressed(
        &mut self,
        point: (f64, f64),
        button: MouseButton,
        buttons: MouseButtons,
        modifiers: input::Modifiers,
    ) -> bool {
        // The palette owns the pointer while it shows: a press outside the card
        // dismisses it and goes no further; a press on the card is the card
        // view's, so it never reaches this method.
        if self.palette.open {
            self.close_palette();
            return true;
        }

        if point.1 < STRIP_HEIGHT {
            if button != MouseButton::Left {
                return true;
            }
            return self.strip_pressed(point);
        }

        // A divider is grabbed before the panes it separates.
        if button == MouseButton::Left {
            let bounds = NSRect::new(NSPoint::new(0.0, 0.0), NSSize::new(self.size.0, self.size.1));
            if let Some((path, axis, split)) = chrome::divider_at(bounds, &self.splits(), point, 3.0) {
                self.divider = Some((path, axis, split));
                return true;
            }
        }

        let Some((pane_id, rect)) = self.pane_at(point) else {
            return true;
        };
        self.focus_at(pane_id);

        if self.find_open() {
            self.close_find();
        }

        if button == MouseButton::Left
            && point.0 >= rect.origin.x + rect.size.width - f64::from(scrollbar::WIDTH)
            && self.scrollbar_pressed(pane_id, point.1)
        {
            return true;
        }

        let Some(tab) = self.tabs.active() else {
            return true;
        };
        let Some(pane_ref) = tab.pane(pane_id) else {
            return true;
        };
        let tracking = pane_ref.control.is_tracking_mouse();
        let cell = self.cell_in(pane_ref, rect, point);
        let forward = input::forwards_to_application(tracking, modifiers.shift);
        self.pointer_cell = cell;

        if button != MouseButton::Left && !forward {
            self.paste_clipboard();
            return true;
        }

        let Some(tab) = self.tabs.active_mut() else {
            return true;
        };
        let Some(pane) = tab.pane_mut(pane_id) else {
            return true;
        };

        if forward {
            if let Some(position) = cell {
                let chord = input::key_modifiers(modifiers, self.sides.right_alt, self.sides.right_ctrl);
                pane.control.send_mouse(&MouseEvent {
                    position,
                    kind: MouseEventKind::Press(button),
                    buttons,
                    modifiers: chord,
                    delta: 0,
                });
                self.last_mouse_cell = Some(position);
            }
            return true;
        }

        let Some((column, row)) = cell else {
            return true;
        };
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
            pane.view.clear_selection();
            self.press = Some((position, modifiers.alt));
            self.press_pane = Some(pane_id);
        }
        true
    }

    /// Handles a pointer move while `buttons` are held.
    pub(crate) fn pointer_moved(&mut self, point: (f64, f64), buttons: MouseButtons, modifiers: input::Modifiers) {
        // The palette's list is the only thing the pointer moves while it
        // shows.
        if self.palette.open {
            return;
        }
        if let Some(from) = self.tab_drag {
            if !buttons.left {
                self.tab_drag = None;
                return;
            }
            let layout = self.strip();
            if let Some(target) = chrome::drag_target(&layout, from, point.0)
                && self.tabs.move_to(from, target)
            {
                self.tab_drag = Some(target);
            }
            // A tab drag owns the pointer wherever it goes vertically, so the
            // pane beneath it never starts a selection.
            return;
        }

        if let Some((path, axis, split)) = self.divider.clone() {
            if !buttons.left {
                self.divider = None;
                return;
            }
            let (origin, extent) = match axis {
                Axis::Horizontal => (point.0 - split.origin.x, split.size.width),
                Axis::Vertical => (point.1 - split.origin.y, split.size.height),
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

        if let Some(pane_id) = self.scrollbar_pane {
            if buttons.left {
                self.scrollbar_dragged(pane_id, point.1);
            } else {
                self.scrollbar_pane = None;
                self.scrollbar.release();
            }
            return;
        }

        self.hover(point);
        if point.1 < STRIP_HEIGHT {
            return;
        }

        // A selection in progress belongs to the pane the press started in; a
        // forwarded move follows the pointer.
        let target = self.press_pane.or_else(|| self.pane_at(point).map(|(id, _)| id));
        let Some(pane_id) = target else {
            return;
        };
        let Some(rect) = self.pane_rect(pane_id) else {
            return;
        };
        let Some(tab) = self.tabs.active() else {
            return;
        };
        let Some(pane_ref) = tab.pane(pane_id) else {
            return;
        };
        let tracking = pane_ref.control.is_tracking_mouse();
        let cell = self.clamped_cell_in(pane_ref, rect, point);
        self.pointer_cell = cell;
        let forward = input::forwards_to_application(tracking, modifiers.shift);
        let (columns, rows) = pane_ref.grid;

        let Some(tab) = self.tabs.active_mut() else {
            return;
        };
        let Some(pane) = tab.pane_mut(pane_id) else {
            return;
        };

        if forward {
            if let Some(position) = cell {
                let dragging = buttons.left || buttons.middle || buttons.right;
                if self.last_mouse_cell != Some(position) || dragging {
                    let chord = input::key_modifiers(modifiers, self.sides.right_alt, self.sides.right_ctrl);
                    pane.control.send_mouse(&MouseEvent {
                        position,
                        kind: MouseEventKind::Move,
                        buttons,
                        modifiers: chord,
                        delta: 0,
                    });
                    self.last_mouse_cell = Some(position);
                }
            } else {
                self.last_mouse_cell = None;
            }
            return;
        }

        if !buttons.left {
            return;
        }

        let scale = f64::from(finite_scale(self.scale));
        let width = f64::from(self.cell.0) / scale;
        let height = f64::from(self.cell.1) / scale;
        let outside = if point.1 < rect.origin.y {
            -1
        } else if height > 0.0 && point.1 >= rect.origin.y + f64::from(rows.max(0)) * height {
            1
        } else {
            0
        };
        let column = if width > 0.0 {
            ((point.0 - rect.origin.x - chrome::PANE_LEFT) / width).floor() as i32
        } else {
            0
        };

        let position = if outside != 0 {
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
            Some((anchor, block)) if position != anchor => {
                pane.view.select(anchor, block);
                pane.view.extend(position);
            }
            Some(press) => self.press = Some(press),
            None => pane.view.extend(position),
        }
    }

    /// Finishes a press.
    pub(crate) fn pointer_released(
        &mut self,
        point: (f64, f64),
        button: MouseButton,
        buttons: MouseButtons,
        modifiers: input::Modifiers,
    ) {
        // A press that dismissed the palette started no selection or drag.
        if self.palette.open {
            return;
        }
        if self.tab_drag.take().is_some() {
            return;
        }
        if self.divider.take().is_some() {
            self.press = None;
            self.press_pane = None;
            return;
        }

        if self.scrollbar_pane.take().is_some() {
            self.scrollbar.release();
            self.press = None;
            self.press_pane = None;
            self.last_mouse_cell = None;
            return;
        }

        let target = self.press_pane.take().or_else(|| self.pane_at(point).map(|(id, _)| id));
        let Some(pane_id) = target else {
            self.press = None;
            self.last_mouse_cell = None;
            return;
        };
        let Some(rect) = self.pane_rect(pane_id) else {
            self.press = None;
            self.last_mouse_cell = None;
            return;
        };
        let release = {
            let Some(tab) = self.tabs.active() else {
                return;
            };
            let Some(pane) = tab.pane(pane_id) else {
                return;
            };
            input::forwards_to_application(pane.control.is_tracking_mouse(), modifiers.shift)
                .then(|| self.clamped_cell_in(pane, rect, point))
                .flatten()
        };
        if let Some(position) = release {
            let Some(tab) = self.tabs.active_mut() else {
                return;
            };
            let Some(pane) = tab.pane_mut(pane_id) else {
                return;
            };
            let chord = input::key_modifiers(modifiers, self.sides.right_alt, self.sides.right_ctrl);
            pane.control.send_mouse(&MouseEvent {
                position,
                kind: MouseEventKind::Release(button),
                buttons,
                modifiers: chord,
                delta: 0,
            });
        }
        self.last_mouse_cell = None;
        self.press = None;
    }

    /// Updates the strip's hover state from a pointer position.
    fn hover(&mut self, point: (f64, f64)) {
        if point.1 >= STRIP_HEIGHT {
            self.hover = None;
            self.hovered_tab = None;
            self.hovered_close = None;
            return;
        }

        self.hover = Some(point.0);
        let layout = self.strip();
        self.hovered_tab = None;
        self.hovered_close = None;
        match chrome::strip_hit(&layout, point.0) {
            Some(StripHit::Tab(index)) => self.hovered_tab = Some(index),
            Some(StripHit::Close(index)) => {
                self.hovered_tab = Some(index);
                self.hovered_close = Some(index);
            }
            _ => {}
        }
    }

    pub(crate) fn pointer_left(&mut self) {
        self.hover = None;
        self.hovered_tab = None;
        self.hovered_close = None;
    }

    /// Whether the pointer is over the new-tab button.
    pub(crate) fn hovered_new(&self) -> bool {
        self.hover
            .is_some_and(|x| chrome::strip_hit(&self.strip(), x) == Some(StripHit::New))
    }

    /// Whether the pointer is over the profile drop-down.
    pub(crate) fn hovered_menu(&self) -> bool {
        self.hover
            .is_some_and(|x| chrome::strip_hit(&self.strip(), x) == Some(StripHit::Menu))
    }

    pub(crate) fn hovered_tab(&self) -> Option<usize> {
        self.hovered_tab
    }

    pub(crate) fn hovered_close(&self) -> Option<usize> {
        self.hovered_close
    }

    /// Turns a wheel or trackpad scroll into history rows or a mouse report in
    /// the pane under the pointer.
    pub(crate) fn wheel(&mut self, point: (f64, f64), delta: f64, precise: bool, shift: bool) {
        // The palette's list owns the wheel while it shows; the pane behind it
        // must not move.
        if self.palette.open || delta == 0.0 {
            return;
        }

        let rows = if precise {
            let height = f64::from(self.cell.1) / f64::from(finite_scale(self.scale));
            self.wheel_pixels += delta;
            let rows = if height > 0.0 {
                (self.wheel_pixels / height).trunc() as i32
            } else {
                0
            };
            self.wheel_pixels -= f64::from(rows) * height;
            rows
        } else {
            delta.round() as i32
        };
        if rows == 0 {
            return;
        }

        let target = self
            .pane_at(point)
            .map(|(id, _)| id)
            .or_else(|| self.tabs.active().map(|tab| tab.focused));
        let Some(pane_id) = target else {
            return;
        };
        let Some(rect) = self.pane_rect(pane_id) else {
            return;
        };
        let cell = {
            let Some(tab) = self.tabs.active() else {
                return;
            };
            let Some(pane) = tab.pane(pane_id) else {
                return;
            };
            self.clamped_cell_in(pane, rect, point)
        };
        self.pointer_cell = cell;
        let Some(tab) = self.tabs.active_mut() else {
            return;
        };
        let Some(pane) = tab.pane_mut(pane_id) else {
            return;
        };
        if input::forwards_to_application(pane.control.is_tracking_mouse(), shift)
            && let Some(position) = cell
            && pane
                .control
                .send_mouse(&MouseEvent::wheel(position, WheelAxis::Vertical, rows * WHEEL_DELTA))
        {
            return;
        }

        let max = pane.control.max_scroll();
        pane.view.scroll_by(rows, max);
    }

    /// The pane's scrollbar press, if `y` is on its track.
    fn scrollbar_pressed(&mut self, pane_id: PaneId, y: f64) -> bool {
        let Some(rect) = self.pane_rect(pane_id) else {
            return false;
        };
        let track = rect.size.height.max(0.0);
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

        let fraction = ((y - rect.origin.y) / track).clamp(0.0, 1.0) as f32;
        let positions: Vec<f32> = marks
            .iter()
            .map(|mark| scrollbar::position(mark.row, document))
            .collect();
        if let Some(index) = scrollbar::tick_at(&positions, fraction, scrollbar::MARK_GRAB / track as f32) {
            pane.view.scroll_to_row(marks[index].row, visible, document);
        } else if let Press::Jump(scroll) = self.scrollbar.press(fraction, pane.view.scroll(), document, visible) {
            pane.view.set_scroll(scroll, document - visible);
        }

        self.scrollbar_pane = Some(pane_id);
        true
    }

    fn scrollbar_dragged(&mut self, pane_id: PaneId, y: f64) {
        let Some(rect) = self.pane_rect(pane_id) else {
            return;
        };
        let track = rect.size.height.max(0.0);
        if track <= 0.0 {
            return;
        }
        let fraction = ((y - rect.origin.y) / track).clamp(0.0, 1.0) as f32;
        let Some(tab) = self.tabs.active_mut() else {
            return;
        };
        let Some(pane) = tab.pane_mut(pane_id) else {
            return;
        };
        let visible = pane.control.size().1;
        let document = pane.control.max_scroll() + visible;
        if let Some(scroll) = self.scrollbar.drag(fraction, document, visible) {
            pane.view.set_scroll(scroll, document - visible);
        }
    }
}

/// The settings parts a running shell applies.
fn resolved_settings(document: &settings::Settings) -> (TerminalSettings, Rc<[settings::Keybinding]>, f32) {
    (
        TerminalSettings::resolve(document),
        document.keybindings.clone().into(),
        document.font.size,
    )
}

/// The actions the palette may offer: the ones this shell's `perform` runs.
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

/// A theme colour as red, green and blue.
fn theme_color(color: settings::Color) -> Option<(u8, u8, u8)> {
    match color.text_color().get_type() {
        ColorType::Rgb { r, g, b } => Some((r, g, b)),
        _ => None,
    }
}

/// Sets up the face and metrics the atlas paints with at `scale`.
struct Fonts {
    font: FontInfo,
    settings: Settings,
    shaper: FontDueRasterizer,
    rasterizer: FontDueRasterizer,
}

fn font_setup(face: &FontFace, base_size: f32, scale: f32, grid: (i32, i32)) -> Result<Fonts, String> {
    let scale = finite_scale(scale);
    let size = (base_size * scale).max(1.0);
    let shaper = FontDueRasterizer::from_face(face, size).map_err(|error| error.to_string())?;
    let rasterizer = FontDueRasterizer::from_face(face, size).map_err(|error| error.to_string())?;

    let (ascent, descent) = shaper.vertical_metrics();
    let advance = shaper.advance_width('M');
    let cell_w = (advance.ceil() as u16).max(1);
    let descent = descent.ceil().max(1.0);
    let cell_h = (ascent + descent).ceil().max(1.0) as u16;
    let baseline = ((f32::from(cell_h) - descent).round() as u16).clamp(1, cell_h.saturating_sub(1));

    let mut font = FontInfo::default();
    font.set_from_engine(
        face.family().to_string(),
        0,
        400,
        65001,
        base_size,
        CellSizeInDIP {
            width: f32::from(cell_w) / scale,
            height: f32::from(cell_h) / scale,
        },
        Size::new(i32::from(cell_w), i32::from(cell_h)),
    );

    let settings = Settings {
        font: renderer::FontSettings {
            font_name: face.family().to_string(),
            font_size: size,
            cell_size: (cell_w, cell_h),
            font_weight: 400,
            advance_width: cell_w,
            baseline,
            descender: cell_h.saturating_sub(baseline),
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

    Ok(Fonts {
        font,
        settings,
        shaper,
        rasterizer,
    })
}

fn renderer_for(fonts: Fonts, grid: (i32, i32)) -> (Renderer<TerminalData>, Content) {
    let Fonts {
        font,
        settings,
        shaper,
        rasterizer,
    } = fonts;

    // Metal is the content path; a host without a device, a shader that does
    // not compile, or `WINTER_SOFTWARE=1` keeps the CPU backend the shell
    // shipped with.
    let accelerated = if std::env::var_os("WINTER_SOFTWARE").is_none() {
        metal::Metal::new()
    } else {
        None
    };
    let shared = match accelerated {
        Some(metal) => {
            metal.set_rasterizer(Box::new(rasterizer));
            Content::Metal(metal)
        }
        None => {
            if std::env::var_os("WINTER_SOFTWARE").is_none() {
                eprintln!("winter: Metal is unavailable; using the software renderer");
            }
            Content::Software(SharedBackend::new(SoftwareBackend::with_rasterizer(Box::new(
                rasterizer,
            ))))
        }
    };

    let mut engine = AtlasEngine::with_settings(settings, Box::new(shared.clone()));
    engine.set_shaping(Box::new(shaper));

    let mut renderer = Renderer::new(
        TerminalData::new(font, RenderSettings::new(), grid.0, grid.1),
        RenderSettings::new(),
    );
    renderer.add_engine(Box::new(engine));
    (renderer, shared)
}

/// The content backend a frame is rasterised with.
///
/// Both variants hand the shell the same `0xAARRGGBB` pixels; only the work
/// that produced them differs.
#[derive(Clone)]
pub(crate) enum Content {
    Software(SharedBackend),
    Metal(metal::Metal),
}

impl Content {
    fn width(&self) -> u32 {
        match self {
            Self::Software(backend) => backend.width(),
            Self::Metal(backend) => backend.width(),
        }
    }

    fn height(&self) -> u32 {
        match self {
            Self::Software(backend) => backend.height(),
            Self::Metal(backend) => backend.height(),
        }
    }

    fn with_pixels<T>(&self, read: impl FnOnce(&[u32]) -> T) -> T {
        match self {
            Self::Software(backend) => backend.with_pixels(read),
            Self::Metal(backend) => backend.with_pixels(read),
        }
    }

    fn layer_presented(&self) -> bool {
        match self {
            Self::Software(_) => false,
            Self::Metal(backend) => backend.presented(),
        }
    }

    fn force_readback(&self) {
        match self {
            // The software backend's pixels are the frame; there is no
            // drawable to skip.
            Self::Software(_) => {}
            Self::Metal(backend) => backend.force_readback(),
        }
    }
}

impl IBackend for Content {
    fn release_resources(&mut self) {
        match self {
            Self::Software(backend) => backend.release_resources(),
            Self::Metal(backend) => backend.release_resources(),
        }
    }

    fn render(&mut self, payload: &mut RenderingPayload) {
        match self {
            Self::Software(backend) => backend.render(payload),
            Self::Metal(backend) => backend.render(payload),
        }
    }

    fn requires_continuous_redraw(&mut self) -> bool {
        false
    }
}

/// The pty size a grid asks for.
fn pty_size(grid: (i32, i32)) -> PtySize {
    let side = |cells: i32| u16::try_from(cells.max(1)).unwrap_or(u16::MAX);
    PtySize::new(side(grid.0), side(grid.1))
}

/// The pixel size of a grid of cells, saturated to the settings' `u16`.
fn target_size(cell: (u16, u16), grid: (i32, i32)) -> (u16, u16) {
    let side = |cell: u16, cells: i32| {
        let pixels = u32::from(cell) * u32::try_from(cells.max(1)).unwrap_or(1);
        u16::try_from(pixels).unwrap_or(u16::MAX)
    };
    (side(cell.0, grid.0), side(cell.1, grid.1))
}

/// Starts a shell, falling back to `/bin/sh` when the file's command is gone.
fn spawn_shell(command: &PtyCommand, size: PtySize) -> Result<Session, String> {
    Session::spawn(command, size)
        .or_else(|_| Session::spawn(&PtyCommand::new("/bin/sh"), size))
        .map_err(|error| format!("cannot start {}: {error}", command.program().to_string_lossy()))
}

fn finite_scale(scale: f32) -> f32 {
    if scale.is_finite() && scale > 0.0 {
        scale.clamp(0.5, 8.0)
    } else {
        1.0
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn a_bad_scale_falls_back_to_one() {
        assert_eq!(finite_scale(0.0), 1.0);
        assert_eq!(finite_scale(f32::NAN), 1.0);
        assert_eq!(finite_scale(-2.0), 1.0);
        assert_eq!(finite_scale(2.0), 2.0);
    }

    #[test]
    fn a_cell_target_is_the_grid_in_pixels() {
        assert_eq!(target_size((8, 16), (80, 24)), (640, 384));
        assert_eq!(target_size((8, 16), (0, 0)), (8, 16));
    }

    #[test]
    fn a_theme_colour_names_its_channels() {
        let color = settings::Color::parse("#123456").expect("colour");
        assert_eq!(theme_color(color), Some((0x12, 0x34, 0x56)));
    }

    #[test]
    fn the_palette_offers_exactly_the_actions_this_shell_runs() {
        use settings::Action;

        // Every action in the vocabulary is performed except the palette's own
        // row, which would recurse, and `Unbound`, which never reaches a list.
        for action in Action::ALL {
            let expected = !matches!(action, Action::CommandPalette);
            assert_eq!(palette_supports(*action), expected, "{action:?}");
        }
        assert!(!palette_supports(Action::Unbound));
    }
}
