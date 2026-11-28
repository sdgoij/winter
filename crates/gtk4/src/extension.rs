//! Extensions in the GTK shell: the [`Host`] the engine calls, the pump that
//! turns the window's changes into events, the extensions screen, and what it
//! lists.
//!
//! Loading is off by default: `[extensions] enabled = true` in the settings file
//! turns it on, and `WINTER_EXT` overrides the file for one run either way —
//! set it to `0` to keep the engine off, to anything else to force it on.
//! `WINTER_EXTENSIONS` adds a discovery root beside the standard ones.
//!
//! The engine is rebuilt without a restart when the `[extensions]` table
//! changes, when the switches are saved, and when a screen installs or removes a
//! directory. `[extensions] watch = true` (or `WINTER_WATCH` for one run) also
//! rebuilds when a file the loader read changes (ADR 088).
//!
//! Reads go through [`Host`] before a handler runs; writes are queued by the
//! engine and applied when it returns. The shell therefore never lets an
//! extension re-enter it mid-frame, and the engine is moved out of the field
//! for the duration of a dispatch so the shell can be its own host.

use std::cell::RefCell;
use std::collections::HashMap;
use std::path::PathBuf;
use std::rc::Rc;
use std::time::Duration;

use gtk::prelude::*;
use gtk4 as gtk;

use winter::grid::MarkTick;
use winter::layout::PaneId;

use super::Shell;

/// One extension the extensions screen lists.
#[derive(Debug, Clone, PartialEq, Eq)]
pub(crate) struct Found {
    /// The identifier its manifest declares, or the directory's name when the
    /// manifest could not be read.
    pub(crate) id: String,
    /// The name a user sees.
    pub(crate) name: String,
    /// `script`, `compiled`, `script + compiled`, `assets`, or `broken`.
    pub(crate) kind: &'static str,
    /// Whether loading it is switched on and off by this row.
    pub(crate) switchable: bool,
    /// Whether it is enabled: switched on, and not on the disabled list.
    pub(crate) enabled: bool,
    /// Why it is not running, when it is not.
    pub(crate) note: String,
    /// The registry record beside the manifest, when a registry installed it.
    pub(crate) source: Option<registry::Provenance>,
}

/// Every extension on disk, as the extensions screen lists it.
///
/// Discovery runs *without* the disabled list, because the screen has to show a
/// switched-off extension in order to offer it again. `failures` is what the
/// live engine reported at start, keyed by identifier; an installation with the
/// engine off has none, and every row then just follows the file.
pub(crate) fn installed(roots: Vec<PathBuf>, disabled: &[String], failures: &[(String, String)]) -> Vec<Found> {
    let report = extensions::discover(&extensions::Options {
        roots,
        disabled: Vec::new(),
    });
    let mut rows: Vec<Found> = Vec::new();
    for loaded in &report.loaded {
        let kind = match (loaded.entry.is_some(), loaded.module.is_some()) {
            (true, true) => "script + compiled",
            (true, false) => "script",
            (false, true) => "compiled",
            (false, false) => "assets",
        };
        let id = loaded.manifest.id.clone();
        let failure = failures
            .iter()
            .find(|(failed, _)| *failed == id)
            .map(|(_, reason)| reason.clone());
        let off = disabled.contains(&id);
        rows.push(Found {
            note: failure.unwrap_or_default(),
            enabled: !off,
            switchable: true,
            id,
            name: loaded.manifest.name.clone(),
            kind,
            source: registry::provenance(&loaded.dir),
        });
    }
    for skipped in &report.skipped {
        let name = skipped
            .path
            .file_name()
            .map(|name| name.to_string_lossy().into_owned())
            .unwrap_or_else(|| skipped.path.display().to_string());
        rows.push(Found {
            enabled: false,
            switchable: false,
            id: skipped.id.clone().unwrap_or_else(|| name.clone()),
            name,
            kind: "broken",
            note: skipped.reason.clone(),
            source: None,
        });
    }
    rows.sort_by(|a, b| a.id.cmp(&b.id));
    rows
}

/// The engine, and what the shell has already reported to it.
pub(crate) struct Extensions {
    engine: extensions::Engine,
    /// How many marks each pane had at its last report.
    ///
    /// A reload keeps this, so `commandFinished` history is not replayed to an
    /// extension that was already told about it (ADR 088).
    marks: HashMap<u64, usize>,
    /// The pane whose focus was last reported.
    focused: Option<PaneId>,
    /// The title last reported for the focused pane.
    title: Option<String>,
    /// The extensions that did not load, and why, for the screen.
    failures: Vec<(String, String)>,
    /// Whether the opt-in file watcher runs.
    watch: bool,
    /// When each file the engine was built from last changed.
    stamps: extensions::Stamps,
}

