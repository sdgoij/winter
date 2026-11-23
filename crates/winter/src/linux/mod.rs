//! The GTK 4 shell: a native window over the renderer's pixels (ADR 061).
//!
//! A `GtkApplicationWindow` hosts one `GtkDrawingArea`; the terminal itself is
//! the shared `Control` over a pty, and the renderer paints its screen into the
//! area's Cairo context. Keys go through the shared `input` rules, and the wheel
//! scrolls history unless a tracking program owns it.
//!
//! `WINTER_SCREENSHOT=<path>` writes the toplevel exactly as GTK draws it —
//! client-side decorations included — and quits, which is how a machine with no
//! display server photographs the shell.
//!
//! Tabs, selection, the scrollbar and the native menus are later milestones.

use std::cell::{Cell, RefCell};
use std::path::{Path, PathBuf};
use std::rc::Rc;
use std::time::{Duration, Instant};

use gtk4 as gtk;

use gtk::cairo;
use gtk::graphene;
use gtk::prelude::*;
use gtk::{
    Application, ApplicationWindow, DrawingArea, EventControllerKey, EventControllerMotion, EventControllerScroll,
    EventControllerScrollFlags, Snapshot, WidgetPaintable, gdk, glib,
};

use renderer::renderer::Renderer;
use renderer::{
    AtlasEngine, CellSizeInDIP, FontDueRasterizer, FontInfo, RenderSettings, Settings, SharedBackend, SoftwareBackend,
};
use server::pty::PtySize;
use term::input::{Key, MouseEvent, WHEEL_DELTA, WheelAxis};
use utils::{InclusiveRect, Size};

use crate::input::{self, Event as InputEvent, ModifierSides, Modifiers};
use crate::session::Session;
use crate::term_control::Control;
use crate::term_events::TerminalEvent;
use crate::term_settings::TerminalSettings;
use crate::terminal_render::TerminalData;
use crate::view::View;

/// The window opens at this many cells, like the WinUI shell.
const CELLS: (i32, i32) = (100, 28);
/// The face's size in points before the surface scale is applied.
const FONT_SIZE: f32 = 14.0;
/// How often the pty is polled, matching the WinUI shell's timer (ADR 006).
const TICK: Duration = Duration::from_millis(16);

/// Starts the GTK shell and runs it until its window closes.
pub fn run() -> Result<(), Box<dyn std::error::Error>> {
    let app = Application::builder().application_id("org.winter.Winter").build();
    app.connect_activate(activate);
    app.run();
    Ok(())
}

