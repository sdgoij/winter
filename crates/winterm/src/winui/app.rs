//! The WinUI 3 shell: one pane, a native window, the renderer's pixels.
//!
//! The component owns the `Control`, the renderer and the swap-chain surface.
//! A timer polls the pty on the UI thread (the connection is not `Send`, so it
//! cannot be moved to `spawn_background`), paints a frame, and presents it
//! through [`Surface`] (ADRs 006, 040, 041).

use std::cell::RefCell;
use std::path::PathBuf;
use std::rc::Rc;
use std::time::{Duration, Instant};

use renderer::renderer::Renderer;
use renderer::{
    AtlasEngine, CellSizeInDIP, FontDueRasterizer, FontInfo, RenderSettings, Settings, SharedBackend, SoftwareBackend,
};
use server::pty::{PtyCommand, PtySize};
use term::input::Key;
use utils::Size;
use windows_reactor::*;

use crate::input;
use crate::session::Session;
use crate::term_control::Control;
use crate::term_events::TerminalEvent;
use crate::terminal_render::TerminalData;
use crate::view::View as ViewState;

use super::surface::Surface;

const COLUMNS: i32 = 80;
const ROWS: i32 = 24;
const FONT_SIZE: f32 = 16.0;

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
    cell: (u16, u16),
}

/// What the window sends back to the component.
///
/// The variants carry the toolkit's event records; the timer carries nothing,
/// which is what lets `set_timeout` stay `Send`.
enum Message {
    Tick,
    Key(KeyEventInfo),
    Char(CharacterEventInfo),
}

impl Component for Shell {
    type Input = ();
    type Message = Message;

    fn create(_input: &(), context: &ComponentContext<Self>) -> Self {
        let size = PtySize::new(COLUMNS as u16, ROWS as u16);
        let session = Session::spawn(&PtyCommand::new("powershell.exe"), size)
            .or_else(|_| Session::spawn(&PtyCommand::new("cmd.exe"), size))
            .expect("a shell");
        let control = Control::new(Box::new(session), (COLUMNS, ROWS));

        let (font, settings, shaper, rasterizer) = font_setup();
        let cell = settings.font.cell_size;
        let target = (u32::from(cell.0) * COLUMNS as u32, u32::from(cell.1) * ROWS as u32);

        let shared = SharedBackend::new(SoftwareBackend::with_rasterizer(Box::new(rasterizer)));
        let mut engine = AtlasEngine::with_settings(settings, Box::new(shared.clone()));
        engine.set_shaping(Box::new(shaper));

        let mut renderer = Renderer::new(
            TerminalData::new(font, RenderSettings::new(), COLUMNS, ROWS),
            RenderSettings::new(),
        );
        renderer.add_engine(Box::new(engine));

        let panel = ElementRef::<SwapChainPanel>::new();
        let surface = Rc::new(RefCell::new(None::<Surface>));
        let observation = {
            let panel = panel.clone();
            let bound = panel.clone();
            let surface = Rc::clone(&surface);
            panel.observe_surface(move |event| {
                // The panel reports its size once it is in a live tree; that is
                // the first moment a swap chain can be attached.
                if matches!(event, SwapChainPanelEvent::Metrics { .. }) {
                    let mut slot = surface.borrow_mut();
                    if slot.is_none() {
                        *slot = Surface::new(target.0, target.1).ok();
                    }
                    if let Some(surface) = slot.as_ref() {
                        let swap_chain = surface.swap_chain();
                        let _ = bound.request_set_swap_chain(swap_chain, |_| {});
                    }
                }
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
            title: "winterm".to_string(),
            blink_on: true,
            blink_at: Instant::now(),
            focused: false,
            cell,
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
                .client_size(
                    f64::from(self.cell.0) * f64::from(COLUMNS as u16),
                    f64::from(self.cell.1) * f64::from(ROWS as u16),
                ),
        );

        pane.into()
    }

    fn update(&mut self, message: Message, context: &ComponentContext<Self>) {
        match message {
            Message::Tick => self.tick(context),
            Message::Key(info) => self.key(info),
            Message::Char(info) => self.character(info),
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
        self.renderer.invalidate_all();

        if self.renderer.paint_frame().is_ok() {
            let width = self.shared.width();
            let height = self.shared.height();
            let surface = Rc::clone(&self.surface);
            self.shared.with_pixels(|pixels| {
                if let Some(surface) = surface.borrow_mut().as_mut() {
                    let _ = surface.present(pixels, width, height);
                }
            });
        }

        self.timer = Some(context.set_timeout(Duration::from_millis(16), Message::Tick));
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

/// Finds a monospace face and the metrics the atlas paints with.
fn font_setup() -> (FontInfo, Settings, FontDueRasterizer, FontDueRasterizer) {
    let path = PathBuf::from(std::env::var_os("WINDIR").unwrap_or_else(|| "C:\\Windows".into()))
        .join("Fonts")
        .join("consola.ttf");

    let shaper = FontDueRasterizer::from_file(&path, FONT_SIZE).expect("Consolas is installed");
    let rasterizer = FontDueRasterizer::from_file(&path, FONT_SIZE).expect("Consolas is installed");

    let advance = shaper.advance_width('M');
    let cell_w = (advance.ceil() as u16).max(1);
    let cell_h = (FONT_SIZE * 1.25).ceil() as u16;

    let mut font = FontInfo::default();
    font.set_from_engine(
        "Consolas".to_string(),
        0,
        400,
        65001,
        FONT_SIZE * 0.75,
        CellSizeInDIP {
            width: f32::from(cell_w),
            height: f32::from(cell_h),
        },
        Size::new(i32::from(cell_w), i32::from(cell_h)),
    );

    let settings = Settings {
        font: renderer::FontSettings {
            font_name: "Consolas".to_string(),
            font_size: FONT_SIZE,
            cell_size: (cell_w, cell_h),
            font_weight: 400,
            advance_width: cell_w,
            baseline: cell_h.saturating_sub(4),
            descender: 4,
            thin_line_width: 1,
            dpi: 96,
            ..renderer::FontSettings::default()
        },
        misc: renderer::MiscellaneousSettings {
            background_color: 0x000C_0C0C,
            foreground_color: 0x00CC_CCCC,
            ..renderer::MiscellaneousSettings::default()
        },
        target_size: (cell_w * COLUMNS as u16, cell_h * ROWS as u16),
        viewport_cell_count: (COLUMNS as u16, ROWS as u16),
        ..Settings::default()
    };

    (font, settings, shaper, rasterizer)
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