impl Extensions {
    /// Starts the engine when the settings or the environment ask for it.
    ///
    /// A broken extension is logged and skipped: the screen opens either way,
    /// because an extension must never be able to stop the terminal.
    fn start(settings: &settings::Extensions) -> Option<Self> {
        if !enabled(settings.enabled) {
            return None;
        }
        let (engine, failures, stamps) = build(settings);
        Some(Self {
            engine,
            marks: HashMap::new(),
            focused: None,
            title: None,
            failures,
            watch: watching(settings.watch),
            stamps,
        })
    }

    /// Rebuilds the engine from `settings` without a restart (ADR 088).
    ///
    /// The marks the shell already reported survive; the focus and title are
    /// forgotten so the new engine observes the current pane once. An extension
    /// that no longer loads is logged and skipped, and the rest keep running.
    fn rebuild(&mut self, settings: &settings::Extensions) {
        // Drop the old realms before the tree is read again.
        self.engine = extensions::Engine::default();
        let (engine, failures, stamps) = build(settings);
        self.engine = engine;
        self.failures = failures;
        self.watch = watching(settings.watch);
        self.stamps = stamps;
        self.focused = None;
        self.title = None;
    }

    /// Every command the loaded extensions registered.
    pub(crate) fn commands(&self) -> Vec<extensions::Command> {
        self.engine.commands()
    }

    /// The extensions that did not load, and why.
    pub(crate) fn failures(&self) -> &[(String, String)] {
        &self.failures
    }

    /// The events the window's state implies since the last call.
    fn changes(&mut self, shell: &Shell, marks: Option<(PaneId, &[MarkTick])>) -> Vec<extensions::Event> {
        let mut events = Vec::new();
        let Some(pane) = shell.tabs.active().and_then(|tab| tab.focused_pane()) else {
            self.focused = None;
            self.title = None;
            return events;
        };
        if self.focused != Some(pane.id) {
            self.focused = Some(pane.id);
            self.title = None;
            events.push(extensions::Event::PaneFocused);
        }
        if pane.title != self.title {
            self.title = pane.title.clone();
            if let Some(title) = &self.title {
                events.push(extensions::Event::TitleChanged { title: title.clone() });
            }
        }
        if let Some((id, marks)) = marks {
            let reported = self.marks.entry(id.0).or_insert(0);
            for mark in marks.iter().skip(*reported) {
                events.push(extensions::Event::CommandFinished {
                    row: mark.row.max(0) as usize,
                    error: mark.error,
                });
            }
            *reported = marks.len();
        }
        events
    }
}

impl Shell {
    /// Starts the engine if the settings or the environment asked for it.
    /// Called once, by `activate`.
    pub(crate) fn start_extensions(&mut self) {
        if self.extensions.is_none() {
            let settings = self.document.extensions.clone();
            self.extensions = Extensions::start(&settings);
        }
    }

    /// Rebuilds the live engine from the settings the shell currently holds.
    ///
    /// Called when the `[extensions]` table changes, after the switches are
    /// saved, and by a screen that installed or removed a directory (ADR 088).
    pub(crate) fn reload_extensions(&mut self) {
        let settings = self.document.extensions.clone();
        if !enabled(settings.enabled) {
            self.extensions = None;
            return;
        }
        match &mut self.extensions {
            Some(holder) => holder.rebuild(&settings),
            None => self.extensions = Extensions::start(&settings),
        }
    }

    /// Rebuilds when a watched file changed. A no-op unless the table asked.
    pub(crate) fn watch_extensions(&mut self) {
        let changed = self
            .extensions
            .as_mut()
            .is_some_and(|holder| holder.watch && holder.stamps.changed());
        if changed {
            self.reload_extensions();
        }
    }

    /// Every extension on disk, for the extensions screen.
    pub(crate) fn installed_extensions(&self) -> Vec<Found> {
        let failures = self
            .extensions
            .as_ref()
            .map(|extensions| extensions.failures().to_vec())
            .unwrap_or_default();
        installed(roots(), &self.document.extensions.disabled, &failures)
    }

