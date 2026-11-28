//! Extensions in the WinUI shell: the [`Host`] the engine calls, the pump that
//! turns the window's changes into events, and what the extensions dialog
//! lists.
//!
//! Loading is off by default: `[extensions] enabled = true` in the settings file
//! turns it on, and `WINTER_EXT` overrides the file for one run either way —
//! set it to `0` to keep the engine off, to anything else to force it on.
//! `WINTER_EXTENSIONS` adds a discovery root beside the standard ones.
//!
//! Reads go through [`Host`] before a handler runs; writes are queued by the
//! engine and applied when it returns. The shell therefore never lets an
//! extension re-enter it mid-frame, and the engine is moved out of the field
//! for the duration of a dispatch so the shell can be its own host.

use std::collections::HashMap;
use std::path::PathBuf;

use winter::grid::MarkTick;
use winter::layout::PaneId;

use super::app::Shell;

/// One extension the extensions dialog lists.
#[derive(Debug, Clone, PartialEq, Eq)]
pub(super) struct Found {
    /// The identifier its manifest declares, or the directory's name when the
    /// manifest could not be read.
    pub(super) id: String,
    /// The name a user sees.
    pub(super) name: String,
    /// `script`, `compiled`, `script + compiled`, `assets`, or `broken`.
    pub(super) kind: &'static str,
    /// Whether loading it is switched on and off by this row.
    pub(super) switchable: bool,
    /// Whether it is enabled: switched on, and not on the disabled list.
    pub(super) enabled: bool,
    /// Why it is not running, when it is not.
    pub(super) note: String,
}

/// Every extension on disk, as the extensions dialog lists it.
///
/// Discovery runs *without* the disabled list, because the dialog has to show a
/// switched-off extension in order to offer it again. `failures` is what the
/// live engine reported at start, keyed by identifier; an installation with the
/// engine off has none, and every row then just follows the file.
pub(super) fn installed(roots: Vec<PathBuf>, disabled: &[String], failures: &[(String, String)]) -> Vec<Found> {
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
        });
    }
    rows.sort_by(|a, b| a.id.cmp(&b.id));
    rows
}

/// The engine, and what the shell has already reported to it.
pub(super) struct Extensions {
    engine: extensions::Engine,
    /// How many marks each pane had at its last report.
    marks: HashMap<u64, usize>,
    /// The pane whose focus was last reported.
    focused: Option<PaneId>,
    /// The title last reported for the focused pane.
    title: Option<String>,
    /// The extensions that did not load, and why, for the dialog.
    failures: Vec<(String, String)>,
}

impl Extensions {
    /// Starts the engine when the settings or the environment ask for it.
    ///
    /// A broken extension is logged and skipped: the dialog opens either way,
    /// because an extension must never be able to stop the terminal.
    fn start(settings: &settings::Extensions) -> Option<Self> {
        if !enabled(settings.enabled) {
            return None;
        }
        let report = extensions::discover(&extensions::Options {
            roots: roots(),
            disabled: settings.disabled.clone(),
        });
        let mut engine = extensions::Engine::default();
        let mut failures = Vec::new();
        for loaded in &report.loaded {
            if let Err(error) = engine.load(loaded) {
                eprintln!("winter: extension {error}");
                failures.push((error.id.clone(), error.message.clone()));
            }
        }
        engine.freeze();
        for line in engine.take_logs() {
            eprintln!("winter: {line}");
        }
        for skipped in &report.skipped {
            eprintln!("winter: skipped {}: {}", skipped.path.display(), skipped.reason);
        }
        Some(Self {
            engine,
            marks: HashMap::new(),
            focused: None,
            title: None,
            failures,
        })
    }

    /// Every command the loaded extensions registered.
    pub(super) fn commands(&self) -> Vec<extensions::Command> {
        self.engine.commands()
    }

    /// The extensions that did not load, and why.
    pub(super) fn failures(&self) -> &[(String, String)] {
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
    /// Called once, by `create`.
    pub(super) fn start_extensions(&mut self) {
        if self.extensions.is_none() {
            let settings = self.settings.extensions.clone();
            self.extensions = Extensions::start(&settings);
        }
    }

    /// Every extension on disk, for the extensions dialog.
    pub(super) fn installed_extensions(&self) -> Vec<Found> {
        let failures = self
            .extensions
            .as_ref()
            .map(|extensions| extensions.failures().to_vec())
            .unwrap_or_default();
        installed(roots(), &self.settings.extensions.disabled, &failures)
    }

    /// Reports the window's changes to every extension.
    ///
    /// `marks` is the active pane's scrollbar marks, taken by the caller
    /// because reading them needs the pane mutably. `observed` are the pty
    /// events the tick saw (a bell, a notification, a pane exiting) in the order
    /// it saw them; `settings_reloaded` is the settings poll's answer for this
    /// tick.
    pub(super) fn pump_extensions(
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
    pub(super) fn report_extension(&mut self, event: extensions::Event) {
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
    pub(super) fn run_extension(&mut self, key: &str) {
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

    /// Writes the `[extensions]` table, then re-reads the file.
    ///
    /// Loading itself does not change: an extension is switched by the next
    /// start, because dropping a live realm is not a reload.
    pub(super) fn save_extensions(&mut self, enabled: bool, disabled: Vec<String>) -> Result<(), String> {
        self.settings.extensions.enabled = enabled;
        self.settings.extensions.disabled = disabled;
        match settings::save(&self.settings) {
            Ok(()) => {
                self.reload_settings();
                Ok(())
            }
            Err(error) => Err(error.to_string()),
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
            let _ = self.focus.request_focus();
        }
    }

    fn open_tab(&mut self) -> bool {
        let before = self.tabs.len();
        Shell::open_tab(self);
        self.tabs.len() > before
    }

    fn close_tab(&mut self, index: usize) -> bool {
        // An extension may not close the last tab: that is the window, and a
        // handler has no business ending it.
        if index >= self.tabs.len() || self.tabs.len() == 1 {
            return false;
        }
        if self.tabs.close(index) {
            let _ = self.focus.request_focus();
            true
        } else {
            false
        }
    }

    fn send_text(&mut self, text: &str) {
        if let Some(pane) = self.tabs.active_mut().and_then(|tab| tab.focused_pane_mut()) {
            pane.control.write(text);
        }
    }

    fn copy(&mut self, text: &str) {
        if let Ok(mut clipboard) = arboard::Clipboard::new() {
            let _ = clipboard.set_text(text.to_string());
        }
    }

    fn notify(&mut self, title: &str, body: &str) {
        winter::notifications::show(title, body);
    }

    /// The WinUI shell has no URL opener: its About dialog navigates a
    /// `HyperlinkButton` and the shell never calls `ShellExecute` (ADR 070), so
    /// an extension sees a normal `false` and the engine logs it.
    fn open_url(&mut self, _url: &str) -> bool {
        false
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
        let mut manifest = format!("id = \"{id}\"\nname = \"{id}\"\nversion = \"1\"\napi = 1\n");
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
    fn the_dialog_lists_every_extension_on_disk() {
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
    fn an_empty_installation_lists_nothing() {
        let temp = tempfile::tempdir().expect("tempdir");
        assert!(installed(vec![temp.path().to_path_buf()], &[], &[]).is_empty());
    }
}
