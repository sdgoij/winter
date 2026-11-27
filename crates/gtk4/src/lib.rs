#![cfg(target_os = "linux")]
//! The GTK 4 shell: a native window over the renderer's pixels (ADR 061).
//!
//! A `GtkHeaderBar` holds the tab strip, which this module paints itself; below
//! it a base `GtkDrawingArea` paints the pane's fill, scrollbar and margins, and
//! a `GtkGLArea` inset by those margins draws the grid through GTK's OpenGL
//! context (ADR 076), with the software blit as the fallback. The terminal is
//! the shared `Control` over a pty, and every tab keeps draining while it is in
//! the background. Keys go through the shared `input` rules, the wheel scrolls
//! history unless a tracking program owns it, and a chord the settings file
//! binds is the window's before the shell sees it.
//!
//! `WINTER_SCREENSHOT=<path>` writes the toplevel exactly as GTK draws it —
//! client-side decorations and the tab strip included — and quits, which is how
//! a machine with no display server photographs the shell.
//!
//! The profile menu is a native `GtkPopoverMenu`. A saved settings file is
//! reloaded within a second, a bell flashes the pane and a program's OSC 777
//! becomes a desktop notification. The GPU path is the default when the host
//! has GL; `WINTER_SOFTWARE=1` or a failed context selects the software blit.

use std::cell::{Cell, RefCell};
use std::path::{Path, PathBuf};
use std::rc::{Rc, Weak};
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
    AtlasEngine, CellSizeInDIP, Composition, FontDueRasterizer, FontId, FontInfo, GlyphBitmap, GlyphRasterizer,
    IBackend, RenderSettings, Settings, SharedBackend, SoftwareBackend,
};
use server::pty::PtySize;
use term::input::{Key, MouseButton, MouseButtons, MouseEvent, MouseEventKind, WHEEL_DELTA, WheelAxis};
use utils::{InclusiveRect, Size};

use winter::about;
use winter::find::FindBar;
use winter::input::{self, Event as InputEvent, ModifierSides, Modifiers};
use winter::notifications;
use winter::palette::{self, Palette};
use winter::scrollbar::{self, Press, Scrollbar};
use winter::session::Session;
use winter::tabs::{self, StripHit, TabBox, Tabs};
use winter::term_control::Control;
use winter::term_events::TerminalEvent;
use winter::term_settings::TerminalSettings;
use winter::terminal_render::TerminalData;
use winter::view::View;

mod chrome;
mod gl;
mod icons;

use chrome::Chrome;
use gl::Gl;
use icons::Icons;

/// The window opens at this many cells, like the WinUI shell.
const CELLS: (i32, i32) = (100, 28);
/// The terminal text's left inset, matching the WinUI and macOS panes (ADR 068).
const PANE_LEFT: f64 = 8.0;
/// How often the pty is polled, matching the WinUI shell's timer (ADR 006).
const TICK: Duration = Duration::from_millis(16);
/// The cursor's blink half-period, matching the macOS and WinUI shells.
const BLINK: Duration = Duration::from_millis(500);
/// The height the strip asks for, matching the Breeze decoration's title bar.
const STRIP_HEIGHT: i32 = 28;
/// The space between a tab's left edge and its title.
const TITLE_INSET: i32 = 10;
/// The tab's corner radius, matching Breeze's frame radius.
const RADIUS: f64 = 5.0;
/// The close and new-tab glyphs' size.
const GLYPH: f64 = 9.0;
/// What an elided title ends with.
const ELLIPSIS: char = '…';
/// The title bar's own layout: Breeze pads the row and gives its window
/// controls 20-pixel buttons, while a GTK header bar pads for a 24-pixel one.
const CHROME_CSS: &str = r#"
.winter-titlebar {
    padding: 0;
    margin: 0;
    min-height: 28px;
    border: none;
    border-radius: 0;
    box-shadow: none;
    background-image: none;
}
.winter-titlebar windowhandle {
    padding: 0;
    margin: 0;
}
.winter-titlebar windowcontrols {
    padding: 0;
    margin: 0;
    border-spacing: 2px;
}
.winter-titlebar windowcontrols button {
    min-width: 20px;
    min-height: 20px;
    margin: 0 2px;
    padding: 0;
    border-radius: 4px;
    background: none;
    box-shadow: none;
}
.winter-titlebar windowcontrols button > image {
    min-width: 16px;
    min-height: 16px;
    padding: 0;
    border-radius: 0;
    background: none;
    box-shadow: none;
}
.winter-titlebar windowcontrols button:hover {
    background-color: alpha(currentColor, 0.12);
}
.winter-titlebar windowcontrols button:hover > image {
    background: none;
    box-shadow: none;
}
.winter-titlebar windowcontrols button.close:hover {
    background-color: #da4453;
    color: #ffffff;
}
.winter-palette {
    padding: 8px;
    border-radius: 8px;
}
.winter-palette .palette-row {
    padding: 4px 8px;
    border-radius: 4px;
}
.winter-palette .palette-row.selected {
    background-color: alpha(currentColor, 0.18);
}
"#;

/// The title bar's stylesheet, with the desktop's palette appended.
///
/// The window controls sit beside the strip rather than inside it, so the row
/// colour has to reach the whole title bar; `:backdrop` carries the unfocused
/// palette that KDE publishes alongside the focused one.
fn chrome_css(palette: Option<Chrome>) -> String {
    let mut css = String::from(CHROME_CSS);
    if let Some(chrome) = palette {
        css.push_str(&format!(
            ".winter-titlebar {{ background-image: none; background-color: {}; color: {}; }}\n\
             .winter-titlebar:backdrop {{ background-image: none; background-color: {}; color: {}; }}\n",
            hex(chrome.titlebar),
            hex(chrome.foreground),
            hex(chrome.titlebar_inactive),
            hex(chrome.foreground_inactive),
        ));
        // The palette is a card over the pane, so it takes the desktop's window
        // colour and its selection colour for the chosen row.
        css.push_str(&format!(
            ".winter-palette {{ background-image: none; background-color: {}; color: {}; border: 1px solid {}; }}\n\
             .winter-palette .palette-row.selected {{ background-image: none; background-color: {}; color: {}; }}\n",
            hex(chrome.window),
            hex(chrome.foreground),
            hex_channels(blend(channels(chrome.window), channels(chrome.foreground), 0.15)),
            hex(chrome.accent),
            hex_channels(contrasting(channels(chrome.accent))),
        ));
    }
    css
}

/// A colour as a CSS hex literal.
fn hex(color: u32) -> String {
    format!("#{:06x}", color & 0x00FF_FFFF)
}

/// One of the shell's `(red, green, blue)` colours as a CSS hex literal.
fn hex_channels(color: (f64, f64, f64)) -> String {
    let channel = |value: f64| (value * 255.0).round().clamp(0.0, 255.0) as u32;
    hex((channel(color.0) << 16) | (channel(color.1) << 8) | channel(color.2))
}

/// Starts the GTK shell and runs it until its window closes.
///
/// `--new-instance` opts out of GtkApplication's session-bus handoff, so a
/// rebuilt shell can run beside an installed one instead of activating it (and
/// showing the old build's window).
pub fn run() -> Result<(), Box<dyn std::error::Error>> {
    let (new_instance, arguments) = take_new_instance(std::env::args_os().collect());

    let flags = if new_instance {
        gtk::gio::ApplicationFlags::NON_UNIQUE
    } else {
        gtk::gio::ApplicationFlags::default()
    };
    let app = Application::builder()
        .application_id("org.winter.Winter")
        .flags(flags)
        .build();
    app.connect_activate(activate);
    app.run_with_args_os(&arguments);
    Ok(())
}

/// Takes `--new-instance` out of the argument list.
///
/// GApplication parses the process arguments itself and exits with `Unknown
/// option` for a flag it does not declare, so the shell cannot leave this one
/// in place.
fn take_new_instance(arguments: Vec<std::ffi::OsString>) -> (bool, Vec<std::ffi::OsString>) {
    let new_instance = arguments.iter().any(|argument| argument == "--new-instance");
    let arguments = arguments
        .into_iter()
        .filter(|argument| argument != "--new-instance")
        .collect();
    (new_instance, arguments)
}

