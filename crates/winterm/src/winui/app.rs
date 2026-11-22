//! The WinUI 3 shell: one pane, a native window, the renderer's pixels.
//!
//! The component owns the `Control`, the renderer and the swap-chain surface.
//! A timer polls the pty on the UI thread (the connection is not `Send`, so it
//! cannot be moved to `spawn_background`), paints a frame, and presents it
//! through [`Surface`] (ADRs 006, 040, 041).
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
use term::input::Key;
use types::color_table::ColorAlias;
use utils::{InclusiveRect, Size};
use windows_reactor::*;

use crate::input;
use crate::session::Session;
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

/// The application's state and its messages.
struct Shell {
    control: Control,
    view: ViewState,
    renderer: Renderer<TerminalData>,
    shared: SharedBackend,
    panel: ElementRef<SwapChainPanel>,
    focus: ElementRef<Border>,
    surface: Rc<RefCell<Option<Surface>>>,
    _observation: ElementObservation,
    timer: Option<ComponentTimer>,
    sides: input::ModifierSides,
    title: String,
    blink_on: bool,
    blink_at: Instant,
    focused: bool,
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
    /// The default background and foreground, as `COLORREF`s, so a face rebuilt
    /// at a new scale keeps the scheme.
    background: u32,
    foreground: u32,
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
        let terminal = TerminalSettings::resolve(&loaded.settings);
        let font_size = loaded.settings.font.size;

        let session = Session::spawn(&terminal.command, size)
            .or_else(|_| Session::spawn(&PtyCommand::new("powershell.exe"), size))
            .or_else(|_| Session::spawn(&PtyCommand::new("cmd.exe"), size))
            .expect("a shell");
        let mut control = Control::new(Box::new(session), grid);
        control.apply_scheme(terminal.scheme.as_ref());

        let face = face_path();
        let fonts = font_setup(&face, font_size, 1.0, grid);
        let cell = fonts.settings.font.cell_size;
        let (mut renderer, shared) = renderer_for(fonts, grid);
        let (background, foreground) = default_colors(&control);
        let _ = renderer.set_default_colors(background, foreground);

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
            control,
            view: ViewState::new(),
            renderer,
            shared,
            panel,
            focus: ElementRef::<Border>::new(),
            surface,
            _observation: observation,
            timer,
            sides: input::ModifierSides::default(),
            title: terminal.title.clone(),
            blink_on: true,
            blink_at: Instant::now(),
            focused: false,
            face,
            font_size,
            cell,
            scale: 1.0,
            grid,
            initial_client: (
                f64::from(cell.0) * f64::from(INITIAL_COLUMNS as u16),
                f64::from(cell.1) * f64::from(INITIAL_ROWS as u16),
            ),
            background,
            foreground,
        }
    }

    fn view(&self, _input: &(), context: &mut ViewContext<Self>) -> View {
        let keys = context.sender();
        let characters = keys.clone();

        let pane = Border::new()
            .is_tab_stop(true)
            .focus_on_pointer_release(true)
            .element_ref(&self.focus)
            .background(Color::rgb(12, 12, 12))
            .content(SwapChainPanel::new().element_ref(&self.panel))
            .on_preview_key_down(RoutedCallback::new(move |info| {
                let _ = keys.send(Message::Key(info));
                // A printable key still has to raise `CharacterReceived`, which
                // is the text path; only the keys the terminal encodes itself
                // are marked handled.
                !matches!(key_of(info.key), Some(Key::Char(_) | Key::Space))
            }))
            .on_character_received(RoutedCallback::new(move |info| {
                let _ = characters.send(Message::Char(info));
                true
            }));

        context.window_title(&self.title);
        context.window_visuals(
            WindowVisuals::new()
                .theme(WindowTheme::Dark)
                .backdrop(WindowBackdrop::Mica)
                .client_size(self.initial_client.0, self.initial_client.1),
        );

        pane.into()
    }

    fn update(&mut self, message: Message, context: &ComponentContext<Self>) {
        match message {
            Message::Tick => self.tick(context),
            Message::Key(info) => self.key(info),
            Message::Char(info) => self.character(info),
            Message::Resize {
                width,
                height,
                scale_x,
                scale_y,
            } => self.resize(width, height, scale_x, scale_y),
        }
    }
}

impl Shell {
    /// Pumps the pty, paints a frame and re-arms the timer.
    fn tick(&mut self, context: &ComponentContext<Self>) {
        self.control.pump();

        let now = Instant::now();
        for event in self.control.take_events(now) {
            if event == TerminalEvent::Exited {
                let _ = context.close_window();
            }
        }

        if let Some(title) = self.control.title()
            && self.title != title
        {
            self.title = title.to_string();
        }

        if now.duration_since(self.blink_at) >= Duration::from_millis(500) {
            self.blink_on = !self.blink_on;
            self.blink_at = now;
        }

        if !self.focused {
            self.focused = self.focus.request_focus();
        }

        self.renderer
            .data_mut()
            .refresh(&mut self.control, &self.view, self.blink_on);
        // The engine resolves a cell's colours through the renderer's table, not
        // the data's copy, so the shell hands it the palette it painted with
        // (`RenderSettings::set_color_table`).
        sync_palette(&mut self.renderer, &self.control);
        self.renderer.invalidate_all();

        if self.renderer.paint_frame().is_ok() {
            let width = self.shared.width();
            let height = self.shared.height();
            let background = self.background;
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
    fn rebuild_face(&mut self, scale: f32) {
        let fonts = font_setup(&self.face, self.font_size, scale, self.grid);
        self.cell = fonts.settings.font.cell_size;
        let (mut renderer, shared) = renderer_for(fonts, self.grid);
        let _ = renderer.set_default_colors(self.background, self.foreground);
        self.renderer = renderer;
        self.shared = shared;
    }

    /// Tells the terminal and the renderer how large the grid and target are.
    fn apply_grid(&mut self, (columns, rows): (i32, i32)) {
        self.grid = (columns, rows);
        self.control.resize(columns, rows);

        let target = target_size(self.cell, self.grid);
        let viewport = InclusiveRect::new(0, 0, columns.max(1) - 1, rows.max(1) - 1);
        let _ = self.renderer.resize(target, viewport);
    }

    /// Turns a key press into the terminal's encoding.
    fn key(&mut self, info: KeyEventInfo) {
        let Some(key) = key_of(info.key) else {
            return;
        };

        let event = input::Event::Key {
            key,
            pressed: !info.status.is_released,
            repeat: info.status.was_down && !info.status.is_released,
            modifiers: modifiers(info.modifiers),
        };

        let outcome = input::translate(&event, &mut self.sides);
        for key in outcome.keys {
            self.control.send_key(&key);
        }
    }

    /// Hands composed text to the parser's text path.
    fn character(&mut self, info: CharacterEventInfo) {
        let Some(character) = char::from_u32(u32::from(info.character)) else {
            return;
        };
        if character.is_control() {
            return;
        }

        let outcome = input::translate(&input::Event::Text(character.to_string()), &mut self.sides);
        for key in outcome.keys {
            self.control.send_key(&key);
        }
    }
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
