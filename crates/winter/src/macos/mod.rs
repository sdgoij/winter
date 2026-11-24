//! The macOS shell: AppKit hosts the window, `renderer` paints the terminal.
//!
//! The split is the one ADR 061 describes for Linux: the window, its menus, its
//! dialogs and its text input are AppKit's; the tab strip, the scrollbar and the
//! grid are pixels this shell draws, so the terminal looks the same as it does
//! on Windows and a colour scheme reaches the chrome unchanged.
//!
//! `main.rs` selects this shell under `cfg(target_os = "macos")`. The crate's
//! toolkit-free modules — `session`, `term_control`, `view`, `tabs`, `input`,
//! `find`, `scrollbar`, `terminal_render` — are shared unchanged.
//!
//! `unsafe` is confined to the AppKit boundary: this module declares the
//! allowance for its own calls and for `view`, `chrome`, `frame`, `icon` and
//! `settings`, which call objc2's window, menu and bitmap entry points (ADR
//! 072). `keys` and `shell` contain none.

#![allow(unsafe_code)]

use std::cell::{Cell, RefCell};
use std::path::{Path, PathBuf};
use std::rc::Rc;
use std::time::Instant;

use objc2::rc::Retained;
use objc2::runtime::{AnyObject, ProtocolObject, Sel};
use objc2::{DefinedClass, MainThreadMarker, MainThreadOnly, define_class, msg_send, sel};
use objc2_app_kit::{
    NSAlert, NSAppearance, NSAppearanceCustomization, NSAppearanceNameAqua, NSAppearanceNameDarkAqua, NSApplication,
    NSApplicationActivationPolicy, NSApplicationDelegate, NSBackingStoreType, NSBeep, NSBezelStyle,
    NSBitmapImageFileType, NSBitmapImageRepPropertyKey, NSButton, NSColor, NSControlTextEditingDelegate,
    NSEventModifierFlags, NSFont, NSMenu, NSMenuItem, NSSearchField, NSSearchFieldDelegate, NSTextField,
    NSTextFieldDelegate, NSTextView, NSTitlebarSeparatorStyle, NSWindow, NSWindowDelegate, NSWindowStyleMask,
    NSWindowTitleVisibility,
};
use objc2_foundation::{
    NSDictionary, NSNotification, NSObjectProtocol, NSPoint, NSRect, NSRunLoop, NSRunLoopCommonModes, NSSize, NSString,
    NSTimer,
};

use self::chrome::STRIP_HEIGHT;
use self::shell::Shell;
use self::view::TerminalView;

pub(crate) mod chrome;
pub(crate) mod frame;
pub(crate) mod icon;
pub(crate) mod keys;
pub(crate) mod settings;
pub(crate) mod shell;
pub(crate) mod view;

