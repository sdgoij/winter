//! The half that runs extensions: one `slag::Context` per extension, the
//! `winter` global, and event and command dispatch (feature `host`).
//!
//! Each extension gets its own agent and realm, so no extension can see
//! another's globals, and a reload is a dropped context. The entry is an ES
//! module evaluated in that realm; the modules beside it are registered with
//! the agent first, so a static import resolves without a network or a
//! filesystem (the loader read the text already).
//!
//! Reads are a handover, writes are a queue: before a handler runs the host
//! takes one snapshot of the shell ([`Host::tabs`], [`Host::focused`],
//! [`Host::visible_text`]), and the `winter` calls that change something append
//! to a list the engine applies after the handler returns. A handler therefore
//! never re-enters the shell mid-frame, and the worst a second handler can
//! observe is the state the first one was given.

use std::cell::RefCell;
use std::collections::HashSet;
use std::rc::Rc;

use serde::Deserialize;
use slag::{Context, HostCallbacks, JsError, JsValue, PromiseState};

use crate::{Host, Loaded, manifest};

/// How many queued command runs (across extensions) one dispatch may perform.
const HOPS_MAX: usize = 8;

/// How many rounds of history continuations one dispatch may answer. A
/// continuation that asks for more history is answered again, but the count is
/// bounded the way command hops are.
const HISTORY_ROUNDS: usize = 8;

/// An event the shell reports, as `winter.events` names it.
#[derive(Debug, Clone, PartialEq, Eq)]
pub enum Event {
    /// A new shell-integration mark appeared: a command finished.
    CommandFinished {
        /// The row the mark is on.
        row: usize,
        /// Whether the command exited non-zero.
        error: bool,
    },
    /// The focused pane's title changed.
    TitleChanged {
        /// The new title.
        title: String,
    },
    /// The focused pane moved.
    PaneFocused,
    /// A tab was opened.
    TabOpened {
        /// The new tab's index.
        index: usize,
    },
    /// A tab was closed.
    TabClosed {
        /// The closed tab's index.
        index: usize,
    },
    /// The settings file was reloaded.
    SettingsReloaded,
}

impl Event {
    /// The name `winter.events.on` uses.
    pub fn name(&self) -> &'static str {
        match self {
            Event::CommandFinished { .. } => "commandFinished",
            Event::TitleChanged { .. } => "titleChanged",
            Event::PaneFocused => "paneFocused",
            Event::TabOpened { .. } => "tabOpened",
            Event::TabClosed { .. } => "tabClosed",
            Event::SettingsReloaded => "settingsReloaded",
        }
    }

    /// The event as JSON, which the bootstrap parses for the handler.
    fn payload(&self) -> String {
        match self {
            Event::CommandFinished { row, error } => serde_json::json!({ "row": row, "error": error }).to_string(),
            Event::TitleChanged { title } => serde_json::json!({ "title": title }).to_string(),
            Event::TabOpened { index } | Event::TabClosed { index } => {
                serde_json::json!({ "index": index }).to_string()
            }
            Event::PaneFocused | Event::SettingsReloaded => "{}".to_string(),
        }
    }
}

/// One command an extension registered, as the palette sees it.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct Command {
    /// `<extension id>:<command id>`, the key the palette dispatches with.
    pub key: String,
    /// The extension that registered it.
    pub extension: String,
    /// The command's own identifier.
    pub id: String,
    /// The row's label.
    pub label: String,
}

/// The extensions this process loaded.
///
/// Not `Send`: a Slag context is a single-threaded agent, and handlers run on
/// the shell's thread, like the pty pump.
#[derive(Default)]
pub struct Engine {
    extensions: Vec<Running>,
    logs: Vec<String>,
}

/// One loaded extension: its realm, its entry points into the bootstrap, and
/// the queue its calls land in.
struct Running {
    id: String,
    context: Context,
    state: Rc<RefCell<State>>,
    logs: Rc<RefCell<Vec<String>>>,
    snapshot: JsValue,
    dispatch: JsValue,
    command: JsValue,
    freeze: JsValue,
    /// The two halves of a history continuation: what the handler asked for
    /// beyond the snapshot, and the answer.
    history_take: JsValue,
    history_resolve: JsValue,
    commands: Vec<Command>,
}

/// What a handler asked the shell to do, applied after it returns.
#[derive(Debug, Clone, PartialEq, Eq)]
enum Call {
    Send(String),
    Copy(String),
    Notify(String, String),
    OpenUrl(String),
    SelectTab(usize),
    Run(String),
}

#[derive(Default)]
struct State {
    calls: Vec<Call>,
}

impl Engine {
    /// Load one discovered extension.
    ///
    /// A fresh context is built and discarded on failure, so a broken
    /// extension leaves nothing behind; the caller reports the error and moves
    /// to the next extension.
    pub fn load(&mut self, loaded: &Loaded) -> Result<(), Error> {
        let id = loaded.manifest.id.clone();
        if loaded.entry.is_none() && loaded.module.is_none() {
            return Err(Error::new(
                &id,
                "the manifest declares neither an `entry` nor a `module`",
            ));
        }

        let mut context = Context::new().map_err(|error| Error::new(&id, error.to_string()))?;
        let logs = Rc::new(RefCell::new(Vec::new()));
        let state = Rc::new(RefCell::new(State::default()));

        context.set_host_callbacks(host_callbacks(&id, &logs));
        // The engine's name resolution: a relative specifier resolves from the
        // file that wrote it. Unhandled rejections still reach the console
        // through the callbacks above.
        context.set_host_hooks(Box::new(Specifiers));
        install_primitives(&mut context, &id, &state, &logs).map_err(|error| Error::new(&id, error.to_string()))?;

        let identity = serde_json::json!({
            "id": loaded.manifest.id,
            "name": loaded.manifest.name,
            "version": loaded.manifest.version,
            "api": crate::manifest::API,
        })
        .to_string();
        let assets = serde_json::to_string(&loaded.manifest.assets.keys().collect::<Vec<_>>())
            .map_err(|error| Error::new(&id, error.to_string()))?;
        let bootstrap = BOOTSTRAP
            .replace("__IDENTITY__", &identity)
            .replace("__ASSETS__", &assets);

        // The assets are read once, here: a compiled module copies them through
        // its own memory during a callback, and a read that could fail
        // mid-frame would be a trap in a handler rather than a skipped
        // extension. The order matches the names the bootstrap is handed.
        let mut asset_buffers = Vec::with_capacity(loaded.manifest.assets.len());
        let mut asset_total: u64 = 0;
        for name in loaded.manifest.assets.keys() {
            let Some(path) = loaded.assets.get(name) else {
                continue;
            };
            let bytes =
                manifest::read_capped(path, manifest::ASSET_MAX).map_err(|error| Error::new(&id, error.to_string()))?;
            asset_total = asset_total.saturating_add(bytes.len() as u64);
            if asset_total > manifest::ASSETS_MAX {
                return Err(Error::new(
                    &id,
                    format!(
                        "the assets total {asset_total} bytes, over the {}-byte cap",
                        manifest::ASSETS_MAX
                    ),
                ));
            }
            let buffer = context
                .array_buffer_from_bytes(&bytes)
                .map_err(|error| Error::new(&id, format!("asset `{name}`: {error}")))?;
            asset_buffers.push(buffer);
        }
        let registry = context
            .create_array(&asset_buffers)
            .map_err(|error| Error::new(&id, format!("the assets were not handed over: {error}")))?;
        context
            .set_global("__winter_asset_bytes", registry)
            .map_err(|error| Error::new(&id, format!("the assets were not handed over: {error}")))?;

        context
            .eval(&bootstrap)
            .map_err(|error| Error::new(&id, format!("the bootstrap failed: {error}")))?;

        let keys: HashSet<String> = loaded.sources.iter().map(|source| source.name.clone()).collect();
        let entry_name = loaded.entry.as_ref().map(|entry| entry.name.as_str()).unwrap_or("");

        // Every module is compiled once to read what it imports, so a bad
        // specifier is reported against the file that wrote it instead of
        // surfacing as `Cannot find module` mid-link.
        let mut problems = Vec::new();
        for source in &loaded.sources {
            let source_module = context
                .compile_module(&source.name, Some(&source.name), &source.text)
                .map_err(|error| Error::new(&id, format!("{}: {error}", source.name)))?;
            problems.extend(validate_specifiers(&source_module, &source.name, &keys, entry_name));
        }
        if let Some(entry) = &loaded.entry {
            let entry_module = context
                .compile_module(&entry.name, Some(&entry.name), &entry.text)
                .map_err(|error| Error::new(&id, format!("{}: {error}", entry.name)))?;
            problems.extend(validate_specifiers(&entry_module, &entry.name, &keys, entry_name));
            if !problems.is_empty() {
                return Err(Error::new(&id, problems.join("; ")));
            }

            for source in &loaded.sources {
                // The supported registration call, with the kind declared rather
                // than guessed from the suffix: a `.json` file is a JSON module.
                let kind = if source.name.ends_with(".json") {
                    slag::ModuleSourceKind::Json
                } else {
                    slag::ModuleSourceKind::Js
                };
                context.add_module_source(&source.name, source.text.as_bytes(), kind);
            }
            context
                .instantiate_module(&entry_module)
                .map_err(|error| Error::new(&id, format!("{}: {error}", entry.name)))?;

            // The evaluation promise is the only reliable answer: the engine
            // marks a module with a pending top-level await `Evaluated`, as V8
            // does. No timer and no I/O exists in this realm, so a promise
            // still pending after the drain can never settle.
            let promise = context
                .evaluate_module(&entry_module)
                .map_err(|error| Error::new(&id, format!("{}: {error}", entry.name)))?;
            match context.promise_state(promise.clone()) {
                Ok(PromiseState::Fulfilled) => {}
                Ok(PromiseState::Rejected) => {
                    let detail = context
                        .promise_result(promise)
                        .map(|reason| context.describe_thrown(&reason))
                        .unwrap_or_else(|error| error.to_string());
                    return Err(Error::new(&id, detail));
                }
                Ok(PromiseState::Pending) => {
                    // The engine knows which modules are still waiting, which is a
                    // better sentence than "the promise never settled".
                    let stalled: Vec<String> = entry_module
                        .stalled_top_level_await_modules()
                        .iter()
                        .filter_map(|module| module.as_api_module().name())
                        .collect();
                    let detail = if stalled.is_empty() {
                        "the entry module's evaluation never settled".to_string()
                    } else {
                        format!(
                            "{} awaits something that will never settle; no timer or I/O exists in this realm",
                            stalled.join(", ")
                        )
                    };
                    return Err(Error::new(&id, detail));
                }
                Err(error) => return Err(Error::new(&id, error.to_string())),
            }
        } else if !problems.is_empty() {
            return Err(Error::new(&id, problems.join("; ")));
        }

        let winter = context
            .global()
            .and_then(|global| global.get("winter"))
            .and_then(|winter| context.to_object(&winter))
            .map_err(|error| Error::new(&id, format!("the bootstrap left no `winter` global: {error}")))?;

        // The compiled half runs after the entry, in the same realm and against
        // the same registries, so an extension may be either or both. Its
        // `winter_init` is where registration happens; a wasm start function
        // runs before the host owns the module's memory, so it cannot.
        if let Some(bytes) = &loaded.module {
            let buffer = context
                .array_buffer_from_bytes(bytes)
                .map_err(|error| Error::new(&id, format!("the module was not handed over: {error}")))?;
            let instantiate = winter
                .get("__wasm")
                .map_err(|error| Error::new(&id, error.to_string()))?;
            let outcome = context
                .call(&instantiate, &JsValue::undefined(), &[buffer])
                .map_err(|error| Error::new(&id, format!("the module failed: {error}")))?;
            // `__wasm` answers the sentence that failed it, or the empty string.
            let failed = context.to_string(&outcome).unwrap_or_default();
            if !failed.is_empty() {
                return Err(Error::new(&id, failed));
            }
        }

        let snapshot = winter
            .get("__snapshot")
            .map_err(|error| Error::new(&id, error.to_string()))?;
        let dispatch = winter
            .get("__dispatch")
            .map_err(|error| Error::new(&id, error.to_string()))?;
        let command = winter
            .get("__command")
            .map_err(|error| Error::new(&id, error.to_string()))?;
        let freeze = winter
            .get("__freeze")
            .map_err(|error| Error::new(&id, error.to_string()))?;
        let history_take = winter
            .get("__historyTake")
            .map_err(|error| Error::new(&id, error.to_string()))?;
        let history_resolve = winter
            .get("__historyResolve")
            .map_err(|error| Error::new(&id, error.to_string()))?;

        self.extensions.push(Running {
            id,
            context,
            state,
            logs,
            snapshot,
            dispatch,
            command,
            freeze,
            history_take,
            history_resolve,
            commands: Vec::new(),
        });
        Ok(())
    }

