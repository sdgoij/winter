//! The window, its tabs and the shell's chords, from `App.cpp`, `AppHost.cpp`
//! and `Palette.cpp`.
//!
//! This is the application: it owns the tabs, drains what every terminal
//! reports, and turns a chord into a window action. The widget over one
//! terminal is `terminal_widget.rs`, the settings window is `config.rs`, and a
//! platform command is `window.rs` (ADR 038).

use std::io;
use std::path::{Path, PathBuf};
use std::time::{Duration, Instant};

use eframe::egui;
use settings::Settings;

use crate::config::Draft;
use crate::input::ModifierSides;
use crate::layout::{self, Axis, Direction, Fraction, Layout, PaneId};
use crate::term_events::TerminalEvent;
use crate::term_settings::TerminalSettings;
use crate::terminal_widget::{GridPaint, Pane, contrasting_text, egui_color};
use crate::{blink::Blink, grid, notifications, sound::Sound, tabs::Tabs, window};

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
pub(crate) struct Faces {
    regular: bool,
    italic: bool,
}

impl Faces {
    /// The font for a style, falling back to what is available.
    pub(crate) fn font(&self, size: f32, italic: bool) -> egui::FontId {
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
            .with_title("winterm")
            .with_icon(crate::icon::data()),
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

/// A divider the pointer is dragging.
struct DividerDrag {
    path: layout::SplitPath,
    axis: Axis,
    /// The split's area when the drag began, for turning pixels into a ratio.
    rect: egui::Rect,
}

/// One tab: a pane tree, its focused pane and a title.
pub(crate) struct Tab {
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
    fn new(ctx: &egui::Context, settings: TerminalSettings) -> io::Result<Self> {
        let id = PaneId(0);
        Ok(Self {
            panes: vec![Pane::new(ctx, id, &settings)?],
            layout: Layout::leaf(id),
            focused: id,
            divider: None,
            fallback_title: settings.title,
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
            .and_then(|pane| pane.control.title())
            .unwrap_or(&self.fallback_title)
    }

    /// Splits the focused pane, spawning a shell for the new half.
    fn split(&mut self, axis: Axis, ctx: &egui::Context, settings: &TerminalSettings) -> bool {
        let id = PaneId(self.next_id);
        let Ok(pane) = Pane::new(ctx, id, settings) else {
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

    fn take_events(&mut self, now: Instant) -> Vec<TerminalEvent> {
        self.panes.iter_mut().flat_map(|pane| pane.take_events(now)).collect()
    }

    /// Takes the DECPS notes every pane was asked to play.
    fn take_sounds(&mut self) -> Vec<audio::Note> {
        self.panes.iter_mut().flat_map(|pane| pane.take_sounds()).collect()
    }

    /// When a pane's bell flash needs its next frame.
    fn bell_repaint(&self, now: Instant) -> Option<Duration> {
        self.panes
            .iter()
            .filter_map(|pane| pane.control.bell_repaint(now))
            .min()
    }

    fn handle_events(&mut self, events: Vec<egui::Event>, sides: &mut ModifierSides) {
        if let Some(pane) = self.focused_pane_mut() {
            pane.handle_events(events, sides);
        }
    }

    /// Shows or hides the focused pane's find bar.
    fn toggle_find(&mut self) {
        if let Some(pane) = self.focused_pane_mut() {
            pane.toggle_find();
        }
    }

    /// Draws the focused pane's find bar.
    fn find_bar(&mut self, ctx: &egui::Context) {
        if let Some(pane) = self.focused_pane_mut() {
            pane.find_bar(ctx);
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
    pub(crate) fn apply_scheme(&mut self, scheme: Option<&settings::Scheme>) {
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

    fn paint(&mut self, ui: &egui::Ui, area: egui::Rect, cursor_on: bool, paint: GridPaint) {
        let focused = self.focused;
        let multiple = self.panes.len() > 1;

        for (id, fraction) in self.layout.rects() {
            let rect = fraction_rect(fraction, area);
            if let Some(pane) = self.panes.iter_mut().find(|pane| pane.id == id) {
                pane.paint(ui, rect, cursor_on && id == focused, paint);
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

/// The window: a tab strip and the active tab.
pub(crate) struct App {
    pub(crate) tabs: Tabs<Tab>,
    pub(crate) settings: Settings,
    /// The faces egui renders the grid with.
    pub(crate) faces: Faces,
    /// The scheme every pane starts with, if the file names one.
    pub(crate) scheme: Option<settings::Scheme>,
    /// The grid's point size, from the settings file.
    pub(crate) font_size: f32,
    /// The path the settings were read from, when the platform has one.
    pub(crate) settings_path: Option<PathBuf>,
    /// The earlier file these settings were migrated from, if they were.
    pub(crate) settings_migrated_from: Option<PathBuf>,
    /// Why the file was not used, if it was not.
    pub(crate) settings_error: Option<String>,
    /// What normalization had to decide for the file, in the window's words.
    pub(crate) settings_warnings: Vec<String>,
    /// The editable copy the settings window shows, while it is open.
    pub(crate) draft: Option<Draft>,
    /// The file changed on disk while the draft had unsaved edits.
    pub(crate) settings_conflict: bool,
    /// Whether the settings window is showing.
    pub(crate) settings_open: bool,
    /// The settings file's stamp when it was last read, so a save is noticed.
    pub(crate) settings_stamp: Option<settings::Stamp>,
    /// The sides egui collapses but the encoder distinguishes.
    pub(crate) sides: ModifierSides,
    pub(crate) clipboard: Option<arboard::Clipboard>,
    pub(crate) blink: Blink,
    /// Whether the cursor is drawn this frame.
    pub(crate) cursor_on: bool,
    /// The command palette's state.
    pub(crate) palette: Palette,
    /// The tones the emulation asked to play, and the thread playing them.
    pub(crate) sound: Sound,
    /// The title last sent to the window.
    pub(crate) window_title: Option<String>,
}

/// The command palette's state between frames.
#[derive(Debug, Default)]

pub(crate) struct Palette {
    /// Whether the palette is showing.
    open: bool,
    /// What the user has typed to narrow the list.
    filter: String,
    /// The entry the arrows have moved to, within the filtered list.
    selected: usize,
    /// Whether the filter field still needs the keyboard focus.
    focus: bool,
}

impl Palette {
    /// Opens the palette with an empty query and the first entry selected.
    fn show(&mut self) {
        self.open = true;
        self.filter.clear();
        self.selected = 0;
        self.focus = true;
    }

    /// Closes the palette and forgets the query.
    fn hide(&mut self) {
        self.open = false;
        self.filter.clear();
        self.selected = 0;
        self.focus = false;
    }

    fn toggle(&mut self) {
        if self.open { self.hide() } else { self.show() }
    }
}

/// What the tab strip or a chord asked for.
#[derive(Debug, Clone, PartialEq, Eq)]
enum TabAction {
    Select(usize),
    Close(usize),
    CloseActive,
    New,
    /// Open the named profile in a new tab.
    NewProfile(String),
    Next,
    Previous,
    Copy,
    Paste,
    Split(Axis),
    Focus(Direction),
    Resize(Direction),
    ToggleSettings,
    TogglePalette,
    Find,
}

impl App {
    fn new(ctx: &egui::Context, faces: Faces) -> Result<Self, Box<dyn std::error::Error + Send + Sync>> {
        // A fresh installation gets a file to edit, as upstream writes one
        // (ADR 034).
        settings::ensure_file();
        let loaded = settings::load_with_report();
        let settings_stamp = loaded.path.as_deref().and_then(settings::stamp);
        let settings_warnings = loaded.warnings.iter().map(ToString::to_string).collect();
        let settings = loaded.settings;
        let scheme = settings.scheme().cloned();
        let font_size = settings.font.size;
        let tab = Tab::new(ctx, TerminalSettings::resolve(&settings))?;

        Ok(Self {
            tabs: Tabs::new(tab),
            settings,
            faces,
            scheme,
            font_size,
            settings_path: loaded.path,
            settings_migrated_from: loaded.migrated_from,
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
            palette: Palette::default(),
            sound: Sound::new(),
            window_title: None,
        })
    }

    /// Opens another shell in a new tab: the default profile's.
    fn open_tab(&mut self, ctx: &egui::Context) {
        let settings = TerminalSettings::resolve(&self.settings);
        self.add_tab(ctx, settings);
    }

    /// Opens the named profile's shell in a new tab.
    ///
    /// The name came from the tab strip's menu, so it is only as good as the
    /// settings that drew it: a name they no longer hold starts nothing rather
    /// than some other profile.
    fn open_profile(&mut self, ctx: &egui::Context, name: &str) {
        let Some(profile) = self.settings.profile_named(name) else {
            return;
        };
        let settings = TerminalSettings::for_profile(&self.settings, Some(profile));
        self.add_tab(ctx, settings);
    }

    fn add_tab(&mut self, ctx: &egui::Context, settings: TerminalSettings) {
        if let Ok(tab) = Tab::new(ctx, settings) {
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
            TabAction::NewProfile(name) => self.open_profile(ctx, &name),
            TabAction::Next => self.tabs.step(1),
            TabAction::Previous => self.tabs.step(-1),
            TabAction::Split(axis) => {
                let settings = TerminalSettings::resolve(&self.settings);
                if let Some(tab) = self.tabs.active_mut() {
                    tab.split(axis, ctx, &settings);
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
            TabAction::TogglePalette => self.palette.toggle(),
            TabAction::Find => {
                if let Some(tab) = self.tabs.active_mut() {
                    tab.toggle_find();
                }
            }
        }
    }
}

impl App {
    fn close_tab(&mut self, index: usize, ctx: &egui::Context) {
        if !self.tabs.close(index) {
            window::close(ctx);
        }
    }

    /// Draws the command palette and returns the command chosen this frame.
    fn palette_window(&mut self, ctx: &egui::Context) -> Option<TabAction> {
        if !self.palette.open {
            return None;
        }

        let (up, down, enter, escape) = ctx.input(|input| {
            (
                input.key_pressed(egui::Key::ArrowUp),
                input.key_pressed(egui::Key::ArrowDown),
                input.key_pressed(egui::Key::Enter),
                input.key_pressed(egui::Key::Escape),
            )
        });
        if escape {
            self.palette.hide();
            return None;
        }

        let mut chosen = None;
        egui::Window::new("Command palette")
            .anchor(egui::Align2::CENTER_TOP, egui::vec2(0.0, 64.0))
            .collapsible(false)
            .resizable(false)
            .show(ctx, |ui| {
                ui.set_min_width(380.0);
                let field = ui.add(egui::TextEdit::singleline(&mut self.palette.filter).hint_text("Type a command"));
                if self.palette.focus {
                    field.request_focus();
                    self.palette.focus = false;
                }

                let matching: Vec<PaletteEntry> = palette_entries(&self.settings)
                    .into_iter()
                    .filter(|entry| matches_filter(&entry.label, &self.palette.filter))
                    .collect();
                if matching.is_empty() {
                    self.palette.selected = 0;
                    ui.label(egui::RichText::new("No matching command").weak());
                    return;
                }

                if up {
                    self.palette.selected = (self.palette.selected + matching.len() - 1) % matching.len();
                }
                if down {
                    self.palette.selected = (self.palette.selected + 1) % matching.len();
                }
                self.palette.selected = self.palette.selected.min(matching.len() - 1);

                if enter {
                    chosen = Some(matching[self.palette.selected].action.clone());
                }

                egui::ScrollArea::vertical().max_height(280.0).show(ui, |ui| {
                    for (index, entry) in matching.iter().enumerate() {
                        ui.horizontal(|ui| {
                            if ui
                                .selectable_label(index == self.palette.selected, entry.label.as_str())
                                .clicked()
                            {
                                chosen = Some(entry.action.clone());
                            }
                            if let Some(chord) = &entry.chord {
                                ui.label(egui::RichText::new(chord).weak());
                            }
                        });
                    }
                });
            });

        if chosen.is_some() {
            self.palette.hide();
        }
        chosen
    }
}

impl App {
    /// Draws the tab strip and returns what the user clicked.
    fn tab_bar(&self, ui: &mut egui::Ui) -> Vec<TabAction> {
        let mut actions = Vec::new();
        let active = self.tabs.index();
        let theme = self.settings.theme();
        let focused = ui.ctx().input(|input| input.focused);

        let mut panel = egui::Panel::top("winterm-tabs");
        if let Some(fill) = theme
            .and_then(|theme| theme.tab_row_background(focused))
            .and_then(|color| egui_color(color.text_color()))
        {
            panel = panel.frame(egui::Frame::new().fill(fill));
        }

        panel.show(ui, |ui| {
            ui.horizontal(|ui| {
                for index in 0..self.tabs.len() {
                    let Some(tab) = self.tabs.get(index) else {
                        continue;
                    };

                    let fill = theme
                        .and_then(|theme| theme.tab_background(index == active))
                        .and_then(|color| egui_color(color.text_color()));
                    let button = match fill {
                        Some(fill) => egui::Button::selectable(
                            index == active,
                            egui::RichText::new(tab.display_title()).color(contrasting_text(fill)),
                        )
                        .fill(fill),
                        None => egui::Button::selectable(index == active, tab.display_title()),
                    };
                    if ui.add(button).clicked() {
                        actions.push(TabAction::Select(index));
                    }
                    if ui.small_button("×").clicked() {
                        actions.push(TabAction::Close(index));
                    }
                }

                ui.menu_button("+", |ui| {
                    for profile in &self.settings.profiles {
                        if ui
                            .button(profile.name.as_str())
                            .on_hover_text(profile.command.as_str())
                            .clicked()
                        {
                            actions.push(TabAction::NewProfile(profile.name.clone()));
                            ui.close();
                        }
                    }
                });

                if ui.button("⚙").on_hover_text("Settings").clicked() {
                    actions.push(TabAction::ToggleSettings);
                }
            });
        });

        actions
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
                window::set_title(ctx, &title);
            }
        }

        let now = Instant::now();
        let mut events = Vec::new();
        for index in 0..self.tabs.len() {
            if let Some(tab) = self.tabs.get_mut(index) {
                events.extend(tab.take_events(now).into_iter().map(|event| (index, event)));
            }
        }

        // A shell that exited closes its tab, and the last tab closes the
        // window: upstream's `closeOnExit`. The tabs close from the back, so an
        // index still names the tab it was taken from.
        let mut exited: Vec<usize> = events
            .iter()
            .filter(|(_, event)| *event == TerminalEvent::Exited)
            .map(|(index, _)| *index)
            .collect();
        exited.sort_unstable();
        exited.dedup();
        for index in exited.into_iter().rev() {
            self.close_tab(index, ctx);
        }

        for (_, event) in events {
            match event {
                // One tone and one flash per bell event: the control already
                // ignores a second bell while the light is fading.
                TerminalEvent::Bell => {
                    window::request_attention(ctx);
                    self.sound.play(vec![audio::beep::tone()]);
                    ctx.request_repaint();
                }
                TerminalEvent::Notification { title, body } => notifications::show(&title, &body),
                TerminalEvent::Exited => {}
            }
        }

        let mut sounds = Vec::new();
        for index in 0..self.tabs.len() {
            if let Some(tab) = self.tabs.get_mut(index) {
                sounds.extend(tab.take_sounds());
            }
        }
        if !sounds.is_empty() {
            self.sound.play(audio::midi::tones(&sounds));
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
        // egui owns the keyboard while a window's text field has focus, and the
        // palette owns it while it is open. Forwarding as well would type the
        // same characters into the shell.
        if shell_takes_keys(self.palette.open, ctx.egui_wants_keyboard_input()) {
            tab.handle_events(forwarded, &mut self.sides);
        }

        let screen = tab.focused_snapshot();
        let now = Instant::now();
        self.cursor_on = self.blink.update(screen.cursor_blinking, focused, now);
        if let Some(delay) = self.blink.next_toggle(screen.cursor_blinking, focused, now) {
            ctx.request_repaint_after(delay);
        }
        tab.paint(
            ui,
            area,
            self.cursor_on,
            GridPaint {
                cell,
                font_size: self.font_size,
                faces: self.faces,
                now,
            },
        );
        if let Some(delay) = tab.bell_repaint(now) {
            ctx.request_repaint_after(delay);
        }
        tab.find_bar(&ctx);

        if self.settings_open {
            self.settings_window(&ctx);
        }

        if let Some(action) = self.palette_window(&ctx) {
            self.apply(action, &ctx);
        }
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
        settings::Action::CommandPalette => TabAction::TogglePalette,
        settings::Action::Find => TabAction::Find,
        // `unbound` is resolved away when the file loads.
        settings::Action::Unbound => return None,
    })
}

/// One command the palette can run.
#[derive(Debug, Clone, PartialEq, Eq)]
struct PaletteEntry {
    /// The row's text.
    label: String,
    /// What choosing it asks the window to do.
    action: TabAction,
    /// The chord the settings bind to it, for the row's right-hand side.
    chord: Option<String>,
}

/// Every command the palette offers: one tab per profile, then the actions.
fn palette_entries(settings: &Settings) -> Vec<PaletteEntry> {
    let mut entries = Vec::new();
    for profile in &settings.profiles {
        entries.push(PaletteEntry {
            label: format!("New tab: {}", profile.name),
            action: TabAction::NewProfile(profile.name.clone()),
            chord: None,
        });
    }

    for action in settings::Action::ALL.iter().copied() {
        // The palette is not a command to run from the palette.
        if action == settings::Action::CommandPalette {
            continue;
        }
        if let Some(tab_action) = action_for(action) {
            entries.push(PaletteEntry {
                label: action_label(action).to_string(),
                action: tab_action,
                chord: settings
                    .keybindings
                    .iter()
                    .find(|binding| binding.action == action)
                    .map(|binding| binding.keys.to_string()),
            });
        }
    }

    entries
}

/// The name the palette gives an action.
fn action_label(action: settings::Action) -> &'static str {
    use settings::Action;

    match action {
        Action::NewTab => "New tab",
        Action::CloseTab => "Close pane or tab",
        Action::NextTab => "Next tab",
        Action::PrevTab => "Previous tab",
        Action::SplitHorizontal => "Split pane right",
        Action::SplitVertical => "Split pane down",
        Action::FocusLeft => "Focus pane left",
        Action::FocusRight => "Focus pane right",
        Action::FocusUp => "Focus pane up",
        Action::FocusDown => "Focus pane down",
        Action::ResizeLeft => "Resize pane left",
        Action::ResizeRight => "Resize pane right",
        Action::ResizeUp => "Resize pane up",
        Action::ResizeDown => "Resize pane down",
        Action::Copy => "Copy selection",
        Action::Paste => "Paste",
        Action::OpenSettings => "Settings",
        Action::CommandPalette => "Command palette",
        Action::Find => "Find",
        Action::Unbound => "Unbound",
    }
}

/// Whether `label` matches `filter`, ignoring case and surrounding space.
fn matches_filter(label: &str, filter: &str) -> bool {
    let filter = filter.trim();
    filter.is_empty() || label.to_lowercase().contains(&filter.to_lowercase())
}

/// The match Enter moves to, wrapping at both ends.
pub(crate) fn next_match(current: Option<usize>, len: usize, forward: bool) -> Option<usize> {
    if len == 0 {
        return None;
    }

    Some(match current {
        Some(index) if forward => (index + 1) % len,
        Some(index) => (index + len - 1) % len,
        None if forward => 0,
        None => len - 1,
    })
}

/// Whether this frame's key events belong to the shell.
///
/// A window's text field owns the keyboard, and so does an open palette;
/// forwarding as well would type the same characters into the shell.
fn shell_takes_keys(palette_open: bool, keyboard_owned: bool) -> bool {
    !palette_open && !keyboard_owned
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

#[cfg(test)]
mod tests {
    use super::*;
    use term::input::Key;

    use crate::scrollbar;
    use types::TextAttribute;
    use types::color_table::{ColorAlias, ColorTable};
    use types::text_color::TextColor;

    use crate::config::{BindingDraft, Draft, ProfileDraft, SchemeDraft, ThemeDraft, settings_rows};
    use crate::terminal_widget::{FindBar, attribute_colors, keymap, resolve, scrollbar_track};

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
    fn the_scrollbar_sits_on_the_right_edge() {
        let area = egui::Rect::from_min_size(egui::pos2(10.0, 20.0), egui::vec2(200.0, 100.0));
        let track = scrollbar_track(area);

        assert_eq!(track.max, area.max);
        assert_eq!(track.min.y, area.min.y);
        assert_eq!(track.width(), scrollbar::WIDTH);
    }

    #[test]
    fn the_palette_chord_toggles_it() {
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

        assert_eq!(
            command_for(settings::DEFAULT_KEYBINDINGS, &key(egui::Key::P, true, true)),
            Some(TabAction::TogglePalette)
        );
        assert_eq!(
            command_for(settings::DEFAULT_KEYBINDINGS, &key(egui::Key::F, true, true)),
            Some(TabAction::Find)
        );
    }

    #[test]
    fn the_next_match_wraps_at_both_ends() {
        assert_eq!(next_match(None, 0, true), None);
        assert_eq!(next_match(None, 3, true), Some(0));
        assert_eq!(next_match(None, 3, false), Some(2));
        assert_eq!(next_match(Some(1), 3, true), Some(2));
        assert_eq!(next_match(Some(2), 3, true), Some(0));
        assert_eq!(next_match(Some(0), 3, false), Some(2));
    }

    #[test]
    fn the_find_bar_keeps_its_query_between_openings() {
        let mut find = FindBar {
            query: "error".to_string(),
            ..FindBar::default()
        };

        find.toggle();
        assert!(find.open);
        assert!(find.focus);

        find.toggle();
        assert!(!find.open);
        assert!(!find.focus);
        assert_eq!(find.query, "error");
    }

    #[test]
    fn the_palette_offers_every_profile_and_the_actions() {
        let settings = settings::parse(
            "[[profiles]]\nname = \"one\"\ncommand = \"sh\"\n\n[[profiles]]\nname = \"two\"\ncommand = \"zsh\"\n",
        )
        .expect("settings");
        let entries = palette_entries(&settings);

        // A profile is reachable by name before the fixed commands.
        assert_eq!(entries[0].label, "New tab: one");
        assert_eq!(entries[0].action, TabAction::NewProfile("one".to_string()));
        assert_eq!(entries[1].label, "New tab: two");

        let settings_entry = entries
            .iter()
            .find(|entry| entry.action == TabAction::ToggleSettings)
            .expect("a settings entry");
        assert_eq!(settings_entry.label, "Settings");
        assert_eq!(settings_entry.chord.as_deref(), Some("ctrl+,"));

        let copy = entries
            .iter()
            .find(|entry| entry.action == TabAction::Copy)
            .expect("a copy entry");
        assert_eq!(copy.chord.as_deref(), Some("ctrl+shift+c"));

        let find = entries
            .iter()
            .find(|entry| entry.action == TabAction::Find)
            .expect("a find entry");
        assert_eq!(find.label, "Find");
        assert_eq!(find.chord.as_deref(), Some("ctrl+shift+f"));

        // The palette is not a command to run from the palette.
        assert!(!entries.iter().any(|entry| entry.action == TabAction::TogglePalette));
    }

    #[test]
    fn a_palette_filter_matches_any_case() {
        assert!(matches_filter("Split pane right", ""));
        assert!(matches_filter("Split pane right", "split"));
        assert!(matches_filter("Split pane right", "PANE"));
        assert!(!matches_filter("Split pane right", "paste"));
    }

    #[test]
    fn opening_the_palette_starts_a_fresh_query() {
        let mut palette = Palette {
            filter: "abc".to_string(),
            selected: 2,
            ..Palette::default()
        };

        palette.show();
        assert!(palette.open);
        assert!(palette.filter.is_empty());
        assert_eq!(palette.selected, 0);
        assert!(palette.focus);

        palette.toggle();
        assert!(!palette.open);
        assert!(palette.filter.is_empty());
    }

    #[test]
    fn a_text_field_or_the_palette_keeps_keys_from_the_shell() {
        assert!(shell_takes_keys(false, false));
        assert!(!shell_takes_keys(false, true));
        assert!(!shell_takes_keys(true, false));
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
    fn the_preview_is_the_document_a_save_writes() {
        let settings = settings::parse("[[profiles]]\nname = \"a\"\ncommand = \"sh\"\n").expect("settings");
        let mut draft = Draft::from_settings(&settings);
        draft.font_size = 22.0;

        let preview = draft.preview(&settings).expect("a preview");
        assert!(preview.contains("size = 22"), "{preview}");
        assert!(preview.contains("command = \"sh\""), "{preview}");

        // A value that does not parse leaves nothing to show.
        let broken = settings::parse("[[schemes]]\nname = \"s\"\nred = \"#FF0000\"").expect("settings");
        let mut draft = Draft::from_settings(&broken);
        draft.schemes[0].colours[5] = "nope".to_string();
        assert!(draft.preview(&broken).is_none());
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
    fn a_scheme_round_trips_through_the_draft() {
        let colour = |spec: &str| Some(settings::Color::parse(spec).expect("colour"));
        let scheme = settings::Scheme {
            name: "muted".to_string(),
            foreground: colour("#C8C8C8"),
            background: colour("#101014"),
            cursor_color: colour("#E6E6E6"),
            selection_background: colour("#264F78"),
            black: colour("#000000"),
            red: colour("#FF0000"),
            green: colour("#00FF00"),
            yellow: colour("#FFFF00"),
            blue: colour("#0000FF"),
            purple: colour("#FF00FF"),
            cyan: colour("#00FFFF"),
            white: colour("#FFFFFF"),
            bright_black: colour("#111111"),
            bright_red: colour("#EE0000"),
            bright_green: colour("#00EE00"),
            bright_yellow: colour("#EEEE00"),
            bright_blue: colour("#0000EE"),
            bright_purple: colour("#EE00EE"),
            bright_cyan: colour("#00EEEE"),
            bright_white: colour("#EEEEEE"),
        };

        let draft = SchemeDraft::from_scheme(&scheme);
        assert_eq!(draft.name, "muted");
        assert_eq!(draft.colours.len(), settings::SchemeColor::ALL.len());
        assert_eq!(draft.to_scheme().expect("scheme"), scheme);

        // A field that is not a colour is named, and does not become a silent
        // "unset".
        let mut bad = draft;
        bad.colours[5] = "#12345".to_string();
        let error = bad.to_scheme().expect_err("a bad colour");
        assert!(error.contains("red"), "{error}");
        assert!(error.contains("#12345"), "{error}");
    }

    #[test]
    fn the_draft_edits_the_schemes_and_blocks_a_bad_colour() {
        let settings = settings::parse("[[schemes]]\nname = \"s\"\nred = \"#FF0000\"").expect("settings");
        let mut draft = Draft::from_settings(&settings);
        assert!(!draft.differs_from(&settings));
        assert_eq!(draft.schemes.len(), 1);

        draft.schemes[0].colours[5] = "#EE0000".to_string();
        assert!(draft.differs_from(&settings));
        let document = draft.document(&settings).expect("document");
        assert_eq!(
            document.schemes[0].red,
            Some(settings::Color::parse("#EE0000").expect("colour"))
        );

        draft.schemes[0].colours[5] = "nope".to_string();
        assert!(draft.document(&settings).is_none());
        assert_eq!(draft.scheme_errors().len(), 1);
        assert!(draft.differs_from(&settings));
    }

    #[test]
    fn a_theme_round_trips_through_the_draft() {
        let colour = |spec: &str| Some(settings::Color::parse(spec).expect("colour"));
        let theme = settings::Theme {
            name: "night".to_string(),
            tab_row: settings::TabRowTheme {
                background: colour("#101010"),
                unfocused_background: colour("#0A0A0A"),
            },
            tab: settings::TabTheme {
                background: colour("#202020"),
                unfocused_background: None,
            },
        };

        let draft = ThemeDraft::from_theme(&theme);
        assert_eq!(draft.name, "night");
        assert_eq!(draft.to_theme().expect("theme"), theme);

        // An empty field is an unset colour, not a broken one.
        assert_eq!(draft.tab_unfocused, "");

        let mut bad = draft;
        bad.tab_background = "#12345".to_string();
        let error = bad.to_theme().expect_err("a bad colour");
        assert!(error.contains("tab:"), "{error}");
        assert!(error.contains("#12345"), "{error}");
    }

    #[test]
    fn the_draft_edits_the_themes_and_blocks_a_bad_colour() {
        let settings = settings::parse("theme = \"night\"\n[[themes]]\nname = \"night\"").expect("settings");
        let mut draft = Draft::from_settings(&settings);
        assert!(!draft.differs_from(&settings));
        assert_eq!(draft.themes.len(), 1);

        draft.themes[0].tab_background = "#202020".to_string();
        assert!(draft.differs_from(&settings));
        let document = draft.document(&settings).expect("document");
        assert_eq!(
            document.themes[0].tab.background,
            Some(settings::Color::parse("#202020").expect("colour"))
        );

        draft.themes[0].tab_background = "nope".to_string();
        assert!(draft.document(&settings).is_none());
        assert_eq!(draft.theme_errors().len(), 1);
        assert!(draft.differs_from(&settings));
    }

    #[test]
    fn the_settings_rows_report_what_the_file_asked_for() {
        let settings = settings::parse(
            r#"
                default_profile = "zsh"
                default_scheme = "muted"
                theme = "night"

                [[profiles]]
                name = "zsh"
                command = "/bin/zsh"

                [[schemes]]
                name = "muted"

                [[themes]]
                name = "night"

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
        assert_eq!(value("Keybindings"), "19");
        assert_eq!(value("Scheme"), "muted");
        assert_eq!(value("Theme"), "night");
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
        assert_eq!(value("Theme"), "(shell)");
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
}