/// Runs the macOS shell until its window closes.
pub fn run() -> Result<(), Box<dyn std::error::Error>> {
    let mtm = MainThreadMarker::new().ok_or("the macOS shell must start on the main thread")?;
    let app = NSApplication::sharedApplication(mtm);
    app.setActivationPolicy(NSApplicationActivationPolicy::Regular);
    install_icon(&app, mtm);

    let face = find_face().ok_or("no monospaced face was found; install Menlo or Monaco")?;
    let shell = Rc::new(RefCell::new(Shell::new(face, mtm)?));

    let initial = view::default_size(shell.borrow().cell());
    let initial_title = shell.borrow().title().to_string();
    // SAFETY: every window and control here is created on the main thread,
    // which AppKit requires, and each is retained by the controller or by its
    // superview.
    let (window, view, search, previous, next, count) = unsafe {
        let window = NSWindow::initWithContentRect_styleMask_backing_defer(
            NSWindow::alloc(mtm),
            NSRect::new(NSPoint::new(0.0, 0.0), initial),
            NSWindowStyleMask::Titled
                | NSWindowStyleMask::Closable
                | NSWindowStyleMask::Miniaturizable
                | NSWindowStyleMask::Resizable
                | NSWindowStyleMask::FullSizeContentView,
            NSBackingStoreType::Buffered,
            false,
        );
        window.setTitle(&NSString::from_str(&initial_title));
        window.setTitlebarAppearsTransparent(true);
        window.setTitleVisibility(NSWindowTitleVisibility::Hidden);
        window.setTitlebarSeparatorStyle(NSTitlebarSeparatorStyle::None);
        window.setBackgroundColor(Some(&NSColor::windowBackgroundColor()));
        window.setMinSize(NSSize::new(360.0, 220.0));
        window.setReleasedWhenClosed(false);
        window.setAcceptsMouseMovedEvents(true);
        window.center();

        let view = TerminalView::new(NSRect::new(NSPoint::new(0.0, 0.0), initial), Rc::clone(&shell), mtm);
        window.setContentView(Some(&view));

        let search = NSSearchField::initWithFrame(
            NSSearchField::alloc(mtm),
            NSRect::new(NSPoint::new(0.0, 0.0), NSSize::new(170.0, 24.0)),
        );
        search.setPlaceholderString(Some(&NSString::from_str("Find")));
        search.setSendsSearchStringImmediately(true);

        let previous = NSButton::initWithFrame(
            NSButton::alloc(mtm),
            NSRect::new(NSPoint::new(0.0, 0.0), NSSize::new(24.0, 24.0)),
        );
        previous.setTitle(&NSString::from_str("↑"));
        previous.setBezelStyle(NSBezelStyle::Circular);

        let next = NSButton::initWithFrame(
            NSButton::alloc(mtm),
            NSRect::new(NSPoint::new(0.0, 0.0), NSSize::new(24.0, 24.0)),
        );
        next.setTitle(&NSString::from_str("↓"));
        next.setBezelStyle(NSBezelStyle::Circular);

        let count = NSTextField::labelWithString(&NSString::from_str(""), mtm);
        count.setFont(Some(&NSFont::systemFontOfSize(11.0)));

        view.addSubview(&search);
        view.addSubview(&previous);
        view.addSubview(&next);
        view.addSubview(&count);
        search.setHidden(true);
        previous.setHidden(true);
        next.setHidden(true);
        count.setHidden(true);

        (window, view, search, previous, next, count)
    };

    set_appearance(&window, shell.borrow().prefers_dark());

    let controller = Controller::new(
        Rc::clone(&shell),
        window.clone(),
        view.clone(),
        search.clone(),
        previous.clone(),
        next.clone(),
        count.clone(),
        mtm,
    );

    // SAFETY: each target is the controller, which outlives the controls and
    // the menu, and the selectors are the methods it implements.
    unsafe {
        search.setTarget(Some(controller.as_ref()));
        search.setAction(Some(sel!(findQuery:)));
        search.setDelegate(Some(ProtocolObject::from_ref(&*controller)));
        previous.setTarget(Some(controller.as_ref()));
        previous.setAction(Some(sel!(findPrevious:)));
        next.setTarget(Some(controller.as_ref()));
        next.setAction(Some(sel!(findNext:)));

        window.setDelegate(Some(ProtocolObject::from_ref(&*controller)));
        app.setDelegate(Some(ProtocolObject::from_ref(&*controller)));
    }

    install_menu(&app, &controller, mtm);

    // The timer is the pty's poll and the frame repaint; the connection is not
    // `Send`, so it runs on the UI thread (ADR 006).
    let timer = unsafe {
        NSTimer::timerWithTimeInterval_target_selector_userInfo_repeats(
            shell::TICK.as_secs_f64(),
            controller.as_ref(),
            sel!(tick:),
            None,
            true,
        )
    };
    *controller.ivars().timer.borrow_mut() = Some(timer);
    // The timer belongs to every mode, so a window drag or a menu's tracking
    // loop does not freeze the terminal.
    if let Some(timer) = &*controller.ivars().timer.borrow() {
        // SAFETY: the run loop and the mode are process-wide AppKit objects.
        unsafe {
            NSRunLoop::mainRunLoop().addTimer_forMode(timer, NSRunLoopCommonModes);
        }
    }

    {
        let bounds = view.bounds();
        let scale = window.backingScaleFactor();
        shell
            .borrow_mut()
            .resize((bounds.size.width, bounds.size.height), scale as f32);
    }

    window.makeKeyAndOrderFront(None);
    window.makeFirstResponder(Some(&view));
    app.activate();
    if std::env::var_os("WINTER_OPEN_SETTINGS").is_some() {
        controller.open_settings_window();
    }
    if std::env::var_os("WINTER_OPEN_FIND").is_some() {
        shell.borrow_mut().toggle_find();
        controller.sync_find();
    }
    app.run();

    Ok(())
}