    /// Close registration in every extension and read what each one added.
    ///
    /// Called once, after the last extension has loaded: an extension may
    /// register for its whole load, and none may register after it.
    pub fn freeze(&mut self) {
        for index in 0..self.extensions.len() {
            let running = &mut self.extensions[index];
            let value = match running.context.call(&running.freeze, &JsValue::undefined(), &[]) {
                Ok(value) => value,
                Err(error) => {
                    push_log(&running.logs, &running.id, &format!("freeze failed: {error}"));
                    continue;
                }
            };
            let Some(text) = running.context.to_string(&value).ok() else {
                continue;
            };
            match serde_json::from_str::<Vec<Frozen>>(&text) {
                Ok(frozen) => {
                    let extension = running.id.clone();
                    running.commands = frozen
                        .into_iter()
                        .map(|frozen| Command {
                            key: format!("{extension}:{}", frozen.id),
                            extension: extension.clone(),
                            id: frozen.id,
                            label: frozen.label,
                        })
                        .collect();
                }
                Err(error) => {
                    push_log(&running.logs, &running.id, &format!("freeze returned junk: {error}"));
                }
            }
        }
    }

    /// Every command any extension registered, in palette order.
    pub fn commands(&self) -> Vec<Command> {
        self.extensions
            .iter()
            .flat_map(|running| running.commands.iter().cloned())
            .collect()
    }

    /// Report an observation to every extension.
    pub fn emit(&mut self, host: &mut dyn Host, event: &Event) {
        let snapshot = snapshot(host);
        let payload = event.payload();
        for index in 0..self.extensions.len() {
            let mut calls = self.call_hook(host, index, &snapshot, event.name(), &payload);
            self.drain_history(host, index, &mut calls);
            self.apply(host, index, calls);
        }
    }

    /// Run the command `key` (`<extension id>:<command id>`). `false` when no
    /// extension registered it.
    pub fn run(&mut self, host: &mut dyn Host, key: &str) -> bool {
        let mut hops = HOPS_MAX;
        self.run_key(host, key, &mut hops)
    }

    /// Take everything the extensions logged since the last call, in order,
    /// each line prefixed with the extension that wrote it.
    pub fn take_logs(&mut self) -> Vec<String> {
        for running in &self.extensions {
            let mut logs = running.logs.borrow_mut();
            self.logs.append(&mut logs);
        }
        std::mem::take(&mut self.logs)
    }

    /// Set a snapshot on an extension and call one of the bootstrap's entry
    /// points, returning what the handler queued.
    fn call_hook(
        &mut self,
        host: &mut dyn Host,
        index: usize,
        snapshot_json: &str,
        name: &str,
        payload: &str,
    ) -> Vec<Call> {
        let running = &mut self.extensions[index];
        if let Err(error) = running.context.call(
            &running.snapshot,
            &JsValue::undefined(),
            &[JsValue::string(snapshot_json)],
        ) {
            push_log(
                &running.logs,
                &running.id,
                &format!("the snapshot was refused: {error}"),
            );
        }
        if let Err(error) = running.context.call(
            &running.dispatch,
            &JsValue::undefined(),
            &[JsValue::string(name), JsValue::string(payload)],
        ) {
            push_log(&running.logs, &running.id, &format!("{name} dispatch failed: {error}"));
        }
        let _ = host;
        std::mem::take(&mut running.state.borrow_mut().calls)
    }

    /// Run one extension's command, then whatever its handler queued.
    fn run_key(&mut self, host: &mut dyn Host, key: &str, hops: &mut usize) -> bool {
        let Some(index) = self
            .extensions
            .iter()
            .position(|running| running.commands.iter().any(|command| command.key == key))
        else {
            self.logs.push(format!("no extension registered the command `{key}`"));
            return false;
        };
        if *hops == 0 {
            let id = self.extensions[index].id.clone();
            self.logs
                .push(format!("{id}: command `{key}` re-entered too deeply; dropped"));
            return false;
        }
        *hops -= 1;

        let snapshot_json = snapshot(host);
        let calls = {
            let running = &mut self.extensions[index];
            let Some(command) = running.commands.iter().find(|command| command.key == key).cloned() else {
                return false;
            };
            if let Err(error) = running.context.call(
                &running.snapshot,
                &JsValue::undefined(),
                &[JsValue::string(&snapshot_json)],
            ) {
                push_log(
                    &running.logs,
                    &running.id,
                    &format!("the snapshot was refused: {error}"),
                );
            }
            if let Err(error) =
                running
                    .context
                    .call(&running.command, &JsValue::undefined(), &[JsValue::string(&command.id)])
            {
                push_log(
                    &running.logs,
                    &running.id,
                    &format!("command `{}` failed: {error}", command.id),
                );
                return false;
            }
            std::mem::take(&mut running.state.borrow_mut().calls)
        };
        let mut calls = calls;
        self.drain_history(host, index, &mut calls);
        self.apply(host, index, calls);
        true
    }

    /// Answer the history a handler asked for beyond the snapshot.
    ///
    /// The bootstrap holds the callbacks; this walks the buffer for each
    /// request, hands the text back, and takes whatever the callbacks queued.
    /// A continuation may ask for more, so the rounds are bounded rather than
    /// trusted.
    fn drain_history(&mut self, host: &mut dyn Host, index: usize, calls: &mut Vec<Call>) {
        for _ in 0..HISTORY_ROUNDS {
            let requested = {
                let running = &mut self.extensions[index];
                let taken = match running.context.call(&running.history_take, &JsValue::undefined(), &[]) {
                    Ok(value) => running.context.to_string(&value).unwrap_or_default(),
                    Err(error) => {
                        push_log(
                            &running.logs,
                            &running.id,
                            &format!("the history request was refused: {error}"),
                        );
                        String::new()
                    }
                };
                serde_json::from_str::<Vec<usize>>(&taken).unwrap_or_default()
            };
            if requested.is_empty() {
                return;
            }
            for (position, lines) in requested.iter().enumerate() {
                let text = host.history(*lines);
                let running = &mut self.extensions[index];
                if let Err(error) = running.context.call(
                    &running.history_resolve,
                    &JsValue::undefined(),
                    &[JsValue::number(position as f64), JsValue::string(text)],
                ) {
                    push_log(
                        &running.logs,
                        &running.id,
                        &format!("a history continuation failed: {error}"),
                    );
                }
            }
            calls.extend(std::mem::take(&mut self.extensions[index].state.borrow_mut().calls));
        }
        let id = self.extensions[index].id.clone();
        self.logs
            .push(format!("{id}: history continuations kept asking; stopped"));
    }

