//! The GTK 4 shell: a native window over the renderer's pixels (ADR 061).
//!
//! A `GtkHeaderBar` holds the tab strip, which this module paints itself; below
//! it a `GtkDrawingArea` paints the active terminal from `renderer`. The
//! terminal is the shared `Control` over a pty, and every tab keeps draining
//! while it is in the background. Keys go through the shared `input` rules, the
//! wheel scrolls history unless a tracking program owns it, and a chord the
//! settings file binds is the window's before the shell sees it.
//!
//! `WINTER_SCREENSHOT=<path>` writes the toplevel exactly as GTK draws it —
//! client-side decorations and the tab strip included — and quits, which is how
//! a machine with no display server photographs the shell.
//!
//! The profile menu, the find bar and the settings window were never built:
//! ADR 062 parks this shell at its current state, and Linux ships the egui
//! one. What is here is kept as a working reference for its traps.

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
    EventControllerScrollFlags, EventSequenceState, GestureClick, Snapshot, WidgetPaintable, gdk, glib,
};

use renderer::renderer::Renderer;
use renderer::{
    AtlasEngine, CellSizeInDIP, FontDueRasterizer, FontId, FontInfo, GlyphBitmap, GlyphRasterizer, RenderSettings,
    Settings, SharedBackend, SoftwareBackend,
};
use server::pty::PtySize;
use term::input::{Key, MouseEvent, WHEEL_DELTA, WheelAxis};
use utils::{InclusiveRect, Size};