/// Hands the application its drawn icon (ADR 058), so the Dock and the app
/// switcher show the same mark the Windows shell embeds.
fn install_icon(app: &NSApplication, _mtm: MainThreadMarker) {
    let Some(image) = icon::own() else {
        return;
    };
    // SAFETY: the image is alive for the call and is retained by the
    // application.
    unsafe { app.setApplicationIconImage(Some(&image)) };
}

/// Makes the window's native controls follow the terminal's background.
fn set_appearance(window: &NSWindow, dark: bool) {
    // SAFETY: both names are AppKit's appearance constants.
    let name = unsafe {
        if dark {
            NSAppearanceNameDarkAqua
        } else {
            NSAppearanceNameAqua
        }
    };
    if let Some(appearance) = NSAppearance::appearanceNamed(name) {
        window.setAppearance(Some(&appearance));
    }
}

/// The host's monospaced face, preferring one with an italic.
fn find_face() -> Option<renderer::FontFace> {
    let directories = crate::app::font_directories();
    let paths: Vec<&std::path::Path> = directories.iter().map(PathBuf::as_path).collect();
    let collection = renderer::FontCollection::from_directories(paths);
    collection
        .default_monospace_with_italic(renderer::PREFERRED_MONOSPACE_FAMILIES)
        .map(|(regular, _)| regular.clone())
        .or_else(|| {
            collection
                .default_monospace(renderer::PREFERRED_MONOSPACE_FAMILIES)
                .cloned()
        })
}

/// The window's controller: the timer, the menus and the find bar.
///
/// It owns the shell and the AppKit objects the shell's requests need; the view
/// owns the same `Rc` and paints without going through the controller.
struct ControllerIvars {
    shell: Rc<RefCell<Shell>>,
    window: Retained<NSWindow>,
    view: Retained<TerminalView>,
    search: Retained<NSSearchField>,
    previous: Retained<NSButton>,
    next: Retained<NSButton>,
    count: Retained<NSTextField>,
    settings_window: RefCell<Option<Retained<settings::SettingsWindow>>>,
    timer: RefCell<Option<Retained<NSTimer>>>,
    find_shown: Cell<bool>,
    /// The appearance last applied to the window.
    dark: Cell<bool>,
    /// Whether a tick is already running; a menu's tracking loop can fire the
    /// timer again before the first tick returns.
    busy: Cell<bool>,
    /// `WINTER_SCREENSHOT`: the path a first frame is written to, then quit.
    screenshot: Option<PathBuf>,
    screenshot_at: Instant,
}