    /// Apply one extension's queued calls, in the order it made them.
    fn apply(&mut self, host: &mut dyn Host, index: usize, calls: Vec<Call>) {
        let mut hops = HOPS_MAX;
        for call in calls {
            match call {
                Call::Send(text) => host.send_text(&text),
                Call::Copy(text) => host.copy(&text),
                Call::Notify(title, body) => host.notify(&title, &body),
                Call::OpenUrl(url) => {
                    if !host.open_url(&url)
                        && let Some(running) = self.extensions.get(index)
                    {
                        push_log(&running.logs, &running.id, "the shell has no URL opener");
                    }
                }
                Call::SelectTab(tab) => host.select_tab(tab),
                Call::Run(key) => {
                    self.run_key(host, &key, &mut hops);
                }
            }
        }
    }
}

/// What `__freeze` answers for each command.
#[derive(Debug, Deserialize)]
struct Frozen {
    id: String,
    label: String,
}

/// The engine's name resolution for one extension: a relative specifier is
/// resolved against the importing module's directory, and the answer is the key
/// the module was registered under.
#[derive(Debug)]
struct Specifiers;

impl slag::HostHooks for Specifiers {
    fn resolve_imported_specifier(
        &self,
        specifier: &slag::JsString,
        referrer: Option<&slag::JsString>,
    ) -> Option<slag::JsString> {
        let referrer = referrer?.to_string_lossy();
        let specifier = specifier.to_string_lossy();
        resolve_specifier(&referrer, &specifier).map(|key| slag::JsString::from_utf8(&key))
    }
}

/// Check that every specifier a module declares resolves to one the extension
/// ships.
///
/// A relative specifier resolves against the importing file, exactly as the
/// engine's resolver will at instantiation, so a miss names the file the
/// specifier actually denotes. A bare specifier is refused because nothing is
/// installed under one, a specifier that climbs out of the root is refused for
/// the same reason, and the entry is named as unimportable.
fn validate_specifiers(module: &slag::JsModule, referrer: &str, keys: &HashSet<String>, entry: &str) -> Vec<String> {
    let mut problems = Vec::new();
    for index in 0..module.request_count() {
        let Some(specifier) = module
            .request_specifier(index)
            .map(|specifier| specifier.to_string_lossy())
        else {
            continue;
        };
        if !is_relative(&specifier) {
            problems.push(format!(
                "`{referrer}` imports `{specifier}`, a bare specifier; every import resolves from the importing file, so write it as `./{specifier}`"
            ));
            continue;
        }
        let Some(resolved) = resolve_specifier(referrer, &specifier) else {
            problems.push(format!(
                "`{referrer}` imports `{specifier}`, which climbs out of the extension root"
            ));
            continue;
        };
        if resolved == entry {
            problems.push(format!(
                "`{referrer}` imports `{specifier}`, which resolves to the entry `{entry}`; the entry cannot be imported, so move the shared code to another module"
            ));
        } else if !keys.contains(&resolved) {
            problems.push(format!(
                "`{referrer}` imports `{specifier}`, which resolves to `{resolved}`, a module the extension does not ship"
            ));
        }
    }
    problems
}

/// Resolve `specifier` against the importing module's directory, relative to the
/// extension root. `None` for a bare specifier (the engine has no module under
/// it) or for one that climbs out of the root.
fn resolve_specifier(referrer: &str, specifier: &str) -> Option<String> {
    if !is_relative(specifier) {
        return None;
    }
    join(parent(referrer), specifier)
}

/// Whether `specifier` names a file relative to the importer rather than a bare
/// module name.
fn is_relative(specifier: &str) -> bool {
    matches!(specifier, "." | ".." | "./" | "../") || specifier.starts_with("./") || specifier.starts_with("../")
}

/// The directory part of a root-relative module name.
fn parent(name: &str) -> &str {
    name.rsplit_once('/').map(|(dir, _)| dir).unwrap_or("")
}

/// Resolve `specifier` against `dir`, or `None` when it climbs out of the root.
fn join(dir: &str, specifier: &str) -> Option<String> {
    let mut parts: Vec<&str> = if dir.is_empty() {
        Vec::new()
    } else {
        dir.split('/').collect()
    };
    for segment in specifier.split('/') {
        match segment {
            "" | "." => {}
            ".." => {
                parts.pop()?;
            }
            other => parts.push(other),
        }
    }
    if parts.is_empty() { None } else { Some(parts.join("/")) }
}

/// The most scrollback the host gathers for one dispatch. An extension may ask
/// for less; asking for more returns what the host has, because the snapshot is
/// taken before the handler runs and cannot grow underneath it.
pub const HISTORY_LINES: usize = 200;

/// The one picture of the shell a handler sees.
///
/// Taken before the handler runs, so every read in it agrees: the tabs, the
/// focused pane, the visible screen, and a bounded walk of the scrollback above
/// that screen. The walk is the only part that needs the pane mutably, which is
/// why the whole snapshot takes the host mutably.
fn snapshot(host: &mut dyn Host) -> String {
    let tabs = host.tabs();
    let focused = host.focused();
    let text = host.visible_text();
    let history = host.history(HISTORY_LINES);
    serde_json::json!({
        "tabs": tabs,
        "focused": focused,
        "text": text,
        "history": history,
    })
    .to_string()
}

/// Console output and unhandled rejections, prefixed with the extension's id.
fn host_callbacks(id: &str, logs: &Rc<RefCell<Vec<String>>>) -> HostCallbacks {
    let sink = {
        let id = id.to_string();
        let logs = Rc::clone(logs);
        move |text: &str| push_log(&logs, &id, text)
    };
    let console = || Some(Box::new(sink.clone()) as Box<dyn Fn(&str)>);
    HostCallbacks {
        console_log: console(),
        console_info: console(),
        console_warn: console(),
        console_error: console(),
        console_debug: console(),
        promise_rejection: console(),
        ..Default::default()
    }
}

/// Install the primitives the bootstrap reads and then deletes.
fn install_primitives(
    context: &mut Context,
    id: &str,
    state: &Rc<RefCell<State>>,
    logs: &Rc<RefCell<Vec<String>>>,
) -> Result<(), JsError> {
    {
        let state = Rc::clone(state);
        context.register_fn(
            "__winter_send",
            1,
            Box::new(move |call| {
                queue(&state, Call::Send(argument(call, 0)?));
                Ok(JsValue::undefined())
            }),
        )?;
    }
    {
        let state = Rc::clone(state);
        context.register_fn(
            "__winter_copy",
            1,
            Box::new(move |call| {
                queue(&state, Call::Copy(argument(call, 0)?));
                Ok(JsValue::undefined())
            }),
        )?;
    }
    {
        let state = Rc::clone(state);
        context.register_fn(
            "__winter_notify",
            2,
            Box::new(move |call| {
                let title = argument(call, 0)?;
                let body = argument(call, 1).unwrap_or_default();
                queue(&state, Call::Notify(title, body));
                Ok(JsValue::undefined())
            }),
        )?;
    }
    {
        let state = Rc::clone(state);
        context.register_fn(
            "__winter_open_url",
            1,
            Box::new(move |call| {
                queue(&state, Call::OpenUrl(argument(call, 0)?));
                Ok(JsValue::undefined())
            }),
        )?;
    }
    {
        let state = Rc::clone(state);
        context.register_fn(
            "__winter_run",
            1,
            Box::new(move |call| {
                queue(&state, Call::Run(argument(call, 0)?));
                Ok(JsValue::undefined())
            }),
        )?;
    }
    {
        let state = Rc::clone(state);
        let logs = Rc::clone(logs);
        let id = id.to_string();
        context.register_fn(
            "__winter_select",
            1,
            Box::new(move |call| {
                let number = call.arg(0).map(|value| call.to_number(&value)).transpose()?;
                match number {
                    Some(number) if number >= 0.0 && number.fract() == 0.0 => {
                        let index = number as usize;
                        queue(&state, Call::SelectTab(index));
                    }
                    _ => push_log(&logs, &id, "tabs.select was given a negative or fractional index"),
                }
                Ok(JsValue::undefined())
            }),
        )?;
    }
    {
        let logs = Rc::clone(logs);
        let id = id.to_string();
        context.register_fn(
            "__winter_log",
            1,
            Box::new(move |call| {
                let message = argument(call, 0).unwrap_or_else(|_| "<unprintable>".to_string());
                push_log(&logs, &id, &message);
                Ok(JsValue::undefined())
            }),
        )?;
    }
    Ok(())
}

fn argument(call: &slag::FunctionCall<'_>, index: usize) -> Result<String, JsError> {
    match call.arg(index) {
        Some(value) => call.to_string(&value),
        None => Ok(String::new()),
    }
}

fn queue(state: &Rc<RefCell<State>>, call: Call) {
    state.borrow_mut().calls.push(call);
}

fn push_log(logs: &Rc<RefCell<Vec<String>>>, id: &str, message: &str) {
    let mut logs = logs.borrow_mut();
    if message.is_empty() {
        return;
    }
    for line in message.lines() {
        logs.push(format!("{id}: {line}"));
    }
}

/// Why an extension did not load.
#[derive(Debug, thiserror::Error)]
#[error("{id}: {message}")]
pub struct Error {
    /// The extension's identifier.
    pub id: String,
    /// What went wrong, in a line fit for the log.
    pub message: String,
}