fn activate(app: &Application) {
    let screenshot = std::env::var_os("WINTER_SCREENSHOT").map(PathBuf::from);

    let palette = Chrome::system();
    // A second activation builds another window in this process; one provider
    // is enough for the display.
    static CSS_INSTALLED: std::sync::Once = std::sync::Once::new();
    CSS_INSTALLED.call_once(|| {
        if let Some(display) = gdk::Display::default() {
            let css = gtk::CssProvider::new();
            css.load_from_data(&chrome_css(palette));
            gtk::style_context_add_provider_for_display(&display, &css, gtk::STYLE_PROVIDER_PRIORITY_APPLICATION);
        }
    });

    settings::ensure_file();
    let loaded = settings::load_with_report();
    let settings_path = loaded.path.clone();
    let settings_stamp = settings_path.as_deref().and_then(settings::stamp);
    let warnings: Vec<String> = loaded.warnings.iter().map(ToString::to_string).collect();
    let migrated_from = loaded.migrated_from.clone();
    let settings_error = loaded.error.as_ref().map(ToString::to_string);
    let document = loaded.settings;
    let font_size = document.font.size;

    // The window is sized in logical pixels, but the face is rasterised for the
    // screen's scale, so a HiDPI monitor gets physical pixels (ADR 072's macOS
    // trap applies here too).
    let scale = display_scale();
    let Some(fonts) = fonts(font_size, CELLS, scale) else {
        eprintln!("winter: no monospace face found in the usual font directories");
        app.quit();
        return;
    };
    let cell_dip = (
        (f64::from(fonts.cell.0) / f64::from(scale)).ceil(),
        (f64::from(fonts.cell.1) / f64::from(scale)).ceil(),
    );
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
    // The GPU path is the default when this host has GL (ADR 076); the
    // software backend remains built because it is the fallback and the
    // screenshot blit.
    let gpu = if std::env::var_os("WINTER_SOFTWARE").is_some() {
        None
    } else {
        Gl::new()
    };
    if let Some(gl) = &gpu {
        gl.set_rasterizer(Box::new(fonts.gpu));
    }
    let backend: Box<dyn IBackend> = match &gpu {
        Some(gl) => Box::new(gl.clone()),
        None => Box::new(shared.clone()),
    };
    let mut engine = AtlasEngine::with_settings(fonts.settings.clone(), backend);
    engine.set_shaping(Box::new(fonts.shaper));

    let mut renderer = Renderer::new(
        TerminalData::new(fonts.info, RenderSettings::new(), CELLS.0, CELLS.1),
        RenderSettings::new(),
    );
    renderer.add_engine(Box::new(engine));

    let window = ApplicationWindow::builder()
        .application(app)
        .title(&template.title)
        // The window is sized for whole cells inside the pane's inset and the
        // scrollbar, so the first prompt is laid out at the size the pty opens
        // with.
        .default_width(PANE_LEFT as i32 + (cell_dip.0 * f64::from(CELLS.0)) as i32 + scrollbar::WIDTH as i32)
        .default_height((cell_dip.1 * f64::from(CELLS.1)) as i32)
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
    bar.add_css_class("winter-titlebar");
    bar.append(&gtk::WindowControls::new(gtk::PackType::Start));
    bar.append(&handle);
    bar.append(&gtk::WindowControls::new(gtk::PackType::End));
    window.set_titlebar(Some(&bar));

    let area = DrawingArea::new();
    // The pane takes the keyboard and the free space below the find bar, which
    // is how a terminal behaves; a click asks for the keyboard back when a
    // dialog or a menu took it away.
    area.set_can_focus(true);
    area.set_vexpand(true);
    area.set_hexpand(true);

    // The find bar is GTK's own: a native search entry that reveals over the
    // pane, so Escape and the match keys are the desktop's (ADR 075).
    let search_entry = gtk::SearchEntry::new();
    search_entry.set_placeholder_text(Some("Find"));
    search_entry.set_hexpand(true);
    let search_count = gtk::Label::new(None);
    search_count.add_css_class("dim-label");
    // The other shells give the bar a previous/next pair beside the field, so a
    // pointer can step the matches without knowing the keyboard shortcuts.
    let search_previous = gtk::Button::with_label("↑");
    search_previous.set_tooltip_text(Some("Previous match"));
    search_previous.set_focus_on_click(false);
    let search_next = gtk::Button::with_label("↓");
    search_next.set_tooltip_text(Some("Next match"));
    search_next.set_focus_on_click(false);
    let search_row = gtk::Box::new(gtk::Orientation::Horizontal, 6);
    search_row.append(&search_entry);
    search_row.append(&search_previous);
    search_row.append(&search_next);
    search_row.append(&search_count);
    let search_bar = gtk::SearchBar::new();
    search_bar.set_child(Some(&search_row));
    search_bar.set_show_close_button(true);

    let column = gtk::Box::new(gtk::Orientation::Vertical, 0);
    column.append(&search_bar);

    // The pane is the base widget; the GL area sits above it, inset by the left
    // pane margin and the scrollbar's gutter, so its framebuffer is exactly the
    // grid. The base widget paints the margins, the scrollbar and the bell's
    // background; the GL area paints the cells (ADR 076).
    let gl_area = gtk::GLArea::new();
    gl_area.set_hexpand(true);
    gl_area.set_vexpand(true);
    gl_area.set_margin_start(PANE_LEFT as i32);
    gl_area.set_margin_end(scrollbar::WIDTH as i32);
    gl_area.set_visible(gpu.is_some());

    let overlay = gtk::Overlay::new();
    overlay.set_hexpand(true);
    overlay.set_vexpand(true);
    overlay.set_child(Some(&area));
    overlay.add_overlay(&gl_area);
    let commands_view = palette_view(&overlay);
    let command_entry = commands_view.entry.clone();
    column.append(&overlay);
    window.set_child(Some(&column));

    let menu = gtk::PopoverMenu::from_model(Some(&profile_menu(&document)));
    let shell_gpu = gpu.is_some();

    let shell = Rc::new(RefCell::new(Shell {
        window: window.clone(),
        area: area.clone(),
        strip: strip.clone(),
        renderer,
        shared,
        gl: gpu,
        gl_area: gl_area.clone(),
        gpu: shell_gpu,
        tabs: Tabs::new(first),
        template,
        document,
        settings_path,
        settings_stamp,
        settings_at: Instant::now(),
        warnings,
        migrated_from,
        settings_error,
        font_size,
        scale,
        palette,
        bindings,
        menu,
        icons: Icons::new(),
        commands: Palette::default(),
        palette_view: commands_view,
        chrome: fonts.chrome,
        cell: fonts.cell,
        grid: CELLS,
        pointer: (0, 0),
        last_mouse_cell: None,
        press: None,
        held: MouseButtons::default(),
        tab_drag: None,
        scrollbar: Scrollbar::default(),
        scrollbar_press: false,
        composition: Composition::default(),
        paste_requested: false,
        find_seed: None,
        settings_requested: false,
        blink_on: true,
        blink_at: Instant::now(),
        wheel_pixels: 0.0,
        hover: None,
        window_title: String::new(),
        accessible_text: String::new(),
        screenshot,
        settings_window: None,
    }));

    shell.borrow_mut().palette_view.owner = Rc::downgrade(&shell);

    {
        let action = gtk::gio::SimpleAction::new("open", Some(glib::VariantTy::STRING));
        let open = Rc::clone(&shell);
        action.connect_activate(move |_action, parameter| {
            let name = parameter.and_then(|value| value.str().map(str::to_string));
            open.borrow_mut().open_profile(name.as_deref());
        });
        let settings = gtk::gio::SimpleAction::new("settings", None);
        let editor = Rc::clone(&shell);
        settings.connect_activate(move |_action, _parameter| {
            editor.borrow_mut().request_settings();
        });
        let about = gtk::gio::SimpleAction::new("about", None);
        let parent = window.clone();
        about.connect_activate(move |_action, _parameter| show_about(&parent));
        let group = gtk::gio::SimpleActionGroup::new();
        group.add_action(&action);
        group.add_action(&settings);
        group.add_action(&about);
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
        // GTK makes the context current for the render signal, so this is
        // where the engine's frame is drawn; a failed context falls back to
        // the software blit.
        let realized = Rc::clone(&shell);
        gl_area.connect_realize(move |area| {
            area.make_current();
            if area.error().is_some()
                && let Ok(mut shell) = realized.try_borrow_mut()
            {
                shell.disable_gpu();
            }
        });
        let rendered = Rc::clone(&shell);
        gl_area.connect_render(move |area, _context| {
            let Ok(mut shell) = rendered.try_borrow_mut() else {
                return glib::Propagation::Proceed;
            };
            shell.paint_gpu(area.width(), area.height());
            glib::Propagation::Stop
        });
    }

    // The input method goes first on every key: it turns a composition into a
    // commit and reports the marked text, and a key it consumed must not also
    // reach the encoder (ADR 075).
    let im = gtk::IMMulticontext::new();
    im.set_client_widget(Some(&area));
    {
        let focus = gtk::EventControllerFocus::new();
        let context = im.clone();
        focus.connect_enter(move |_controller| context.focus_in());
        let context = im.clone();
        focus.connect_leave(move |_controller| context.focus_out());
        area.add_controller(focus);
    }
    {
        let committed = Rc::clone(&shell);
        im.connect_commit(move |_im, text| {
            if let Ok(mut shell) = committed.try_borrow_mut() {
                shell.text_input(text.to_string());
            }
        });
        let marked = Rc::clone(&shell);
        im.connect_preedit_changed(move |im| {
            let (text, _attributes, cursor) = im.preedit_string();
            let selected = utf16_offset(text.as_str(), usize::try_from(cursor.max(0)).unwrap_or(0));
            let Ok(mut shell) = marked.try_borrow_mut() else {
                return;
            };
            shell.set_composition(text.as_str(), selected);
            let caret = shell.caret_rect();
            drop(shell);
            // The candidate window follows the caret, as AppKit's does.
            im.set_cursor_location(&caret);
        });
        let ended = Rc::clone(&shell);
        im.connect_preedit_end(move |_im| {
            if let Ok(mut shell) = ended.try_borrow_mut() {
                shell.set_composition("", 0);
            }
        });
    }

    {
        let entry = search_entry.clone();
        let palette = command_entry.clone();
        let owner = window.clone();
        let context = im.clone();
        let keys = EventControllerKey::new();
        let pressed = Rc::clone(&shell);
        keys.connect_key_pressed(move |controller, key, _code, state| {
            // A text field owns its keys: the find field's text, Enter, Escape
            // and match shortcuts are the widget's, as are the palette's.
            if focus_is_within(&owner, &entry) || focus_is_within(&owner, &palette) {
                return glib::Propagation::Proceed;
            }
            if let Some(event) = controller.current_event()
                && context.filter_keypress(&event)
            {
                return glib::Propagation::Stop;
            }
            pressed.borrow_mut().key(key, state, true)
        });
        let entry = search_entry.clone();
        let palette = command_entry.clone();
        let owner = window.clone();
        let released = Rc::clone(&shell);
        keys.connect_key_released(move |_controller, key, _code, state| {
            if focus_is_within(&owner, &entry) || focus_is_within(&owner, &palette) {
                return;
            }
            released.borrow_mut().key(key, state, false);
        });
        window.add_controller(keys);
    }

    {
        let changed = Rc::clone(&shell);
        search_entry.connect_search_changed(move |entry| {
            if let Ok(mut shell) = changed.try_borrow_mut() {
                shell.find_query(entry.text().to_string());
            }
        });
        let activated = Rc::clone(&shell);
        search_entry.connect_activate(move |_entry| {
            if let Ok(mut shell) = activated.try_borrow_mut() {
                shell.find_step(true);
            }
        });
        let next = Rc::clone(&shell);
        search_entry.connect_next_match(move |_entry| {
            if let Ok(mut shell) = next.try_borrow_mut() {
                shell.find_step(true);
            }
        });
        let previous = Rc::clone(&shell);
        search_entry.connect_previous_match(move |_entry| {
            if let Ok(mut shell) = previous.try_borrow_mut() {
                shell.find_step(false);
            }
        });
        let previous = Rc::clone(&shell);
        search_previous.connect_clicked(move |_button| {
            if let Ok(mut shell) = previous.try_borrow_mut() {
                shell.find_step(false);
            }
        });
        let next = Rc::clone(&shell);
        search_next.connect_clicked(move |_button| {
            if let Ok(mut shell) = next.try_borrow_mut() {
                shell.find_step(true);
            }
        });
        let stopped = Rc::clone(&shell);
        search_entry.connect_stop_search(move |_entry| {
            if let Ok(mut shell) = stopped.try_borrow_mut() {
                shell.close_find();
            }
        });
        // The bar's own close button turns search mode off without a
        // `stop-search`, so the tab's find state follows the widget.
        let closed = Rc::clone(&shell);
        search_bar.connect_search_mode_enabled_notify(move |bar| {
            if !bar.is_search_mode()
                && let Ok(mut shell) = closed.try_borrow_mut()
            {
                shell.close_find();
            }
        });
    }

    {
        // The palette's field narrows the list as it is typed; the rows and
        // their selection are the shell's, so the signal rebuilds them.
        let changed = Rc::clone(&shell);
        command_entry.connect_changed(move |entry| {
            if let Ok(mut shell) = changed.try_borrow_mut() {
                shell.command_filter(entry.text().to_string());
            }
        });

        // Enter is the entry's own activation: the inner text consumes the key
        // before a controller on the entry sees it, so this is the signal that
        // says the field was activated.
        let activated = Rc::clone(&shell);
        command_entry.connect_activate(move |_entry| {
            if let Ok(mut shell) = activated.try_borrow_mut() {
                shell.run_selected_command();
            }
        });

        let keys = EventControllerKey::new();
        let pressed = Rc::clone(&shell);
        keys.connect_key_pressed(move |_controller, key, _code, _state| {
            let Ok(mut shell) = pressed.try_borrow_mut() else {
                return glib::Propagation::Proceed;
            };
            match key {
                gdk::Key::Escape => {
                    shell.close_commands();
                    glib::Propagation::Stop
                }
                // The arrows move the selection; the field must not move its
                // cursor instead.
                gdk::Key::Up | gdk::Key::KP_Up => {
                    shell.command_step(false);
                    glib::Propagation::Stop
                }
                gdk::Key::Down | gdk::Key::KP_Down => {
                    shell.command_step(true);
                    glib::Propagation::Stop
                }
                _ => glib::Propagation::Proceed,
            }
        });
        command_entry.add_controller(keys);
    }

    {
        let shell = Rc::clone(&shell);
        let motion = EventControllerMotion::new();
        motion.connect_motion(move |controller, x, y| {
            let modifiers = controller
                .current_event()
                .map_or_else(Modifiers::default, |event| modifiers(event.modifier_state()));
            shell.borrow_mut().pointer_moved((x, y), modifiers);
        });
        overlay.add_controller(motion);
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
        overlay.add_controller(scroll);
    }

    {
        // A press on the pane takes the keyboard back from a tab, a menu or a
        // dialog that had it, and starts a selection or a program's press.
        let click = GestureClick::new();
        // Every button: a tracking program sees all three, and without one a
        // non-primary press pastes.
        click.set_button(0);

        // A drag reports the pointer even after it leaves the pane, which is how
        // a selection scrolls when it is held past the top or bottom edge.
        //
        // The two gestures share a group. A claimed sequence is denied to every
        // *other* group on the widget, so a separately grouped drag would never
        // see a move after the press claimed it; a gesture in the same group is
        // initialised with the group's state and keeps reporting.
        let drag = gtk::GestureDrag::new();
        click.group_with(&drag);

        let pressed = Rc::clone(&shell);
        click.connect_pressed(move |gesture, _presses, x, y| {
            let button = mouse_button(gesture.current_button());
            let modifiers = gesture
                .current_event()
                .map_or_else(Modifiers::default, |event| modifiers(event.modifier_state()));
            let claimed = pressed.borrow_mut().pointer_pressed((x, y), button, modifiers);
            gesture.set_state(if claimed {
                EventSequenceState::Claimed
            } else {
                EventSequenceState::Denied
            });
        });
        let released = Rc::clone(&shell);
        click.connect_released(move |gesture, _presses, x, y| {
            let button = mouse_button(gesture.current_button());
            let modifiers = gesture
                .current_event()
                .map_or_else(Modifiers::default, |event| modifiers(event.modifier_state()));
            released.borrow_mut().pointer_released((x, y), button, modifiers);
        });

        let shell = Rc::clone(&shell);
        let start = Rc::new(Cell::new((0.0f64, 0.0f64)));
        let begin = Rc::clone(&start);
        drag.connect_drag_begin(move |_gesture, x, y| begin.set((x, y)));
        let from = Rc::clone(&start);
        drag.connect_drag_update(move |gesture, dx, dy| {
            let (x, y) = from.get();
            let modifiers = gesture
                .current_event()
                .map_or_else(Modifiers::default, |event| modifiers(event.modifier_state()));
            shell.borrow_mut().pointer_moved((x + dx, y + dy), modifiers);
        });

        overlay.add_controller(click);
        overlay.add_controller(drag);
    }

    {
        let shell = Rc::clone(&shell);
        let click = GestureClick::new();
        // Every button, because the middle click is how a tab closes.
        click.set_button(0);
        let menu = shell.borrow().menu.clone();
        let tall = strip.clone();
        let pressed = Rc::clone(&shell);
        click.connect_pressed(move |gesture, _presses, x, _y| {
            let (claimed, popup) = pressed.borrow_mut().press_strip(x, gesture.current_button());
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
        let released = Rc::clone(&shell);
        click.connect_released(move |_gesture, _presses, _x, _y| {
            released.borrow_mut().drop_tab_drag();
        });

        // The drag shares the click's group, for the same reason the pane's
        // does: the press claims the sequence so `GtkWindowHandle` never takes
        // it as a window gesture, and the drag inherits the claim and still
        // reports where the pointer went, including below the strip.
        let drag = gtk::GestureDrag::new();
        click.group_with(&drag);
        let start = Rc::new(Cell::new((0.0f64, 0.0f64)));
        let begin = Rc::clone(&start);
        drag.connect_drag_begin(move |_gesture, x, y| begin.set((x, y)));
        let moving = Rc::clone(&shell);
        let from = Rc::clone(&start);
        drag.connect_drag_update(move |_gesture, dx, _dy| {
            let (x, _) = from.get();
            if let Ok(mut shell) = moving.try_borrow_mut() {
                shell.drag_strip(x + dx);
            }
        });
        let ended = Rc::clone(&shell);
        drag.connect_drag_end(move |_gesture, _dx, _dy| {
            if let Ok(mut shell) = ended.try_borrow_mut() {
                shell.drop_tab_drag();
            }
        });

        strip.add_controller(click);
        strip.add_controller(drag);
    }

    {
        // The strip shows what is under the pointer. The reorder is a drag, so
        // it is reported by the gesture above rather than by this controller.
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
        let bar = search_bar.clone();
        let entry = search_entry.clone();
        let count = search_count.clone();
        let pane = area.clone();
        let parent = window.clone();
        glib::timeout_add_local(TICK, move || {
            let mut borrowed = shell.borrow_mut();
            borrowed.pump(&app);
            let paste = borrowed.take_paste_request();
            let seed = borrowed.take_find_seed();
            let settings = borrowed.take_settings_request();
            let (open, query, current, total) = borrowed.find_state();
            drop(borrowed);

            bar.set_search_mode(open);
            // The field owns its text while the bar is open. Only a seed the
            // shell chose -- opening the bar or switching tabs -- writes into
            // it, so a frame can never revert what the user just typed.
            if let Some(seed) = &seed {
                if let Some(text) = frame_text(Some(seed), entry.text().as_str()) {
                    entry.set_text(text);
                }
                if seed.focus {
                    entry.grab_focus();
                    entry.select_region(0, -1);
                }
            } else if !open && focus_is_within(&parent, &entry) {
                pane.grab_focus();
            }

            let text = if open {
                winter::find::status(current, total, &query)
            } else {
                String::new()
            };
            if count.text().as_str() != text {
                count.set_text(&text);
            }

            if paste {
                paste_from_clipboard(&shell);
            }
            if settings {
                show_settings(&shell, &parent);
            }
            glib::ControlFlow::Continue
        });
    }

    window.present();
    area.grab_focus();

    if std::env::var_os("WINTER_OPEN_SETTINGS").is_some() {
        show_settings(&shell, &window);
    }
    if std::env::var_os("WINTER_OPEN_FIND").is_some() {
        shell.borrow_mut().toggle_find();
    }
    if std::env::var_os("WINTER_OPEN_PALETTE").is_some() {
        shell.borrow_mut().toggle_commands();
    }

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
    /// The tab's own find bar, so switching tabs keeps its query.
    find: FindBar,
    /// The program the tab runs, which its icon is resolved from.
    program: String,
}

impl Tab {
    /// Starts the profile's shell on a pty.
    fn spawn(template: &TerminalSettings, cells: (i32, i32)) -> std::io::Result<Self> {
        let session = Session::spawn(&template.command, pty_size(cells))?;
        let mut control = Control::new(Box::new(session), cells);
        control.apply_scheme(template.scheme.as_ref());
        Ok(Self {
            control,
            view: View::new(),
            sides: ModifierSides::default(),
            title: template.title.clone(),
            find: FindBar::default(),
            program: template.command.program().to_string_lossy().into_owned(),
        })
    }
}

/// A query the find field should show on the next frame.
struct FindSeed {
    /// The text to put in the field.
    query: String,
    /// Whether the field should also take the keyboard, as it does when the bar
    /// opens rather than when a tab switch merely follows the new tab's query.
    focus: bool,
}

/// The command palette's widgets: a card over the pane.
struct PaletteView {
    /// The card that holds the field, the rows and the empty notice.
    card: gtk::Box,
    /// The field the filter is typed into.
    entry: gtk::SearchEntry,
    /// The scroller the rows live in.
    list: gtk::ScrolledWindow,
    /// The rows, in the order the filter leaves them.
    rows: gtk::Box,
    /// The notice shown when the filter matches nothing.
    empty: gtk::Label,
    /// The overlay the card is anchored to, for its bounds.
    overlay: gtk::Overlay,
    /// The shell a row calls back into; weak, so the view does not keep it
    /// alive.
    owner: Weak<RefCell<Shell>>,
}

/// Builds the command palette's card, hidden until the action opens it.
fn palette_view(overlay: &gtk::Overlay) -> PaletteView {
    let entry = gtk::SearchEntry::new();
    entry.set_placeholder_text(Some("Type a command"));
    entry.set_hexpand(true);

    let rows = gtk::Box::new(gtk::Orientation::Vertical, 1);
    let list = gtk::ScrolledWindow::builder()
        .child(&rows)
        .propagate_natural_height(true)
        .max_content_height(320)
        .hscrollbar_policy(gtk::PolicyType::Never)
        .build();

    let empty = gtk::Label::new(Some("No matching command"));
    empty.add_css_class("dim-label");
    empty.set_visible(false);

    let card = gtk::Box::new(gtk::Orientation::Vertical, 6);
    card.add_css_class("card");
    card.add_css_class("winter-palette");
    card.set_halign(gtk::Align::Center);
    card.set_valign(gtk::Align::Start);
    card.set_margin_top(48);
    card.set_width_request(480);
    card.append(&entry);
    card.append(&list);
    card.append(&empty);
    card.set_visible(false);
    overlay.add_overlay(&card);

    PaletteView {
        card,
        entry,
        list,
        rows,
        empty,
        overlay: overlay.clone(),
        owner: Weak::new(),
    }
}

/// The actions the palette may offer: the ones `command` runs.
fn palette_supports(action: settings::Action) -> bool {
    use settings::Action;

    matches!(
        action,
        Action::NewTab
            | Action::CloseTab
            | Action::NextTab
            | Action::PrevTab
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

/// One clickable palette row: the label, the bound chord, and what it runs.
fn command_row(entry: &palette::Entry, selected: bool, owner: &Weak<RefCell<Shell>>) -> gtk::Button {
    let row = gtk::Button::new();
    row.add_css_class("flat");
    row.add_css_class("palette-row");
    if selected {
        row.add_css_class("selected");
    }
    row.set_focus_on_click(false);

    let content = gtk::Box::new(gtk::Orientation::Horizontal, 12);
    let label = gtk::Label::new(Some(&entry.label));
    label.set_xalign(0.0);
    label.set_hexpand(true);
    content.append(&label);
    if let Some(chord) = &entry.chord {
        let chord = gtk::Label::new(Some(chord));
        chord.add_css_class("dim-label");
        content.append(&chord);
    }
    row.set_child(Some(&content));

    let command = entry.command.clone();
    let owner = owner.clone();
    row.connect_clicked(move |_| {
        if let Some(shell) = owner.upgrade()
            && let Ok(mut shell) = shell.try_borrow_mut()
        {
            shell.close_commands();
            shell.run_palette_command(command.clone());
        }
    });

    row
}

/// The text a frame writes into the find field, if any.
///
/// Only a seed the shell chose is a source: `None` means the shell has nothing
/// to say, so the user's typing stands. The old rule compared the field's text
/// with the shell's query on every frame, and a change notification that had
/// not arrived yet made that comparison revert the last keystroke.
fn frame_text<'a>(seed: Option<&'a FindSeed>, entry: &str) -> Option<&'a str> {
    seed.map(|seed| seed.query.as_str()).filter(|query| *query != entry)
}

/// Whether the keyboard focus is `widget` or one of its children.
///
/// `GtkSearchEntry` delegates focus to its inner text, so `has_focus` on the
/// entry itself is false while the user types into it; asking the window for
/// its focus widget and walking up from there is the answer that stays true.
fn focus_is_within(window: &impl IsA<gtk::Window>, widget: &impl IsA<gtk::Widget>) -> bool {
    let Some(focus) = GtkWindowExt::focus(window) else {
        return false;
    };
    let widget = widget.upcast_ref::<gtk::Widget>();
    focus.is_ancestor(widget) || focus == *widget
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
    /// The GPU content path, when this host has one.
    gl: Option<Gl>,
    /// The GL area the grid is drawn in.
    gl_area: gtk::GLArea,
    /// Whether the renderer's engine draws through the GPU path.
    gpu: bool,
    tabs: Tabs<Tab>,
    /// The profile a new tab starts from.
    template: TerminalSettings,
    /// The settings file, for the strip's colours.
    document: settings::Settings,
    /// Where the settings file lives, when the platform has a path.
    settings_path: Option<PathBuf>,
    /// The file's stamp when it was last read, so a save is noticed.
    settings_stamp: Option<settings::Stamp>,
    /// When the settings file was last checked.
    settings_at: Instant,
    /// What normalization had to decide for the file in force.
    warnings: Vec<String>,
    /// The earlier file these settings were migrated from.
    migrated_from: Option<PathBuf>,
    /// Why the file on disk was not used, if it was not.
    settings_error: Option<String>,
    /// The face's size in points.
    font_size: f32,
    /// The composition scale the face was rasterised for. GTK reports the
    /// window's size in logical pixels, so the cell, the grid and the renderer's
    /// target are all multiplied by this to reach physical pixels.
    scale: f32,
    /// The desktop's window-chrome palette, when it publishes one.
    palette: Option<Chrome>,
    /// The chords the window matches before the terminal sees a key.
    bindings: Rc<[settings::Keybinding]>,
    /// The profile menu the strip's button pops up.
    menu: gtk::PopoverMenu,
    /// The icons the strip's tabs wear, resolved once per program and size.
    icons: Icons,
    /// The command palette's entries, filter and selection.
    commands: Palette,
    /// The palette's widget, a card over the pane.
    palette_view: PaletteView,
    /// The face the strip's titles are drawn with.
    chrome: FontDueRasterizer,
    /// The cell size in physical pixels.
    cell: (u16, u16),
    /// The grid the renderer's target currently holds.
    grid: (i32, i32),
    /// Where the pointer last was, in cells, for a tracking program's wheel.
    pointer: (i32, i32),
    /// The cell a forwarded move last reported, so a still pointer is silent.
    last_mouse_cell: Option<(i32, i32)>,
    /// The cell a press started on, so the first move starts a selection.
    press: Option<((i32, i32), bool)>,
    /// The buttons a forwarded event reports as held.
    held: MouseButtons,
    /// The tab a strip press grabbed, until the drag ends.
    tab_drag: Option<usize>,
    /// The scrollbar's drag state.
    scrollbar: Scrollbar,
    /// Whether a press grabbed the scrollbar's thumb.
    scrollbar_press: bool,
    /// The input method's marked text, drawn at the cursor until it commits.
    composition: Composition,
    /// Whether GDK should read the clipboard on the next tick.
    paste_requested: bool,
    /// The text the next frame should put in the find field, when the shell
    /// rather than the user is choosing it. `None` leaves the widget alone, so
    /// typing is never overwritten by the previous tick's query.
    find_seed: Option<FindSeed>,
    /// The settings window the timer should open.
    settings_requested: bool,
    /// Whether the cursor is in the on half of its blink.
    blink_on: bool,
    /// When the cursor last changed halves.
    blink_at: Instant,
    /// Wheel pixels that have not yet made a row: a touchpad reports pixels
    /// where a wheel reports whole detents.
    wheel_pixels: f64,
    /// Where the pointer is over the strip, so the element under it can show
    /// that it is pressable.
    hover: Option<i32>,
    /// The title the window last showed, so it is not rewritten every tick.
    window_title: String,
    /// The last visible screen published to the accessibility tree.
    accessible_text: String,
    /// Where `WINTER_SCREENSHOT` asked for a picture, if it did.
    screenshot: Option<PathBuf>,
    /// The settings editor, kept so a capture can include it.
    settings_window: Option<gtk::Window>,
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
        let mut announced = Vec::new();

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
                match event {
                    TerminalEvent::Exited => exited.push(index),
                    TerminalEvent::Notification { title, body } => announced.push((title, body)),
                    // The flash is read from the control while it paints, so a
                    // bell needs no state of its own here.
                    TerminalEvent::Bell => {}
                }
            }
        }

        for (title, body) in announced {
            notifications::show(&title, &body);
        }

        if now.duration_since(self.settings_at) >= Duration::from_secs(1) {
            self.settings_at = now;
            self.poll_settings();
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

        if now.duration_since(self.blink_at) >= BLINK {
            self.blink_on = !self.blink_on;
            self.blink_at = now;
        }

        if titled {
            self.after_select();
        } else {
            self.queue_paint();
        }
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
    ///
    /// Chords, the strip's colours and the font size take effect at once; a
    /// scheme repaints every tab, and a changed profile affects the next tab,
    /// because a running shell keeps the command it started with.
    fn reload_settings(&mut self) {
        let loaded = settings::load_with_report();
        self.settings_stamp = loaded.path.as_deref().and_then(settings::stamp);
        self.warnings = loaded.warnings.iter().map(ToString::to_string).collect();
        self.migrated_from = loaded.migrated_from.clone();
        if let Some(error) = loaded.error {
            self.settings_error = Some(error.to_string());
            return;
        }

        self.document = loaded.settings;
        self.settings_path = loaded.path;
        self.settings_error = None;
        self.template = TerminalSettings::resolve(&self.document);
        self.bindings = self.document.keybindings.clone().into();
        // The menu is a snapshot of the profile list, so a reloaded file's
        // profiles appear without a restart.
        self.menu.set_menu_model(Some(&profile_menu(&self.document)));

        let font_size = self.document.font.size;
        if (font_size - self.font_size).abs() > f32::EPSILON {
            self.font_size = font_size;
            self.rebuild_fonts();
            // A different cell means a different number of whole cells fits.
            let (width, height) = (self.area.width(), self.area.height());
            if width > 0 && height > 0 {
                self.grid = (0, 0);
                self.fit(width, height);
            }
        }

        let scheme = self.template.scheme.clone();
        for index in 0..self.tabs.len() {
            if let Some(tab) = self.tabs.get_mut(index) {
                tab.control.apply_scheme(scheme.as_ref());
            }
        }
        self.after_select();
    }

    /// Replaces the face, the renderer and the cell with ones at `font_size`.
    fn rebuild_fonts(&mut self) {
        let Some(fonts) = fonts(self.font_size, self.grid, self.scale) else {
            return;
        };
        self.cell = fonts.cell;
        self.chrome = fonts.chrome;

        let shared = SharedBackend::new(SoftwareBackend::with_rasterizer(Box::new(fonts.rasterizer)));
        let backend: Box<dyn IBackend> = match (&self.gl, self.gpu) {
            (Some(gl), true) => {
                gl.set_rasterizer(Box::new(fonts.gpu));
                Box::new(gl.clone())
            }
            _ => Box::new(shared.clone()),
        };
        let mut engine = AtlasEngine::with_settings(fonts.settings.clone(), backend);
        engine.set_shaping(Box::new(fonts.shaper));
        let mut renderer = Renderer::new(
            TerminalData::new(fonts.info, RenderSettings::new(), self.grid.0, self.grid.1),
            RenderSettings::new(),
        );
        renderer.add_engine(Box::new(engine));

        let (columns, rows) = self.grid;
        let target = (
            fonts
                .cell
                .0
                .saturating_mul(u16::try_from(columns.max(1)).unwrap_or(u16::MAX)),
            fonts
                .cell
                .1
                .saturating_mul(u16::try_from(rows.max(1)).unwrap_or(u16::MAX)),
        );
        let _ = renderer.resize(target, InclusiveRect::new(0, 0, columns - 1, rows - 1));

        self.renderer = renderer;
        self.shared = shared;
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

        // A printable key's press reaches the shell as text, through the input
        // method or the character path, and carries no key identity; its
        // release would be a release without a press.
        if !pressed && input::produces_text(named, modifiers) {
            return glib::Propagation::Stop;
        }

        let character = if matches!(named, Key::Char(_)) {
            none_if_control(key.to_unicode())
        } else {
            None
        };

        // A printable key is text, not a chord: the shared translation sends a
        // character event for it, as the macOS and WinUI toolkits do.
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

        self.send_input(&event);
        glib::Propagation::Stop
    }

    /// Sends one portable event through the shared translation rules.
    fn send_input(&mut self, event: &InputEvent) {
        // A printable key travels as the character it typed, unless the program
        // asked for every key: then the key and its release travel instead.
        let modes = self.tabs.active().map(|tab| tab.control.input_modes());
        let win32 = modes.is_some_and(|modes| modes.win32_input);
        let keys = modes.is_some_and(|modes| modes.reports_all_keys());
        let Some(tab) = self.tabs.active_mut() else {
            return;
        };
        let outcome = input::translate(event, &mut tab.sides, win32, keys);
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

    /// Sends composed text, from the keyboard or an input method.
    fn text_input(&mut self, text: String) {
        if !text.is_empty() {
            self.send_input(&InputEvent::Text(text));
        }
    }

    /// Shows or clears the input method's marked text.
    fn set_composition(&mut self, text: &str, selected_utf16: usize) {
        self.composition = winter::terminal_render::composition(text, selected_utf16);
    }

    /// The caret's box in the pane's coordinates, for the candidate window.
    fn caret_rect(&self) -> gdk::Rectangle {
        let point = self.renderer.data().cursor_screen();
        let (width, height) = self.dip_cell();
        let width = width.round().max(1.0) as i32;
        let height = height.round().max(1.0) as i32;
        gdk::Rectangle::new(PANE_LEFT as i32 + point.x() * width, point.y() * height, width, height)
    }

    /// Runs a window action the tab strip can perform; declines the rest.
    fn command(&mut self, action: settings::Action) -> bool {
        match action {
            settings::Action::NewTab => self.open_tab(None),
            settings::Action::CloseTab => self.close_tab(self.tabs.index()),
            settings::Action::NextTab => {
                self.tabs.step(1);
                self.seed_find(false);
                self.focus_active();
            }
            settings::Action::PrevTab => {
                self.tabs.step(-1);
                self.seed_find(false);
                self.focus_active();
            }
            settings::Action::Copy => return self.copy_selection(),
            settings::Action::Paste => self.paste_clipboard(),
            settings::Action::ScrollUp => self.scroll_active(1),
            settings::Action::ScrollDown => self.scroll_active(-1),
            settings::Action::ScrollUpPage => self.scroll_page(1),
            settings::Action::ScrollDownPage => self.scroll_page(-1),
            settings::Action::ScrollToTop => self.scroll_to(false),
            settings::Action::ScrollToBottom => self.scroll_to(true),
            settings::Action::Find => self.toggle_find(),
            settings::Action::OpenSettings => self.request_settings(),
            settings::Action::CommandPalette => self.toggle_commands(),
            _ => return false,
        }
        true
    }

    /// Shows or hides the command palette.
    fn toggle_commands(&mut self) {
        if self.commands.open {
            self.close_commands();
        } else {
            self.open_commands();
        }
    }

    /// Opens the palette with an empty filter and the first row selected.
    fn open_commands(&mut self) {
        self.commands.show();
        // The field takes the keyboard, so a half-composed terminal preview is
        // no longer where the user is typing.
        self.composition = Composition::default();
        // Setting the field emits `changed`; the rebuild below is the one that
        // counts, because this borrow is still held when the signal runs.
        self.palette_view.entry.set_text("");
        self.commands.filter.clear();
        self.rebuild_commands();
        self.palette_view.card.set_visible(true);
        self.palette_view.entry.grab_focus();
    }

    /// Closes the palette and gives the keyboard back to the pane.
    fn close_commands(&mut self) {
        self.commands.hide();
        self.palette_view.card.set_visible(false);
        self.palette_view.entry.set_text("");
        self.area.grab_focus();
    }

    /// Narrows the rows to `filter` and selects the first one.
    fn command_filter(&mut self, filter: String) {
        self.commands.filter = filter;
        self.commands.selected = 0;
        self.rebuild_commands();
    }

    /// The rows the current filter leaves.
    fn command_rows(&self) -> Vec<palette::Entry> {
        let entries = palette::entries(&self.document, palette_supports);
        self.commands.rows(&entries).into_iter().cloned().collect()
    }

    /// Rebuilds the row widgets for the current filter and selection.
    fn rebuild_commands(&mut self) {
        while let Some(child) = self.palette_view.rows.first_child() {
            self.palette_view.rows.remove(&child);
        }

        let rows = self.command_rows();
        self.commands.clamp(rows.len());
        self.palette_view.empty.set_visible(rows.is_empty());

        let owner = self.palette_view.owner.clone();
        for (index, entry) in rows.iter().enumerate() {
            let selected = index == self.commands.selected;
            self.palette_view.rows.append(&command_row(entry, selected, &owner));
        }
    }

    /// Moves the palette's selection one row, wrapping at both ends.
    fn command_step(&mut self, forward: bool) {
        let len = self.command_rows().len();
        self.commands.step(len, forward);
        self.restyle_commands();
        self.scroll_commands();
    }

    /// Paints the selected row and clears the mark on the others.
    fn restyle_commands(&self) {
        let mut child = self.palette_view.rows.first_child();
        let mut index = 0;
        while let Some(row) = child {
            if index == self.commands.selected {
                row.add_css_class("selected");
            } else {
                row.remove_css_class("selected");
            }
            index += 1;
            child = row.next_sibling();
        }
    }

    /// Brings the selected row into the list's viewport.
    fn scroll_commands(&self) {
        let mut child = self.palette_view.rows.first_child();
        for _ in 0..self.commands.selected {
            child = child.and_then(|row| row.next_sibling());
        }
        let Some(row) = child else {
            return;
        };
        let Some(bounds) = row.compute_bounds(&self.palette_view.rows) else {
            return;
        };

        let adjustment = self.palette_view.list.vadjustment();
        let top = f64::from(bounds.y());
        let bottom = top + f64::from(bounds.height());
        if top < adjustment.value() {
            adjustment.set_value(top);
        } else if bottom > adjustment.value() + adjustment.page_size() {
            adjustment.set_value(bottom - adjustment.page_size());
        }
    }

    /// Runs the row Enter chose, if any.
    fn run_selected_command(&mut self) {
        if !self.commands.open {
            return;
        }
        let rows = self.command_rows();
        let Some(entry) = rows.get(self.commands.selected) else {
            return;
        };
        let command = entry.command.clone();
        self.close_commands();
        self.run_palette_command(command);
    }

    /// Runs one palette row after the palette has closed.
    fn run_palette_command(&mut self, command: palette::Command) {
        match command {
            palette::Command::Action(action) => {
                self.command(action);
            }
            palette::Command::Profile(name) => self.open_profile(Some(&name)),
            // Extensions are not hosted on this shell yet; the row cannot be
            // offered because no extension registers here.
            palette::Command::Extension(_) => {}
        }
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
        // The new tab's bar has its own query, or none showing.
        self.seed_find(false);
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
        // A press on the strip dismisses the palette and goes no further.
        if self.commands.open {
            self.close_commands();
            return (true, None);
        }

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
                // The find field follows the tab it belongs to, without taking
                // the keyboard the strip press just gave the pane.
                self.seed_find(false);
                // A press on a tab body arms a reorder; the close box, the
                // buttons and the empty strip do not, so a close never drags.
                self.tab_drag = Some(index);
                self.focus_active();
            }
            // Middle-click closes a tab, the way upstream's tabs do.
            StripHit::Tab(index) | StripHit::Close(index) if button == 2 => self.close_tab(index),
            StripHit::Close(index) if button == 1 => self.close_tab(index),
            StripHit::Tab(_) | StripHit::Close(_) | StripHit::New => return (false, None),
        }

        (true, None)
    }

    /// Reorders the tabs while a strip press drags, once it has crossed a
    /// neighbour's midpoint.
    fn drag_strip(&mut self, x: f64) {
        let Some(from) = self.tab_drag else {
            return;
        };
        let (layout, _) = strip_and_menu(self.strip.width(), self.tabs.len());
        if let Some(target) = tabs::drag_target(&layout.tabs, from, x as i32)
            && self.tabs.move_to(from, target)
        {
            self.tab_drag = Some(target);
            self.strip.queue_draw();
        }
    }

    /// The cell a point in the pane names.
    fn cell_at(&self, point: (f64, f64)) -> Option<(i32, i32)> {
        let (width, height) = self.dip_cell();
        if width <= 0.0 || height <= 0.0 {
            return None;
        }
        let column = ((point.0 - PANE_LEFT) / width).floor() as i32;
        let row = (point.1 / height).floor() as i32;
        let (columns, rows) = self.grid;
        (column >= 0 && row >= 0 && column < columns && row < rows).then_some((column, row))
    }

    /// The cell in the pane's logical pixels, which is the unit the toolkit's
    /// pointer positions are reported in. The cell itself is physical pixels.
    fn dip_cell(&self) -> (f64, f64) {
        let scale = f64::from(finite_scale(self.scale));
        (f64::from(self.cell.0) / scale, f64::from(self.cell.1) / scale)
    }

    /// Records the pointer's cell and forwards the move a tracking program
    /// asked for.
    fn pointer_motion(&mut self, point: (f64, f64), modifiers: Modifiers) {
        if let Some(cell) = self.cell_at(point) {
            self.pointer = cell;
        }

        if !input::forwards_to_application(self.active_tracking(), modifiers.shift) {
            self.last_mouse_cell = None;
            return;
        }

        let Some(position) = self.cell_at(point) else {
            self.last_mouse_cell = None;
            return;
        };
        if self.last_mouse_cell == Some(position) {
            return;
        }

        let held = self.held;
        let Some(tab) = self.tabs.active_mut() else {
            return;
        };
        let chord = input::key_modifiers(modifiers, tab.sides.right_alt, tab.sides.right_ctrl);
        tab.control.send_mouse(&MouseEvent {
            position,
            kind: MouseEventKind::Move,
            buttons: held,
            modifiers: chord,
            delta: 0,
        });
        self.last_mouse_cell = Some(position);
    }

    /// Handles a pointer move, extending the selection a press started.
    fn pointer_moved(&mut self, point: (f64, f64), modifiers: Modifiers) {
        // The palette's list is the only thing the pointer moves while it shows.
        if self.commands.open {
            return;
        }
        if self.scrollbar_press {
            self.scrollbar_dragged(point.1);
            return;
        }
        self.pointer_motion(point, modifiers);
        // The press is consumed by the first move (its anchor is handed to the
        // selection), so the held button — not the pending press — decides
        // whether later moves extend.
        if self.held.left {
            self.pointer_drag(point, modifiers);
        }
    }

    /// Forgets the tab a strip press grabbed.
    fn drop_tab_drag(&mut self) {
        self.tab_drag = None;
    }

    /// Handles a press in the pane, returning whether it was consumed.
    fn pointer_pressed(&mut self, point: (f64, f64), button: MouseButton, modifiers: Modifiers) -> bool {
        // The palette owns the pointer while it shows: a press outside it
        // dismisses it, and one on it belongs to its rows.
        if self.commands.open {
            let inside = self
                .palette_view
                .card
                .compute_bounds(&self.palette_view.overlay)
                .is_some_and(|bounds| {
                    let (x, y) = (point.0 as f32, point.1 as f32);
                    x >= bounds.x()
                        && x <= bounds.x() + bounds.width()
                        && y >= bounds.y()
                        && y <= bounds.y() + bounds.height()
                });
            if !inside {
                self.close_commands();
                return true;
            }
            // Inside the card the press belongs to the row's own button, so the
            // pane keeps neither the gesture nor the keyboard.
            return false;
        }

        // A press on the pane takes the keyboard back from a tab, a menu or a
        // dialog that had it.
        self.area.grab_focus();

        // A press in the pane dismisses the find bar, the way it does upstream.
        if self.find_open() {
            self.close_find();
        }

        if button == MouseButton::Left
            && point.0 >= f64::from(self.area.width()) - f64::from(scrollbar::WIDTH)
            && self.scrollbar_pressed(point.1)
        {
            return true;
        }

        self.hold(button, true);
        let forward = input::forwards_to_application(self.active_tracking(), modifiers.shift);
        let cell = self.cell_at(point);
        if let Some(cell) = cell {
            self.pointer = cell;
        }

        // Without a tracking program, a non-primary press pastes: the pane's
        // middle- and right-click rule in every shell.
        if button != MouseButton::Left && !forward {
            self.paste_clipboard();
            return true;
        }

        let held = self.held;
        let Some(tab) = self.tabs.active_mut() else {
            return true;
        };

        if forward {
            if let Some(position) = cell {
                let chord = input::key_modifiers(modifiers, tab.sides.right_alt, tab.sides.right_ctrl);
                tab.control.send_mouse(&MouseEvent {
                    position,
                    kind: MouseEventKind::Press(button),
                    buttons: held,
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

    /// Extends the selection while the primary button is held, scrolling the
    /// view when the drag leaves the pane.
    fn pointer_drag(&mut self, point: (f64, f64), modifiers: Modifiers) {
        if input::forwards_to_application(self.active_tracking(), modifiers.shift) {
            return;
        }

        let (columns, rows) = self.grid;
        let (cell_width, cell_height) = self.dip_cell();
        let height = cell_height * f64::from(rows.max(0));
        let outside = if point.1 < 0.0 {
            -1
        } else if height > 0.0 && point.1 >= height {
            1
        } else {
            0
        };
        let column = cell_index(point.0 - PANE_LEFT, cell_width);

        let Some(tab) = self.tabs.active_mut() else {
            return;
        };
        let position = if outside != 0 {
            let max = tab.control.max_scroll();
            tab.view.drag_scroll(outside, max, Instant::now());
            let row = if outside < 0 { 0 } else { rows.max(1) - 1 };
            let column = column.clamp(0, columns.max(1) - 1);
            Some((column, input::screen_to_document(max, tab.view.scroll(), row)))
        } else {
            let row = cell_index(point.1, cell_height);
            (column >= 0 && row >= 0 && column < columns.max(1) && row < rows.max(1)).then(|| {
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

    /// Finishes a press: ends a scrollbar grab or forwards a tracking program's
    /// release.
    fn pointer_released(&mut self, point: (f64, f64), button: MouseButton, modifiers: Modifiers) {
        self.hold(button, false);

        if self.scrollbar_press {
            self.scrollbar_press = false;
            self.scrollbar.release();
            self.press = None;
            self.last_mouse_cell = None;
            return;
        }

        if self.tab_drag.take().is_some() {
            return;
        }

        if !input::forwards_to_application(self.active_tracking(), modifiers.shift) {
            self.press = None;
            self.last_mouse_cell = None;
            return;
        }

        let held = self.held;
        let Some(position) = self.cell_at(point) else {
            return;
        };
        let Some(tab) = self.tabs.active_mut() else {
            return;
        };
        let chord = input::key_modifiers(modifiers, tab.sides.right_alt, tab.sides.right_ctrl);
        tab.control.send_mouse(&MouseEvent {
            position,
            kind: MouseEventKind::Release(button),
            buttons: held,
            modifiers: chord,
            delta: 0,
        });
        self.last_mouse_cell = None;
    }

    /// Records a button's state for the next forwarded event.
    fn hold(&mut self, button: MouseButton, pressed: bool) {
        match button {
            MouseButton::Left => self.held.left = pressed,
            MouseButton::Middle => self.held.middle = pressed,
            MouseButton::Right => self.held.right = pressed,
        }
    }

    /// Whether the active program is tracking the mouse.
    fn active_tracking(&self) -> bool {
        self.tabs.active().is_some_and(|tab| tab.control.is_tracking_mouse())
    }

    /// The scrollbar's scroll, document height and visible rows.
    fn scroll_facts(&mut self) -> (i32, i32, i32) {
        let Some(tab) = self.tabs.active_mut() else {
            return (0, 0, 0);
        };
        let visible = tab.control.size().1;
        let history = tab.control.max_scroll();
        (tab.view.scroll(), history + visible, visible)
    }

    /// Takes a press on the scrollbar, jumping or grabbing the thumb.
    fn scrollbar_pressed(&mut self, y: f64) -> bool {
        let (scroll, document, visible) = self.scroll_facts();
        let track = f64::from(self.area.height());
        if document <= visible || track <= 0.0 {
            return false;
        }

        let fraction = (y / track).clamp(0.0, 1.0) as f32;
        match self.scrollbar.press(fraction, scroll, document, visible) {
            Press::Jump(to) => {
                if let Some(tab) = self.tabs.active_mut() {
                    tab.view.set_scroll(to, document - visible);
                }
            }
            Press::Grab => self.scrollbar_press = true,
        }
        true
    }

    /// Moves the view while the scrollbar's thumb is held.
    fn scrollbar_dragged(&mut self, y: f64) {
        let (_, document, visible) = self.scroll_facts();
        let track = f64::from(self.area.height());
        if track <= 0.0 {
            return;
        }

        let fraction = (y / track).clamp(0.0, 1.0) as f32;
        if let Some(to) = self.scrollbar.drag(fraction, document, visible)
            && let Some(tab) = self.tabs.active_mut()
        {
            tab.view.set_scroll(to, document - visible);
        }
    }

    /// Puts the selection on the clipboard, clearing it like upstream.
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

        let Some(display) = gdk::Display::default() else {
            return false;
        };
        display.clipboard().set_text(&text);
        tab.view.clear_selection();
        true
    }

    /// Asks for the clipboard, which GDK delivers asynchronously.
    fn paste_clipboard(&mut self) {
        self.paste_requested = true;
    }

    /// Sends clipboard text through the terminal's bracketed-paste filter.
    fn paste_text(&mut self, text: &str) {
        if let Some(tab) = self.tabs.active_mut() {
            tab.control.paste(text);
        }
    }

    /// Takes the deferred clipboard read, if one is waiting.
    fn take_paste_request(&mut self) -> bool {
        std::mem::take(&mut self.paste_requested)
    }

    /// Takes the query the find field should show, if one is waiting.
    fn take_find_seed(&mut self) -> Option<FindSeed> {
        self.find_seed.take()
    }

    /// Queues the active tab's query for the find field when its bar is open.
    fn seed_find(&mut self, focus: bool) {
        let Some(tab) = self.tabs.active() else {
            return;
        };
        if tab.find.open {
            self.find_seed = Some(FindSeed {
                query: tab.find.query.clone(),
                focus,
            });
        }
    }

    /// The document the settings window edits.
    fn settings(&self) -> &settings::Settings {
        &self.document
    }

    fn settings_path(&self) -> Option<&Path> {
        self.settings_path.as_deref()
    }

    fn warnings(&self) -> &[String] {
        &self.warnings
    }

    fn migrated_from(&self) -> Option<&Path> {
        self.migrated_from.as_deref()
    }

    fn settings_error(&self) -> Option<&str> {
        self.settings_error.as_deref()
    }

    /// Asks the timer to show the settings editor.
    fn request_settings(&mut self) {
        self.settings_requested = true;
    }

    /// Takes the settings request, if one is waiting.
    fn take_settings_request(&mut self) -> bool {
        std::mem::take(&mut self.settings_requested)
    }

    /// Writes a document and reloads it, returning the writer's error.
    fn save_document(&mut self, document: &settings::Settings) -> Result<(), String> {
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

    /// Whether the active tab's find bar is showing.
    fn find_open(&self) -> bool {
        self.tabs.active().is_some_and(|tab| tab.find.open)
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
            // The field takes the keyboard, so a half-composed terminal preview
            // is no longer where the user is typing.
            self.composition = Composition::default();
            let query = tab.find.query.clone();
            self.find_seed = Some(FindSeed { query, focus: true });
        }
    }

    /// Sets the active tab's find query and recomputes the matches.
    fn find_query(&mut self, query: String) {
        let Some(tab) = self.tabs.active_mut() else {
            return;
        };
        tab.find.query = query;
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

    /// Closes the active tab's find bar.
    fn close_find(&mut self) {
        if let Some(tab) = self.tabs.active_mut() {
            tab.find.hide();
        }
    }

    /// The find bar's query, current match and match count.
    fn find_state(&self) -> (bool, String, usize, usize) {
        let Some(tab) = self.tabs.active() else {
            return (false, String::new(), 0, 0);
        };
        let current = tab.find.current.map_or(0, |index| index + 1);
        (tab.find.open, tab.find.query.clone(), current, tab.find.matches.len())
    }

    /// Scrolls the viewport, or hands the wheel to a tracking program.
    fn wheel(&mut self, delta: f64, unit: gdk::ScrollUnit, shift: bool) -> glib::Propagation {
        // The palette's list owns the wheel while it shows; the pane behind it
        // must not move.
        if self.commands.open {
            return glib::Propagation::Stop;
        }

        let rows = wheel_rows(delta, unit, self.dip_cell().1, &mut self.wheel_pixels);
        if rows == 0 {
            return glib::Propagation::Proceed;
        }

        let Some(tab) = self.tabs.active_mut() else {
            return glib::Propagation::Proceed;
        };

        // Shift keeps the wheel for the viewport; otherwise a tracking program
        // owns it, the same rule the macOS and WinUI shells apply.
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
        self.sync_scale();
        self.fit(width, height);
        if self.gpu {
            self.paint_background(context, width, height);
            return;
        }

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
            if renderer.paint_frame().is_err() {
                return;
            }
            (background, tab.control.bell_alpha(Instant::now()))
        };
        let (background, bell) = painted;

        // Whatever the whole-cell grid does not cover stays the pane's colour.
        let (red, green, blue) = channels(background);
        context.set_source_rgb(red, green, blue);
        context.paint().ok();

        let (pixels_wide, pixels_high) = (self.shared.width(), self.shared.height());
        if pixels_wide > 0 && pixels_high > 0 {
            let mut bytes = Vec::with_capacity(4 * pixels_wide as usize * pixels_high as usize);
            self.shared.with_pixels(|pixels| {
                for pixel in pixels.iter().take(pixels_wide as usize * pixels_high as usize) {
                    bytes.extend_from_slice(&pixel.to_ne_bytes());
                }
            });

            // `0xAARRGGBB` in a `u32` is BGRA in memory, which is `ARGB32` on a
            // little-endian host -- Cairo's own layout, so no swizzle is needed.
            if let Ok(surface) = cairo::ImageSurface::create_for_data(
                bytes,
                cairo::Format::ARgb32,
                pixels_wide as i32,
                pixels_high as i32,
                4 * pixels_wide as i32,
            ) {
                // The pixels are physical, the context is logical: one image
                // pixel covers 1/scale of a logical pixel.
                let scale = f64::from(finite_scale(self.scale));
                context.save().ok();
                context.translate(PANE_LEFT, 0.0);
                context.scale(1.0 / scale, 1.0 / scale);
                if context.set_source_surface(&surface, 0.0, 0.0).is_ok() {
                    context.paint().ok();
                }
                context.restore().ok();
            }
        }

        let grid_height = self.dip_cell().1 * f64::from(self.grid.1);
        if let Some(alpha) = bell {
            context.set_source_rgba(1.0, 1.0, 1.0, f64::from(alpha) * 0.12);
            context.rectangle(PANE_LEFT, 0.0, f64::from(width) - PANE_LEFT, grid_height);
            context.fill().ok();
        }

        self.paint_scrollbar(context, width, height);
        self.announce_text();
    }

    /// Paints what the GL area does not cover: the pane's fill, the left margin
    /// and the scrollbar's gutter.
    fn paint_background(&self, context: &cairo::Context, width: i32, height: i32) {
        let (background, _) = self
            .tabs
            .active()
            .map_or((0x000C_0C0C, 0x00CC_CCCC), |tab| tab.control.default_colors());
        let (red, green, blue) = channels(background);
        context.set_source_rgb(red, green, blue);
        context.paint().ok();
        self.paint_scrollbar(context, width, height);
    }

    /// Draws one frame through the GL backend.
    ///
    /// GTK makes the pane's context current before the GL area's render signal,
    /// which is where the engine's backend draws; nothing is read back.
    fn paint_gpu(&mut self, width: i32, height: i32) {
        if !self.gpu {
            return;
        }
        // The GL area is inset by the margins, so its logical size is the grid.
        self.sync_scale();
        self.fit_grid(width, height);

        let blink_on = self.blink_on;
        let bell = {
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
            tab.control.bell_alpha(Instant::now())
        };

        if let Some(gl) = &self.gl {
            gl.set_bell(bell);
        }
        if self.renderer.paint_frame().is_err() || self.gl.as_ref().is_some_and(Gl::failed) {
            self.disable_gpu();
            return;
        }
        self.announce_text();
    }

    /// Falls back to the software path after a GL context or a frame failed.
    fn disable_gpu(&mut self) {
        if !self.gpu {
            return;
        }
        eprintln!("winter: the GPU content path is off; using the software path");
        self.gpu = false;
        self.gl_area.set_visible(false);
        self.rebuild_fonts();
        self.queue_paint();
    }

    /// Publishes the visible screen to the accessibility tree.
    ///
    /// A full `GtkAccessibleText` needs a custom widget class, which this shell
    /// does not own yet, so the screen is the pane's description: a screen
    /// reader can read the terminal even though it cannot address a cell. The
    /// remaining gap is named in ADR 075.
    fn announce_text(&mut self) {
        let text = self.renderer.data().visible_text();
        if text.is_empty() || text == self.accessible_text {
            return;
        }

        let label = if self.window_title.is_empty() {
            "Winter".to_string()
        } else {
            self.window_title.clone()
        };
        self.area.update_property(&[
            gtk::accessible::Property::Label(&label),
            gtk::accessible::Property::Description(&text),
        ]);
        self.accessible_text = text;
    }

    /// Paints the scrollbar, its thumb and the prompt marks over the pane.
    fn paint_scrollbar(&self, context: &cairo::Context, width: i32, height: i32) {
        let Some(tab) = self.tabs.active() else {
            return;
        };
        let visible = tab.control.size().1;
        let document = self.renderer.data().document_height();
        let track = f64::from(height);
        if document <= visible || track <= 0.0 {
            return;
        }

        let bar = f64::from(scrollbar::WIDTH);
        let left = f64::from(width) - bar;
        context.set_source_rgba(0.0, 0.0, 0.0, 0.35);
        context.rectangle(left, 0.0, bar, track);
        context.fill().ok();

        let (top, thumb) = scrollbar::thumb_pixels(tab.view.scroll(), document, visible, track);
        context.set_source_rgba(0.63, 0.63, 0.63, 0.8);
        context.rectangle(left + 1.0, top, (bar - 2.0).max(1.0), thumb);
        context.fill().ok();

        // Marks come after the thumb, the way upstream declares its mark image
        // above the scroll bar (ADR 059).
        for mark in self.renderer.data().marks() {
            let y = f64::from(scrollbar::position(mark.row, document)) * track - 0.75;
            if mark.error {
                context.set_source_rgba(0.92, 0.35, 0.35, 0.95);
            } else {
                context.set_source_rgba(0.43, 0.67, 1.0, 0.9);
            }
            context.rectangle(left, y, bar, 1.5);
            context.fill().ok();
        }
    }

    /// Paints the strip: the row's fill, a box per tab and the new-tab button.
    fn paint_strip(&mut self, context: &cairo::Context, width: i32, height: i32) {
        let colors = self.strip_colors();
        let scale = f64::from(finite_scale(self.scale));

        // A theme that names no row colour leaves GTK's own header bar showing.
        if let Some(row) = colors.row {
            set_rgb(context, row);
            context.paint().ok();
        }

        // The hairline under the row separates every tab that does not merge
        // with the pane; an inactive tab paints over it and redraws its own.
        set_rgb(context, colors.border);
        context.rectangle(0.0, f64::from(height) - 1.0, f64::from(width), 1.0);
        context.fill().ok();

        let (layout, menu) = strip_and_menu(width, self.tabs.len());
        let active = self.tabs.index();
        let baseline = self.title_baseline(height);
        // The icon is rasterised at the display's scale, like the title's face.
        let icon_pixels = (icons::SIZE * scale).round().max(1.0) as i32;

        for (index, slots) in layout.tabs.iter().enumerate() {
            let Some(tab) = self.tabs.get(index) else {
                continue;
            };
            let is_active = index == active;
            let (fill, text) = if is_active {
                (colors.active, colors.active_text)
            } else {
                (colors.inactive, colors.inactive_text)
            };
            let fill = if !is_active && self.pointer_is_over(*slots) {
                hovered(fill)
            } else {
                fill
            };

            if is_active {
                rounded_top(context, *slots, height, fill);
                // Breeze marks the selected tab with an accent line along its
                // top edge, inset so it does not square the rounded corners.
                let radius = RADIUS.min(f64::from(slots.width()) / 2.0);
                set_rgb(context, colors.accent);
                context.rectangle(
                    f64::from(slots.left) + radius,
                    0.0,
                    f64::from(slots.width()) - 2.0 * radius,
                    2.0,
                );
                context.fill().ok();
            } else {
                // Only the selected tab rounds its corners above the pane;
                // the rest are flat and separated by a hairline, as in Breeze.
                set_rgb(context, fill);
                context.rectangle(f64::from(slots.left), 0.0, f64::from(slots.width()), f64::from(height));
                context.fill().ok();
                if index > 0 {
                    set_rgb(context, colors.border);
                    context.rectangle(f64::from(slots.left), 0.0, 1.0, f64::from(height));
                    context.fill().ok();
                }
                set_rgb(context, colors.border);
                context.rectangle(
                    f64::from(slots.left),
                    f64::from(height) - 1.0,
                    f64::from(slots.width()),
                    1.0,
                );
                context.fill().ok();
            }

            // A program icon takes the title's room, so a themed icon shifts
            // the title right; a tab whose theme offers none keeps the title
            // where it was.
            let icon = self.icons.for_program(&self.strip, &tab.program, icon_pixels);
            // The box is in DIP like the strip, while `icon_pixels` is the
            // physical size the theme was asked to draw at.
            let icon_side = icons::SIZE.round() as i32;
            let (title_left, title_width) = match &icon {
                Some(_) => (
                    slots.left + TITLE_INSET + icon_side + icons::GAP,
                    slots.width() - TITLE_INSET - icon_side - icons::GAP - tabs::CLOSE,
                ),
                None => (slots.left + TITLE_INSET, slots.width() - TITLE_INSET - tabs::CLOSE),
            };
            if let Some(surface) = &icon {
                let top = (f64::from(height) - f64::from(icon_side)) / 2.0;
                draw_icon(context, surface, f64::from(slots.left + TITLE_INSET), top, scale);
            }
            if title_width > 0 {
                draw_text(
                    context,
                    &mut self.chrome,
                    &tab.title,
                    (f64::from(title_left), baseline),
                    f64::from(title_width),
                    text,
                    scale,
                );
            }

            let close = f64::from(slots.right - tabs::CLOSE / 2);
            glyph_cross(context, close, f64::from(height) / 2.0, blend(text, fill, 0.2));
        }

        // The two buttons are separate flat targets; a hover paints only the
        // one under the pointer, the way Breeze's toolbar buttons behave.
        let base = colors.row.unwrap_or(colors.active);
        let hover_fill = hovered(base);
        for (slots, glyph) in [
            (
                layout.new_tab,
                glyph_plus as fn(&cairo::Context, f64, f64, (f64, f64, f64)),
            ),
            (menu, glyph_chevron as fn(&cairo::Context, f64, f64, (f64, f64, f64))),
        ] {
            if self.pointer_is_over(slots) {
                fill_rounded(
                    context,
                    f64::from(slots.left) + 2.0,
                    2.0,
                    f64::from(slots.width()) - 4.0,
                    f64::from(height) - 4.0,
                    4.0,
                    hover_fill,
                );
            }
            glyph(context, centre(slots), f64::from(height) / 2.0, colors.button_text);
        }
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
    /// that reads on its own tab. Without a theme the desktop's window palette
    /// is the source, so the strip matches the other windows on KDE; only a
    /// desktop that publishes none leaves the header bar's own background and
    /// the pane-derived fallbacks.
    fn strip_colors(&self) -> StripColors {
        let (background, foreground) = self
            .tabs
            .active()
            .map(|tab| tab.control.default_colors())
            .unwrap_or((0x000C_0C0C, 0x00CC_CCCC));
        let pane = channels(background);
        let text = channels(foreground);
        let focused = self.window.is_active();

        let theme = self.document.theme();
        let themed_row = theme
            .and_then(|theme| theme.tab_row_background(focused))
            .map(color_channels);
        let themed_active = theme.and_then(|theme| theme.tab_background(true)).map(color_channels);
        let themed_inactive = theme.and_then(|theme| theme.tab_background(false)).map(color_channels);

        if themed_row.is_some() || themed_active.is_some() || themed_inactive.is_some() {
            let active = themed_active.unwrap_or(pane);
            let inactive = themed_inactive.unwrap_or_else(|| blend(pane, text, 0.08));
            let base = themed_row.unwrap_or(pane);
            return StripColors {
                row: themed_row,
                active,
                inactive,
                active_text: contrasting(active),
                inactive_text: contrasting(inactive),
                button_text: contrasting(base),
                accent: self
                    .palette
                    .map_or_else(|| blend(base, contrasting(base), 0.4), |chrome| channels(chrome.accent)),
                border: blend(base, contrasting(base), 0.15),
            };
        }

        if let Some(chrome) = self.palette {
            let row = channels(if focused {
                chrome.titlebar
            } else {
                chrome.titlebar_inactive
            });
            let foreground = channels(if focused {
                chrome.foreground
            } else {
                chrome.foreground_inactive
            });
            let dim = channels(chrome.foreground_inactive);
            return StripColors {
                row: Some(row),
                // The active tab merges with its pane, as in the other shells.
                active: pane,
                // Breeze's other tabs are a shade below the row.
                inactive: shade(row, 0.82),
                active_text: foreground,
                inactive_text: dim,
                button_text: foreground,
                accent: channels(chrome.accent),
                border: blend(row, foreground, 0.12),
            };
        }

        let inactive = blend(pane, text, 0.08);
        StripColors {
            row: None,
            active: pane,
            inactive,
            active_text: contrasting(pane),
            inactive_text: contrasting(inactive),
            button_text: contrasting(pane),
            accent: blend(pane, text, 0.35),
            border: blend(pane, text, 0.12),
        }
    }

    /// Where the title's baseline sits in a strip `height` tall.
    fn title_baseline(&self, height: i32) -> f64 {
        let scale = f64::from(finite_scale(self.scale));
        let (ascent, descent) = self.chrome.vertical_metrics();
        let ascent = f64::from(ascent) / scale;
        let box_height = ascent + f64::from(descent) / scale;
        // A short strip keeps the text on it rather than centred off the top.
        let top = ((f64::from(height) - box_height) / 2.0).max(0.0);
        top + ascent
    }

    /// Fits the terminal and the renderer to whole cells inside the pane's
    /// inset and the scrollbar.
    fn fit(&mut self, width: i32, height: i32) {
        let usable = f64::from(width) - PANE_LEFT - f64::from(scrollbar::WIDTH);
        let columns = cells_across(usable, self.cell.0, self.scale).max(1);
        let rows = cells_across(f64::from(height), self.cell.1, self.scale).max(1);
        self.apply_grid(columns, rows);
    }

    /// Fits the terminal to whole cells when the GL area is already exactly the
    /// grid, so no inset is subtracted.
    fn fit_grid(&mut self, width: i32, height: i32) {
        let columns = cells_across(f64::from(width), self.cell.0, self.scale).max(1);
        let rows = cells_across(f64::from(height), self.cell.1, self.scale).max(1);
        self.apply_grid(columns, rows);
    }

    /// Rebuilds the face when the widget's scale factor changed: the window
    /// moved to a monitor with a different density, or the desktop changed its
    /// scale. The new face is physical pixels at the new scale, and the next
    /// `fit` recomputes the grid from logical pixels.
    fn sync_scale(&mut self) {
        let scale = finite_scale(self.area.scale_factor() as f32);
        if (scale - self.scale).abs() > f32::EPSILON {
            self.scale = scale;
            self.rebuild_fonts();
        }
    }

    /// Tells every tab and the renderer how large the grid is.
    fn apply_grid(&mut self, columns: i32, rows: i32) {
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

    /// Asks the pane to paint: the GL area when the GPU path is active, and the
    /// base widget that holds the margins and the scrollbar either way.
    fn queue_paint(&self) {
        if self.gpu {
            self.gl_area.queue_render();
        }
        self.area.queue_draw();
    }

    /// Shows the active tab's title and asks both widgets to repaint.
    fn after_select(&mut self) {
        let title = self.tabs.active().map(|tab| tab.title.clone()).unwrap_or_default();
        if title != self.window_title {
            self.window.set_title(Some(&title));
            self.window_title = title;
        }
        self.strip.queue_draw();
        self.queue_paint();
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

        capture_widget(&self.window, path);
        if let Some(settings) = &self.settings_window {
            capture_widget(settings, &with_suffix(path, "-settings"));
        }
        app.quit();
    }
}

/// Writes `widget` exactly as GTK last painted it.
fn capture_widget(widget: &impl IsA<gtk::Widget>, path: &Path) -> bool {
    let paintable = WidgetPaintable::new(Some(widget));
    let width = f64::from(widget.width());
    let height = f64::from(widget.height());
    let snapshot = Snapshot::new();
    paintable.snapshot(&snapshot, width, height);

    let Some(node) = snapshot.to_node() else {
        eprintln!("winter: the window produced no render node");
        return false;
    };
    let Some(native) = widget.native() else {
        eprintln!("winter: the window has no native surface");
        return false;
    };
    let viewport = graphene::Rect::new(0.0, 0.0, width as f32, height as f32);
    let Some(renderer) = native.renderer() else {
        eprintln!("winter: the surface has no renderer");
        return false;
    };
    let texture = renderer.render_texture(&node, Some(&viewport));

    match texture.save_to_png(path) {
        Ok(()) => {
            println!("winter: wrote {}", path.display());
            true
        }
        Err(error) => {
            eprintln!("winter: cannot write {}: {error}", path.display());
            false
        }
    }
}

/// A sibling of `path` with `suffix` before its extension.
fn with_suffix(path: &Path, suffix: &str) -> PathBuf {
    let stem = path.file_stem().and_then(|stem| stem.to_str()).unwrap_or("winter-shot");
    let extension = path
        .extension()
        .and_then(|extension| extension.to_str())
        .unwrap_or("png");
    path.with_file_name(format!("{stem}{suffix}.{extension}"))
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
    /// The new-tab and menu glyphs.
    button_text: (f64, f64, f64),
    /// The line along the selected tab's top edge.
    accent: (f64, f64, f64),
    /// Separators between tabs and along the row's bottom edge.
    border: (f64, f64, f64),
}

/// A fill lifted towards its own text colour, for the element under the pointer.
fn hovered(fill: (f64, f64, f64)) -> (f64, f64, f64) {
    blend(fill, contrasting(fill), 0.15)
}

/// A fill scaled towards black, so a shade keeps its hue.
fn shade(fill: (f64, f64, f64), factor: f64) -> (f64, f64, f64) {
    (
        (fill.0 * factor).clamp(0.0, 1.0),
        (fill.1 * factor).clamp(0.0, 1.0),
        (fill.2 * factor).clamp(0.0, 1.0),
    )
}

/// Fills a rounded rectangle.
fn fill_rounded(
    context: &cairo::Context,
    left: f64,
    top: f64,
    width: f64,
    height: f64,
    radius: f64,
    fill: (f64, f64, f64),
) {
    use std::f64::consts::{FRAC_PI_2, PI};

    let radius = radius.min(width / 2.0).min(height / 2.0);
    let (right, bottom) = (left + width, top + height);
    set_rgb(context, fill);
    context.new_sub_path();
    context.arc(right - radius, top + radius, radius, -FRAC_PI_2, 0.0);
    context.arc(right - radius, bottom - radius, radius, 0.0, FRAC_PI_2);
    context.arc(left + radius, bottom - radius, radius, FRAC_PI_2, PI);
    context.arc(left + radius, top + radius, radius, PI, 3.0 * FRAC_PI_2);
    context.close_path();
    context.fill().ok();
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

/// The profile list the strip's button pops up, with the settings editor after
/// a separator.
fn profile_menu(document: &settings::Settings) -> gtk::gio::Menu {
    let menu = gtk::gio::Menu::new();
    for profile in &document.profiles {
        let item = gtk::gio::MenuItem::new(Some(&profile.name), None);
        item.set_action_and_target_value(Some("profiles.open"), Some(&profile.name.to_variant()));
        menu.append_item(&item);
    }

    let trailing = gtk::gio::Menu::new();
    trailing.append(Some("Settings…"), Some("profiles.settings"));
    trailing.append(Some("About Winter"), Some("profiles.about"));
    menu.append_section(None, &trailing);
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
    origin: (f64, f64),
    width: f64,
    color: (f64, f64, f64),
    scale: f64,
) {
    let shown = elide(rasterizer, text, width * scale);
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
    // The mask is physical; the context is logical.
    set_rgb(context, color);
    context.save().ok();
    context.scale(1.0 / scale, 1.0 / scale);
    context
        .mask_surface(&surface, origin.0 * scale, origin.1 * scale - f64::from(ascent_rows))
        .ok();
    context.restore().ok();
}

/// Paints a cached icon surface of `icons::SIZE` DIP at `(x, y)`.
///
/// The surface holds physical pixels while the strip draws in logical ones, so
/// the context is scaled the same way the title's mask is.
fn draw_icon(context: &cairo::Context, surface: &cairo::ImageSurface, x: f64, y: f64, scale: f64) {
    context.save().ok();
    context.translate(x, y);
    context.scale(1.0 / scale, 1.0 / scale);
    context.set_source_surface(surface, 0.0, 0.0).ok();
    context.paint().ok();
    context.restore().ok();
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
/// stops reading as dark; each shell's strip uses the same rule.
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

/// GDK's button number as the terminal's vocabulary.
fn mouse_button(button: u32) -> MouseButton {
    match button {
        2 => MouseButton::Middle,
        3 => MouseButton::Right,
        _ => MouseButton::Left,
    }
}

/// The UTF-16 offset of the character `chars` into `text`.
///
/// GTK counts the preedit caret in characters; the renderer's preview and
/// AppKit's are both in UTF-16 units, so the shared builder takes the converted
/// offset.
fn utf16_offset(text: &str, chars: usize) -> usize {
    text.chars().take(chars).map(char::len_utf16).sum()
}

/// Reads the clipboard and pastes it, which GDK delivers asynchronously.
fn paste_from_clipboard(shell: &Rc<RefCell<Shell>>) {
    let Some(display) = gdk::Display::default() else {
        return;
    };
    let clipboard = display.clipboard();
    let shell = Rc::clone(shell);
    glib::spawn_future_local(async move {
        if let Ok(Some(text)) = clipboard.read_text_future().await {
            shell.borrow_mut().paste_text(text.as_str());
        }
    });
}

/// Opens the native settings editor over the shell's whole document.
///
/// Shows the settings editor, focusing the one already open rather than
/// stacking a second copy.
fn show_settings(shell: &Rc<RefCell<Shell>>, parent: &ApplicationWindow) {
    let existing = shell.borrow().settings_window.clone();
    if let Some(window) = existing {
        window.present();
        return;
    }
    let opened = open_settings_window(shell, parent);
    shell.borrow_mut().settings_window = Some(opened);
}

/// Shows the About window: the version this build was compiled as, what the
/// program is, and where its source lives.
fn show_about(parent: &ApplicationWindow) {
    let mut builder = gtk::Window::builder()
        .title("About Winter")
        .transient_for(parent)
        .modal(true)
        .resizable(false)
        .default_width(360);
    if let Some(application) = parent.application() {
        builder = builder.application(&application);
    }
    let window = builder.build();

    let root = gtk::Box::new(gtk::Orientation::Vertical, 12);
    root.set_margin_top(20);
    root.set_margin_bottom(16);
    root.set_margin_start(20);
    root.set_margin_end(20);

    let title = gtk::Label::new(None);
    title.set_markup(&format!("<b>{}</b> {}", about::NAME, env!("CARGO_PKG_VERSION")));
    title.set_halign(gtk::Align::Center);
    let details_text = about::DESCRIPTION.join("\n");
    let details = gtk::Label::new(Some(details_text.as_str()));
    details.set_justify(gtk::Justification::Center);
    let source = gtk::LinkButton::with_label(about::REPOSITORY, about::REPOSITORY);
    source.set_halign(gtk::Align::Center);
    let close = gtk::Button::with_label("Close");
    close.add_css_class("suggested-action");
    close.set_halign(gtk::Align::Center);

    root.append(&title);
    root.append(&details);
    root.append(&source);
    root.append(&close);
    window.set_child(Some(&root));

    {
        let closer = window.clone();
        let keys = EventControllerKey::new();
        keys.connect_key_pressed(move |_controller, key, _code, _state| {
            if key == gdk::Key::Escape {
                closer.close();
                glib::Propagation::Stop
            } else {
                glib::Propagation::Proceed
            }
        });
        window.add_controller(keys);
    }
    {
        let closer = window.clone();
        close.connect_clicked(move |_| closer.close());
    }

    window.present();
}

/// Builds the settings editor.
///
/// Saving parses the text with the same reader startup uses and writes it
/// through `settings::save`, so what is in force is always the normalized file
/// (ADR 030's rule; ADR 075 records why the document is the editor).
fn open_settings_window(shell: &Rc<RefCell<Shell>>, parent: &ApplicationWindow) -> gtk::Window {
    let mut builder = gtk::Window::builder()
        .title("Winter Settings")
        .transient_for(parent)
        .default_width(640)
        .default_height(540);
    if let Some(application) = parent.application() {
        builder = builder.application(&application);
    }
    let window = builder.build();

    let root = gtk::Box::new(gtk::Orientation::Vertical, 8);
    root.set_margin_top(12);
    root.set_margin_bottom(12);
    root.set_margin_start(12);
    root.set_margin_end(12);

    let hint = gtk::Label::new(Some("Settings file — Save validates the TOML and writes it."));
    hint.set_xalign(0.0);

    let editor = gtk::TextView::new();
    editor.set_monospace(true);
    editor.set_wrap_mode(gtk::WrapMode::None);
    editor.set_left_margin(8);
    editor.set_right_margin(8);
    editor.set_top_margin(8);
    editor.set_bottom_margin(8);

    let scroll = gtk::ScrolledWindow::builder()
        .child(&editor)
        .vexpand(true)
        .has_frame(true)
        .build();

    let status = gtk::Label::new(None);
    status.set_xalign(0.0);
    status.set_wrap(true);
    status.add_css_class("dim-label");

    let buttons = gtk::Box::new(gtk::Orientation::Horizontal, 6);
    buttons.set_halign(gtk::Align::End);
    let close = gtk::Button::with_label("Close");
    let revert = gtk::Button::with_label("Revert");
    let save = gtk::Button::with_label("Save");
    save.add_css_class("suggested-action");
    buttons.append(&close);
    buttons.append(&revert);
    buttons.append(&save);

    root.append(&hint);
    root.append(&scroll);
    root.append(&status);
    root.append(&buttons);
    window.set_child(Some(&root));

    let sync = Rc::new({
        let shell = Rc::clone(shell);
        let editor = editor.clone();
        let status = status.clone();
        move || {
            let shell = shell.borrow();
            editor.buffer().set_text(&settings::to_toml(shell.settings()));
            status.set_text(&winter::config::status_text(
                shell.settings_path(),
                shell.migrated_from(),
                shell.settings_error(),
                shell.warnings(),
            ));
        }
    });

    {
        let shell = Rc::clone(shell);
        let editor = editor.clone();
        let status = status.clone();
        let sync = Rc::clone(&sync);
        save.connect_clicked(move |_| {
            let buffer = editor.buffer();
            let text = buffer.text(&buffer.start_iter(), &buffer.end_iter(), true);
            let document = match settings::parse(text.as_str()) {
                Ok(document) => document,
                Err(error) => {
                    status.set_text(&format!("The document is not valid: {error}"));
                    return;
                }
            };
            let result = shell.borrow_mut().save_document(&document);
            match result {
                Ok(()) => {
                    sync();
                    status.set_text("Saved.");
                }
                Err(error) => status.set_text(&error),
            }
        });
    }

    {
        let shell = Rc::clone(shell);
        let status = status.clone();
        let sync = Rc::clone(&sync);
        revert.connect_clicked(move |_| {
            shell.borrow_mut().reload_settings();
            sync();
            status.set_text("Reverted to the file on disk.");
        });
    }

    {
        let window = window.clone();
        close.connect_clicked(move |_| window.close());
    }

    {
        let closer = window.clone();
        let keys = EventControllerKey::new();
        keys.connect_key_pressed(move |_controller, key, _code, _state| {
            if key == gdk::Key::Escape {
                closer.close();
                glib::Propagation::Stop
            } else {
                glib::Propagation::Proceed
            }
        });
        window.add_controller(keys);
    }

    {
        // Closing leaves no editor behind, so the next request reads the file
        // again instead of presenting a disposed window.
        let cleared = Rc::clone(shell);
        window.connect_close_request(move |_window| {
            if let Ok(mut shell) = cleared.try_borrow_mut() {
                shell.settings_window = None;
            }
            glib::Propagation::Proceed
        });
    }

    sync();
    window.present();
    window
}

/// The cell a logical pixel position falls in; a toolkit reports sub-pixel
/// offsets and a motion that starts above the pane.
fn cell_index(position: f64, cell: f64) -> i32 {
    (position.max(0.0) / cell.max(1.0)) as i32
}

/// Whole cells that fit `pixels` logical pixels, for a cell of `cell` physical
/// pixels at `scale`.
///
/// GTK reports a widget's size in logical pixels while the face is rasterised
/// in physical ones, so the two units meet here: a 2x screen keeps the column
/// count a 1x screen had, instead of halving it in each direction.
fn cells_across(pixels: f64, cell: u16, scale: f32) -> i32 {
    let scale = f64::from(finite_scale(scale));
    let cell = f64::from(cell.max(1));
    if !pixels.is_finite() {
        return 1;
    }
    ((pixels.max(0.0) * scale / cell).floor() as i64).clamp(0, 4096) as i32
}

/// A scale a widget reported, or 1.0 when it reported nonsense.
fn finite_scale(scale: f32) -> f32 {
    if scale.is_finite() && scale > 0.0 { scale } else { 1.0 }
}

/// The composition scale of the display the window opens on.
///
/// `Shell::sync_scale` corrects this from the widget once it is mapped, so this
/// only needs to be good enough that the opening window's cells are not laid
/// out at the wrong density.
fn display_scale() -> f32 {
    let Some(display) = gdk::Display::default() else {
        return 1.0;
    };
    let Some(monitor) = display
        .monitors()
        .item(0)
        .and_then(|object| object.downcast::<gdk::Monitor>().ok())
    else {
        return 1.0;
    };
    finite_scale(monitor.scale_factor() as f32)
}

/// Turns one scroll event into whole rows towards the history.
///
/// A wheel reports detents and a touchpad reports pixels, so the pixels
/// accumulate until they make a row. GTK's delta is positive when the content
/// should move down, which is the opposite of the view's rows-above-the-screen.
fn wheel_rows(delta: f64, unit: gdk::ScrollUnit, cell: f64, remainder: &mut f64) -> i32 {
    let height = cell.max(1.0);
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
    /// The GPU path's own rasteriser, so the software fallback keeps its.
    gpu: FontDueRasterizer,
    chrome: FontDueRasterizer,
    cell: (u16, u16),
}

/// Finds the host's monospace face and measures it at `size` points for a
/// screen at `scale`.
///
/// The face, the cell and the renderer's target are physical pixels; the
/// `FontInfo` reports the same cell in DIPs, because the terminal and the pty
/// talk in logical units.
fn fonts(size: f32, cells: (i32, i32), scale: f32) -> Option<Fonts> {
    let scale = finite_scale(scale);
    let pixels = size * scale;
    let directories = winter::fonts::font_directories();
    if directories.is_empty() {
        return None;
    }

    let paths: Vec<&Path> = directories.iter().map(PathBuf::as_path).collect();
    let collection = renderer::FontCollection::from_directories(paths);
    let face = collection.default_monospace(renderer::PREFERRED_MONOSPACE_FAMILIES)?;

    let shaper = FontDueRasterizer::from_file(face.path(), pixels).ok()?;
    let rasterizer = FontDueRasterizer::from_file(face.path(), pixels).ok()?;
    let gpu = FontDueRasterizer::from_file(face.path(), pixels).ok()?;
    let chrome = chrome_face(&collection, face, size, scale)?;

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
            width: f32::from(cell_w) / scale,
            height: f32::from(cell_h) / scale,
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
            font_size: pixels,
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
        gpu,
        chrome,
        cell: (cell_w, cell_h),
    })
}

/// The face the strip's titles are drawn with.
///
/// A native title bar uses the desktop's UI font, so the terminal's monospace
/// face is only a fallback. The face is rasterised at the composition scale
/// because `draw_text` hands Cairo the coverage at `1/scale`.
fn chrome_face(
    collection: &renderer::FontCollection,
    terminal: &renderer::FontFace,
    terminal_size: f32,
    scale: f32,
) -> Option<FontDueRasterizer> {
    let description = gtk::Settings::default()
        .and_then(|settings| settings.gtk_font_name())
        .map(|name| gtk::pango::FontDescription::from_string(name.as_str()));
    let (family, points) = match description {
        Some(description) if description.size() > 0 => (
            description
                .family()
                .map(|family| family.to_string())
                .unwrap_or_default(),
            description.size() as f32 / gtk::pango::SCALE as f32,
        ),
        _ => (terminal.family().to_string(), terminal_size * 0.85),
    };
    let face = collection.resolve(&family, 400, false).unwrap_or(terminal);
    let pixels = points * 96.0 / 72.0 * finite_scale(scale);
    FontDueRasterizer::from_file(face.path(), pixels).ok()
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
        assert_eq!(cell_index(0.0, 9.0), 0);
        assert_eq!(cell_index(8.9, 9.0), 0);
        assert_eq!(cell_index(9.0, 9.0), 1);
        // A motion that starts above the pane still has a cell.
        assert_eq!(cell_index(-4.0, 9.0), 0);
    }

    #[test]
    fn a_high_dpi_screen_keeps_the_grid_it_had_at_1x() {
        // A 790-logical-pixel pane. At 1x the face's cell is 8 physical pixels;
        // at 2x it is 16, so both fit 98 columns. A scale-blind fit used the
        // logical width against the physical cell and halved the grid in each
        // direction, which is the quarter-window bug.
        assert_eq!(cells_across(790.0, 8, 1.0), 98);
        assert_eq!(cells_across(790.0, 16, 2.0), 98);
        // A 500-logical-pixel pane, 16-pixel cell at 1x and 32 at 2x: 31 rows.
        assert_eq!(cells_across(500.0, 16, 1.0), 31);
        assert_eq!(cells_across(500.0, 32, 2.0), 31);
    }

    #[test]
    fn a_scaled_pointer_position_names_the_same_cell() {
        // At 200% the physical cell is twice the logical one, and the toolkit
        // reports pointer positions in logical pixels.
        assert_eq!(cell_index(9.0, 18.0 / 2.0), 1);
        assert!(finite_scale(0.0) == 1.0 && finite_scale(f32::NAN) == 1.0);
    }

    #[test]
    fn the_new_instance_flag_is_taken_before_gapplication_sees_it() {
        let (flag, rest) = take_new_instance(vec!["winter".into(), "--new-instance".into()]);
        assert!(flag);
        assert_eq!(rest, vec![std::ffi::OsString::from("winter")]);

        let (flag, rest) = take_new_instance(vec!["winter".into()]);
        assert!(!flag);
        assert_eq!(rest, vec![std::ffi::OsString::from("winter")]);
    }

    #[test]
    fn a_wheel_detent_is_one_row_towards_the_history() {
        let mut remainder = 0.0;
        // GTK counts a delta positive when the content moves down; the view's
        // rows count up from the live screen.
        assert_eq!(wheel_rows(1.0, gdk::ScrollUnit::Wheel, 17.0, &mut remainder), -1);
        assert_eq!(wheel_rows(-1.0, gdk::ScrollUnit::Wheel, 17.0, &mut remainder), 1);
        assert_eq!(wheel_rows(0.0, gdk::ScrollUnit::Wheel, 17.0, &mut remainder), 0);
    }

    #[test]
    fn a_touchpad_accumulates_pixels_until_a_row() {
        let mut remainder = 0.0;
        assert_eq!(wheel_rows(8.0, gdk::ScrollUnit::Surface, 17.0, &mut remainder), 0);
        assert_eq!(wheel_rows(8.0, gdk::ScrollUnit::Surface, 17.0, &mut remainder), 0);
        assert_eq!(wheel_rows(8.0, gdk::ScrollUnit::Surface, 17.0, &mut remainder), -1);
        assert!(remainder > 0.0 && remainder < 17.0);

        // The pixels that did not make a row are kept in the other direction.
        assert_eq!(wheel_rows(-24.0, gdk::ScrollUnit::Surface, 17.0, &mut remainder), 1);
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
    fn the_preedit_caret_counts_utf16_units() {
        assert_eq!(utf16_offset("ni", 0), 0);
        assert_eq!(utf16_offset("ni", 1), 1);
        // GTK counts characters; an emoji is two UTF-16 units.
        assert_eq!(utf16_offset("😀日", 0), 0);
        assert_eq!(utf16_offset("😀日", 1), 2);
        assert_eq!(utf16_offset("😀日", 2), 3);
        // A caret past the end clamps.
        assert_eq!(utf16_offset("ni", 9), 2);
    }

    #[test]
    fn a_capture_name_keeps_its_folder_and_extension() {
        assert_eq!(
            with_suffix(Path::new("/tmp/winter-shot.png"), "-settings"),
            PathBuf::from("/tmp/winter-shot-settings.png")
        );
        assert_eq!(
            with_suffix(Path::new("shot"), "-settings"),
            PathBuf::from("shot-settings.png")
        );
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
    fn the_profile_menu_names_every_profile_and_offers_the_editor() {
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
        assert_eq!(menu.n_items(), 3);

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

        // The settings editor and About sit after the profiles, inside a section
        // so the popover draws a separator.
        let section = menu.item_link(2, "section").expect("a trailing section");
        assert_eq!(section.n_items(), 2);
        let settings = section.item_attribute_value(0, "label", None);
        assert_eq!(
            settings.and_then(|value| value.str().map(str::to_string)).as_deref(),
            Some("Settings…")
        );
        let about = section.item_attribute_value(1, "label", None);
        assert_eq!(
            about.and_then(|value| value.str().map(str::to_string)).as_deref(),
            Some("About Winter")
        );
    }

    #[test]
    fn a_frame_with_no_seed_does_not_rewrite_the_find_field() {
        // The shell's query lags the field: typing "he" leaves `find.query` at
        // "h" until the delayed change notification lands. A frame with no
        // seed must leave the two characters alone rather than restore "h".
        assert_eq!(frame_text(None, "he"), None);

        // A seed the shell chose wins, and an equal one writes nothing.
        let seed = FindSeed {
            query: "h".to_string(),
            focus: false,
        };
        assert_eq!(frame_text(Some(&seed), "he"), Some("h"));
        assert_eq!(frame_text(Some(&seed), "h"), None);
    }
}