use crate::input::{self, Event as InputEvent, ModifierSides, Modifiers};
use crate::session::Session;
use crate::tabs::{self, StripHit, TabBox, Tabs};
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
/// The height the strip asks for; the header bar may give it more.
const STRIP_HEIGHT: i32 = 34;
/// The space between a tab's left edge and its title.
const TITLE_INSET: i32 = 12;
/// The tab's corner radius.
const RADIUS: f64 = 6.0;
/// The close and new-tab glyphs' size.
const GLYPH: f64 = 9.0;
/// What an elided title ends with.
const ELLIPSIS: char = '…';

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

    let document = settings::load();
    let template = TerminalSettings::resolve(&document);
    let first = match Tab::spawn(&template, CELLS) {
        Ok(tab) => tab,
        Err(error) => {
            eprintln!("winter: cannot start the shell: {error}");
            app.quit();
            return;
        }
    };
    let bindings = document.keybindings.clone().into();

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
        .title(&template.title)
        .default_width(i32::from(fonts.cell.0) * CELLS.0)
        .default_height(i32::from(fonts.cell.1) * CELLS.1)
        .build();

    // The title bar is the tab strip (ADRs 051, 061). A header bar centres its
    // title widget, so the bar is built from the handle and the window's own
    // controls instead: the strip fills it, tabs start at the left, and the
    // buttons follow the desktop's decoration layout.
    let strip = DrawingArea::new();
    strip.set_hexpand(true);
    strip.set_height_request(STRIP_HEIGHT);

    let handle = gtk::WindowHandle::new();
    handle.set_hexpand(true);
    handle.set_child(Some(&strip));

    let bar = gtk::Box::new(gtk::Orientation::Horizontal, 0);
    bar.add_css_class("titlebar");
    bar.append(&gtk::WindowControls::new(gtk::PackType::Start));
    bar.append(&handle);
    bar.append(&gtk::WindowControls::new(gtk::PackType::End));
    window.set_titlebar(Some(&bar));

    let area = DrawingArea::new();
    // The pane takes the keyboard, which is how a terminal behaves; a click
    // asks for it back when a dialog or a menu took it away.
    area.set_can_focus(true);
    window.set_child(Some(&area));

    let menu = gtk::PopoverMenu::from_model(Some(&profile_menu(&document)));

    let shell = Rc::new(RefCell::new(Shell {
        window: window.clone(),
        area: area.clone(),
        strip: strip.clone(),
        renderer,
        shared,
        tabs: Tabs::new(first),
        template,
        document,
        bindings,
        menu,
        chrome: fonts.chrome,
        cell: fonts.cell,
        grid: CELLS,
        pointer: (0, 0),
        wheel_pixels: 0.0,
        hover: None,
        window_title: String::new(),
        screenshot,
    }));

    {
        let action = gtk::gio::SimpleAction::new("open", Some(glib::VariantTy::STRING));
        let open = Rc::clone(&shell);
        action.connect_activate(move |_action, parameter| {
            let name = parameter.and_then(|value| value.str().map(str::to_string));
            open.borrow_mut().open_profile(name.as_deref());
        });
        let group = gtk::gio::SimpleActionGroup::new();
        group.add_action(&action);
        strip.insert_action_group("profiles", Some(&group));

        // The popover lives as long as the strip it hangs under.
        shell.borrow().menu.set_parent(&strip);
    }

    {
        let shell = Rc::clone(&shell);
        area.set_draw_func(move |_area, context, width, height| {
            // A draw can be asked for while a callback still holds the shell --
            // a popover grab is one such moment -- and a frame that waits for
            // the next tick is better than an abort.
            if let Ok(mut shell) = shell.try_borrow_mut() {
                shell.paint(context, width, height);
            }
        });
    }

    {
        let shell = Rc::clone(&shell);
        strip.set_draw_func(move |_strip, context, width, height| {
            if let Ok(mut shell) = shell.try_borrow_mut() {
                shell.paint_strip(context, width, height);
            }
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
        // A press on the pane takes the keyboard back from a tab, a menu or a
        // dialog that had it.
        let pane = area.clone();
        let click = GestureClick::new();
        click.set_button(0);
        click.connect_pressed(move |_gesture, _presses, _x, _y| {
            pane.grab_focus();
        });
        area.add_controller(click);
    }

    {
        let shell = Rc::clone(&shell);
        let click = GestureClick::new();
        // Every button, because the middle click is how a tab closes.
        click.set_button(0);
        let menu = shell.borrow().menu.clone();
        let tall = strip.clone();
        click.connect_pressed(move |gesture, _presses, x, _y| {
            let (claimed, popup) = shell.borrow_mut().press_strip(x, gesture.current_button());
            // Claiming is per gesture, not per event sequence: a mouse press has
            // no sequence, and GTK's `GtkWindowHandle` claims every primary
            // double press to toggle maximize, so a press the strip does not
            // claim becomes a title-bar gesture even over a button.
            gesture.set_state(if claimed {
                EventSequenceState::Claimed
            } else {
                EventSequenceState::Denied
            });

            // Popping the menu up emits `leave` on the strip, which borrows the
            // shell again: it must happen after the borrow above has ended.
            if let Some(slots) = popup {
                let below = gdk::Rectangle::new(slots.left, tall.height(), slots.width(), 1);
                menu.set_pointing_to(Some(&below));
                menu.popup();
            }
        });
        strip.add_controller(click);
    }

    {
        // The strip shows what is under the pointer, so a tab or a button that
        // is about to be pressed looks alive. A hover change the shell cannot
        // take right now is a repaint that waits for the next motion.
        let motion = EventControllerMotion::new();
        let over = Rc::clone(&shell);
        motion.connect_motion(move |_controller, x, _y| {
            if let Ok(mut shell) = over.try_borrow_mut() {
                shell.hover(Some(x));
            }
        });
        let leave = Rc::clone(&shell);
        motion.connect_leave(move |_controller| {
            if let Ok(mut shell) = leave.try_borrow_mut() {
                shell.hover(None);
            }
        });
        strip.add_controller(motion);
    }

    {
        // A themed row has a colour for each focus state.
        let strip = strip.clone();
        window.connect_notify_local(Some("is-active"), move |_window, _property| strip.queue_draw());
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

/// One tab: its terminal, its viewport and the title the strip shows.
struct Tab {
    /// The terminal and the pty behind it.
    control: Control,
    /// Where the viewport sits in the scrollback.
    view: View,
    /// The modifier sides the toolkit collapses but the encoder distinguishes.
    sides: ModifierSides,
    /// The program's OSC title, or the profile's name until it sets one.
    title: String,
}

impl Tab {
    /// Starts the profile's shell on a pty.
    fn spawn(template: &TerminalSettings, cells: (i32, i32)) -> std::io::Result<Self> {
        let session = Session::spawn(&template.command, pty_size(cells))?;
        Ok(Self {
            control: Control::new(Box::new(session), cells),
            view: View::new(),
            sides: ModifierSides::default(),
            title: template.title.clone(),
        })
    }
}

/// The pty's size, in the units it takes.
fn pty_size(cells: (i32, i32)) -> PtySize {
    PtySize::new(
        u16::try_from(cells.0.max(1)).unwrap_or(u16::MAX),
        u16::try_from(cells.1.max(1)).unwrap_or(u16::MAX),
    )
}

/// The window, the frame it paints and the tabs behind it.
struct Shell {
    window: ApplicationWindow,
    area: DrawingArea,
    strip: DrawingArea,
    renderer: Renderer<TerminalData>,
    shared: SharedBackend,
    tabs: Tabs<Tab>,
    /// The profile a new tab starts from.
    template: TerminalSettings,
    /// The settings file, for the strip's colours.
    document: settings::Settings,
    /// The chords the window matches before the terminal sees a key.
    bindings: Rc<[settings::Keybinding]>,
    /// The profile menu the strip's button pops up.
    menu: gtk::PopoverMenu,
    /// The face the strip's titles are drawn with.
    chrome: FontDueRasterizer,
    /// The cell size in physical pixels.
    cell: (u16, u16),
    /// The grid the renderer's target currently holds.
    grid: (i32, i32),
    /// Where the pointer last was, in cells, for a tracking program's wheel.
    pointer: (i32, i32),
    /// Wheel pixels that have not yet made a row: a touchpad reports pixels
    /// where a wheel reports whole detents.
    wheel_pixels: f64,
    /// Where the pointer is over the strip, so the element under it can show
    /// that it is pressable.
    hover: Option<i32>,
    /// The title the window last showed, so it is not rewritten every tick.
    window_title: String,
    /// Where `WINTER_SCREENSHOT` asked for a picture, if it did.
    screenshot: Option<PathBuf>,
}

impl Shell {
    /// Drains every tab, closes the ones whose shell exited and repaints.
    ///
    /// An inactive tab keeps draining: a program that fills its pty buffer
    /// blocks on the write, and a title changes while its tab is in the
    /// background.
    fn pump(&mut self, app: &Application) {
        let now = Instant::now();
        let mut exited = Vec::new();
        let mut titled = false;

        for index in 0..self.tabs.len() {
            let Some(tab) = self.tabs.get_mut(index) else {
                continue;
            };
            tab.control.pump();

            if let Some(title) = tab.control.title()
                && tab.title != title
            {
                tab.title = title.to_string();
                titled = true;
            }

            for event in tab.control.take_events(now) {
                if matches!(event, TerminalEvent::Exited) {
                    exited.push(index);
                }
            }
        }

        if !exited.is_empty() {
            // From the back, so an index still names the tab it came from.
            for index in exited.into_iter().rev() {
                self.close_tab(index);
            }
            if self.tabs.is_empty() {
                app.quit();
                return;
            }
        }

        if titled {
            self.after_select();
        } else {
            self.area.queue_draw();
        }
    }

    /// Turns one toolkit key into a window command or what the terminal gets.
    fn key(&mut self, key: gdk::Key, state: gdk::ModifierType, pressed: bool) -> glib::Propagation {
        let modifiers = modifiers(state);
        let Some(named) = key_of(key) else {
            return glib::Propagation::Proceed;
        };

        // A bound chord is the window's: it is consumed before the tab sees it
        // (ADR 023). An action this shell cannot perform is declined, so the
        // key still reaches the terminal -- ctrl+c must interrupt even though
        // the settings bind it to a copy that has nothing to copy (ADR 053).
        if pressed
            && let Some(action) = input::chord_action(&self.bindings, named, modifiers)
            && self.command(action)
        {
            return glib::Propagation::Stop;
        }

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

        let Some(tab) = self.tabs.active_mut() else {
            return glib::Propagation::Stop;
        };
        let win32 = tab.control.input_modes().win32_input;
        let outcome = input::translate(&event, &mut tab.sides, win32);
        for key in outcome.keys {
            tab.control.send_key(&key);
        }
        if outcome.activity {
            tab.view.scroll_to_bottom();
        }

        glib::Propagation::Stop
    }

    /// Runs a window action the tab strip can perform; declines the rest.
    fn command(&mut self, action: settings::Action) -> bool {
        match action {
            settings::Action::NewTab => self.open_tab(None),
            settings::Action::CloseTab => self.close_tab(self.tabs.index()),
            settings::Action::NextTab => {
                self.tabs.step(1);
                self.focus_active();
            }
            settings::Action::PrevTab => {
                self.tabs.step(-1);
                self.focus_active();
            }
            _ => return false,
        }
        true
    }

    /// Opens the profile's shell in a new tab and activates it.
    fn open_tab(&mut self, profile: Option<&settings::Profile>) {
        let template = match profile {
            Some(profile) => TerminalSettings::for_profile(&self.document, Some(profile)),
            None => self.template.clone(),
        };

        match Tab::spawn(&template, self.grid) {
            Ok(tab) => {
                self.tabs.add(tab);
                self.focus_active();
            }
            Err(error) => eprintln!("winter: cannot start the shell: {error}"),
        }
    }

    /// Closes the tab at `index`, or the window when it was the last one.
    fn close_tab(&mut self, index: usize) {
        if !self.tabs.close(index) {
            self.window.close();
            return;
        }
        self.after_select();
    }

    /// Opens the named profile in a new tab, or the default one when the name
    /// is unknown.
    fn open_profile(&mut self, name: Option<&str>) {
        let profile = name
            .and_then(|name| self.document.profiles.iter().find(|profile| profile.name == name))
            .cloned();
        self.open_tab(profile.as_ref());
    }

    /// Remembers the pointer's cell for the wheel a tracking program receives.
    /// Handles a press on the strip.
    ///
    /// Returns whether the strip took it, and the box to pop the profile menu
    /// under when that was the press. A press the strip declines belongs to the
    /// title bar, which drags the window with it and offers the window menu on
    /// the right button.
    fn press_strip(&mut self, x: f64, button: u32) -> (bool, Option<TabBox>) {
        let (layout, menu) = strip_and_menu(self.strip.width(), self.tabs.len());
        if menu.contains(x as i32) {
            return (button == 1, (button == 1).then_some(menu));
        }

        let Some(hit) = tabs::strip_hit(&layout, x as i32) else {
            return (false, None);
        };

        match hit {
            StripHit::New if button == 1 => self.open_tab(None),
            StripHit::Tab(index) if button == 1 => {
                self.tabs.select(index);
                self.focus_active();
            }
            // Middle-click closes a tab, the way upstream's tabs do.
            StripHit::Tab(index) | StripHit::Close(index) if button == 2 => self.close_tab(index),
            StripHit::Close(index) if button == 1 => self.close_tab(index),
            StripHit::Tab(_) | StripHit::Close(_) | StripHit::New => return (false, None),
        }

        (true, None)
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

        let Some(tab) = self.tabs.active_mut() else {
            return glib::Propagation::Proceed;
        };

        // Shift keeps the wheel for the viewport; otherwise a tracking program
        // owns it, the same rule the egui and WinUI shells apply.
        let forwards = input::forwards_to_application(tab.control.is_tracking_mouse(), shift);
        let pointer = (
            self.pointer.0.clamp(0, self.grid.0.max(1) - 1),
            self.pointer.1.clamp(0, self.grid.1.max(1) - 1),
        );
        let wheel = MouseEvent::wheel(pointer, WheelAxis::Vertical, rows * WHEEL_DELTA);
        if !(forwards && tab.control.send_mouse(&wheel)) {
            let max = tab.control.max_scroll();
            tab.view.scroll_by(rows, max);
        }

        glib::Propagation::Stop
    }

    /// Fits the grid to the area and paints one frame.
    fn paint(&mut self, context: &cairo::Context, width: i32, height: i32) {
        self.fit(width, height);

        let Some(tab) = self.tabs.active_mut() else {
            return;
        };
        self.renderer
            .data_mut()
            .refresh(&mut tab.control, &tab.view, true, None);

        let (background, foreground) = tab.control.default_colors();
        let _ = self.renderer.set_default_colors(background, foreground);
        if let Ok(table) = tab.control.color_table().lock() {
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

    /// Paints the strip: the row's fill, a box per tab and the new-tab button.
    fn paint_strip(&mut self, context: &cairo::Context, width: i32, height: i32) {
        let colors = self.strip_colors();

        // A theme that names no row colour leaves GTK's own header bar showing.
        if let Some(row) = colors.row {
            set_rgb(context, row);
            context.paint().ok();
        }

        let (layout, menu) = strip_and_menu(width, self.tabs.len());
        let active = self.tabs.index();
        let baseline = self.title_baseline(height);

        for (index, slots) in layout.tabs.iter().enumerate() {
            let Some(tab) = self.tabs.get(index) else {
                continue;
            };
            let (mut fill, text) = if index == active {
                (colors.active, colors.active_text)
            } else {
                (colors.inactive, colors.inactive_text)
            };
            if self.pointer_is_over(*slots) && index != active {
                fill = hovered(fill);
            }

            rounded_top(context, *slots, height, fill);

            let title_width = slots.width() - tabs::CLOSE - TITLE_INSET;
            if title_width > 0 {
                draw_text(
                    context,
                    &mut self.chrome,
                    &tab.title,
                    f64::from(slots.left + TITLE_INSET),
                    baseline,
                    f64::from(title_width),
                    text,
                );
            }

            let close = f64::from(slots.right - tabs::CLOSE / 2);
            glyph_cross(context, close, f64::from(height) / 2.0, text);
        }

        // The two buttons are one group, painted last so an overflowing strip
        // cannot cover them; a hairline splits them so they read as one block.
        let group = TabBox {
            left: layout.new_tab.left,
            right: menu.right,
        };
        let base = colors.row.unwrap_or(colors.active);
        let fill = if self.pointer_is_over(group) {
            hovered(base)
        } else {
            base
        };
        rounded_top(context, group, height, fill);

        set_rgb(context, blend(fill, contrasting(fill), 0.3));
        context.set_line_width(1.0);
        let split = f64::from(layout.new_tab.right) + 0.5;
        context.move_to(split, f64::from(height) * 0.3);
        context.line_to(split, f64::from(height) * 0.7);
        context.stroke().ok();

        let text = contrasting(fill);
        glyph_plus(context, centre(layout.new_tab), f64::from(height) / 2.0, text);
        glyph_chevron(context, centre(menu), f64::from(height) / 2.0, text);
    }

    /// Whether the pointer sits inside `slots`.
    fn pointer_is_over(&self, slots: TabBox) -> bool {
        self.hover.is_some_and(|x| slots.contains(x))
    }

    /// Remembers where the pointer is over the strip, or that it left.
    fn hover(&mut self, x: Option<f64>) {
        let x = x.map(|x| x as i32);
        if self.hover != x {
            self.hover = x;
            self.strip.queue_draw();
        }
    }

    /// The colours the strip paints with.
    ///
    /// A theme names fills and no text colour, so each label picks the side
    /// that reads on its own tab. Without a theme the row keeps the header
    /// bar's own background and the tabs sit on the pane's colour, which is
    /// what makes the active tab merge with the terminal.
    fn strip_colors(&self) -> StripColors {
        let (background, foreground) = self
            .tabs
            .active()
            .map(|tab| tab.control.default_colors())
            .unwrap_or((0x000C_0C0C, 0x00CC_CCCC));
        let pane = channels(background);
        let text = channels(foreground);

        let theme = self.document.theme();
        let row = theme
            .and_then(|theme| theme.tab_row_background(self.window.is_active()))
            .map(color_channels);
        let active = theme
            .and_then(|theme| theme.tab_background(true))
            .map(color_channels)
            .unwrap_or(pane);
        let inactive = theme
            .and_then(|theme| theme.tab_background(false))
            .map(color_channels)
            .unwrap_or_else(|| blend(pane, text, 0.08));

        StripColors {
            row,
            active,
            inactive,
            active_text: contrasting(active),
            inactive_text: contrasting(inactive),
        }
    }

    /// Where the title's baseline sits in a strip `height` tall.
    fn title_baseline(&self, height: i32) -> f64 {
        let (ascent, descent) = self.chrome.vertical_metrics();
        let box_height = ascent + descent;
        // A short strip keeps the text on it rather than centred off the top.
        let top = ((f64::from(height) - f64::from(box_height)) / 2.0).max(0.0);
        top + f64::from(ascent)
    }

    /// Fits the terminal and the renderer to whole cells.
    fn fit(&mut self, width: i32, height: i32) {
        let columns = (width / i32::from(self.cell.0)).max(1);
        let rows = (height / i32::from(self.cell.1)).max(1);
        if (columns, rows) == self.grid {
            return;
        }

        self.grid = (columns, rows);
        for index in 0..self.tabs.len() {
            if let Some(tab) = self.tabs.get_mut(index) {
                tab.control.resize(columns, rows);
            }
        }

        let target = (
            u16::try_from(i32::from(self.cell.0) * columns).unwrap_or(u16::MAX),
            u16::try_from(i32::from(self.cell.1) * rows).unwrap_or(u16::MAX),
        );
        let _ = self
            .renderer
            .resize(target, InclusiveRect::new(0, 0, columns - 1, rows - 1));
    }

    /// Shows the active tab's title and asks both widgets to repaint.
    fn after_select(&mut self) {
        let title = self.tabs.active().map(|tab| tab.title.clone()).unwrap_or_default();
        if title != self.window_title {
            self.window.set_title(Some(&title));
            self.window_title = title;
        }
        self.strip.queue_draw();
        self.area.queue_draw();
    }

    /// Gives the keyboard back to the active pane, then repaints the chrome.
    ///
    /// A press on the strip or a chord that switches tabs moves the focus away
    /// from the pane; without this, the next keystroke would go nowhere.
    fn focus_active(&mut self) {
        self.area.grab_focus();
        self.after_select();
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

/// The strip's fills and the two label colours.
struct StripColors {
    /// The row behind the tabs, or `None` to leave the header bar's own.
    row: Option<(f64, f64, f64)>,
    /// The active tab's fill.
    active: (f64, f64, f64),
    /// Every other tab's fill.
    inactive: (f64, f64, f64),
    /// The active tab's title.
    active_text: (f64, f64, f64),
    /// Every other tab's title.
    inactive_text: (f64, f64, f64),
}

/// A fill lifted towards its own text colour, for the element under the pointer.
fn hovered(fill: (f64, f64, f64)) -> (f64, f64, f64) {
    blend(fill, contrasting(fill), 0.15)
}

/// Fills a tab's box with rounded top corners.
fn rounded_top(context: &cairo::Context, slots: TabBox, height: i32, fill: (f64, f64, f64)) {
    let left = f64::from(slots.left);
    let right = f64::from(slots.right);
    let bottom = f64::from(height);
    let radius = RADIUS.min(f64::from(slots.width()) / 2.0).min(bottom);

    set_rgb(context, fill);
    context.new_path();
    context.move_to(left, bottom);
    context.line_to(left, radius);
    context.arc(
        left + radius,
        radius,
        radius,
        std::f64::consts::PI,
        1.5 * std::f64::consts::PI,
    );
    context.line_to(right - radius, 0.0);
    context.arc(
        right - radius,
        radius,
        radius,
        1.5 * std::f64::consts::PI,
        2.0 * std::f64::consts::PI,
    );
    context.line_to(right, bottom);
    context.close_path();
    context.fill().ok();
}

/// The strip's tabs and its two buttons: the tab row with its new-tab button,
/// and the profile menu's box after them.
fn strip_and_menu(width: i32, count: usize) -> (tabs::StripLayout, TabBox) {
    let layout = tabs::strip_layout(width - tabs::NEW_TAB, count);
    let left = layout.new_tab.right;
    (
        layout,
        TabBox {
            left,
            right: left + tabs::NEW_TAB,
        },
    )
}

/// A box's horizontal centre.
fn centre(slots: TabBox) -> f64 {
    f64::from(slots.left) + f64::from(slots.width()) / 2.0
}

/// The profile list the strip's button pops up.
fn profile_menu(document: &settings::Settings) -> gtk::gio::Menu {
    let menu = gtk::gio::Menu::new();
    for profile in &document.profiles {
        let item = gtk::gio::MenuItem::new(Some(&profile.name), None);
        item.set_action_and_target_value(Some("profiles.open"), Some(&profile.name.to_variant()));
        menu.append_item(&item);
    }
    menu
}

/// Draws the profile menu's chevron.
fn glyph_chevron(context: &cairo::Context, x: f64, y: f64, color: (f64, f64, f64)) {
    let half = GLYPH / 2.0;
    set_rgb(context, color);
    context.set_line_width(1.5);
    context.move_to(x - half, y - half / 2.0);
    context.line_to(x, y + half / 2.0);
    context.line_to(x + half, y - half / 2.0);
    context.stroke().ok();
}

/// Draws a tab's close box.
fn glyph_cross(context: &cairo::Context, x: f64, y: f64, color: (f64, f64, f64)) {
    let half = GLYPH / 2.0;
    set_rgb(context, color);
    context.set_line_width(1.5);
    context.move_to(x - half, y - half);
    context.line_to(x + half, y + half);
    context.move_to(x + half, y - half);
    context.line_to(x - half, y + half);
    context.stroke().ok();
}

/// Draws the new-tab button's glyph.
fn glyph_plus(context: &cairo::Context, x: f64, y: f64, color: (f64, f64, f64)) {
    let half = GLYPH / 2.0;
    set_rgb(context, color);
    context.set_line_width(1.5);
    context.move_to(x - half, y);
    context.line_to(x + half, y);
    context.move_to(x, y - half);
    context.line_to(x, y + half);
    context.stroke().ok();
}

/// Draws `text` on one line, elided to `width`, in `color`.
///
/// The glyphs come from the same face the grid is drawn with, so the strip
/// follows the profile's font. Cairo is given the coverage as an `A8` mask,
/// which paints the text in `color` without touching the pixels around it.
fn draw_text(
    context: &cairo::Context,
    rasterizer: &mut FontDueRasterizer,
    text: &str,
    left: f64,
    baseline: f64,
    width: f64,
    color: (f64, f64, f64),
) {
    let shown = elide(rasterizer, text, width);
    if shown.is_empty() {
        return;
    }

    let (ascent, descent) = rasterizer.vertical_metrics();
    let ascent_rows = ascent.ceil().max(1.0) as i32;
    let line = ascent_rows + descent.ceil().max(0.0) as i32;

    let mut pen = 0.0f64;
    let mut placed = Vec::new();
    for character in shown.chars() {
        placed.push((character, pen));
        pen += f64::from(rasterizer.advance_width(character));
    }
    let mask_width = pen.ceil().max(1.0) as i32;
    // Cairo wants a stride that is a multiple of four bytes for every format.
    let stride = (mask_width + 3) & !3;

    let mut mask = vec![0u8; stride as usize * line as usize];
    for (character, x) in placed {
        let index = rasterizer.glyph_index(character);
        let Some(bitmap) = rasterizer.rasterize(FontId(0), index) else {
            continue;
        };
        blit(
            &mut mask,
            stride,
            mask_width,
            line,
            x.round() as i32 + bitmap.left,
            ascent_rows + bitmap.top,
            &bitmap,
        );
    }

    let Ok(surface) = cairo::ImageSurface::create_for_data(mask, cairo::Format::A8, mask_width, line, stride) else {
        return;
    };
    set_rgb(context, color);
    context
        .mask_surface(&surface, left, baseline - f64::from(ascent_rows))
        .ok();
}

/// Composites one glyph's coverage into a mask at `(x, y)`.
///
/// `stride` is the mask's row length in bytes, which Cairo pads; `width` is the
/// part of a row that holds pixels. The mask keeps the stronger coverage where
/// two glyphs overlap, so a kerning pair does not darken itself.
fn blit(mask: &mut [u8], stride: i32, width: i32, height: i32, x: i32, y: i32, bitmap: &GlyphBitmap) {
    let bitmap_width = i32::try_from(bitmap.width).unwrap_or(0);
    let bitmap_height = i32::try_from(bitmap.height).unwrap_or(0);

    for row in 0..bitmap_height {
        let destination_y = y + row;
        if !(0..height).contains(&destination_y) {
            continue;
        }

        let source_row = row as usize * bitmap_width as usize;
        for column in 0..bitmap_width {
            let destination_x = x + column;
            if !(0..width).contains(&destination_x) {
                continue;
            }

            let coverage = bitmap.coverage[source_row + column as usize];
            let slot = &mut mask[destination_y as usize * stride as usize + destination_x as usize];
            *slot = (*slot).max(coverage);
        }
    }
}

/// The text, cut and ended with an ellipsis so it fits `width`.
fn elide(rasterizer: &FontDueRasterizer, text: &str, width: f64) -> String {
    let advances: Vec<f64> = text
        .chars()
        .map(|character| f64::from(rasterizer.advance_width(character)))
        .collect();
    let ellipsis = f64::from(rasterizer.advance_width(ELLIPSIS));

    match elided_length(&advances, width, ellipsis) {
        None => text.to_string(),
        Some(0) if ellipsis <= width => ELLIPSIS.to_string(),
        Some(0) => String::new(),
        Some(count) => text.chars().take(count).chain(std::iter::once(ELLIPSIS)).collect(),
    }
}

/// How many characters fit in `width`, or `None` when they all do.
///
/// A cut always leaves room for the ellipsis, so the result is `Some`.
fn elided_length(advances: &[f64], width: f64, ellipsis: f64) -> Option<usize> {
    if advances.iter().sum::<f64>() <= width {
        return None;
    }

    let mut budget = width - ellipsis;
    let mut taken = 0;
    for advance in advances {
        if *advance > budget {
            break;
        }
        budget -= advance;
        taken += 1;
    }
    Some(taken)
}

/// Sets the drawing colour from three components.
fn set_rgb(context: &cairo::Context, color: (f64, f64, f64)) {
    context.set_source_rgb(color.0, color.1, color.2);
}

/// A `settings` colour as its three components.
fn color_channels(color: settings::Color) -> (f64, f64, f64) {
    let color = color.text_color();
    (
        f64::from(color.get_r()) / 255.0,
        f64::from(color.get_g()) / 255.0,
        f64::from(color.get_b()) / 255.0,
    )
}

/// Mixes two colours, `t` of the way from `from` to `to`.
fn blend(from: (f64, f64, f64), to: (f64, f64, f64), t: f64) -> (f64, f64, f64) {
    (
        from.0 * (1.0 - t) + to.0 * t,
        from.1 * (1.0 - t) + to.1 * t,
        from.2 * (1.0 - t) + to.2 * t,
    )
}

/// Black or grey-white, whichever reads on `fill`.
///
/// The weights are the usual luminance ones and the cut-off is where a mid grey
/// stops reading as dark; `terminal_widget::contrasting_text` uses the same rule
/// for the egui strip.
fn contrasting(fill: (f64, f64, f64)) -> (f64, f64, f64) {
    let luminance = 0.299 * fill.0 * 255.0 + 0.587 * fill.1 * 255.0 + 0.114 * fill.2 * 255.0;
    if luminance > 140.0 {
        (0.08, 0.08, 0.08)
    } else {
        (0.9, 0.9, 0.9)
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

/// The face, its metrics and the rasterisers a renderer and the strip want.
struct Fonts {
    info: FontInfo,
    settings: Settings,
    shaper: FontDueRasterizer,
    rasterizer: FontDueRasterizer,
    chrome: FontDueRasterizer,
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
    // The strip's titles use the same face, so a tab reads like its own text.
    let chrome = FontDueRasterizer::from_file(face.path(), size * 0.85).ok()?;

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
        chrome,
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

    #[test]
    fn a_title_that_fits_is_not_cut() {
        assert_eq!(elided_length(&[10.0, 10.0, 10.0], 100.0, 10.0), None);
        assert_eq!(elided_length(&[10.0, 10.0, 10.0], 30.0, 10.0), None);
    }

    #[test]
    fn a_cut_title_leaves_room_for_the_ellipsis() {
        // 30 of text in 25 of room: one character, then the ellipsis.
        assert_eq!(elided_length(&[10.0, 10.0, 10.0], 25.0, 10.0), Some(1));
        // Too narrow for anything but the ellipsis itself.
        assert_eq!(elided_length(&[10.0, 10.0], 5.0, 10.0), Some(0));
    }

    #[test]
    fn the_strip_text_contrasts_with_its_tab() {
        let dark = contrasting((0.05, 0.05, 0.05));
        assert!(dark.0 > 0.5, "a dark tab takes light text");
        let light = contrasting((0.95, 0.95, 0.95));
        assert!(light.0 < 0.5, "a light tab takes dark text");
    }

    #[test]
    fn a_blend_reaches_both_ends() {
        let from = (0.0, 0.5, 1.0);
        let to = (1.0, 0.0, 0.0);
        assert_eq!(blend(from, to, 0.0), from);
        assert_eq!(blend(from, to, 1.0), to);
    }

    #[test]
    fn the_menu_button_sits_after_the_new_tab_button() {
        let (layout, menu) = strip_and_menu(900, 2);

        assert_eq!(menu.left, layout.new_tab.right);
        assert_eq!(menu.width(), tabs::NEW_TAB);
        assert!(menu.right <= 900);
    }

    #[test]
    fn the_menu_button_stays_on_an_overflowing_strip() {
        let (_, menu) = strip_and_menu(300, 8);

        assert_eq!(menu.right, 300);
    }

    #[test]
    fn the_profile_menu_names_every_profile() {
        let document = settings::parse(
            r##"
                [[profiles]]
                name = "one"
                command = "sh"

                [[profiles]]
                name = "two"
                command = "sh"
            "##,
        )
        .expect("settings");

        let menu = profile_menu(&document);
        assert_eq!(menu.n_items(), 2);

        let label = menu.item_attribute_value(0, "label", None);
        assert_eq!(
            label.and_then(|value| value.str().map(str::to_string)).as_deref(),
            Some("one")
        );
        let parameter = menu.item_attribute_value(1, "target", None);
        assert_eq!(
            parameter.and_then(|value| value.str().map(str::to_string)).as_deref(),
            Some("two")
        );
    }
}