impl Error {
    fn new(id: &str, message: impl Into<String>) -> Self {
        Self {
            id: id.to_string(),
            message: message.into(),
        }
    }
}

/// The `winter` global and the dispatcher the host calls back into.
///
/// The identity and the asset names are spliced in before evaluation; the
/// primitives it reads are deleted from the global as soon as it has captured
/// them, so an extension cannot reach the shell except through `winter`.
const BOOTSTRAP: &str = r##"(function (identity, assetNames) {
  "use strict";

  const sendText = globalThis.__winter_send;
  const copyText = globalThis.__winter_copy;
  const notify = globalThis.__winter_notify;
  const openUrl = globalThis.__winter_open_url;
  const log = globalThis.__winter_log;
  const runElsewhere = globalThis.__winter_run;
  const selectTab = globalThis.__winter_select;

  delete globalThis.__winter_send;
  delete globalThis.__winter_copy;
  delete globalThis.__winter_notify;
  delete globalThis.__winter_open_url;
  delete globalThis.__winter_log;
  delete globalThis.__winter_run;
  delete globalThis.__winter_select;

  const handlers = new Map();
  const commands = new Map();
  // The history a handler asked for beyond the snapshot, and the batch the host
  // is resolving now; both are drained by the host after the handler returns.
  const historyRequests = [];
  let historyTaken = [];
  // The declared assets, by the opaque name the extension asks for: the bytes
  // arrive as `ArrayBuffer`s in the same order as the names.
  const bytesByAsset = globalThis.__winter_asset_bytes || [];
  delete globalThis.__winter_asset_bytes;
  const assets = new Map();
  for (let i = 0; i < assetNames.length; i++) {
    assets.set(assetNames[i], bytesByAsset[i]);
  }
  let current = null;
  let frozen = false;

  function fail(where, message) {
    throw new TypeError("winter." + where + ": " + message);
  }

  function report(where, error) {
    let detail;
    try {
      detail = error && error.stack ? String(error.stack) : String(error);
    } catch (_) {
      detail = "an unprintable error";
    }
    log(where + " threw: " + detail);
  }

  function invoke(where, handler, argument) {
    try {
      handler(argument);
    } catch (error) {
      report(where, error);
    }
  }

  function open(where) {
    if (frozen) {
      throw new Error("winter." + where + ": registration is closed");
    }
  }

  const winter = {
    api: identity.api,
    extension: Object.freeze({
      id: identity.id,
      name: identity.name,
      version: identity.version,
    }),

    events: {
      on(name, handler) {
        open("events.on");
        if (typeof name !== "string" || name === "") {
          fail("events.on", "the event name must be a non-empty string");
        }
        if (typeof handler !== "function") {
          fail("events.on", "the handler must be a function");
        }
        const list = handlers.get(name) || [];
        list.push(handler);
        handlers.set(name, list);
        let removed = false;
        return function unsubscribe() {
          if (removed) return;
          removed = true;
          const current = handlers.get(name) || [];
          const at = current.indexOf(handler);
          if (at >= 0) current.splice(at, 1);
        };
      },
    },

    commands: {
      add(id, label, handler) {
        open("commands.add");
        if (typeof id !== "string" || id === "" || id.indexOf(":") >= 0) {
          fail("commands.add", "the id must be a non-empty string without `:`");
        }
        if (typeof label !== "string") {
          fail("commands.add", "the label must be a string");
        }
        if (typeof handler !== "function") {
          fail("commands.add", "the handler must be a function");
        }
        if (commands.has(id)) {
          fail("commands.add", "`" + id + "` is already registered");
        }
        commands.set(id, { label: label, handler: handler });
      },
      run(id) {
        if (typeof id !== "string" || id === "") {
          fail("commands.run", "the command id must be a non-empty string");
        }
        const prefix = identity.id + ":";
        const local = id.slice(0, prefix.length) === prefix ? id.slice(prefix.length) : id;
        if (commands.has(local)) {
          invoke("command " + prefix + local, commands.get(local).handler, undefined);
          return;
        }
        if (id.slice(0, prefix.length) === prefix || id.indexOf(":") < 0) {
          throw new Error("winter.commands.run: `" + id + "` is not registered");
        }
        runElsewhere(id);
      },
    },

    panes: {
      focused() {
        return current ? current.focused : null;
      },
      text() {
        return current ? current.text : "";
      },
      // The last `lines` rows of the scrollback the host gathered before this
      // handler ran. Without a callback, a larger request returns what was
      // gathered; with one, the host walks the buffer for exactly that many rows
      // and calls back before the dispatch ends.
      history(lines, callback) {
        if (!Number.isInteger(lines) || lines < 1) {
          fail("panes.history", "the row count must be a positive integer");
        }
        if (callback !== undefined && typeof callback !== "function") {
          fail("panes.history", "the second argument must be a function");
        }
        const all = current && typeof current.history === "string" ? current.history : "";
        const rows = all === "" ? [] : all.split("\n");
        if (callback === undefined) {
          return rows.length === 0 ? "" : rows.slice(Math.max(0, rows.length - lines)).join("\n");
        }
        if (lines <= rows.length) {
          invoke("panes.history", callback, rows.slice(rows.length - lines).join("\n"));
          return;
        }
        historyRequests.push({ lines: lines, callback: callback });
      },
      send(text) {
        if (typeof text !== "string") fail("panes.send", "the text must be a string");
        sendText(text);
      },
    },

    tabs: {
      list() {
        return current ? current.tabs : [];
      },
      select(index) {
        if (!Number.isInteger(index) || index < 0) {
          fail("tabs.select", "the index must be a non-negative integer");
        }
        selectTab(index);
      },
    },

    app: {
      notify(title, body) {
        if (typeof title !== "string") fail("app.notify", "the title must be a string");
        notify(title, body === undefined ? "" : String(body));
      },
      clipboard: {
        write(text) {
          if (typeof text !== "string") fail("clipboard.write", "the text must be a string");
          copyText(text);
        },
      },
      openUrl(url) {
        if (typeof url !== "string") fail("app.openUrl", "the url must be a string");
        openUrl(url);
      },
      log(message) {
        log(message === undefined ? "" : String(message));
      },
    },

    assets: {
      get(name) {
        if (typeof name !== "string" || !assets.has(name)) {
          fail("assets.get", "`" + name + "` is not declared in the manifest");
        }
        return name;
      },
      // A copy: an extension that overwrites a buffer must not change what the
      // next reader of the same asset sees.
      bytes(name) {
        if (typeof name !== "string") {
          fail("assets.bytes", "the name must be a string");
        }
        const buffer = assets.get(name);
        if (buffer === undefined) {
          fail("assets.bytes", "`" + name + "` is not declared in the manifest");
        }
        return copyBytes(buffer);
      },
    },
  };

  function hidden(target, name, value) {
    Object.defineProperty(target, name, {
      value: value,
      enumerable: false,
      writable: false,
      configurable: false,
    });
  }

  hidden(winter, "__snapshot", function (json) {
    current = json ? JSON.parse(json) : null;
  });

  hidden(winter, "__dispatch", function (name, payload) {
    const list = handlers.get(name);
    if (!list || list.length === 0) return;
    const event = payload ? JSON.parse(payload) : {};
    for (const handler of list.slice()) {
      invoke("event " + name, handler, event);
    }
  });

  hidden(winter, "__command", function (id) {
    const command = commands.get(id);
    if (!command) return;
    invoke("command " + identity.id + ":" + id, command.handler, undefined);
  });

  hidden(winter, "__freeze", function () {
    frozen = true;
    const list = [];
    for (const entry of commands) {
      list.push({ id: entry[0], label: entry[1].label });
    }
    return JSON.stringify(list);
  });

  // ── The compiled half ────────────────────────────────────────────────────
  // A module declares the capabilities it wants as imports from `winter`, so
  // its import list *is* its API: anything else fails to link. It registers
  // through those imports during `winter_init` and is driven through four
  // optional exports; every string crosses as UTF-8 at a pointer into the
  // module's own exported memory.
  let memory = null;
  let wasmExports = null;

  function encodeUtf8(text) {
    const bytes = [];
    for (let i = 0; i < text.length; i++) {
      let code = text.charCodeAt(i);
      if (code >= 0xd800 && code <= 0xdbff && i + 1 < text.length) {
        const low = text.charCodeAt(i + 1);
        if (low >= 0xdc00 && low <= 0xdfff) {
          code = 0x10000 + ((code - 0xd800) << 10) + (low - 0xdc00);
          i++;
        }
      }
      if (code < 0x80) {
        bytes.push(code);
      } else if (code < 0x800) {
        bytes.push(0xc0 | (code >> 6), 0x80 | (code & 0x3f));
      } else if (code < 0x10000) {
        bytes.push(0xe0 | (code >> 12), 0x80 | ((code >> 6) & 0x3f), 0x80 | (code & 0x3f));
      } else {
        bytes.push(
          0xf0 | (code >> 18),
          0x80 | ((code >> 12) & 0x3f),
          0x80 | ((code >> 6) & 0x3f),
          0x80 | (code & 0x3f),
        );
      }
    }
    return bytes;
  }

  function readText(ptr, len) {
    if (!memory) {
      throw new Error("winter: the module's memory is not in hand yet; register from winter_init, not a start function");
    }
    if (ptr < 0 || len < 0 || ptr + len > memory.buffer.byteLength) {
      throw new RangeError("winter: the module pointed outside its own memory");
    }
    const bytes = new Uint8Array(memory.buffer, ptr, len);
    let out = "";
    for (let i = 0; i < bytes.length; ) {
      const first = bytes[i++];
      if (first < 0x80) {
        out += String.fromCharCode(first);
        continue;
      }
      let need = 0;
      let code = 0;
      if ((first & 0xe0) === 0xc0) {
        need = 1;
        code = first & 0x1f;
      } else if ((first & 0xf0) === 0xe0) {
        need = 2;
        code = first & 0x0f;
      } else if ((first & 0xf8) === 0xf0) {
        need = 3;
        code = first & 0x07;
      } else {
        throw new Error("winter: the module wrote an invalid UTF-8 lead byte");
      }
      if (i + need > bytes.length) {
        throw new Error("winter: the module wrote a truncated UTF-8 sequence");
      }
      for (let k = 0; k < need; k++) {
        const next = bytes[i++];
        if ((next & 0xc0) !== 0x80) {
          throw new Error("winter: the module wrote an invalid UTF-8 continuation");
        }
        code = (code << 6) | (next & 0x3f);
      }
      if (code > 0x10ffff) {
        throw new Error("winter: the module wrote a code point past U+10FFFF");
      }
      if (code > 0xffff) {
        const rest = code - 0x10000;
        out += String.fromCharCode(0xd800 + (rest >> 10), 0xdc00 + (rest & 0x3ff));
      } else {
        out += String.fromCharCode(code);
      }
    }
    return out;
  }

  // Writes a host string into the module's memory through its own allocator,
  // so the module decides where its arena is.
  function writeText(text) {
    if (!wasmExports || typeof wasmExports.winter_alloc !== "function") {
      throw new Error("winter: the module exports no winter_alloc, so no string can reach it");
    }
    const bytes = encodeUtf8(text);
    const ptr = wasmExports.winter_alloc(bytes.length);
    if (!memory || ptr < 0 || ptr + bytes.length > memory.buffer.byteLength) {
      throw new RangeError("winter: the module allocated outside its own memory");
    }
    const view = new Uint8Array(memory.buffer, ptr, bytes.length);
    for (let i = 0; i < bytes.length; i++) view[i] = bytes[i];
    return { ptr: ptr, len: bytes.length };
  }

  // Writes `bytes` at `ptr` into the module's memory, at most `cap` of them,
  // and answers how many the value needs — the `snprintf` convention, so a
  // module can ask with an empty buffer first and allocate what it is told.
  function writeBytes(ptr, cap, bytes) {
    if (!memory) {
      throw new Error("winter: the module's memory is not in hand yet; read from winter_init onwards, not from a start function");
    }
    if (ptr < 0 || cap < 0) {
      throw new RangeError("winter: the module pointed outside its own memory");
    }
    const written = Math.min(bytes.length, cap);
    if (ptr + written > memory.buffer.byteLength) {
      throw new RangeError("winter: the module's buffer runs past its own memory");
    }
    const view = new Uint8Array(memory.buffer, ptr, written);
    for (let i = 0; i < written; i++) view[i] = bytes[i];
    return bytes.length;
  }

  // A fresh typed array over the same bytes, so a caller cannot write through
  // to the host's copy of an asset.
  function copyBytes(buffer) {
    const source = new Uint8Array(buffer);
    const copy = new Uint8Array(source.length);
    for (let i = 0; i < source.length; i++) copy[i] = source[i];
    return copy;
  }

  function callWasmCommand(id) {
    if (!wasmExports || typeof wasmExports.winter_command !== "function") return;
    try {
      const written = writeText(id);
      wasmExports.winter_command(written.ptr, written.len);
    } catch (error) {
      report("command " + identity.id + ":" + id, error);
    }
  }

  function callWasmEvent(name, event) {
    if (!wasmExports || typeof wasmExports.winter_event !== "function") return;
    try {
      const written = writeText(name);
      const payload = writeText(JSON.stringify(event));
      wasmExports.winter_event(written.ptr, written.len, payload.ptr, payload.len);
    } catch (error) {
      report("event " + name, error);
    }
  }

  const granted = {
    // The same one picture a script handler sees, as JSON. It is `null` before
    // the first dispatch, so a module can tell that `winter_init` has no shell
    // to read yet.
    snapshot(ptr, cap) {
      return writeBytes(ptr, cap, encodeUtf8(JSON.stringify(current)));
    },
    // The typed half of the same picture, for a module that would rather not
    // carry a parser. `-1` is "no such tab" and "no focused pane"; a string
    // accessor answers the bytes it needs, like `snapshot`.
    tabs_count() {
      return current ? current.tabs.length : 0;
    },
    tabs_active() {
      if (!current) return -1;
      for (let i = 0; i < current.tabs.length; i++) {
        if (current.tabs[i].active) return current.tabs[i].index;
      }
      return -1;
    },
    tab_panes(index) {
      if (!current) return -1;
      for (let i = 0; i < current.tabs.length; i++) {
        if (current.tabs[i].index === index) return current.tabs[i].panes;
      }
      return -1;
    },
    tab_title(index, ptr, cap) {
      if (!current) return -1;
      for (let i = 0; i < current.tabs.length; i++) {
        if (current.tabs[i].index === index) {
          return writeBytes(ptr, cap, encodeUtf8(current.tabs[i].title));
        }
      }
      return -1;
    },
    pane_columns() {
      const pane = current ? current.focused : null;
      return pane ? pane.columns : 0;
    },
    pane_rows() {
      const pane = current ? current.focused : null;
      return pane ? pane.rows : 0;
    },
    pane_id(ptr, cap) {
      const pane = current ? current.focused : null;
      return pane ? writeBytes(ptr, cap, encodeUtf8(pane.id)) : -1;
    },
    pane_title(ptr, cap) {
      const pane = current ? current.focused : null;
      return pane ? writeBytes(ptr, cap, encodeUtf8(pane.title)) : -1;
    },
    pane_program(ptr, cap) {
      const pane = current ? current.focused : null;
      return pane ? writeBytes(ptr, cap, encodeUtf8(pane.program)) : -1;
    },
    // The scrollback the host gathered before this dispatch, as one string
    // separated by newlines — the same text `panes.history` trims.
    history(ptr, cap) {
      return writeBytes(ptr, cap, encodeUtf8(current && current.history ? current.history : ""));
    },
    // The bytes of a declared asset, copied into the module's own memory. `-1`
    // is "no such asset", so an empty asset and a missing one are different
    // answers.
    asset(namePtr, nameLen, ptr, cap) {
      const name = readText(namePtr, nameLen);
      const buffer = assets.get(name);
      if (buffer === undefined) {
        return -1;
      }
      return writeBytes(ptr, cap, new Uint8Array(buffer));
    },
    log(ptr, len) {
      log(readText(ptr, len));
    },
    send(ptr, len) {
      sendText(readText(ptr, len));
    },
    notify(titlePtr, titleLen, bodyPtr, bodyLen) {
      notify(readText(titlePtr, titleLen), readText(bodyPtr, bodyLen));
    },
    clipboard_write(ptr, len) {
      copyText(readText(ptr, len));
    },
    open_url(ptr, len) {
      openUrl(readText(ptr, len));
    },
    select_tab(index) {
      try {
        if (!Number.isInteger(index) || index < 0) {
          fail("wasm.select_tab", "the index must be a non-negative integer");
        }
        selectTab(index);
      } catch (error) {
        report("wasm.select_tab", error);
      }
    },
    register_command(idPtr, idLen, labelPtr, labelLen) {
      try {
        open("wasm.register_command");
        const id = readText(idPtr, idLen);
        const label = readText(labelPtr, labelLen);
        if (id === "" || id.indexOf(":") >= 0) {
          fail("wasm.register_command", "the id must be a non-empty string without `:`");
        }
        if (commands.has(id)) {
          fail("wasm.register_command", "`" + id + "` is already registered");
        }
        commands.set(id, {
          label: label,
          handler: function () {
            callWasmCommand(id);
          },
        });
        return 0;
      } catch (error) {
        report("wasm.register_command", error);
        return 1;
      }
    },
    register_event(namePtr, nameLen) {
      try {
        open("wasm.register_event");
        const name = readText(namePtr, nameLen);
        if (name === "") {
          fail("wasm.register_event", "the event name must be a non-empty string");
        }
        const list = handlers.get(name) || [];
        list.push(function (event) {
          callWasmEvent(name, event);
        });
        handlers.set(name, list);
        return 0;
      } catch (error) {
        report("wasm.register_event", error);
        return 1;
      }
    },
  };

  hidden(winter, "__wasm", function (bytes) {
    // Instantiation errors are answered as text: a thrown value reaches the
    // host as a generic `Uncaught exception`, and every failure here is the
    // host's own sentence about the module.
    try {
      const module = new WebAssembly.Module(bytes);
      for (const want of WebAssembly.Module.imports(module)) {
        if (want.module !== "winter" || want.kind !== "function" || !(want.name in granted)) {
          throw new Error(
            "winter: the module imports `" + want.module + "." + want.name + "`, which this host does not grant",
          );
        }
      }
      const instance = new WebAssembly.Instance(module, { winter: granted });
      const exports = instance.exports;
      if (!exports.memory) {
        throw new Error("winter: the module exports no memory, so the host cannot read its strings");
      }
      memory = exports.memory;
      wasmExports = exports;
      if (typeof exports.winter_abi !== "function") {
        throw new Error("winter: the module exports no winter_abi");
      }
      const abi = exports.winter_abi();
      if (abi !== 1) {
        throw new Error("winter: the module targets ABI " + abi + ", and this host implements 1");
      }
      if (typeof exports.winter_init === "function") {
        const status = exports.winter_init();
        if (status !== 0) {
          throw new Error("winter: winter_init answered " + status);
        }
      }
      return "";
    } catch (error) {
      return error && error.message ? String(error.message) : String(error);
    }
  });

  hidden(winter, "__historyTake", function () {
    historyTaken = historyRequests.slice();
    historyRequests.length = 0;
    const counts = [];
    for (let i = 0; i < historyTaken.length; i++) counts.push(historyTaken[i].lines);
    return JSON.stringify(counts);
  });

  hidden(winter, "__historyResolve", function (index, text) {
    const request = historyTaken[index];
    if (!request) return;
    invoke("panes.history", request.callback, text);
  });

  globalThis.winter = winter;
})(__IDENTITY__, __ASSETS__);
"##;