    /// Reports the window's changes to every extension.
    ///
    /// `marks` is the active pane's scrollbar marks, taken by the caller
    /// because reading them needs the pane mutably. `observed` are the pty
    /// events the tick saw (a bell, a notification, a pane exiting) in the order
    /// it saw them; `settings_reloaded` is the settings poll's answer for this
    /// tick.
    pub(crate) fn pump_extensions(
        &mut self,
        marks: Option<(PaneId, &[MarkTick])>,
        observed: Vec<extensions::Event>,
        settings_reloaded: bool,
    ) {
        let Some(mut holder) = self.extensions.take() else {
            return;
        };
        let mut events = observed;
        events.extend(holder.changes(self, marks));
        if settings_reloaded {
            events.push(extensions::Event::SettingsReloaded);
        }
        if !events.is_empty() {
            let mut engine = std::mem::take(&mut holder.engine);
            for event in &events {
                engine.emit(self, event);
            }
            let logs = engine.take_logs();
            holder.engine = engine;
            for line in logs {
                eprintln!("winter: {line}");
            }
        }
        self.extensions = Some(holder);
    }

    /// Reports one event, for the changes a method makes rather than the tick
    /// polling for.
    pub(crate) fn report_extension(&mut self, event: extensions::Event) {
        let Some(mut holder) = self.extensions.take() else {
            return;
        };
        let mut engine = std::mem::take(&mut holder.engine);
        engine.emit(self, &event);
        let logs = engine.take_logs();
        holder.engine = engine;
        self.extensions = Some(holder);
        for line in logs {
            eprintln!("winter: {line}");
        }
    }

    /// Runs an extension command from the palette or a chord.
    pub(crate) fn run_extension(&mut self, key: &str) {
        let Some(mut holder) = self.extensions.take() else {
            return;
        };
        let mut engine = std::mem::take(&mut holder.engine);
        engine.run(self, key);
        let logs = engine.take_logs();
        holder.engine = engine;
        self.extensions = Some(holder);
        for line in logs {
            eprintln!("winter: {line}");
        }
    }

    /// Writes the `[extensions]` table, re-reads the file, and rebuilds the
    /// engine now (ADR 088).
    pub(crate) fn save_extensions(&mut self, enabled: bool, disabled: Vec<String>) -> Result<(), String> {
        self.document.extensions.enabled = enabled;
        self.document.extensions.disabled = disabled;
        match settings::save(&self.document) {
            Ok(()) => {
                self.reload_settings();
                self.reload_extensions();
                Ok(())
            }
            Err(error) => {
                let message = error.to_string();
                self.settings_error = Some(message.clone());
                Err(message)
            }
        }
    }
}

impl extensions::Host for Shell {
    fn tabs(&self) -> Vec<extensions::TabInfo> {
        (0..self.tabs.len())
            .filter_map(|index| {
                self.tabs.get(index).map(|tab| extensions::TabInfo {
                    index,
                    title: tab.label().to_string(),
                    active: index == self.tabs.index(),
                    panes: tab.panes.len(),
                })
            })
            .collect()
    }

    fn focused(&self) -> Option<extensions::PaneInfo> {
        let pane = self.tabs.active().and_then(|tab| tab.focused_pane())?;
        Some(extensions::PaneInfo {
            id: pane.id.0.to_string(),
            title: pane.title.clone().unwrap_or_default(),
            program: pane.program.clone(),
            columns: pane.grid.0.max(0) as usize,
            rows: pane.grid.1.max(0) as usize,
        })
    }

    fn visible_text(&self) -> String {
        self.tabs
            .active()
            .and_then(|tab| tab.focused_pane())
            .map(|pane| pane.renderer.data().visible_text())
            .unwrap_or_default()
    }

    fn history(&mut self, lines: usize) -> String {
        let Some(pane) = self.tabs.active_mut().and_then(|tab| tab.focused_pane_mut()) else {
            return String::new();
        };
        let (document, _) = pane.control.scroll_facts();
        let visible = pane.control.size().1.max(1);
        // The view may be scrolled back, and the walk stops where it starts.
        let end = (document - visible - pane.view.scroll()).max(0);
        pane.control.history_text(end, lines)
    }

    fn select_tab(&mut self, index: usize) {
        if self.tabs.select(index) {
            self.seed_find(false);
            self.focus_active();
        }
    }

    fn open_tab(&mut self) -> bool {
        let before = self.tabs.len();
        Shell::open_tab(self, None);
        self.tabs.len() > before
    }

    fn close_tab(&mut self, index: usize) -> bool {
        // An extension may not close the last tab: that is the window, and a
        // handler has no business ending it.
        if index >= self.tabs.len() || self.tabs.len() == 1 {
            return false;
        }
        Shell::close_tab(self, index);
        true
    }

    fn send_text(&mut self, text: &str) {
        if let Some(pane) = self.tabs.active_mut().and_then(|tab| tab.focused_pane_mut()) {
            pane.control.write(text);
        }
    }