define_class!(
    // SAFETY: NSObject has no subclassing requirements. Every ivar is a main
    // thread AppKit object or the shell's `Rc`; the class is main-thread only.
    #[unsafe(super(objc2_foundation::NSObject))]
    #[thread_kind = MainThreadOnly]
    #[name = "WinterController"]
    #[ivars = ControllerIvars]
    struct Controller;

    impl Controller {
        #[unsafe(method(tick:))]
        fn tick(&self, _timer: Option<&NSTimer>) {
            let Some(mtm) = MainThreadMarker::new() else {
                return;
            };
            if self.ivars().busy.replace(true) {
                return;
            }
            let tick = {
                let mut shell = self.ivars().shell.borrow_mut();
                shell.tick(Instant::now(), mtm)
            };

            if let Some(title) = tick.title {
                self.ivars().window.setTitle(&NSString::from_str(&title));
            }

            if tick.bell {
                NSBeep();
            }

            for (title, body) in &tick.notifications {
                crate::notifications::show(title, body);
            }

            if tick.close_window {
                self.ivars().window.performClose(None);
            }

            // The shell raises requests rather than calling AppKit, because the
            // view holds the same shell and a callback would re-enter it.
            let (menu, settings) = {
                let mut shell = self.ivars().shell.borrow_mut();
                (shell.menu_requested(), shell.settings_requested())
            };
            if menu {
                self.show_profiles();
            }
            if settings {
                self.open_settings_window();
            }

            self.sync_find();
            if tick.redraw {
                self.ivars().view.setNeedsDisplay(true);
            }

            let dark = self.ivars().shell.borrow().prefers_dark();
            if dark != self.ivars().dark.replace(dark) {
                set_appearance(&self.ivars().window, dark);
            }

            if let Some(path) = self.ivars().screenshot.clone()
                && Instant::now() >= self.ivars().screenshot_at
            {
                self.write_screenshot(&path);
                NSApplication::sharedApplication(mtm).terminate(None);
            }

            self.ivars().busy.set(false);
        }

        #[unsafe(method(newTab:))]
        fn new_tab(&self, _sender: Option<&AnyObject>) {
            self.ivars().shell.borrow_mut().open_tab();
            self.ivars().view.setNeedsDisplay(true);
        }

        #[unsafe(method(closeTab:))]
        fn close_tab(&self, _sender: Option<&AnyObject>) {
            self.ivars().shell.borrow_mut().close_active_tab();
        }

        #[unsafe(method(nextTab:))]
        fn next_tab(&self, _sender: Option<&AnyObject>) {
            self.ivars().shell.borrow_mut().step_tab(1);
            self.ivars().view.setNeedsDisplay(true);
        }

        #[unsafe(method(previousTab:))]
        fn previous_tab(&self, _sender: Option<&AnyObject>) {
            self.ivars().shell.borrow_mut().step_tab(-1);
            self.ivars().view.setNeedsDisplay(true);
        }

        #[unsafe(method(toggleFind:))]
        fn toggle_find(&self, _sender: Option<&AnyObject>) {
            self.ivars().shell.borrow_mut().toggle_find();
            self.sync_find();
        }

        #[unsafe(method(findNext:))]
        fn find_next(&self, _sender: Option<&AnyObject>) {
            self.ivars().shell.borrow_mut().find_step(true);
            self.ivars().view.setNeedsDisplay(true);
        }

        #[unsafe(method(findPrevious:))]
        fn find_previous(&self, _sender: Option<&AnyObject>) {
            self.ivars().shell.borrow_mut().find_step(false);
            self.ivars().view.setNeedsDisplay(true);
        }

        #[unsafe(method(findQuery:))]
        fn find_query(&self, _sender: Option<&AnyObject>) {
            let text = self.ivars().search.stringValue().to_string();
            self.ivars().shell.borrow_mut().find_query(text);
            self.ivars().view.setNeedsDisplay(true);
            self.update_count();
        }

        #[unsafe(method(findDone:))]
        fn find_done(&self, _sender: Option<&AnyObject>) {
            self.finish_find();
        }

        #[unsafe(method(openSettings:))]
        fn open_settings(&self, _sender: Option<&AnyObject>) {
            self.open_settings_window();
        }

        #[unsafe(method(showAbout:))]
        fn show_about(&self, _sender: Option<&AnyObject>) {
            self.show_about_window();
        }

        #[unsafe(method(selectProfile:))]
        fn select_profile(&self, sender: Option<&NSMenuItem>) {
            let Some(sender) = sender else {
                return;
            };
            let index = sender.tag();
            if index < 0 {
                return;
            }
            let name = {
                let shell = self.ivars().shell.borrow();
                shell
                    .profiles()
                    .get(index as usize)
                    .map(|profile| profile.name.clone())
            };
            if let Some(name) = name {
                self.ivars().shell.borrow_mut().open_profile(&name);
                self.ivars().view.setNeedsDisplay(true);
            }
        }
    }

    unsafe impl NSObjectProtocol for Controller {}

    unsafe impl NSApplicationDelegate for Controller {
        #[unsafe(method(applicationShouldTerminateAfterLastWindowClosed:))]
        fn terminate_after_last_window(&self, _sender: &NSApplication) -> bool {
            true
        }
    }

    unsafe impl NSWindowDelegate for Controller {
        #[unsafe(method(windowDidResize:))]
        fn window_did_resize(&self, notification: &NSNotification) {
            let Some(window) = notification.object().and_then(|object| object.downcast::<NSWindow>().ok()) else {
                return;
            };
            if window != self.ivars().window {
                return;
            }
            let bounds = self.ivars().view.bounds();
            self.ivars()
                .shell
                .borrow_mut()
                .resize((bounds.size.width, bounds.size.height), window.backingScaleFactor() as f32);
            self.layout_find();
            self.ivars().view.setNeedsDisplay(true);
        }

        #[unsafe(method(windowDidBecomeKey:))]
        fn window_did_become_key(&self, notification: &NSNotification) {
            let _ = notification;
            self.ivars().shell.borrow_mut().set_focused(true);
            self.ivars().view.setNeedsDisplay(true);
        }

        #[unsafe(method(windowDidResignKey:))]
        fn window_did_resign_key(&self, notification: &NSNotification) {
            let _ = notification;
            self.ivars().shell.borrow_mut().set_focused(false);
            self.ivars().view.setNeedsDisplay(true);
        }
    }

    unsafe impl NSControlTextEditingDelegate for Controller {
        #[unsafe(method(control:textView:doCommandBySelector:))]
        fn control_text_view_do_command(
            &self,
            _control: &objc2_app_kit::NSControl,
            _text_view: &NSTextView,
            selector: Sel,
        ) -> objc2::runtime::Bool {
            if selector == sel!(cancelOperation:) {
                self.finish_find();
                return true.into();
            }
            false.into()
        }
    }

    unsafe impl NSTextFieldDelegate for Controller {}

    unsafe impl NSSearchFieldDelegate for Controller {}
);