#[cfg(test)]
mod tests {
    use std::path::{Path, PathBuf};

    use super::*;
    use crate::discover::{Options, discover};
    use crate::{PaneInfo, TabInfo};

    /// The shell side of the contract, recorded rather than acted on.
    #[derive(Default)]
    struct Recorder {
        tabs: Vec<TabInfo>,
        focused: Option<PaneInfo>,
        text: String,
        history: String,
        sent: Vec<String>,
        copied: Vec<String>,
        notified: Vec<(String, String)>,
        opened: Vec<String>,
        selected: Vec<usize>,
    }

    impl Host for Recorder {
        fn tabs(&self) -> Vec<TabInfo> {
            self.tabs.clone()
        }
        fn focused(&self) -> Option<PaneInfo> {
            self.focused.clone()
        }
        fn visible_text(&self) -> String {
            self.text.clone()
        }
        fn history(&mut self, lines: usize) -> String {
            let rows: Vec<&str> = if self.history.is_empty() {
                Vec::new()
            } else {
                self.history.split('\n').collect()
            };
            rows[rows.len().saturating_sub(lines)..].join("\n")
        }
        fn send_text(&mut self, text: &str) {
            self.sent.push(text.to_string());
        }
        fn copy(&mut self, text: &str) {
            self.copied.push(text.to_string());
        }
        fn notify(&mut self, title: &str, body: &str) {
            self.notified.push((title.to_string(), body.to_string()));
        }
        fn open_url(&mut self, url: &str) -> bool {
            self.opened.push(url.to_string());
            true
        }
        fn select_tab(&mut self, index: usize) {
            self.selected.push(index);
        }
    }