    fn copy(&mut self, text: &str) {
        if let Some(display) = gtk::gdk::Display::default() {
            display.clipboard().set_text(text);
        }
    }

    fn notify(&mut self, title: &str, body: &str) {
        winter::notifications::show(title, body);
    }

    fn open_url(&mut self, url: &str) -> bool {
        gtk::gio::AppInfo::launch_default_for_uri(url, None::<&gtk::gio::AppLaunchContext>).is_ok()
    }
}

/// Whether the engine runs: the environment overrides the file either way, so
/// `WINTER_EXT=0` can keep an enabled installation quiet for one run.
fn enabled(configured: bool) -> bool {
    match std::env::var_os("WINTER_EXT") {
        Some(value) if value == "0" => false,
        Some(_) => true,
        None => configured,
    }
}

/// Whether the opt-in file watcher runs: `WINTER_WATCH` forces it on or off for
/// one run, otherwise the file decides (ADR 088).
fn watching(configured: bool) -> bool {
    match std::env::var_os("WINTER_WATCH") {
        Some(value) if value == "0" => false,
        Some(_) => true,
        None => configured,
    }
}

/// Builds a frozen engine from `settings`, logging what it read.
///
/// A broken extension is logged and skipped: the screen opens either way,
/// because an extension must never be able to stop the terminal.
fn build(settings: &settings::Extensions) -> (extensions::Engine, Vec<(String, String)>, extensions::Stamps) {
    let roots = roots();
    let report = extensions::discover(&extensions::Options {
        roots: roots.clone(),
        disabled: settings.disabled.clone(),
    });
    let (mut engine, errors) = extensions::Engine::load_all(&report);
    let mut failures = Vec::new();
    for error in errors {
        eprintln!("winter: extension {error}");
        failures.push((error.id, error.message));
    }
    for line in engine.take_logs() {
        eprintln!("winter: {line}");
    }
    for skipped in &report.skipped {
        eprintln!("winter: skipped {}: {}", skipped.path.display(), skipped.reason);
    }
    let stamps = extensions::Stamps::of(&report, &roots);
    (engine, failures, stamps)
}

/// The Search screen for `settings`, without reading the network.
///
/// It is created when the extensions window opens; the Search tab's first
/// showing is what calls `open` and starts the only fetch (ADR 088).
fn registry_screen(settings: &settings::Extensions) -> registry::Screen {
    let root = settings::settings_path().and_then(|path| path.parent().map(PathBuf::from));
    let cache = root.as_ref().map_or_else(
        || std::env::temp_dir().join("winter-registry-cache"),
        |dir| dir.join("registry-cache"),
    );
    let extensions = root.map_or_else(
        || std::env::current_dir().unwrap_or_default().join("extensions"),
        |dir| dir.join("extensions"),
    );
    registry::Screen::new(
        registry::configured_urls(settings.registries.as_deref()),
        cache,
        extensions,
    )
}

/// The roots discovery reads, in priority order: the explicit list, the user's
/// config directory, then beside the executable and in the working directory.
fn roots() -> Vec<PathBuf> {
    let mut roots = Vec::new();
    if let Some(list) = std::env::var_os("WINTER_EXTENSIONS") {
        roots.extend(std::env::split_paths(&list));
    }
    if let Some(dir) = settings::settings_path().and_then(|path| path.parent().map(PathBuf::from)) {
        roots.push(dir.join("extensions"));
    }
    if let Some(dir) = std::env::current_exe()
        .ok()
        .and_then(|exe| exe.parent().map(PathBuf::from))
    {
        roots.push(dir.join("extensions"));
    }
    if let Ok(cwd) = std::env::current_dir() {
        roots.push(cwd.join("extensions"));
    }
    roots
}

/// Shows the extensions screen, focusing the one already open rather than
/// stacking a second copy.
pub(crate) fn show_extensions(shell: &Rc<RefCell<Shell>>, parent: &gtk::ApplicationWindow) {
    let existing = shell.borrow().extensions_window.clone();
    if let Some(window) = existing {
        window.present();
        return;
    }
    let opened = open_extensions_window(shell, parent);
    shell.borrow_mut().extensions_window = Some(opened);
}

/// The notebook page that holds the registry rows.
const SEARCH_PAGE: u32 = 1;

/// One Installed row's switch, so Save reads what the window shows after a
/// rebuild.
struct SwitchRow {
    id: String,
    switchable: bool,
    switch: gtk::Switch,
}