fn activate(app: &Application) {
    let screenshot = std::env::var_os("WINTER_SCREENSHOT").map(PathBuf::from);

    let Some(fonts) = fonts(FONT_SIZE, CELLS) else {
        eprintln!("winter: no monospace face found in the usual font directories");
        app.quit();
        return;
    };

    let Some(connection) = session(app, CELLS) else {
        return;
    };

    let shared = SharedBackend::new(SoftwareBackend::with_rasterizer(Box::new(fonts.rasterizer)));
    let mut engine = AtlasEngine::with_settings(fonts.settings.clone(), Box::new(shared.clone()));
    engine.set_shaping(Box::new(fonts.shaper));

    let mut renderer = Renderer::new(
        TerminalData::new(fonts.info, RenderSettings::new(), CELLS.0, CELLS.1),
        RenderSettings::new(),
    );
    renderer.add_engine(Box::new(engine));

    let window = ApplicationWindow::builder()
        .application(app)
        .title("Winter")
        .default_width(i32::from(fonts.cell.0) * CELLS.0)
        .default_height(i32::from(fonts.cell.1) * CELLS.1)
        .build();

    let area = DrawingArea::new();
    // The pane takes the keyboard, which is how a terminal behaves; a click
    // asks for it back when a dialog or a menu took it away.
    area.set_can_focus(true);
    window.set_child(Some(&area));

    let shell = Rc::new(RefCell::new(Shell {
        window: window.clone(),
        area: area.clone(),
        renderer,
        shared,
        control: Control::new(connection, CELLS),
        view: View::new(),
        sides: ModifierSides::default(),
        pointer: (0, 0),
        wheel_pixels: 0.0,
        cell: fonts.cell,
        grid: CELLS,
        screenshot,
    }));

    {
        let shell = Rc::clone(&shell);
        area.set_draw_func(move |_area, context, width, height| {
            shell.borrow_mut().paint(context, width, height);
        });
    }

    {
        let keys = EventControllerKey::new();
        let pressed = Rc::clone(&shell);
        keys.connect_key_pressed(move |_controller, key, _code, state| pressed.borrow_mut().key(key, state, true));
        let released = Rc::clone(&shell);
        keys.connect_key_released(move |_controller, key, _code, state| {
            released.borrow_mut().key(key, state, false);
        });
        window.add_controller(keys);
    }

    {
        let shell = Rc::clone(&shell);
        let motion = EventControllerMotion::new();
        motion.connect_motion(move |_controller, x, y| shell.borrow_mut().motion(x, y));
        area.add_controller(motion);
    }

    {
        let shell = Rc::clone(&shell);
        let scroll = EventControllerScroll::new(EventControllerScrollFlags::VERTICAL);
        scroll.connect_scroll(move |controller, _x, y| {
            let shift = controller
                .current_event()
                .is_some_and(|event| event.modifier_state().contains(gdk::ModifierType::SHIFT_MASK));
            shell.borrow_mut().wheel(y, controller.unit(), shift)
        });
        area.add_controller(scroll);
    }

    {
        let shell = Rc::clone(&shell);
        let app = app.clone();
        glib::timeout_add_local(TICK, move || {
            shell.borrow_mut().pump(&app);
            glib::ControlFlow::Continue
        });
    }

    window.present();
    area.grab_focus();

    // `GtkWidgetPaintable` pictures the last frame GTK painted, and a queued
    // redraw has already dropped that frame's node, so the shot is taken at the
    // end of a frame rather than from a timer.
    if shell.borrow().screenshot.is_some() {
        let Some(clock) = window.frame_clock() else {
            eprintln!("winter: the window has no frame clock");
            app.quit();
            return;
        };
        let shell = Rc::clone(&shell);
        let app = app.clone();
        let due = Instant::now() + Duration::from_millis(900);
        let shot = Cell::new(false);
        clock.connect_after_paint(move |_clock| {
            if shot.get() || Instant::now() < due {
                return;
            }
            shot.set(true);
            shell.borrow().write_screenshot(&app);
        });
    }
}

/// Starts the profile's shell on a pty, or reports why it could not.
fn session(app: &Application, cells: (i32, i32)) -> Option<Box<dyn crate::term_connection::Connection>> {
    let settings = settings::load();
    let terminal = TerminalSettings::resolve(&settings);
    let size = PtySize::new(
        u16::try_from(cells.0.max(1)).unwrap_or(u16::MAX),
        u16::try_from(cells.1.max(1)).unwrap_or(u16::MAX),
    );

    match Session::spawn(&terminal.command, size) {
        Ok(session) => Some(Box::new(session)),
        Err(error) => {
            eprintln!("winter: cannot start the shell: {error}");
            app.quit();
            None
        }
    }
}

/// The window, the frame it paints and the terminal behind it.
struct Shell {
    window: ApplicationWindow,
    area: DrawingArea,
    renderer: Renderer<TerminalData>,
    shared: SharedBackend,
    control: Control,
    view: View,
    /// The modifier sides the toolkit collapses but the encoder distinguishes.
    sides: ModifierSides,
    /// Where the pointer last was, in cells, for a tracking program's wheel.
    pointer: (i32, i32),
    /// Wheel pixels that have not yet made a row: a touchpad reports pixels
    /// where a wheel reports whole detents.
    wheel_pixels: f64,
    /// The cell size in physical pixels.
    cell: (u16, u16),
    /// The grid the renderer's target currently holds.
    grid: (i32, i32),
    /// Where `WINTER_SCREENSHOT` asked for a picture, if it did.
    screenshot: Option<PathBuf>,
}