impl Controller {
    #[allow(clippy::too_many_arguments)]
    fn new(
        shell: Rc<RefCell<Shell>>,
        window: Retained<NSWindow>,
        view: Retained<TerminalView>,
        search: Retained<NSSearchField>,
        previous: Retained<NSButton>,
        next: Retained<NSButton>,
        count: Retained<NSTextField>,
        mtm: MainThreadMarker,
    ) -> Retained<Self> {
        let dark = shell.borrow().prefers_dark();
        let this = Self::alloc(mtm).set_ivars(ControllerIvars {
            shell,
            window,
            view,
            search,
            previous,
            next,
            count,
            settings_window: RefCell::new(None),
            timer: RefCell::new(None),
            find_shown: Cell::new(false),
            dark: Cell::new(dark),
            busy: Cell::new(false),
            screenshot: std::env::var_os("WINTER_SCREENSHOT").map(PathBuf::from),
            screenshot_at: Instant::now() + std::time::Duration::from_secs(1),
        });
        // SAFETY: NSObject's designated initializer, on the main thread.
        unsafe { msg_send![super(this), init] }
    }

    /// Shows, moves or hides the find controls to match the shell.
    fn sync_find(&self) {
        let (open, query, _, _) = self.ivars().shell.borrow().find_state();
        let focus = self.ivars().shell.borrow_mut().take_find_focus();
        let ivars = self.ivars();

        if open {
            if !ivars.find_shown.get() {
                ivars.find_shown.set(true);
                ivars.search.setHidden(false);
                ivars.previous.setHidden(false);
                ivars.next.setHidden(false);
                ivars.count.setHidden(false);
                self.layout_find();
            }
            if ivars.search.stringValue().to_string() != query {
                ivars.search.setStringValue(&NSString::from_str(&query));
            }
            self.update_count();
            if focus {
                ivars.window.makeFirstResponder(Some(&ivars.search));
            }
        } else if ivars.find_shown.replace(false) {
            ivars.search.setHidden(true);
            ivars.previous.setHidden(true);
            ivars.next.setHidden(true);
            ivars.count.setHidden(true);
            ivars.window.makeFirstResponder(Some(&ivars.view));
        }
    }

    fn update_count(&self) {
        let (_, _, current, total) = self.ivars().shell.borrow().find_state();
        let text = if total == 0 {
            "No matches".to_string()
        } else {
            format!("{current} of {total}")
        };
        self.ivars().count.setStringValue(&NSString::from_str(&text));
    }