/// Builds the extensions window: the Installed switches and the Search tab.
///
/// The Search tab is the only place a registry is read, and only once the tab is
/// first shown (ADR 088). The tick polls the screen; a finished install or
/// uninstall rebuilds the engine and both lists without a restart.
fn open_extensions_window(shell: &Rc<RefCell<Shell>>, parent: &gtk::ApplicationWindow) -> gtk::Window {
    let enabled = {
        let mut shell = shell.borrow_mut();
        let settings = shell.document.extensions.clone();
        shell.registry = Some(registry_screen(&settings));
        settings.enabled
    };

    let mut builder = gtk::Window::builder()
        .title("Extensions")
        .transient_for(parent)
        .default_width(620)
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

    let notebook = gtk::Notebook::new();
    notebook.set_vexpand(true);

    let installed_page = gtk::Box::new(gtk::Orientation::Vertical, 8);
    let hint = gtk::Label::new(Some(
        "Anything under an `extensions` folder is listed here. A switch takes effect at once.",
    ));
    hint.set_xalign(0.0);
    hint.set_wrap(true);

    let master = gtk::Switch::new();
    master.set_active(enabled);
    master.set_valign(gtk::Align::Center);
    let master_row = gtk::Box::new(gtk::Orientation::Horizontal, 8);
    master_row.append(&master);
    master_row.append(&gtk::Label::new(Some("Load extensions")));

    let installed_list = gtk::Box::new(gtk::Orientation::Vertical, 8);
    let installed_scroll = gtk::ScrolledWindow::builder()
        .child(&installed_list)
        .vexpand(true)
        .has_frame(true)
        .build();

    installed_page.append(&hint);
    installed_page.append(&master_row);
    installed_page.append(&installed_scroll);
    notebook.append_page(&installed_page, Some(&gtk::Label::new(Some("Installed"))));

    let search_page = gtk::Box::new(gtk::Orientation::Vertical, 8);
    let entry = gtk::SearchEntry::new();
    entry.set_placeholder_text(Some("Search the registries"));
    let search_status = gtk::Label::new(None);
    search_status.set_xalign(0.0);
    search_status.set_wrap(true);
    search_status.add_css_class("dim-label");
    let search_list = gtk::Box::new(gtk::Orientation::Vertical, 10);
    let search_scroll = gtk::ScrolledWindow::builder()
        .child(&search_list)
        .vexpand(true)
        .has_frame(true)
        .build();
    search_page.append(&entry);
    search_page.append(&search_status);
    search_page.append(&search_scroll);
    notebook.append_page(&search_page, Some(&gtk::Label::new(Some("Search"))));

    let status = gtk::Label::new(None);
    status.set_xalign(0.0);
    status.set_wrap(true);
    status.add_css_class("dim-label");

    let buttons = gtk::Box::new(gtk::Orientation::Horizontal, 6);
    buttons.set_halign(gtk::Align::End);
    let close = gtk::Button::with_label("Close");
    let save = gtk::Button::with_label("Save");
    save.add_css_class("suggested-action");
    buttons.append(&close);
    buttons.append(&save);

    root.append(&notebook);
    root.append(&status);
    root.append(&buttons);
    window.set_child(Some(&root));

    let switches: Rc<RefCell<Vec<SwitchRow>>> = Rc::new(RefCell::new(Vec::new()));
    rebuild_installed(shell, &installed_list, &switches);
    rebuild_search(shell, &search_list, &search_status);

    {
        let shell = Rc::clone(shell);
        let search = search_list.clone();
        let search_status = search_status.clone();
        notebook.connect_switch_page(move |_, _, page| {
            if page != SEARCH_PAGE {
                return;
            }
            {
                if let Some(screen) = &mut shell.borrow_mut().registry {
                    screen.open();
                }
            }
            rebuild_search(&shell, &search, &search_status);
        });
    }

    {
        let shell = Rc::clone(shell);
        let search = search_list.clone();
        let search_status = search_status.clone();
        entry.connect_search_changed(move |entry| {
            {
                if let Some(screen) = &mut shell.borrow_mut().registry {
                    screen.set_query(entry.text().to_string());
                }
            }
            rebuild_search(&shell, &search, &search_status);
        });
    }

    {
        let shell = Rc::clone(shell);
        let master = master.clone();
        let switches = Rc::clone(&switches);
        let status = status.clone();
        let installed = installed_list.clone();
        save.connect_clicked(move |_| {
            let disabled: Vec<String> = switches
                .borrow()
                .iter()
                .filter(|row| row.switchable && !row.switch.is_active())
                .map(|row| row.id.clone())
                .collect();
            match shell.borrow_mut().save_extensions(master.is_active(), disabled) {
                Ok(()) => {
                    status.set_text("Saved. The engine was rebuilt.");
                    rebuild_installed(&shell, &installed, &switches);
                }
                Err(error) => status.set_text(&error),
            }
        });
    }

    {
        let shell = Rc::clone(shell);
        let installed = installed_list.clone();
        let search = search_list.clone();
        let switches = Rc::clone(&switches);
        let search_status = search_status.clone();
        gtk::glib::timeout_add_local(Duration::from_millis(200), move || {
            let changed = {
                let Ok(mut shell) = shell.try_borrow_mut() else {
                    return gtk::glib::ControlFlow::Continue;
                };
                let Some(screen) = &mut shell.registry else {
                    return gtk::glib::ControlFlow::Break;
                };
                let changed = screen.poll();
                if changed.tree {
                    shell.reload_extensions();
                }
                changed
            };
            if changed.display || changed.tree {
                rebuild_search(&shell, &search, &search_status);
            }
            if changed.tree {
                rebuild_installed(&shell, &installed, &switches);
            }
            gtk::glib::ControlFlow::Continue
        });
    }

    {
        let window = window.clone();
        close.connect_clicked(move |_| window.close());
    }

    {
        let closer = window.clone();
        let keys = gtk::EventControllerKey::new();
        keys.connect_key_pressed(move |_controller, key, _code, _state| {
            if key == gtk::gdk::Key::Escape {
                closer.close();
                gtk::glib::Propagation::Stop
            } else {
                gtk::glib::Propagation::Proceed
            }
        });
        window.add_controller(keys);
    }

    {
        let cleared = Rc::clone(shell);
        window.connect_close_request(move |_window| {
            if let Ok(mut shell) = cleared.try_borrow_mut() {
                shell.extensions_window = None;
                shell.registry = None;
            }
            gtk::glib::Propagation::Proceed
        });
    }

    window.present();
    window
}