    fn write(dir: &Path, name: &str, text: &str) {
        let path = dir.join(name);
        if let Some(parent) = path.parent() {
            std::fs::create_dir_all(parent).expect("parent");
        }
        std::fs::write(path, text).expect("write");
    }

    /// An extension directory under a fresh temp root, with the given entry and
    /// extra files.
    fn extension(entry: &str, files: &[(&str, &str)]) -> (tempfile::TempDir, PathBuf) {
        let temp = tempfile::tempdir().expect("tempdir");
        let dir = temp.path().join("fixture");
        std::fs::create_dir_all(&dir).expect("dir");
        write(
            &dir,
            "extension.toml",
            "id = \"test.fixture\"\nname = \"Fixture\"\nversion = \"1\"\napi = 1\nentry = \"main.js\"\n",
        );
        write(&dir, "main.js", entry);
        for (name, text) in files {
            write(&dir, name, text);
        }
        (temp, dir)
    }

    fn load(dir: &Path) -> (Engine, Vec<String>) {
        let report = discover(&Options {
            roots: vec![dir.to_path_buf()],
            disabled: Vec::new(),
        });
        assert!(report.skipped.is_empty(), "{:?}", report.skipped);
        let mut engine = Engine::default();
        for loaded in &report.loaded {
            engine.load(loaded).expect("load");
        }
        engine.freeze();
        let logs = engine.take_logs();
        (engine, logs)
    }

    #[test]
    fn a_registered_command_runs_and_reaches_the_shell() {
        let (_temp, dir) = extension(
            r#"
            winter.commands.add("greet", "Greet", () => winter.app.notify("Winter", "hi"));
            "#,
            &[],
        );
        let (mut engine, _logs) = load(&dir);
        let commands = engine.commands();
        assert_eq!(commands.len(), 1);
        assert_eq!(commands[0].key, "test.fixture:greet");
        assert_eq!(commands[0].label, "Greet");

        let mut host = Recorder::default();
        assert!(engine.run(&mut host, "test.fixture:greet"));
        assert_eq!(host.notified, [("Winter".to_string(), "hi".to_string())]);
    }

    #[test]
    fn a_handler_reads_one_snapshot_of_the_shell() {
        let (_temp, dir) = extension(
            r#"
            winter.events.on("commandFinished", (event) => {
              winter.panes.send(winter.panes.text() + event.row + ":" + event.error + "\n");
            });
            "#,
            &[],
        );
        let (mut engine, _logs) = load(&dir);
        let mut host = Recorder {
            text: "prompt$ ".to_string(),
            ..Recorder::default()
        };
        engine.emit(&mut host, &Event::CommandFinished { row: 7, error: true });
        assert_eq!(host.sent, ["prompt$ 7:true\n"]);
    }

    #[test]
    fn a_sibling_module_is_imported_from_the_extension_root() {
        let (_temp, dir) = extension(
            r#"
            import { shout } from "./lib/util.js";
            winter.commands.add("shout", "Shout", () => winter.app.log(shout("ok")));
            "#,
            &[(
                "lib/util.js",
                "export function shout(text) { return text.toUpperCase(); }\n",
            )],
        );
        let (mut engine, _logs) = load(&dir);
        let mut host = Recorder::default();
        assert!(engine.run(&mut host, "test.fixture:shout"));
        assert_eq!(engine.take_logs(), ["test.fixture: OK"]);
    }

    #[test]
    fn an_import_of_a_file_the_extension_does_not_ship_is_refused() {
        let (_temp, dir) = extension(
            r#"
            import { value } from "./util.js";
            winter.app.log(String(value));
            "#,
            &[("lib/util.js", "export const value = 1;\n")],
        );
        // The entry is at the root, so `./util.js` means the root's file, and
        // the only `util.js` is in `lib/`. The error says what it did ship.
        let report = discover(&Options {
            roots: vec![dir.to_path_buf()],
            disabled: Vec::new(),
        });
        let mut engine = Engine::default();
        let error = engine.load(&report.loaded[0]).expect_err("the root has no util.js");
        assert!(error.message.contains("does not ship"), "{}", error.message);
    }

    #[test]
    fn a_nested_import_of_a_sibling_resolves() {
        let (_temp, dir) = extension(
            r#"
            import { value } from "./lib/entry.js";
            winter.app.log(String(value));
            "#,
            &[
                (
                    "lib/entry.js",
                    "import { value } from \"./util.js\";\nexport { value };\n",
                ),
                ("lib/util.js", "export const value = 1;\n"),
            ],
        );
        // `./util.js` from `lib/entry.js` is `lib/util.js`, not the root's.
        let (_engine, logs) = load(&dir);
        assert_eq!(logs, ["test.fixture: 1"]);
    }

    #[test]
    fn two_importers_of_a_sibling_share_one_record() {
        let (_temp, dir) = extension(
            r#"
            import { a } from "./lib/a.js";
            import { b } from "./lib/b.js";
            winter.app.log(String(a + b));
            "#,
            &[
                (
                    "lib/util.js",
                    "winter.app.log(\"evaluated\");\nexport const value = 1;\n",
                ),
                (
                    "lib/a.js",
                    "import { value } from \"./util.js\";\nexport const a = value;\n",
                ),
                (
                    "lib/b.js",
                    "import { value } from \"./util.js\";\nexport const b = value;\n",
                ),
            ],
        );
        // Both importers resolve `./util.js` to the same key, so the module is
        // evaluated once and its side effect is seen once.
        let (_engine, logs) = load(&dir);
        let evaluated = logs.iter().filter(|line| line.ends_with("evaluated")).count();
        assert_eq!(evaluated, 1, "{logs:?}");
        assert!(logs.iter().any(|line| line == "test.fixture: 2"), "{logs:?}");
    }

    #[test]
    fn a_specifier_that_climbs_out_of_the_root_is_refused() {
        let (_temp, dir) = extension(
            r#"
            import { value } from "./lib/a.js";
            winter.app.log(String(value));
            "#,
            &[(
                "lib/a.js",
                "import { value } from \"../../outside.js\";\nexport { value };\n",
            )],
        );
        let report = discover(&Options {
            roots: vec![dir.to_path_buf()],
            disabled: Vec::new(),
        });
        let mut engine = Engine::default();
        let error = engine
            .load(&report.loaded[0])
            .expect_err("two levels up from lib/ leaves the root");
        assert!(error.message.contains("climbs out"), "{}", error.message);
    }

    #[test]
    fn a_bare_specifier_is_refused_with_the_rule() {
        let (_temp, dir) = extension(
            r#"
            import { value } from "winter";
            winter.app.log(String(value));
            "#,
            &[],
        );
        let report = discover(&Options {
            roots: vec![dir.to_path_buf()],
            disabled: Vec::new(),
        });
        let mut engine = Engine::default();
        let error = engine.load(&report.loaded[0]).expect_err("bare");
        assert!(error.message.contains("bare specifier"), "{}", error.message);
    }

