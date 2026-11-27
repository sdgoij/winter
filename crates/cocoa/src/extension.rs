//! Extensions in the AppKit shell: the [`Host`] the engine calls, and the pump
//! that turns the window's changes into events.
//!
//! Loading is opt-in. `WINTER_EXT=1` (or any value but `0`) turns the engine
//! on, and `WINTER_EXTENSIONS` adds a root beside the standard ones, so a build
//! without the variable behaves exactly as it did before and a release can
//! keep the surface dark until the extensions screen exists.
//!
//! Reads go through [`Host`] before a handler runs; writes are queued by the
//! engine and applied when it returns. The shell therefore never lets an
//! extension re-enter it mid-frame, and the engine is moved out of the field
//! for the duration of a dispatch so the shell can be its own host.

use std::collections::HashMap;
use std::path::PathBuf;

use objc2_app_kit::NSWorkspace;
use objc2_foundation::{NSString, NSURL};

use winter::grid::MarkTick;
use winter::layout::PaneId;

use super::shell::Shell;

/// The engine, and what the shell has already reported to it.
pub(crate) struct Extensions {
    engine: extensions::Engine,
    /// How many marks each pane had at its last report.
    marks: HashMap<u64, usize>,
    /// The pane whose focus was last reported.
    focused: Option<PaneId>,
    /// The title last reported for the focused pane.
    title: Option<String>,
}

impl Extensions {
    /// Starts the engine when the environment asks for it.
    ///
    /// A broken extension is logged and skipped: the window opens either way,
    /// because an extension must never be able to stop the terminal.
    fn start(disabled: &[String]) -> Option<Self> {
        if !enabled() {
            return None;
        }
        let report = extensions::discover(&extensions::Options {
            roots: roots(),
            disabled: disabled.to_vec(),
        });
        let mut engine = extensions::Engine::default();
        for loaded in &report.loaded {
            if let Err(error) = engine.load(loaded) {
                eprintln!("winter: extension {error}");
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
        })
    }

    /// Every command the loaded extensions registered.
    pub(crate) fn commands(&self) -> Vec<extensions::Command> {
        self.engine.commands()
    }

    /// The events the window's state implies since the last call.
    fn changes(&mut self, shell: &Shell, marks: Option<(PaneId, &[MarkTick])>) -> Vec<extensions::Event> {
        let mut events = Vec::new();
        let Some(pane) = shell.focused_pane() else {
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
    /// Starts the engine if the environment asked for it. Called once, by
    /// `Shell::new`.
    pub(crate) fn start_extensions(&mut self) {
        if self.extensions.is_none() {
            let disabled = self.settings().extensions.disabled.clone();
            self.extensions = Extensions::start(&disabled);
        }
    }

    /// Reports the window's changes to every extension.
    ///
    /// `marks` is the active pane's scrollbar marks, taken by the caller
    /// because reading them needs the pane mutably. `settings_reloaded` is the
    /// settings poll's answer for this tick.
    pub(crate) fn pump_extensions(&mut self, marks: Option<(PaneId, &[MarkTick])>, settings_reloaded: bool) {
        let Some(mut holder) = self.extensions.take() else {
            return;
        };
        let mut events = holder.changes(self, marks);
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

    /// Runs an extension command from the palette.
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
        let pane = self.focused_pane()?;
        Some(extensions::PaneInfo {
            id: pane.id.0.to_string(),
            title: pane.title.clone().unwrap_or_default(),
            program: pane.program.clone(),
            columns: pane.grid.0.max(0) as usize,
            rows: pane.grid.1.max(0) as usize,
        })
    }

    fn visible_text(&self) -> String {
        self.accessible_text()
    }

    fn select_tab(&mut self, index: usize) {
        Shell::select_tab(self, index);
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

    fn open_url(&mut self, url: &str) -> bool {
        let Some(url) = NSURL::URLWithString(&NSString::from_str(url)) else {
            return false;
        };
        NSWorkspace::sharedWorkspace().openURL(&url)
    }
}

/// Whether the environment turns the engine on.
fn enabled() -> bool {
    std::env::var_os("WINTER_EXT").is_some_and(|value| value != "0")
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