/// Rebuilds the Installed list from what is on disk.
fn rebuild_installed(shell: &Rc<RefCell<Shell>>, list: &gtk::Box, switches: &Rc<RefCell<Vec<SwitchRow>>>) {
    while let Some(child) = list.first_child() {
        list.remove(&child);
    }
    let rows = shell.borrow().installed_extensions();
    switches.borrow_mut().clear();

    if rows.is_empty() {
        let empty = gtk::Label::new(Some("No extensions were found."));
        empty.set_xalign(0.0);
        empty.add_css_class("dim-label");
        list.append(&empty);
        return;
    }

    for row in rows {
        let entry = gtk::Box::new(gtk::Orientation::Vertical, 1);
        let line = gtk::Box::new(gtk::Orientation::Horizontal, 8);
        let switch = gtk::Switch::new();
        switch.set_active(row.enabled);
        switch.set_sensitive(row.switchable);
        switch.set_valign(gtk::Align::Start);
        switch.set_tooltip_text(Some(if row.switchable {
            "Load this extension"
        } else {
            "This extension could not be read, so there is nothing to switch"
        }));
        let title = gtk::Label::new(None);
        title.set_markup(&format!("<b>{}</b>", gtk::glib::markup_escape_text(&row.name)));
        title.set_xalign(0.0);
        line.append(&switch);
        line.append(&title);
        entry.append(&line);

        let detail = if row.note.is_empty() {
            format!("{} · {}", row.id, row.kind)
        } else {
            format!("{} · {} · {}", row.id, row.kind, row.note)
        };
        let detail = gtk::Label::new(Some(&detail));
        detail.set_xalign(0.0);
        detail.set_wrap(true);
        detail.set_margin_start(52);
        detail.add_css_class("dim-label");
        entry.append(&detail);

        if let Some(source) = &row.source {
            let provenance = gtk::Label::new(Some(&format!("Installed {} from {}", source.version, source.registry)));
            provenance.set_xalign(0.0);
            provenance.set_wrap(true);
            provenance.set_margin_start(52);
            provenance.add_css_class("dim-label");
            entry.append(&provenance);

            let remove = gtk::Button::with_label("Remove");
            remove.set_halign(gtk::Align::Start);
            remove.set_margin_start(52);
            {
                let shell = Rc::clone(shell);
                let id = row.id.clone();
                remove.connect_clicked(move |_| {
                    if let Some(screen) = &mut shell.borrow_mut().registry {
                        screen.uninstall(&id, false);
                    }
                });
            }
            entry.append(&remove);
        }

        list.append(&entry);
        switches.borrow_mut().push(SwitchRow {
            id: row.id,
            switchable: row.switchable,
            switch,
        });
    }
}