    #[test]
    fn an_import_of_the_entry_is_refused() {
        let (_temp, dir) = extension(
            r#"
            import { value } from "./lib/a.js";
            winter.app.log(String(value));
            "#,
            &[("lib/a.js", "import { value } from \"../main.js\";\nexport { value };\n")],
        );
        let report = discover(&Options {
            roots: vec![dir.to_path_buf()],
            disabled: Vec::new(),
        });
        let mut engine = Engine::default();
        let error = engine.load(&report.loaded[0]).expect_err("the entry is not a source");
        assert!(error.message.contains("entry cannot be imported"), "{}", error.message);
    }

    #[test]
    fn a_top_level_await_that_settles_loads() {
        let (_temp, dir) = extension(
            r#"
            const ready = await Promise.resolve(41);
            winter.commands.add("answer", "Answer", () => winter.app.log(String(ready + 1)));
            "#,
            &[],
        );
        let (mut engine, _logs) = load(&dir);
        assert_eq!(engine.commands().len(), 1);
        let mut host = Recorder::default();
        engine.run(&mut host, "test.fixture:answer");
        assert_eq!(engine.take_logs(), ["test.fixture: 42"]);
    }

    #[test]
    fn a_throwing_handler_is_logged_and_the_next_extension_still_runs() {
        let temp = tempfile::tempdir().expect("tempdir");
        for (dir, id, body) in [
            ("a.throws", "a.throws", r#"throw new Error("boom");"#),
            (
                "b.logs",
                "b.logs",
                r#"winter.events.on("settingsReloaded", () => winter.app.log("alive"));"#,
            ),
        ] {
            let path = temp.path().join(dir);
            std::fs::create_dir_all(&path).expect("dir");
            write(
                &path,
                "extension.toml",
                &format!("id = \"{id}\"\nname = \"{id}\"\nversion = \"1\"\napi = 1\nentry = \"main.js\"\n"),
            );
            write(&path, "main.js", body);
        }
        let report = discover(&Options {
            roots: vec![temp.path().to_path_buf()],
            disabled: Vec::new(),
        });
        assert_eq!(report.loaded.len(), 2);
        let mut engine = Engine::default();
        let error = engine.load(&report.loaded[0]).expect_err("the entry throws");
        assert!(!error.message.is_empty());
        engine.load(&report.loaded[1]).expect("the second extension loads");
        engine.freeze();

        let mut host = Recorder::default();
        engine.emit(&mut host, &Event::SettingsReloaded);
        let logs = engine.take_logs();
        assert!(logs.iter().any(|line| line == "b.logs: alive"), "{logs:?}");
    }

    #[test]
    fn a_throw_inside_a_handler_is_caught_for_that_handler_only() {
        let (_temp, dir) = extension(
            r#"
            winter.events.on("paneFocused", () => { throw new Error("nope"); });
            winter.events.on("paneFocused", () => winter.app.log("second"));
            "#,
            &[],
        );
        let (mut engine, _logs) = load(&dir);
        let mut host = Recorder::default();
        engine.emit(&mut host, &Event::PaneFocused);
        let logs = engine.take_logs();
        assert!(logs.iter().any(|line| line.contains("nope")), "{logs:?}");
        assert!(logs.iter().any(|line| line.ends_with(": second")), "{logs:?}");
    }

    #[test]
    fn registration_closes_after_freeze() {
        let (_temp, dir) = extension(
            r#"
            winter.events.on("settingsReloaded", () => winter.commands.add("late", "Late", () => {}));
            "#,
            &[],
        );
        let (mut engine, _logs) = load(&dir);
        assert!(engine.commands().is_empty());
        let mut host = Recorder::default();
        engine.emit(&mut host, &Event::SettingsReloaded);
        let logs = engine.take_logs();
        assert!(
            logs.iter().any(|line| line.contains("registration is closed")),
            "{logs:?}"
        );
        assert!(engine.commands().is_empty());
    }

    #[test]
    fn a_duplicate_command_id_fails_the_extension() {
        let (_temp, dir) = extension(
            r#"
            winter.commands.add("same", "One", () => {});
            winter.commands.add("same", "Two", () => {});
            "#,
            &[],
        );
        let report = discover(&Options {
            roots: vec![dir.to_path_buf()],
            disabled: Vec::new(),
        });
        let mut engine = Engine::default();
        let error = engine.load(&report.loaded[0]).expect_err("duplicate id");
        assert!(error.message.contains("already registered"), "{}", error.message);
    }

    #[test]
    fn console_output_carries_the_extension_id() {
        let (_temp, dir) = extension(r#"console.log("hello");"#, &[]);
        let (_engine, logs) = load(&dir);
        assert!(logs.iter().any(|line| line == "test.fixture: hello"), "{logs:?}");
    }

    #[test]
    fn the_clipboard_the_tab_strip_and_the_url_opener_are_writes() {
        let (_temp, dir) = extension(
            r#"
            winter.commands.add("all", "All", () => {
              winter.app.clipboard.write("copied");
              winter.tabs.select(2);
              winter.app.openUrl("https://example.com");
            });
            "#,
            &[],
        );
        let (mut engine, _logs) = load(&dir);
        let mut host = Recorder::default();
        assert!(engine.run(&mut host, "test.fixture:all"));
        assert_eq!(host.copied, ["copied"]);
        assert_eq!(host.selected, [2]);
        assert_eq!(host.opened, ["https://example.com"]);
    }

    #[test]
    fn one_extension_runs_another_extension_s_command() {
        let temp = tempfile::tempdir().expect("tempdir");
        for (id, body) in [
            (
                "a.caller",
                r#"winter.commands.add("call", "Call", () => winter.commands.run("b.callee:work"));"#,
            ),
            (
                "b.callee",
                r#"winter.commands.add("work", "Work", () => winter.app.log("ran"));"#,
            ),
        ] {
            let path = temp.path().join(id);
            std::fs::create_dir_all(&path).expect("dir");
            write(
                &path,
                "extension.toml",
                &format!("id = \"{id}\"\nname = \"{id}\"\nversion = \"1\"\napi = 1\nentry = \"main.js\"\n"),
            );
            write(&path, "main.js", body);
        }
        let report = discover(&Options {
            roots: vec![temp.path().to_path_buf()],
            disabled: Vec::new(),
        });
        let mut engine = Engine::default();
        for loaded in &report.loaded {
            engine.load(loaded).expect("load");
        }
        engine.freeze();
        let _ = engine.take_logs();

        let mut host = Recorder::default();
        assert!(engine.run(&mut host, "a.caller:call"));
        assert_eq!(engine.take_logs(), ["b.callee: ran"]);
    }

    #[test]
    fn an_undeclared_asset_is_refused_by_name() {
        let (_temp, dir) = extension(
            r#"
            winter.events.on("settingsReloaded", () => winter.app.log(winter.assets.get("nope")));
            "#,
            &[],
        );
        let (mut engine, _logs) = load(&dir);
        let mut host = Recorder::default();
        engine.emit(&mut host, &Event::SettingsReloaded);
        let logs = engine.take_logs();
        assert!(logs.iter().any(|line| line.contains("is not declared")), "{logs:?}");
    }

    #[test]
    fn the_identity_and_api_are_visible_to_the_extension() {
        let (_temp, dir) = extension(
            r#"
            winter.app.log(winter.extension.id + "@" + winter.extension.version + " api " + winter.api);
            "#,
            &[],
        );
        let (_engine, logs) = load(&dir);
        assert!(
            logs.iter().any(|line| line.ends_with("test.fixture@1 api 1")),
            "{logs:?}"
        );
    }

    /// The wasm fixture's source, compiled by the tests that need its bytes.
    const WASM_FIXTURE: &str = include_str!("../fixtures/failed-command-wasm/extension.wat");

    /// An extension whose only half is the module in `wasm`.
    fn wasm_extension(wasm: &[u8]) -> (tempfile::TempDir, PathBuf) {
        let temp = tempfile::tempdir().expect("tempdir");
        let dir = temp.path().join("wasm");
        std::fs::create_dir_all(&dir).expect("dir");
        write(
            &dir,
            "extension.toml",
            "id = \"test.wasm\"\nname = \"Wasm\"\nversion = \"1\"\napi = 1\nmodule = \"extension.wasm\"\n",
        );
        std::fs::write(dir.join("extension.wasm"), wasm).expect("wasm");
        (temp, dir)
    }

    fn load_wasm(wasm: &[u8]) -> Result<(Engine, Vec<String>), Error> {
        let (_temp, dir) = wasm_extension(wasm);
        let report = discover(&Options {
            roots: vec![dir.clone()],
            disabled: Vec::new(),
        });
        assert_eq!(report.loaded.len(), 1, "{:?}", report.skipped);
        let mut engine = Engine::default();
        engine.load(&report.loaded[0])?;
        engine.freeze();
        let logs = engine.take_logs();
        Ok((engine, logs))
    }

    #[test]
    fn a_wasm_only_extension_logs_and_registers_its_command() {
        let wasm = wat::parse_str(WASM_FIXTURE).expect("the fixture assembles");
        let (mut engine, logs) = load_wasm(&wasm).expect("the module loads");
        assert!(logs.iter().any(|line| line == "test.wasm: hello from wasm"), "{logs:?}");
        // The module wrote UTF-8; the host decoded it.
        assert!(logs.iter().any(|line| line == "test.wasm: café ✓"), "{logs:?}");
        // Nothing has been dispatched yet, so the snapshot the module read is
        // `null`, which is how it tells `winter_init` apart from a callback.
        assert!(logs.iter().any(|line| line == "test.wasm: null"), "{logs:?}");

        let commands = engine.commands();
        assert_eq!(commands.len(), 1);
        assert_eq!(commands[0].key, "test.wasm:hello-wasm");
        assert_eq!(commands[0].label, "Hello from wasm");

        let mut host = Recorder {
            focused: Some(PaneInfo {
                id: "7".to_string(),
                title: "shell".to_string(),
                program: "zsh".to_string(),
                columns: 80,
                rows: 24,
            }),
            ..Recorder::default()
        };
        assert!(engine.run(&mut host, "test.wasm:hello-wasm"));
        let logs = engine.take_logs();
        assert!(
            logs.iter().any(|line| line == "test.wasm: A wasm command ran"),
            "{logs:?}"
        );
        // The typed accessor answered with the pane the host put in the snapshot.
        assert!(logs.iter().any(|line| line == "test.wasm: 7"), "{logs:?}");
    }

    #[test]
    fn the_event_payload_and_the_snapshot_cross_the_boundary() {
        let wasm = wat::parse_str(WASM_FIXTURE).expect("the fixture assembles");
        let (mut engine, _logs) = load_wasm(&wasm).expect("the module loads");

        let mut host = Recorder {
            focused: Some(PaneInfo {
                id: "7".to_string(),
                title: "shell".to_string(),
                program: "zsh".to_string(),
                columns: 80,
                rows: 24,
            }),
            ..Recorder::default()
        };
        engine.emit(&mut host, &Event::CommandFinished { row: 1, error: true });
        // The module logged the bytes the host wrote into its memory. The key
        // order is serde_json's, so match the fields rather than the text.
        let logs = engine.take_logs();
        assert!(
            logs.iter().any(|line| line.starts_with("test.wasm: {")
                && line.contains("\"row\":1")
                && line.contains("\"error\":true")),
            "{logs:?}"
        );
        // And it read the same picture a script handler is handed: the head of
        // the snapshot, with the pane the host put in it.
        assert!(
            logs.iter()
                .any(|line| line.starts_with("test.wasm: {\"focused\":{") && line.contains("\"columns\":80")),
            "{logs:?}"
        );
        assert_eq!(
            host.notified,
            [("Wasm fixture".to_string(), "a command failed".to_string())]
        );
    }

    #[test]
    fn a_compiled_module_reads_its_asset_bytes() {
        let wasm = wat::parse_str(WASM_FIXTURE).expect("the fixture assembles");
        let temp = tempfile::tempdir().expect("tempdir");
        let dir = temp.path().join("with-asset");
        std::fs::create_dir_all(&dir).expect("dir");
        write(
            &dir,
            "extension.toml",
            "id = \"test.asset\"\nname = \"Asset\"\nversion = \"1\"\napi = 1\nmodule = \"extension.wasm\"\n\n[assets]\ngreeting = \"greeting.txt\"\n",
        );
        std::fs::write(dir.join("extension.wasm"), &wasm).expect("wasm");
        std::fs::write(dir.join("greeting.txt"), "hello from an asset\n").expect("asset");

        let report = discover(&Options {
            roots: vec![dir],
            disabled: Vec::new(),
        });
        assert_eq!(report.loaded.len(), 1, "{:?}", report.skipped);
        let mut engine = Engine::default();
        engine.load(&report.loaded[0]).expect("the module loads");
        engine.freeze();
        let logs = engine.take_logs();
        assert!(
            logs.iter().any(|line| line == "test.asset: hello from an asset"),
            "{logs:?}"
        );
    }

    #[test]
    fn a_module_that_imports_what_the_host_does_not_grant_is_refused() {
        let wasm = wat::parse_str(
            r#"(module
                 (import "winter" "now" (func $now (result f64)))
                 (memory (export "memory") 1)
                 (func (export "winter_abi") (result i32) (i32.const 1)))"#,
        )
        .expect("assembles");
        let error = load_wasm(&wasm).err().expect("an ungranted import must be refused");
        assert!(error.message.contains("winter.now"), "{}", error.message);
    }

    #[test]
    fn a_module_with_another_abi_is_refused() {
        let wasm = wat::parse_str(
            r#"(module
                 (memory (export "memory") 1)
                 (func (export "winter_abi") (result i32) (i32.const 2)))"#,
        )
        .expect("assembles");
        let error = load_wasm(&wasm).err().expect("another ABI must be refused");
        assert!(error.message.contains("ABI 2"), "{}", error.message);
    }