impl Shell {
    /// Drains the pty, answers the program and asks for a repaint.
    fn pump(&mut self, app: &Application) {
        self.control.pump();
        for event in self.control.take_events(Instant::now()) {
            if matches!(event, TerminalEvent::Exited) {
                app.quit();
                return;
            }
        }
        self.area.queue_draw();
    }

    /// Turns one toolkit key into what the terminal should receive.
    fn key(&mut self, key: gdk::Key, state: gdk::ModifierType, pressed: bool) -> glib::Propagation {
        let modifiers = modifiers(state);
        let Some(named) = key_of(key) else {
            return glib::Propagation::Proceed;
        };

        let character = if matches!(named, Key::Char(_)) {
            none_if_control(key.to_unicode())
        } else {
            None
        };

        // A printable key is text, not a chord: the shared translation sends a
        // character event for it, exactly as egui's and WinUI's toolkits do.
        let chord = modifiers.ctrl || modifiers.alt || modifiers.command;
        let event = match (character, pressed, chord) {
            (Some(character), true, false) if input::produces_text(named, modifiers) => {
                InputEvent::Text(character.to_string())
            }
            _ => InputEvent::Key {
                key: named,
                pressed,
                repeat: false,
                modifiers,
                // A pty on this platform never asks for Win32 input records, so
                // there is no per-key identity to report.
                virtual_key: 0,
                scan_code: 0,
                repeat_count: 1,
                codepoint: character,
            },
        };

        let win32 = self.control.input_modes().win32_input;
        let outcome = input::translate(&event, &mut self.sides, win32);
        for key in outcome.keys {
            self.control.send_key(&key);
        }
        if outcome.activity {
            self.view.scroll_to_bottom();
        }

        glib::Propagation::Stop
    }

    /// Remembers the pointer's cell for the wheel a tracking program receives.
    fn motion(&mut self, x: f64, y: f64) {
        self.pointer = (cell_index(x, self.cell.0), cell_index(y, self.cell.1));
    }

    /// Scrolls the viewport, or hands the wheel to a tracking program.
    fn wheel(&mut self, delta: f64, unit: gdk::ScrollUnit, shift: bool) -> glib::Propagation {
        let rows = wheel_rows(delta, unit, self.cell.1, &mut self.wheel_pixels);
        if rows == 0 {
            return glib::Propagation::Proceed;
        }

        // Shift keeps the wheel for the viewport; otherwise a tracking program
        // owns it, the same rule the egui and WinUI shells apply.
        let forwards = input::forwards_to_application(self.control.is_tracking_mouse(), shift);
        let pointer = (
            self.pointer.0.clamp(0, self.grid.0.max(1) - 1),
            self.pointer.1.clamp(0, self.grid.1.max(1) - 1),
        );
        let wheel = MouseEvent::wheel(pointer, WheelAxis::Vertical, rows * WHEEL_DELTA);
        if !(forwards && self.control.send_mouse(&wheel)) {
            let max = self.control.max_scroll();
            self.view.scroll_by(rows, max);
        }

        glib::Propagation::Stop
    }

    /// Fits the grid to the area and paints one frame.
    fn paint(&mut self, context: &cairo::Context, width: i32, height: i32) {
        self.fit(width, height);

        self.renderer
            .data_mut()
            .refresh(&mut self.control, &self.view, true, None);

        let (background, foreground) = self.control.default_colors();
        let _ = self.renderer.set_default_colors(background, foreground);
        if let Ok(table) = self.control.color_table().lock() {
            self.renderer.settings_mut().set_color_table(table.clone());
        }
        self.renderer.invalidate_all();
        if self.renderer.paint_frame().is_err() {
            return;
        }

        // Whatever the whole-cell grid does not cover stays the pane's colour.
        let (red, green, blue) = channels(background);
        context.set_source_rgb(red, green, blue);
        context.paint().ok();

        let (pixels_wide, pixels_high) = (self.shared.width(), self.shared.height());
        if pixels_wide == 0 || pixels_high == 0 {
            return;
        }

        let mut bytes = Vec::with_capacity(4 * pixels_wide as usize * pixels_high as usize);
        self.shared.with_pixels(|pixels| {
            for pixel in pixels.iter().take(pixels_wide as usize * pixels_high as usize) {
                bytes.extend_from_slice(&pixel.to_ne_bytes());
            }
        });

        // `0xAARRGGBB` in a `u32` is BGRA in memory, which is `ARGB32` on a
        // little-endian host -- Cairo's own layout, so no swizzle is needed.
        let Ok(surface) = cairo::ImageSurface::create_for_data(
            bytes,
            cairo::Format::ARgb32,
            pixels_wide as i32,
            pixels_high as i32,
            4 * pixels_wide as i32,
        ) else {
            return;
        };
        if context.set_source_surface(&surface, 0.0, 0.0).is_ok() {
            context.paint().ok();
        }
    }