/// Rebuilds the Search list from the live screen.
fn rebuild_search(shell: &Rc<RefCell<Shell>>, list: &gtk::Box, status: &gtk::Label) {
    while let Some(child) = list.first_child() {
        list.remove(&child);
    }

    let (rows, text, opened, loading) = {
        let shell = shell.borrow();
        let Some(screen) = &shell.registry else {
            return;
        };
        let rows: Vec<(registry::SearchRow, Option<usize>)> = screen
            .rows()
            .into_iter()
            .map(|row| {
                let picked = screen.version(&row).cloned();
                let slot = picked.and_then(|picked| row.versions.iter().position(|version| *version == picked));
                (row, slot)
            })
            .collect();
        (rows, status_for(screen), screen.opened(), screen.is_loading())
    };
    status.set_text(&text);

    if rows.is_empty() {
        if opened && !loading {
            let empty = gtk::Label::new(Some("No extension matches."));
            empty.set_xalign(0.0);
            empty.add_css_class("dim-label");
            list.append(&empty);
        }
        return;
    }

    for (index, (row, slot)) in rows.into_iter().enumerate() {
        let entry = gtk::Box::new(gtk::Orientation::Vertical, 2);

        let title = gtk::Label::new(None);
        title.set_markup(&format!(
            "<b>{}</b>  <span alpha=\"60%\">{}</span>",
            gtk::glib::markup_escape_text(&row.name),
            gtk::glib::markup_escape_text(&row.id)
        ));
        title.set_xalign(0.0);
        title.set_wrap(true);
        entry.append(&title);

        if let Some(description) = &row.description {
            let label = gtk::Label::new(Some(description));
            label.set_xalign(0.0);
            label.set_wrap(true);
            label.add_css_class("dim-label");
            entry.append(&label);
        }

        let mut provenance = format!("{} · {}", row.registry_name, row.registry);
        match &row.installed {
            None => {}
            Some(registry::Local::Manual) => provenance.push_str(" · installed by hand"),
            Some(registry::Local::Registry(found)) => {
                provenance.push_str(&format!(" · installed {}", found.version));
                if found.registry == row.registry {
                    provenance.push_str(" from this registry");
                }
            }
        }
        let label = gtk::Label::new(Some(&provenance));
        label.set_xalign(0.0);
        label.set_wrap(true);
        label.add_css_class("dim-label");
        entry.append(&label);

        if !row.dependencies.is_empty() {
            let label = gtk::Label::new(Some(&format!("Needs: {}", row.dependencies.join(", "))));
            label.set_xalign(0.0);
            label.set_wrap(true);
            label.add_css_class("dim-label");
            entry.append(&label);
        }

        if row.installable() {
            let controls = gtk::Box::new(gtk::Orientation::Horizontal, 6);
            let labels: Vec<String> = row.versions.iter().map(ToString::to_string).collect();
            let strings: Vec<&str> = labels.iter().map(String::as_str).collect();
            let versions = gtk::DropDown::from_strings(&strings);
            versions.set_selected(slot.unwrap_or(0) as u32);
            {
                let shell = Rc::clone(shell);
                versions.connect_selected_notify(move |versions| {
                    if let Some(screen) = &mut shell.borrow_mut().registry {
                        screen.select_index(index, versions.selected() as usize);
                    }
                });
            }
            let install = gtk::Button::with_label(if row.installed.is_some() { "Update" } else { "Install" });
            {
                let shell = Rc::clone(shell);
                let status = status.clone();
                install.connect_clicked(move |_| {
                    if let Some(screen) = &mut shell.borrow_mut().registry {
                        screen.install_index(index);
                    }
                    status.set_text("Working…");
                });
            }
            controls.append(&versions);
            controls.append(&install);
            entry.append(&controls);
        } else {
            let label = gtk::Label::new(Some(
                "Another copy is already installed; remove it before installing this one.",
            ));
            label.set_xalign(0.0);
            label.set_wrap(true);
            label.add_css_class("dim-label");
            entry.append(&label);
        }

        list.append(&entry);
    }
}

/// The status line above the Search rows: what each registry answered and what
/// the last action did.
fn status_for(screen: &registry::Screen) -> String {
    if screen.urls().is_empty() {
        return "No registries are configured. Add one in the settings file.".to_string();
    }
    if !screen.opened() {
        return "Open this tab to read the registries.".to_string();
    }
    let mut lines = Vec::new();
    if screen.is_loading() {
        lines.push("Reading the registries…".to_string());
    }
    for (url, state) in screen.urls().iter().zip(screen.states()) {
        match state {
            registry::State::Loading | registry::State::Ready { cached: false, .. } => {}
            registry::State::Ready { cached: true, note } => {
                let note = note.as_deref().unwrap_or_default();
                lines.push(format!("{url}: the last good copy is in use ({note})"));
            }
            registry::State::Failed(message) => lines.push(format!("{url}: {message}")),
        }
    }
    if let Some(status) = screen.status() {
        lines.push(match status {
            registry::Status::Working => "Working…".to_string(),
            registry::Status::Done(message) | registry::Status::Failed(message) => message.clone(),
        });
    }
    lines.join("\n")
}

