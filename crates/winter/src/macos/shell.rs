//! The window's state: tabs, the renderer, chords and the pointer rules.
//!
//! Nothing here names AppKit except the [`Frame`] the pixels live in; the view
//! translates events into these methods, and the controller turns the requests
//! this module raises (a profile menu, the settings window, closing the window)
//! into AppKit work. That keeps the rules the WinUI and egui shells share in the
//! same shape, and leaves a headless test able to exercise the geometry.

use std::path::{Path, PathBuf};
use std::rc::Rc;
use std::time::{Duration, Instant};

use renderer::renderer::Renderer;
use renderer::{
    AtlasEngine, CellSizeInDIP, Composition, CompositionRange, FontDueRasterizer, FontFace, FontInfo, RenderSettings,
    Settings, SharedBackend, SoftwareBackend,
};
use server::pty::{PtyCommand, PtySize};
use term::input::{Key, MouseButton, MouseButtons, MouseEvent, MouseEventKind, WHEEL_DELTA, WheelAxis};
use types::text_color::ColorType;
use types::{TextAttribute, UnderlineStyle};
use utils::Size;

use crate::find::FindBar;
use crate::input;
use crate::scrollbar::{self, Press, Scrollbar};
use crate::session::Session;
use crate::sound::Sound;
use crate::tabs::Tabs;
use crate::term_control::Control;
use crate::term_events::TerminalEvent;
use crate::term_settings::TerminalSettings;
use crate::terminal_render::TerminalData;
use crate::view::View as ViewState;

use super::chrome::{self, STRIP_HEIGHT, Strip, StripColors, StripHit};
use super::frame::Frame;

/// The grid the window opens with, before its bounds are known.
pub(crate) const INITIAL_GRID: (i32, i32) = (100, 28);
/// How often the pty is polled, matching the WinUI shell (ADR 006).
pub(crate) const TICK: Duration = Duration::from_millis(16);
/// The cursor's blink half-period, matching the egui shell.
const BLINK: Duration = Duration::from_millis(500);

/// One tab: its own terminal, viewport and find bar.
pub(crate) struct Tab {
    pub(crate) control: Control,
    pub(crate) view: ViewState,
    pub(crate) title: String,
    pub(crate) find: FindBar,
    /// The program the tab runs, so the strip can find the tab's icon.
    pub(crate) program: String,
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
    tabs: Tabs<Tab>,
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
    renderer: Renderer<TerminalData>,
    shared: SharedBackend,
    face: FontFace,
    font_size: f32,
    /// The cell in physical pixels.
    cell: (u16, u16),
    /// The backing scale the face was rasterised for.
    scale: f32,
    /// The view's bounds in points.
    size: (f64, f64),
    grid: (i32, i32),
    frame: Frame,
    sides: input::ModifierSides,
    blink_on: bool,
    blink_at: Instant,
    focused: bool,
    scrollbar: Scrollbar,
    scrollbar_press: bool,
    hover: Option<f64>,
    hovered_tab: Option<usize>,
    hovered_close: Option<usize>,
    /// The cell a forwarded move last reported, so a still pointer is silent.
    last_mouse_cell: Option<(i32, i32)>,
    /// The cell a press started on, so the first move starts a selection.
    press: Option<((i32, i32), bool)>,
    /// The tab a press grabbed, until the drag ends.
    tab_drag: Option<usize>,
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
    /// The window should close once the last tab is gone.
    close_requested: bool,
    /// The active tab changed, so the window title must follow it.
    title_dirty: bool,
    /// The input method's marked text, drawn at the cursor until it commits.
    composition: Composition,
    /// The DECPS tones every tab asked for, played on a thread of their own.
    sound: Sound,
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

        let size = PtySize::new(INITIAL_GRID.0 as u16, INITIAL_GRID.1 as u16);
        let session = spawn_shell(&template.command, size)?;
        let mut control = Control::new(Box::new(session), INITIAL_GRID);
        control.apply_scheme(template.scheme.as_ref());
        let (_background, _foreground) = control.default_colors();

        let fonts = font_setup(&face, font_size, scale, INITIAL_GRID)?;
        let cell = fonts.settings.font.cell_size;
        let (renderer, shared) = renderer_for(fonts, INITIAL_GRID);