    /// Makes the terminal and the renderer's target hold whole cells.
    fn fit(&mut self, width: i32, height: i32) {
        let columns = (width / i32::from(self.cell.0)).max(1);
        let rows = (height / i32::from(self.cell.1)).max(1);
        if (columns, rows) == self.grid {
            return;
        }

        self.grid = (columns, rows);
        self.control.resize(columns, rows);
        let target = (
            u16::try_from(i32::from(self.cell.0) * columns).unwrap_or(u16::MAX),
            u16::try_from(i32::from(self.cell.1) * rows).unwrap_or(u16::MAX),
        );
        let _ = self
            .renderer
            .resize(target, InclusiveRect::new(0, 0, columns - 1, rows - 1));
    }

    /// Writes the toplevel as GTK draws it, then quits.
    fn write_screenshot(&self, app: &Application) {
        let Some(path) = self.screenshot.as_deref() else {
            return;
        };

        let paintable = WidgetPaintable::new(Some(&self.window));
        let width = f64::from(self.window.width());
        let height = f64::from(self.window.height());
        let snapshot = Snapshot::new();
        paintable.snapshot(&snapshot, width, height);

        let Some(node) = snapshot.to_node() else {
            eprintln!("winter: the window produced no render node");
            app.quit();
            return;
        };
        let Some(native) = self.window.native() else {
            eprintln!("winter: the window has no native surface");
            app.quit();
            return;
        };

        let viewport = graphene::Rect::new(0.0, 0.0, width as f32, height as f32);
        let Some(renderer) = native.renderer() else {
            eprintln!("winter: the surface has no renderer");
            app.quit();
            return;
        };
        let texture = renderer.render_texture(&node, Some(&viewport));

        match texture.save_to_png(path) {
            Ok(()) => println!("winter: wrote {}", path.display()),
            Err(error) => eprintln!("winter: cannot write {}: {error}", path.display()),
        }
        app.quit();
    }
}

/// The toolkit's modifier state, collapsed the way the shared input expects.
fn modifiers(state: gdk::ModifierType) -> Modifiers {
    Modifiers {
        shift: state.contains(gdk::ModifierType::SHIFT_MASK),
        ctrl: state.contains(gdk::ModifierType::CONTROL_MASK),
        alt: state.contains(gdk::ModifierType::ALT_MASK),
        command: state.contains(gdk::ModifierType::SUPER_MASK),
    }
}

/// Drops the control characters a keysym can carry: Enter and Tab are keys,
/// not text, and reach the terminal through the key path.
fn none_if_control(character: Option<char>) -> Option<char> {
    character.filter(|character| !character.is_control())
}

/// The cell a pixel position falls in; a toolkit reports sub-pixel offsets and
/// a motion that starts above the pane.
fn cell_index(position: f64, cell: u16) -> i32 {
    (position.max(0.0) / f64::from(cell.max(1))) as i32
}