#[cfg(test)]
mod tests {
    use std::path::Path;

    use super::*;

    fn write(dir: &Path, name: &str, text: &str) {
        let path = dir.join(name);
        if let Some(parent) = path.parent() {
            std::fs::create_dir_all(parent).expect("parent");
        }
        std::fs::write(path, text).expect("write");
    }

    fn extension(root: &Path, id: &str, entry: Option<&str>, module: Option<&str>) {
        let dir = root.join(id);
        std::fs::create_dir_all(&dir).expect("dir");
        let mut manifest = format!("id = \"{id}\"\nname = \"{id}\"\nversion = \"1.0.0\"\napi = 1\n");
        if let Some(entry) = entry {
            manifest.push_str(&format!("entry = \"{entry}\"\n"));
            write(&dir, entry, "export {};\n");
        }
        if let Some(module) = module {
            manifest.push_str(&format!("module = \"{module}\"\n"));
            write(&dir, module, "\0asm");
        }
        write(&dir, "extension.toml", &manifest);
    }

    #[test]
    fn the_screen_lists_every_extension_on_disk() {
        let temp = tempfile::tempdir().expect("tempdir");
        extension(temp.path(), "a.script", Some("main.js"), None);
        extension(temp.path(), "b.compiled", None, Some("code.wasm"));

        let rows = installed(vec![temp.path().to_path_buf()], &[], &[]);
        let ids: Vec<&str> = rows.iter().map(|row| row.id.as_str()).collect();
        assert_eq!(ids, ["a.script", "b.compiled"]);
        assert!(rows.iter().all(|row| row.enabled && row.switchable));
        assert_eq!(rows[0].kind, "script");
        assert_eq!(rows[1].kind, "compiled");
    }

    #[test]
    fn a_switched_off_extension_is_still_listed_and_shows_off() {
        let temp = tempfile::tempdir().expect("tempdir");
        extension(temp.path(), "a.one", Some("main.js"), None);
        extension(temp.path(), "b.two", Some("main.js"), None);

        let rows = installed(vec![temp.path().to_path_buf()], &["b.two".to_string()], &[]);
        assert_eq!(rows.len(), 2);
        assert!(rows[0].enabled);
        assert!(!rows[1].enabled, "{:?}", rows[1]);
        assert!(rows[1].switchable);
    }

    #[test]
    fn a_failure_is_attached_to_the_row_it_belongs_to() {
        let temp = tempfile::tempdir().expect("tempdir");
        extension(temp.path(), "a.one", Some("main.js"), None);
        let rows = installed(
            vec![temp.path().to_path_buf()],
            &[],
            &[("a.one".to_string(), "the entry module threw".to_string())],
        );
        assert_eq!(rows[0].note, "the entry module threw");
    }

    #[test]
    fn a_broken_manifest_is_a_row_with_nothing_to_switch() {
        let temp = tempfile::tempdir().expect("tempdir");
        let dir = temp.path().join("broken");
        std::fs::create_dir_all(&dir).expect("dir");
        write(&dir, "extension.toml", "id = [unterminated");

        let rows = installed(vec![temp.path().to_path_buf()], &[], &[]);
        assert_eq!(rows.len(), 1);
        assert_eq!(rows[0].kind, "broken");
        assert!(!rows[0].switchable);
        assert!(!rows[0].enabled);
        assert!(!rows[0].note.is_empty());
    }

    #[test]
    fn a_registry_install_carries_its_record() {
        let temp = tempfile::tempdir().expect("tempdir");
        extension(temp.path(), "a.one", Some("main.js"), None);
        write(
            &temp.path().join("a.one"),
            "winter-installed.toml",
            "registry = \"https://one.example/r.toml\"\nversion = \"0.1.0\"\narchive = \"https://x/y.zip\"\nsha256 = \"aaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaa\"\ninstalled = 1\n",
        );

        let rows = installed(vec![temp.path().to_path_buf()], &[], &[]);
        let source = rows[0].source.as_ref().expect("provenance");
        assert_eq!(source.registry, "https://one.example/r.toml");
    }

    #[test]
    fn an_empty_installation_lists_nothing() {
        let temp = tempfile::tempdir().expect("tempdir");
        assert!(installed(vec![temp.path().to_path_buf()], &[], &[]).is_empty());
    }
}