        let Some(frame) = Frame::new(1, 1, mtm) else {
            return Err("cannot allocate the terminal's pixel buffer".to_string());
        };

        Ok(Self {
            tabs: Tabs::new(Tab {
                control,
                view: ViewState::new(),
                title: template.title.clone(),
                find: FindBar::default(),
                program: template.command.program().to_string_lossy().into_owned(),
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
            renderer,
            shared,
            face,
            font_size,
            cell,
            scale,
            size: (0.0, 0.0),
            grid: INITIAL_GRID,
            frame,
            sides: input::ModifierSides::default(),
            blink_on: true,
            blink_at: Instant::now(),
            focused: true,
            scrollbar: Scrollbar::default(),
            scrollbar_press: false,
            hover: None,
            hovered_tab: None,
            hovered_close: None,
            last_mouse_cell: None,
            press: None,
            tab_drag: None,
            pointer_cell: None,
            wheel_pixels: 0.0,
            menu_requested: false,
            settings_requested: false,
            find_focus: false,
            close_requested: false,
            title_dirty: false,
            composition: Composition::default(),
            sound: Sound::new(),
        })
    }

    /// The active tab's title, or the application's name.
    pub(crate) fn title(&self) -> &str {
        self.tabs.active().map_or("Winter", |tab| tab.title.as_str())
    }

    pub(crate) fn titles(&self) -> Vec<String> {
        (0..self.tabs.len())
            .filter_map(|index| self.tabs.get(index).map(|tab| tab.title.clone()))
            .collect()
    }

    /// The program each tab runs, in strip order.
    pub(crate) fn programs(&self) -> Vec<String> {
        (0..self.tabs.len())
            .filter_map(|index| self.tabs.get(index).map(|tab| tab.program.clone()))
            .collect()
    }

    pub(crate) fn active_index(&self) -> usize {
        self.tabs.index()
    }

    pub(crate) fn strip(&self) -> Strip {
        chrome::strip(self.size.0, self.tabs.len())
    }

    pub(crate) fn cell(&self) -> (u16, u16) {
        self.cell
    }

    pub(crate) fn scale(&self) -> f32 {
        self.scale
    }

    pub(crate) fn grid(&self) -> (i32, i32) {
        self.grid
    }

    pub(crate) fn frame(&self) -> &Frame {
        &self.frame
    }

    /// Shows or clears the input method's marked text.
    pub(crate) fn set_composition(&mut self, text: &str, selected_utf16: usize) {
        self.composition = composition(text, selected_utf16);
    }

    fn clear_composition(&mut self) {
        self.composition = Composition::default();
    }

    /// The caret's box in view points, for anchoring the candidate window.
    pub(crate) fn caret_rect(&self) -> (f64, f64, f64, f64) {
        let point = self.renderer.data().cursor_screen();
        let scale = f64::from(finite_scale(self.scale));
        let width = f64::from(self.cell.0) / scale;
        let height = f64::from(self.cell.1) / scale;
        (
            chrome::PANE_LEFT + f64::from(point.x()) * width,
            STRIP_HEIGHT + f64::from(point.y()) * height,
            width.max(1.0),
            height.max(1.0),
        )
    }

    pub(crate) fn set_focused(&mut self, focused: bool) {
        self.focused = focused;
    }

    /// The visible screen as plain text, for the accessibility client.
    pub(crate) fn accessible_text(&self) -> String {
        self.renderer.data().visible_text()
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

    /// The active pane's background and foreground, as `COLORREF`s.
    pub(crate) fn active_colors(&self) -> (u32, u32) {
        self.tabs
            .active()
            .map_or((0x000C_0C0C, 0x00CC_CCCC), |tab| tab.control.default_colors())
    }

    /// Whether AppKit's dark appearance reads best on the active pane.
    pub(crate) fn prefers_dark(&self) -> bool {
        let (background, _) = self.active_colors();
        chrome::dark_background(background)
    }

    /// The scrollbar's scroll, document height and visible rows.
    pub(crate) fn scroll_facts(&self) -> (i32, i32, i32) {
        let Some(tab) = self.tabs.active() else {
            return (0, 0, 0);
        };
        let document = self.renderer.data().document_height();
        (tab.view.scroll(), document, tab.control.size().1)
    }

    pub(crate) fn marks(&self) -> &[crate::grid::MarkTick] {
        self.renderer.data().marks()
    }

    /// The bell's fade at `now`, while the active pane flashes.
    pub(crate) fn bell_alpha(&self, now: Instant) -> Option<f32> {
        self.tabs.active().and_then(|tab| tab.control.bell_alpha(now))
    }

    /// The find bar's query, current match and match count.
    pub(crate) fn find_state(&self) -> (bool, String, usize, usize) {
        let Some(tab) = self.tabs.active() else {
            return (false, String::new(), 0, 0);
        };
        let current = tab.find.current.map_or(0, |index| index + 1);
        (tab.find.open, tab.find.query.clone(), current, tab.find.matches.len())
    }

    pub(crate) fn find_open(&self) -> bool {
        self.tabs.active().is_some_and(|tab| tab.find.open)
    }

    /// Fits the grid and the face to a new view size.
    pub(crate) fn resize(&mut self, size: (f64, f64), scale: f32) {
        let scale = finite_scale(scale);
        self.size = size;
        let rescaled = (scale - self.scale).abs() > f32::EPSILON;
        if rescaled {
            self.scale = scale;
            self.rebuild_face();
        }

        let bounds = objc2_foundation::NSRect::new(
            objc2_foundation::NSPoint::new(0.0, 0.0),
            objc2_foundation::NSSize::new(size.0, size.1),
        );
        let grid = chrome::grid_for(bounds, f64::from(scale), self.cell);
        if rescaled || grid != self.grid {
            self.apply_grid(grid);
        }
    }

    /// Replaces the renderer with one whose face is `scale` times the DIP size.
    fn rebuild_face(&mut self) {
        let Ok(fonts) = font_setup(&self.face, self.font_size, self.scale, self.grid) else {
            return;
        };
        self.cell = fonts.settings.font.cell_size;
        let (renderer, shared) = renderer_for(fonts, self.grid);
        self.renderer = renderer;
        self.shared = shared;
    }

    /// Tells every tab and the renderer how large the grid is.
    fn apply_grid(&mut self, grid: (i32, i32)) {
        self.grid = grid;
        for index in 0..self.tabs.len() {
            if let Some(tab) = self.tabs.get_mut(index) {
                tab.control.resize(grid.0, grid.1);
            }
        }

        let target = target_size(self.cell, grid);
        let _ = self.renderer.resize(target, chrome::viewport(grid));
    }

    /// Pumps every pty, repaints the active tab and re-arms nothing: the
    /// controller's timer calls this again.
    pub(crate) fn tick(&mut self, now: Instant, mtm: objc2::MainThreadMarker) -> Tick {
        let mut tick = Tick::default();
        let mut exited = Vec::new();
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
            tab.control.pump();
            notes.extend(tab.control.take_sounds());
            for event in tab.control.take_events(now) {
                match event {
                    TerminalEvent::Exited => exited.push(index),
                    TerminalEvent::Bell => tick.bell = true,
                    TerminalEvent::Notification { title, body } => tick.notifications.push((title, body)),
                }
            }
            if let Some(title) = tab.control.title().map(str::to_string)
                && tab.title != title
            {
                tab.title = title;
                title_changed = true;
            }
        }

        if !notes.is_empty() {
            self.sound.play(audio::midi::tones(&notes));
        }

        for index in exited.into_iter().rev() {
            if !self.tabs.close(index) {
                self.close_requested = true;
                tick.close_window = true;
                return tick;
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

        if now.duration_since(self.settings_at) >= Duration::from_secs(1) {
            self.settings_at = now;
            self.poll_settings();
        }

        self.paint(now, mtm);
        tick.redraw = true;
        tick
    }

    /// Refreshes the renderer's copy and paints one frame into the frame buffer.
    fn paint(&mut self, _now: Instant, mtm: objc2::MainThreadMarker) {
        let blink_on = self.blink_on;
        let painted = {
            let Shell {
                renderer,
                tabs,
                composition,
                ..
            } = self;
            let Some(tab) = tabs.active_mut() else {
                return;
            };
            renderer
                .data_mut()
                .refresh(&mut tab.control, &tab.view, blink_on, Some(&tab.find));

            let (background, foreground) = tab.control.default_colors();
            let _ = renderer.set_default_colors(background, foreground);
            if let Ok(table) = tab.control.color_table().lock() {
                renderer.settings_mut().set_color_table(table.clone());
            }
            renderer.data_mut().set_composition(composition.clone());
            renderer.invalidate_all();
            renderer.paint_frame().is_ok()
        };

        if !painted {
            return;
        }

        let width = self.shared.width();
        let height = self.shared.height();
        let frame = &mut self.frame;
        self.shared.with_pixels(|pixels| {
            frame.copy(pixels, width, height, mtm);
        });
    }

    /// Reloads the settings file when a save changed its stamp.
    fn poll_settings(&mut self) {
        let Some(path) = &self.settings_path else {
            return;
        };
        if settings::stamp(path) == self.settings_stamp {
            return;
        }
        self.reload_settings();
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
            let bounds = objc2_foundation::NSRect::new(
                objc2_foundation::NSPoint::new(0.0, 0.0),
                objc2_foundation::NSSize::new(self.size.0, self.size.1),
            );
            self.apply_grid(chrome::grid_for(bounds, f64::from(self.scale), self.cell));
        }

        let scheme = self.template.scheme.clone();
        for index in 0..self.tabs.len() {
            if let Some(tab) = self.tabs.get_mut(index) {
                tab.control.apply_scheme(scheme.as_ref());
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
        self.clear_composition();
        let size = pty_size(self.grid);
        let Ok(session) = spawn_shell(&terminal.command, size) else {
            return;
        };
        let mut control = Control::new(Box::new(session), self.grid);
        control.apply_scheme(terminal.scheme.as_ref());

        self.tabs.add(Tab {
            control,
            view: ViewState::new(),
            title: terminal.title,
            find: FindBar::default(),
            program: terminal.command.program().to_string_lossy().into_owned(),
        });
        self.title_dirty = true;
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
    }

    /// Closes a tab by its position in the strip.
    pub(crate) fn close_tab(&mut self, index: usize) {
        self.clear_composition();
        if !self.tabs.close(index) {
            self.close_requested = true;
        }
        self.title_dirty = true;
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
        if modifiers.command {
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
        match key {
            Key::Char('c') if !shift => {
                self.copy_selection();
            }
            Key::Char('v') if !shift => self.paste_clipboard(),
            Key::Char('t') if !shift => self.open_tab(),
            Key::Char('w') => self.close_active_tab(),
            Key::Char('f') if !shift => self.toggle_find(),
            Key::Char(',') => self.settings_requested = true,
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
            _ => {}
        }
        true
    }

    /// Sends a portable event through the shared translation rules.
    fn send_input(&mut self, event: &input::Event) {
        let win32 = self
            .tabs
            .active()
            .is_some_and(|tab| tab.control.input_modes().win32_input);
        let outcome = input::translate(event, &mut self.sides, win32);
        let Some(tab) = self.tabs.active_mut() else {
            return;
        };
        if outcome.activity {
            tab.view.scroll_to_bottom();
        }
        if let Some(text) = outcome.paste {
            tab.control.paste(&text);
        }
        for key in outcome.keys {
            tab.control.send_key(&key);
        }
    }

    /// Puts the selection on the clipboard, clearing it like upstream.
    pub(crate) fn copy_selection(&mut self) -> bool {
        let Some(tab) = self.tabs.active_mut() else {
            return false;
        };
        let Some(selection) = tab.view.selection().copied() else {
            return false;
        };
        let Some(text) = tab.control.selection_text(&selection) else {
            return false;
        };

        let copied = arboard::Clipboard::new()
            .and_then(|mut clipboard| clipboard.set_text(text))
            .is_ok();
        if copied {
            tab.view.clear_selection();
        }
        copied
    }

    /// Sends the clipboard through the terminal's bracketed-paste filter.
    pub(crate) fn paste_clipboard(&mut self) {
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

    /// Shows or hides the active tab's find bar.
    pub(crate) fn toggle_find(&mut self) {
        let Some(tab) = self.tabs.active_mut() else {
            return;
        };
        tab.find.toggle();
        if tab.find.open {
            tab.find.refresh(&mut tab.control);
            tab.find.scroll_to(&tab.control, &mut tab.view);
            self.find_focus = true;
            // The field takes the keyboard, so a half-composed terminal
            // preview is no longer where the user is typing.
            self.clear_composition();
        }
    }

    /// Sets the find query and recomputes the matches.
    pub(crate) fn find_query(&mut self, query: String) {
        let Some(tab) = self.tabs.active_mut() else {
            return;
        };
        tab.find.query = query;
        tab.find.refresh(&mut tab.control);
        tab.find.scroll_to(&tab.control, &mut tab.view);
    }

    /// Moves to the next (true) or previous match.
    pub(crate) fn find_step(&mut self, forward: bool) {
        let Some(tab) = self.tabs.active_mut() else {
            return;
        };
        if tab.find.searched != tab.find.query {
            tab.find.refresh(&mut tab.control);
        }
        tab.find.step(forward);
        tab.find.scroll_to(&tab.control, &mut tab.view);
    }

    /// Closes the find bar.
    pub(crate) fn close_find(&mut self) {
        if let Some(tab) = self.tabs.active_mut() {
            tab.find.hide();
        }
    }

    fn active_tracking(&self) -> bool {
        self.tabs.active().is_some_and(|tab| tab.control.is_tracking_mouse())
    }

    fn scroll_active(&mut self, rows: i32) {
        let Some(tab) = self.tabs.active_mut() else {
            return;
        };
        let max = tab.control.max_scroll();
        tab.view.scroll_by(rows, max);
    }

    fn scroll_page(&mut self, direction: i32) {
        let Some(tab) = self.tabs.active_mut() else {
            return;
        };
        let rows = (tab.control.size().1 - 1).max(1) * direction.signum();
        let max = tab.control.max_scroll();
        tab.view.scroll_by(rows, max);
    }

    fn scroll_to(&mut self, bottom: bool) {
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

    /// The cell a point in the view names, below the strip and right of its
    /// left inset.
    pub(crate) fn cell_at(&self, point: (f64, f64)) -> Option<(i32, i32)> {
        let scale = f64::from(finite_scale(self.scale));
        let width = f64::from(self.cell.0) / scale;
        let height = f64::from(self.cell.1) / scale;
        if width <= 0.0 || height <= 0.0 {
            return None;
        }
        let column = ((point.0 - chrome::PANE_LEFT) / width).floor() as i32;
        let row = ((point.1 - STRIP_HEIGHT) / height).floor() as i32;
        let (columns, rows) = self.grid;
        (column >= 0 && row >= 0 && column < columns && row < rows).then_some((column, row))
    }

    /// The cell a point names, or the nearest in-range cell.
    fn clamped_cell(&self, point: (f64, f64)) -> Option<(i32, i32)> {
        let scale = f64::from(finite_scale(self.scale));
        let width = f64::from(self.cell.0) / scale;
        let height = f64::from(self.cell.1) / scale;
        if width <= 0.0 || height <= 0.0 {
            return None;
        }
        let column = ((point.0 - chrome::PANE_LEFT) / width).floor() as i32;
        let row = ((point.1 - STRIP_HEIGHT) / height).floor() as i32;
        Some((
            column.clamp(0, self.grid.0.max(1) - 1),
            row.clamp(0, self.grid.1.max(1) - 1),
        ))
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
        if point.1 < STRIP_HEIGHT {
            if button != MouseButton::Left {
                return true;
            }
            return self.strip_pressed(point);
        }

        if self.find_open() {
            self.close_find();
        }

        if button == MouseButton::Left
            && point.0 >= self.size.0 - f64::from(scrollbar::WIDTH)
            && self.scrollbar_pressed(point.1)
        {
            return true;
        }

        let forward = input::forwards_to_application(self.active_tracking(), modifiers.shift);
        let cell = self.cell_at(point);
        self.pointer_cell = cell;

        if button != MouseButton::Left && !forward {
            self.paste_clipboard();
            return true;
        }

        let Some(tab) = self.tabs.active_mut() else {
            return true;
        };

        if forward {
            if let Some(position) = cell {
                let chord = input::key_modifiers(modifiers, self.sides.right_alt, self.sides.right_ctrl);
                tab.control.send_mouse(&MouseEvent {
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
            tab.view.clear_selection();
            self.press = Some((position, modifiers.alt));
        }
        true
    }

    /// Handles a pointer move while `buttons` are held.
    pub(crate) fn pointer_moved(&mut self, point: (f64, f64), buttons: MouseButtons, modifiers: input::Modifiers) {
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

        if self.scrollbar_press {
            if buttons.left {
                self.scrollbar_dragged(point.1);
            } else {
                self.scrollbar_press = false;
                self.scrollbar.release();
            }
            return;
        }

        self.hover(point);
        if point.1 < STRIP_HEIGHT {
            return;
        }

        let (columns, rows) = self.grid;
        let cell = self.cell_at(point);
        self.pointer_cell = cell;
        let forward = input::forwards_to_application(self.active_tracking(), modifiers.shift);

        let scale = f64::from(finite_scale(self.scale));
        let width = f64::from(self.cell.0) / scale;
        let height = f64::from(self.cell.1) / scale;
        let outside = if point.1 < STRIP_HEIGHT {
            -1
        } else if height > 0.0 && point.1 >= STRIP_HEIGHT + f64::from(rows.max(0)) * height {
            1
        } else {
            0
        };
        let column = if width > 0.0 {
            ((point.0 - chrome::PANE_LEFT) / width).floor() as i32
        } else {
            0
        };

        let Some(tab) = self.tabs.active_mut() else {
            return;
        };

        if forward {
            if let Some(position) = cell {
                let dragging = buttons.left || buttons.middle || buttons.right;
                if self.last_mouse_cell != Some(position) || dragging {
                    let chord = input::key_modifiers(modifiers, self.sides.right_alt, self.sides.right_ctrl);
                    tab.control.send_mouse(&MouseEvent {
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

        let position = if outside != 0 {
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
            Some((anchor, block)) if position != anchor => {
                tab.view.select(anchor, block);
                tab.view.extend(position);
            }
            Some(press) => self.press = Some(press),
            None => tab.view.extend(position),
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
        if self.tab_drag.take().is_some() {
            return;
        }

        if self.scrollbar_press {
            self.scrollbar_press = false;
            self.scrollbar.release();
            self.press = None;
            self.last_mouse_cell = None;
            return;
        }

        if !input::forwards_to_application(self.active_tracking(), modifiers.shift) {
            self.last_mouse_cell = None;
            self.press = None;
            return;
        }

        let cell = self.cell_at(point);
        let Some(tab) = self.tabs.active_mut() else {
            return;
        };
        if let Some(position) = cell {
            let chord = input::key_modifiers(modifiers, self.sides.right_alt, self.sides.right_ctrl);
            tab.control.send_mouse(&MouseEvent {
                position,
                kind: MouseEventKind::Release(button),
                buttons,
                modifiers: chord,
                delta: 0,
            });
        }
        self.last_mouse_cell = None;
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

    /// Turns a wheel or trackpad scroll into history rows or a mouse report.
    pub(crate) fn wheel(&mut self, point: (f64, f64), delta: f64, precise: bool, shift: bool) {
        if delta == 0.0 {
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

        self.pointer_cell = self.clamped_cell(point);
        let Some(tab) = self.tabs.active_mut() else {
            return;
        };
        if input::forwards_to_application(tab.control.is_tracking_mouse(), shift)
            && let Some(position) = self.pointer_cell
            && tab
                .control
                .send_mouse(&MouseEvent::wheel(position, WheelAxis::Vertical, rows * WHEEL_DELTA))
        {
            return;
        }

        let max = tab.control.max_scroll();
        tab.view.scroll_by(rows, max);
    }

    /// The scrollbar's press, if `y` is on its strip.
    fn scrollbar_pressed(&mut self, y: f64) -> bool {
        let track = self.size.1 - STRIP_HEIGHT;
        if track <= 0.0 {
            return false;
        }
        let Some(tab) = self.tabs.active_mut() else {
            return false;
        };
        let (document, marks) = tab.control.scroll_facts();
        let visible = tab.control.size().1;
        if document <= visible {
            self.scrollbar.release();
            return false;
        }

        let fraction = ((y - STRIP_HEIGHT) / track).clamp(0.0, 1.0) as f32;
        let positions: Vec<f32> = marks
            .iter()
            .map(|mark| scrollbar::position(mark.row, document))
            .collect();
        if let Some(index) = scrollbar::tick_at(&positions, fraction, scrollbar::MARK_GRAB / track as f32) {
            tab.view.scroll_to_row(marks[index].row, visible, document);
        } else if let Press::Jump(scroll) = self.scrollbar.press(fraction, tab.view.scroll(), document, visible) {
            tab.view.set_scroll(scroll, document - visible);
        }

        self.scrollbar_press = true;
        true
    }

    fn scrollbar_dragged(&mut self, y: f64) {
        let track = self.size.1 - STRIP_HEIGHT;
        if track <= 0.0 {
            return;
        }
        let Some(tab) = self.tabs.active_mut() else {
            return;
        };
        let visible = tab.control.size().1;
        let document = tab.control.max_scroll() + visible;
        let fraction = ((y - STRIP_HEIGHT) / track).clamp(0.0, 1.0) as f32;
        if let Some(scroll) = self.scrollbar.drag(fraction, document, visible) {
            tab.view.set_scroll(scroll, document - visible);
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

fn renderer_for(fonts: Fonts, grid: (i32, i32)) -> (Renderer<TerminalData>, SharedBackend) {
    let Fonts {
        font,
        settings,
        shaper,
        rasterizer,
    } = fonts;

    let shared = SharedBackend::new(SoftwareBackend::with_rasterizer(Box::new(rasterizer)));
    let mut engine = AtlasEngine::with_settings(settings, Box::new(shared.clone()));
    engine.set_shaping(Box::new(shaper));

    let mut renderer = Renderer::new(
        TerminalData::new(font, RenderSettings::new(), grid.0, grid.1),
        RenderSettings::new(),
    );
    renderer.add_engine(Box::new(engine));
    (renderer, shared)
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

/// Builds the renderer's IME preview from marked text and AppKit's selection.
///
/// AppKit reports the selection in UTF-16 units; the renderer's caret counts
/// characters, so the range is converted here. One underline run covers the
/// whole preview, which is how a terminal marks uncommitted text.
pub(crate) fn composition(text: &str, selected_utf16: usize) -> Composition {
    let mut attr = TextAttribute::default();
    attr.set_underline_style(UnderlineStyle::SinglyUnderlined);
    let len = text.chars().count();
    Composition {
        text: text.to_string(),
        attributes: if len == 0 {
            Vec::new()
        } else {
            vec![CompositionRange { len, attr }]
        },
        cursor_pos: char_index(text, selected_utf16),
    }
}

/// The character index `units` UTF-16 units into `text`, clamped to its end.
///
/// A selection in the middle of a surrogate pair names the character that
/// contains it, which is the only cell the caret can sit on.
fn char_index(text: &str, units: usize) -> usize {
    let mut seen = 0;
    for (index, ch) in text.chars().enumerate() {
        if seen >= units {
            return index;
        }
        seen += ch.len_utf16();
    }
    text.chars().count()
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn a_marked_selection_counts_characters_not_utf16_units() {
        assert_eq!(composition("ni", 0).cursor_pos, 0);
        assert_eq!(composition("ni", 1).cursor_pos, 1);
        // A BMP CJK character is one UTF-16 unit.
        assert_eq!(composition("日本語", 3).cursor_pos, 3);
        assert_eq!(composition("日本語", 1).cursor_pos, 1);
        // An emoji is two UTF-16 units but one character, even mid-pair.
        assert_eq!(composition("😀", 2).cursor_pos, 1);
        assert_eq!(composition("😀", 1).cursor_pos, 1);
        assert_eq!(composition("😀日", 3).cursor_pos, 2);
        // An out-of-range selection clamps to the end.
        assert_eq!(composition("ni", 9).cursor_pos, 2);
    }

    #[test]
    fn the_marked_text_is_one_underlined_run() {
        let preview = composition("abc", 0);
        assert_eq!(preview.text, "abc");
        assert_eq!(preview.attributes.len(), 1);
        assert_eq!(preview.attributes[0].len, 3);
        assert_eq!(
            preview.attributes[0].attr.get_underline_style(),
            UnderlineStyle::SinglyUnderlined
        );
        assert!(composition("", 0).attributes.is_empty());
    }

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
}