    fn layout_find(&self) {
        let bounds = self.ivars().view.bounds();
        let width = bounds.size.width;
        let y = STRIP_HEIGHT + 8.0;
        let count_x = (width - 100.0).max(8.0);
        let next_x = (width - 128.0).max(8.0);
        let previous_x = (width - 156.0).max(8.0);
        let search_x = (width - 334.0).max(8.0);

        self.ivars()
            .search
            .setFrame(NSRect::new(NSPoint::new(search_x, y), NSSize::new(170.0, 24.0)));
        self.ivars()
            .previous
            .setFrame(NSRect::new(NSPoint::new(previous_x, y), NSSize::new(24.0, 24.0)));
        self.ivars()
            .next
            .setFrame(NSRect::new(NSPoint::new(next_x, y), NSSize::new(24.0, 24.0)));
        self.ivars()
            .count
            .setFrame(NSRect::new(NSPoint::new(count_x, y + 4.0), NSSize::new(80.0, 16.0)));
    }

    fn show_profiles(&self) {
        let Some(mtm) = MainThreadMarker::new() else {
            return;
        };
        let profiles = {
            let shell = self.ivars().shell.borrow();
            shell.profiles().to_vec()
        };

        // SAFETY: every menu and item is built and shown on the main thread;
        // the controller outlives the menu, and the selectors are its methods.
        unsafe {
            let menu = NSMenu::initWithTitle(NSMenu::alloc(mtm), &NSString::from_str("Profiles"));
            for (index, profile) in profiles.iter().enumerate() {
                let item = NSMenuItem::initWithTitle_action_keyEquivalent(
                    NSMenuItem::alloc(mtm),
                    &NSString::from_str(&profile.name),
                    Some(sel!(selectProfile:)),
                    &NSString::from_str(""),
                );
                item.setTarget(Some(self.as_ref()));
                item.setTag(index as isize);
                menu.addItem(&item);
            }
            menu.addItem(&NSMenuItem::separatorItem(mtm));

            let settings = NSMenuItem::initWithTitle_action_keyEquivalent(
                NSMenuItem::alloc(mtm),
                &NSString::from_str("Settings…"),
                Some(sel!(openSettings:)),
                &NSString::from_str(""),
            );
            settings.setTarget(Some(self.as_ref()));
            menu.addItem(&settings);

            let about = NSMenuItem::initWithTitle_action_keyEquivalent(
                NSMenuItem::alloc(mtm),
                &NSString::from_str("About Winter"),
                Some(sel!(showAbout:)),
                &NSString::from_str(""),
            );
            about.setTarget(Some(self.as_ref()));
            menu.addItem(&about);

            let strip = self.ivars().shell.borrow().strip();
            let point = NSPoint::new(f64::from(strip.menu.left), STRIP_HEIGHT);
            menu.popUpMenuPositioningItem_atLocation_inView(None, point, Some(&self.ivars().view));
        }
    }

    fn finish_find(&self) {
        self.ivars().shell.borrow_mut().close_find();
        self.sync_find();
    }

    /// Writes the window's content view as a PNG, and the settings window too
    /// when it is open.
    fn write_screenshot(&self, path: &Path) {
        if capture_view(&self.ivars().view, path) {
            println!("winter: wrote {}", path.display());
        } else {
            eprintln!("winter: cannot write {}", path.display());
        }

        if let Some(settings) = &*self.ivars().settings_window.borrow() {
            let settings_path = path.with_file_name(format!(
                "{}-settings.png",
                path.file_stem()
                    .map_or_else(|| "winter".into(), |stem| stem.to_string_lossy())
            ));
            if settings.capture(&settings_path) {
                println!("winter: wrote {}", settings_path.display());
            }
        }
    }

    fn open_settings_window(&self) {
        let Some(mtm) = MainThreadMarker::new() else {
            return;
        };
        let mut slot = self.ivars().settings_window.borrow_mut();
        if slot.is_none() {
            *slot = Some(settings::SettingsWindow::new(Rc::clone(&self.ivars().shell), mtm));
        }
        if let Some(window) = slot.as_ref() {
            window.open(mtm);
        }
    }

    fn show_about_window(&self) {
        let Some(mtm) = MainThreadMarker::new() else {
            return;
        };
        let alert = NSAlert::new(mtm);
        alert.setMessageText(&NSString::from_str("Winter"));
        alert.setInformativeText(&NSString::from_str(&format!(
            "A cross-platform terminal emulator.\n\nVersion {}\nhttps://github.com/sdgoij/winterm",
            env!("CARGO_PKG_VERSION")
        )));
        alert.addButtonWithTitle(&NSString::from_str("OK"));
        alert.runModal();
    }
}