    /// Writes `extension.wasm` beside the fixture's source after an edit to the
    /// WAT. Ignored: it writes into the tree, so it is a deliberate step.
    #[test]
    #[ignore = "writes the fixture's compiled module"]
    fn regenerate_the_wasm_fixture() {
        let bytes = wat::parse_str(WASM_FIXTURE).expect("the fixture assembles");
        let path = Path::new(env!("CARGO_MANIFEST_DIR")).join("fixtures/failed-command-wasm/extension.wasm");
        std::fs::write(&path, bytes).expect("write");
    }

    #[test]
    fn the_shipped_fixtures_load() {
        let fixtures = Path::new(env!("CARGO_MANIFEST_DIR")).join("fixtures");
        let report = discover(&Options {
            roots: vec![fixtures],
            disabled: Vec::new(),
        });
        assert_eq!(report.loaded.len(), 2, "{:?}", report.skipped);
        let mut engine = Engine::default();
        for loaded in &report.loaded {
            engine.load(loaded).expect("a fixture loads");
        }
        engine.freeze();
        let mut keys: Vec<String> = engine.commands().into_iter().map(|command| command.key).collect();
        keys.sort();
        assert_eq!(
            keys,
            [
                "winter.failed-command:where-am-i".to_string(),
                "winter.wasm-fixture:hello-wasm".to_string()
            ]
        );
        // The script fixture logs its asset's bytes at load, so the shipped
        // example proves that path too.
        let logs = engine.take_logs();
        assert!(
            logs.iter()
                .any(|line| line == "winter.failed-command: asset: hello from a script asset"),
            "{logs:?}"
        );
    }

    #[test]
    fn a_handler_reads_the_scrollback_it_asks_for() {
        let (_temp, dir) = extension(
            r#"
            winter.events.on("settingsReloaded", () => {
              winter.app.log("history: " + winter.panes.history(2).split("\n").join("|"));
            });
            "#,
            &[],
        );
        let (mut engine, _logs) = load(&dir);
        let mut host = Recorder {
            history: "one\ntwo\nthree\nfour".to_string(),
            ..Recorder::default()
        };
        engine.emit(&mut host, &Event::SettingsReloaded);
        let logs = engine.take_logs();
        assert!(
            logs.iter().any(|line| line == "test.fixture: history: three|four"),
            "{logs:?}"
        );
    }

    #[test]
    fn a_handler_that_asks_for_more_history_than_the_snapshot_holds_gets_what_it_has() {
        let (_temp, dir) = extension(
            r#"
            winter.events.on("settingsReloaded", () => {
              winter.app.log("history: [" + winter.panes.history(500) + "]");
            });
            "#,
            &[],
        );
        let (mut engine, _logs) = load(&dir);
        let mut host = Recorder {
            history: "only".to_string(),
            ..Recorder::default()
        };
        engine.emit(&mut host, &Event::SettingsReloaded);
        let logs = engine.take_logs();
        assert!(
            logs.iter().any(|line| line == "test.fixture: history: [only]"),
            "{logs:?}"
        );
    }

    #[test]
    fn a_handler_can_ask_for_more_history_than_the_snapshot_holds() {
        let (_temp, dir) = extension(
            r#"
            winter.events.on("settingsReloaded", () => {
              winter.panes.history(500, (text) => {
                const rows = text === "" ? [] : text.split("\n");
                winter.app.log("rows: " + rows.length);
                winter.app.notify("History", "continued");
              });
            });
            "#,
            &[],
        );
        let (mut engine, _logs) = load(&dir);

        // More rows than the snapshot gathers, so the answer is a continuation
        // the host serves after the handler returns.
        let long: Vec<String> = (0..300).map(|row| format!("row {row}")).collect();
        let mut host = Recorder {
            history: long.join("\n"),
            ..Recorder::default()
        };
        engine.emit(&mut host, &Event::SettingsReloaded);

        let logs = engine.take_logs();
        assert!(logs.iter().any(|line| line == "test.fixture: rows: 300"), "{logs:?}");
        // And what the continuation queued reached the shell like any other call.
        assert_eq!(host.notified, [("History".to_string(), "continued".to_string())]);
    }

    #[test]
    fn a_history_continuation_may_ask_again() {
        let (_temp, dir) = extension(
            r#"
            winter.events.on("settingsReloaded", () => {
              winter.panes.history(500, () => {
                winter.panes.history(600, (text) => {
                  winter.app.log("second: " + text.split("\n").length);
                });
              });
            });
            "#,
            &[],
        );
        let (mut engine, _logs) = load(&dir);
        let long: Vec<String> = (0..300).map(|row| format!("row {row}")).collect();
        let mut host = Recorder {
            history: long.join("\n"),
            ..Recorder::default()
        };
        engine.emit(&mut host, &Event::SettingsReloaded);
        let logs = engine.take_logs();
        assert!(logs.iter().any(|line| line == "test.fixture: second: 300"), "{logs:?}");
    }

    #[test]
    fn the_dispatcher_is_not_writable_and_not_enumerable() {
        let (_temp, dir) = extension(
            r#"
            const descriptor = Object.getOwnPropertyDescriptor(winter, "__dispatch");
            winter.app.log("hidden:" + descriptor.enumerable + ":" + descriptor.writable);
            "#,
            &[],
        );
        let (mut engine, logs) = load(&dir);
        assert!(
            logs.iter().any(|line| line.ends_with(": hidden:false:false")),
            "{logs:?}"
        );
        let mut host = Recorder::default();
        engine.emit(&mut host, &Event::PaneFocused);
    }
}