/// Turns one scroll event into whole rows towards the history.
///
/// A wheel reports detents and a touchpad reports pixels, so the pixels
/// accumulate until they make a row. GTK's delta is positive when the content
/// should move down, which is the opposite of the view's rows-above-the-screen.
fn wheel_rows(delta: f64, unit: gdk::ScrollUnit, cell: u16, remainder: &mut f64) -> i32 {
    let height = f64::from(cell.max(1));
    match unit {
        gdk::ScrollUnit::Wheel => -(delta.round() as i32),
        _ => {
            *remainder += delta;
            let rows = (*remainder / height).trunc();
            *remainder -= rows * height;
            -(rows as i32)
        }
    }
}

/// Maps a GDK key to the terminal's vocabulary.
///
/// A printable key is named by its unshifted character, which is what the
/// encoder's control chords need; GDK reports the shifted keysym, so the
/// character is lowercased and the shift stays in the modifier state.
fn key_of(key: gdk::Key) -> Option<Key> {
    // Function keys and the keypad digits are regular enough to read off the
    // name rather than listing two dozen constants.
    if let Some(name) = key.name() {
        let name = name.as_str();
        if let Some(digits) = name.strip_prefix('F')
            && let Ok(number) = digits.parse::<u8>()
            && (1..=24).contains(&number)
        {
            return Some(Key::Function(number));
        }
        if let Some(digits) = name.strip_prefix("KP_")
            && let Ok(digit @ 0..=9) = digits.parse::<u8>()
        {
            return Some(Key::Numpad(digit));
        }
    }

    Some(match key {
        gdk::Key::Return => Key::Enter,
        gdk::Key::KP_Enter => Key::NumpadEnter,
        gdk::Key::BackSpace => Key::Backspace,
        gdk::Key::Tab => Key::Tab,
        gdk::Key::Escape => Key::Escape,
        gdk::Key::Left => Key::Left,
        gdk::Key::Right => Key::Right,
        gdk::Key::Up => Key::Up,
        gdk::Key::Down => Key::Down,
        gdk::Key::Home | gdk::Key::KP_Home => Key::Home,
        gdk::Key::End | gdk::Key::KP_End => Key::End,
        gdk::Key::Page_Up | gdk::Key::KP_Page_Up => Key::PageUp,
        gdk::Key::Page_Down | gdk::Key::KP_Page_Down => Key::PageDown,
        gdk::Key::Insert | gdk::Key::KP_Insert => Key::Insert,
        gdk::Key::Delete | gdk::Key::KP_Delete => Key::Delete,
        gdk::Key::Clear => Key::Clear,
        gdk::Key::KP_Decimal => Key::NumpadDecimal,
        gdk::Key::KP_Divide => Key::NumpadDivide,
        gdk::Key::KP_Multiply => Key::NumpadMultiply,
        gdk::Key::KP_Subtract => Key::NumpadSubtract,
        gdk::Key::KP_Add => Key::NumpadAdd,
        gdk::Key::KP_Separator => Key::NumpadSeparator,
        gdk::Key::Shift_L => Key::LeftShift,
        gdk::Key::Shift_R => Key::RightShift,
        gdk::Key::Control_L => Key::LeftControl,
        gdk::Key::Control_R => Key::RightControl,
        gdk::Key::Alt_L | gdk::Key::Meta_L => Key::LeftAlt,
        gdk::Key::Alt_R | gdk::Key::Meta_R => Key::RightAlt,
        gdk::Key::Super_L => Key::LeftSuper,
        gdk::Key::Super_R => Key::RightSuper,
        gdk::Key::Caps_Lock => Key::CapsLock,
        gdk::Key::Num_Lock => Key::NumLock,
        gdk::Key::Scroll_Lock => Key::ScrollLock,
        gdk::Key::Print => Key::PrintScreen,
        gdk::Key::Pause => Key::Pause,
        gdk::Key::Menu => Key::Menu,
        gdk::Key::AudioPlay => Key::MediaPlayPause,
        gdk::Key::AudioStop => Key::MediaStop,
        gdk::Key::AudioNext => Key::MediaNextTrack,
        gdk::Key::AudioPrev => Key::MediaPreviousTrack,
        gdk::Key::AudioRaiseVolume => Key::VolumeUp,
        gdk::Key::AudioLowerVolume => Key::VolumeDown,
        gdk::Key::AudioMute => Key::VolumeMute,
        gdk::Key::space => Key::Space,
        other => Key::Char(
            other
                .to_unicode()
                .map(|character| character.to_ascii_lowercase())
                .unwrap_or(' '),
        ),
    })
}