/// Builds the application menu and the window's commands.
fn install_menu(app: &NSApplication, controller: &Controller, mtm: MainThreadMarker) {
    let target = Some(controller.as_ref());

    // SAFETY: every menu and item is created on the main thread and installed
    // in the application's menu bar, which AppKit reads on the same thread.
    unsafe {
        let main = NSMenu::initWithTitle(NSMenu::alloc(mtm), &NSString::from_str("Main"));

        let app_item = NSMenuItem::initWithTitle_action_keyEquivalent(
            NSMenuItem::alloc(mtm),
            &NSString::from_str("Winter"),
            None,
            &NSString::from_str(""),
        );
        let app_menu = NSMenu::initWithTitle(NSMenu::alloc(mtm), &NSString::from_str("Winter"));
        app_menu.addItem(&item(
            mtm,
            "About Winter",
            Some(sel!(showAbout:)),
            "",
            NSEventModifierFlags::empty(),
            target,
        ));
        app_menu.addItem(&NSMenuItem::separatorItem(mtm));
        app_menu.addItem(&item(
            mtm,
            "Settings…",
            Some(sel!(openSettings:)),
            ",",
            NSEventModifierFlags::Command,
            target,
        ));
        app_menu.addItem(&NSMenuItem::separatorItem(mtm));
        app_menu.addItem(&item(
            mtm,
            "Hide Winter",
            Some(sel!(hide:)),
            "h",
            NSEventModifierFlags::Command,
            None,
        ));
        app_menu.addItem(&item(
            mtm,
            "Hide Others",
            Some(sel!(hideOtherApplications:)),
            "h",
            NSEventModifierFlags::Command | NSEventModifierFlags::Option,
            None,
        ));
        app_menu.addItem(&item(
            mtm,
            "Show All",
            Some(sel!(unhideAllApplications:)),
            "",
            NSEventModifierFlags::empty(),
            None,
        ));
        app_menu.addItem(&NSMenuItem::separatorItem(mtm));
        app_menu.addItem(&item(
            mtm,
            "Quit Winter",
            Some(sel!(terminate:)),
            "q",
            NSEventModifierFlags::Command,
            None,
        ));
        app_item.setSubmenu(Some(&app_menu));
        main.addItem(&app_item);

        let shell_item = NSMenuItem::initWithTitle_action_keyEquivalent(
            NSMenuItem::alloc(mtm),
            &NSString::from_str("Shell"),
            None,
            &NSString::from_str(""),
        );
        let shell_menu = NSMenu::initWithTitle(NSMenu::alloc(mtm), &NSString::from_str("Shell"));
        shell_menu.addItem(&item(
            mtm,
            "New Tab",
            Some(sel!(newTab:)),
            "t",
            NSEventModifierFlags::Command,
            target,
        ));
        shell_menu.addItem(&item(
            mtm,
            "Close Tab",
            Some(sel!(closeTab:)),
            "w",
            NSEventModifierFlags::Command,
            target,
        ));
        shell_menu.addItem(&NSMenuItem::separatorItem(mtm));
        shell_menu.addItem(&item(
            mtm,
            "Next Tab",
            Some(sel!(nextTab:)),
            "]",
            NSEventModifierFlags::Command | NSEventModifierFlags::Shift,
            target,
        ));
        shell_menu.addItem(&item(
            mtm,
            "Previous Tab",
            Some(sel!(previousTab:)),
            "[",
            NSEventModifierFlags::Command | NSEventModifierFlags::Shift,
            target,
        ));
        shell_menu.addItem(&NSMenuItem::separatorItem(mtm));
        shell_menu.addItem(&item(
            mtm,
            "Find…",
            Some(sel!(toggleFind:)),
            "f",
            NSEventModifierFlags::Command,
            target,
        ));
        shell_item.setSubmenu(Some(&shell_menu));
        main.addItem(&shell_item);

        let edit_item = NSMenuItem::initWithTitle_action_keyEquivalent(
            NSMenuItem::alloc(mtm),
            &NSString::from_str("Edit"),
            None,
            &NSString::from_str(""),
        );
        let edit_menu = NSMenu::initWithTitle(NSMenu::alloc(mtm), &NSString::from_str("Edit"));
        edit_menu.addItem(&item(
            mtm,
            "Undo",
            Some(sel!(undo:)),
            "z",
            NSEventModifierFlags::Command,
            None,
        ));
        edit_menu.addItem(&item(
            mtm,
            "Redo",
            Some(sel!(redo:)),
            "Z",
            NSEventModifierFlags::Command | NSEventModifierFlags::Shift,
            None,
        ));
        edit_menu.addItem(&NSMenuItem::separatorItem(mtm));
        edit_menu.addItem(&item(
            mtm,
            "Cut",
            Some(sel!(cut:)),
            "x",
            NSEventModifierFlags::Command,
            None,
        ));
        edit_menu.addItem(&item(
            mtm,
            "Copy",
            Some(sel!(copy:)),
            "c",
            NSEventModifierFlags::Command,
            None,
        ));
        edit_menu.addItem(&item(
            mtm,
            "Paste",
            Some(sel!(paste:)),
            "v",
            NSEventModifierFlags::Command,
            None,
        ));
        edit_menu.addItem(&item(
            mtm,
            "Select All",
            Some(sel!(selectAll:)),
            "a",
            NSEventModifierFlags::Command,
            None,
        ));
        edit_item.setSubmenu(Some(&edit_menu));
        main.addItem(&edit_item);

        let window_item = NSMenuItem::initWithTitle_action_keyEquivalent(
            NSMenuItem::alloc(mtm),
            &NSString::from_str("Window"),
            None,
            &NSString::from_str(""),
        );
        let window_menu = NSMenu::initWithTitle(NSMenu::alloc(mtm), &NSString::from_str("Window"));
        window_menu.addItem(&item(
            mtm,
            "Minimize",
            Some(sel!(performMiniaturize:)),
            "m",
            NSEventModifierFlags::Command,
            None,
        ));
        window_menu.addItem(&item(
            mtm,
            "Zoom",
            Some(sel!(performZoom:)),
            "",
            NSEventModifierFlags::empty(),
            None,
        ));
        window_menu.addItem(&NSMenuItem::separatorItem(mtm));
        window_menu.addItem(&item(
            mtm,
            "Bring All to Front",
            Some(sel!(arrangeInFront:)),
            "",
            NSEventModifierFlags::empty(),
            None,
        ));
        window_item.setSubmenu(Some(&window_menu));
        main.addItem(&window_item);
        app.setWindowsMenu(Some(&window_menu));

        app.setMainMenu(Some(&main));
    }
}

fn item(
    mtm: MainThreadMarker,
    title: &str,
    action: Option<Sel>,
    key: &str,
    mask: NSEventModifierFlags,
    target: Option<&AnyObject>,
) -> Retained<NSMenuItem> {
    // SAFETY: the item is created on the main thread; the selector and target
    // are the caller's, and the caller owns both for the process's life.
    unsafe {
        let item = NSMenuItem::initWithTitle_action_keyEquivalent(
            NSMenuItem::alloc(mtm),
            &NSString::from_str(title),
            action,
            &NSString::from_str(key),
        );
        item.setKeyEquivalentModifierMask(mask);
        item.setTarget(target);
        item
    }
}

/// Captures one view as a PNG.
///
/// `WINTER_SCREENSHOT` uses this for the content view and for the settings
/// window, which is the live check on a host with no WindowServer session.
pub(crate) fn capture_view(view: &objc2_app_kit::NSView, path: &Path) -> bool {
    let bounds = view.bounds();
    let Some(rep) = view.bitmapImageRepForCachingDisplayInRect(bounds) else {
        eprintln!("winter: the window produced no bitmap");
        return false;
    };
    view.cacheDisplayInRect_toBitmapImageRep(bounds, &rep);
    let properties = NSDictionary::<NSBitmapImageRepPropertyKey, AnyObject>::new();
    // SAFETY: `rep` came from this view, and the dictionary is AppKit's
    // properties dictionary for a PNG representation.
    let Some(data) = (unsafe { rep.representationUsingType_properties(NSBitmapImageFileType::PNG, &properties) })
    else {
        eprintln!("winter: the window produced no PNG");
        return false;
    };
    match std::fs::write(path, data.to_vec()) {
        Ok(()) => true,
        Err(error) => {
            eprintln!("winter: cannot write {}: {error}", path.display());
            false
        }
    }
}