/// The face, its metrics and the two rasterisers a renderer wants.
struct Fonts {
    info: FontInfo,
    settings: Settings,
    shaper: FontDueRasterizer,
    rasterizer: FontDueRasterizer,
    cell: (u16, u16),
}

/// Finds the host's monospace face and measures it at `size`.
fn fonts(size: f32, cells: (i32, i32)) -> Option<Fonts> {
    let directories = crate::app::font_directories();
    if directories.is_empty() {
        return None;
    }

    let paths: Vec<&Path> = directories.iter().map(PathBuf::as_path).collect();
    let collection = renderer::FontCollection::from_directories(paths);
    let face = collection.default_monospace(renderer::PREFERRED_MONOSPACE_FAMILIES)?;

    let shaper = FontDueRasterizer::from_file(face.path(), size).ok()?;
    let rasterizer = FontDueRasterizer::from_file(face.path(), size).ok()?;

    let advance = shaper.advance_width('M');
    let (ascent, descent) = shaper.vertical_metrics();
    let cell_w = (advance.ceil() as u16).max(1);
    // The cell is the face's own box, so a glyph cannot spill into the row
    // above -- which on row zero means it cannot be clipped by the widget.
    let baseline = (ascent.ceil() as u16).max(1);
    let descender = (descent.ceil() as u16).max(1);
    let cell_h = baseline.saturating_add(descender);

    let mut info = FontInfo::default();
    info.set_from_engine(
        face.family().to_string(),
        0,
        400,
        65001,
        size * 0.75,
        CellSizeInDIP {
            width: f32::from(cell_w),
            height: f32::from(cell_h),
        },
        Size::new(i32::from(cell_w), i32::from(cell_h)),
    );

    let target = |cells: i32, cell: u16| {
        cell.saturating_mul(u16::try_from(cells.max(1)).unwrap_or(u16::MAX))
            .max(1)
    };
    let settings = Settings {
        font: renderer::FontSettings {
            font_name: face.family().to_string(),
            font_size: size,
            cell_size: (cell_w, cell_h),
            font_weight: 400,
            advance_width: cell_w,
            baseline,
            descender,
            thin_line_width: 1,
            dpi: 96,
            ..renderer::FontSettings::default()
        },
        misc: renderer::MiscellaneousSettings {
            background_color: 0x000C_0C0C,
            foreground_color: 0x00CC_CCCC,
            ..renderer::MiscellaneousSettings::default()
        },
        target_size: (target(cells.0, cell_w), target(cells.1, cell_h)),
        viewport_cell_count: (
            u16::try_from(cells.0.max(1)).unwrap_or(u16::MAX),
            u16::try_from(cells.1.max(1)).unwrap_or(u16::MAX),
        ),
        ..Settings::default()
    };

    Some(Fonts {
        info,
        settings,
        shaper,
        rasterizer,
        cell: (cell_w, cell_h),
    })
}

/// A `COLORREF` as Cairo's three components.
fn channels(color: u32) -> (f64, f64, f64) {
    let red = f64::from((color >> 16) & 0xFF) / 255.0;
    let green = f64::from((color >> 8) & 0xFF) / 255.0;
    let blue = f64::from(color & 0xFF) / 255.0;
    (red, green, blue)
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn the_named_keys_map_to_the_terminal_vocabulary() {
        assert_eq!(key_of(gdk::Key::Return), Some(Key::Enter));
        assert_eq!(key_of(gdk::Key::KP_Enter), Some(Key::NumpadEnter));
        assert_eq!(key_of(gdk::Key::BackSpace), Some(Key::Backspace));
        assert_eq!(key_of(gdk::Key::Page_Up), Some(Key::PageUp));
        assert_eq!(key_of(gdk::Key::Delete), Some(Key::Delete));
        assert_eq!(key_of(gdk::Key::Shift_L), Some(Key::LeftShift));
        assert_eq!(key_of(gdk::Key::Control_R), Some(Key::RightControl));
        assert_eq!(key_of(gdk::Key::Alt_R), Some(Key::RightAlt));
    }

    #[test]
    fn function_keys_and_the_keypad_are_read_off_the_name() {
        assert_eq!(key_of(gdk::Key::F5), Some(Key::Function(5)));
        assert_eq!(key_of(gdk::Key::F24), Some(Key::Function(24)));
        assert_eq!(key_of(gdk::Key::KP_3), Some(Key::Numpad(3)));
        assert_eq!(key_of(gdk::Key::KP_Add), Some(Key::NumpadAdd));
        assert_eq!(key_of(gdk::Key::KP_Divide), Some(Key::NumpadDivide));
    }

    fn keysym(name: &str) -> gdk::Key {
        gdk::Key::from_name(name).expect("a keysym")
    }

    #[test]
    fn a_printable_key_is_its_unshifted_character() {
        // GDK reports the shifted keysym; the encoder wants the base key and
        // keeps the shift in the modifier state.
        assert_eq!(key_of(keysym("q")), Some(Key::Char('q')));
        assert_eq!(key_of(keysym("Q")), Some(Key::Char('q')));
        assert_eq!(key_of(keysym("1")), Some(Key::Char('1')));
        assert_eq!(key_of(gdk::Key::space), Some(Key::Space));
    }

    #[test]
    fn the_modifier_state_collapses_the_way_the_encoder_expects() {
        let all = gdk::ModifierType::SHIFT_MASK
            | gdk::ModifierType::CONTROL_MASK
            | gdk::ModifierType::ALT_MASK
            | gdk::ModifierType::SUPER_MASK;
        let collapsed = modifiers(all);
        assert!(collapsed.shift && collapsed.ctrl && collapsed.alt && collapsed.command);

        let none = modifiers(gdk::ModifierType::empty());
        assert!(!none.shift && !none.ctrl && !none.alt && !none.command);
    }

    #[test]
    fn control_characters_are_keys_not_text() {
        assert_eq!(none_if_control(Some('\r')), None);
        assert_eq!(none_if_control(Some('\t')), None);
        assert_eq!(none_if_control(Some('q')), Some('q'));
    }

    #[test]
    fn a_pixel_position_falls_in_a_cell() {
        assert_eq!(cell_index(0.0, 9), 0);
        assert_eq!(cell_index(8.9, 9), 0);
        assert_eq!(cell_index(9.0, 9), 1);
        // A motion that starts above the pane still has a cell.
        assert_eq!(cell_index(-4.0, 9), 0);
    }

    #[test]
    fn a_wheel_detent_is_one_row_towards_the_history() {
        let mut remainder = 0.0;
        // GTK counts a delta positive when the content moves down; the view's
        // rows count up from the live screen.
        assert_eq!(wheel_rows(1.0, gdk::ScrollUnit::Wheel, 17, &mut remainder), -1);
        assert_eq!(wheel_rows(-1.0, gdk::ScrollUnit::Wheel, 17, &mut remainder), 1);
        assert_eq!(wheel_rows(0.0, gdk::ScrollUnit::Wheel, 17, &mut remainder), 0);
    }

    #[test]
    fn a_touchpad_accumulates_pixels_until_a_row() {
        let mut remainder = 0.0;
        assert_eq!(wheel_rows(8.0, gdk::ScrollUnit::Surface, 17, &mut remainder), 0);
        assert_eq!(wheel_rows(8.0, gdk::ScrollUnit::Surface, 17, &mut remainder), 0);
        assert_eq!(wheel_rows(8.0, gdk::ScrollUnit::Surface, 17, &mut remainder), -1);
        assert!(remainder > 0.0 && remainder < 17.0);

        // The pixels that did not make a row are kept in the other direction.
        assert_eq!(wheel_rows(-24.0, gdk::ScrollUnit::Surface, 17, &mut remainder), 1);
        assert_eq!(remainder, 0.0);
    }
}
