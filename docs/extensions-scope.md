# Extensions — scope and shape

Status: the loader (`crates/extensions`: `manifest`, `discover`) and the macOS
host (the `host` feature and the AppKit `Host`) are implemented, behind
`WINTER_EXT=1`; the WASM half, the extensions screen and the GTK and WinUI hosts
are not. This records the exploration of the Goat and Noctis systems and the
surface this repository offers; ADR 086 is the decision.
Related: ADRs 038 and 080 (the shell split), ADR 082 (the embedding surface), ADR
083 (the frame server) and ADR 084 (the Node addon) for the host boundary;
`docs/plan.md`'s parked chaos layer for the "bugs", which are a separate surface
rather than extensions.

## The runtime: Slag, JavaScript and WASM

Extensions are **JavaScript and WebAssembly only**, both executed by
[Slag](https://github.com/sdgoij/slag): one `slag::Context` per extension, each
with its own realm and its own `winter` global, and the engine's own
`WebAssembly` surface for the compiled half. There is no compile-time Rust
extension path and no second wasm runtime. A JavaScript extension's entry is an
**ES module**, compiled and evaluated in that realm, so its top-level bindings
are private by construction and an extension can split itself across files.

That choice is the one both reference projects made:

- [Goats](https://github.com/sdgoij/goats) loads **mods**: a `mod.json` beside a
  JavaScript entry, discovered from a directory or a zip, sorted by id and
  `loadAfter`, evaluated one per wrapper, and talking to the scene through a
  single `goats` global. Worth keeping:
  - `goats.on(event, handler)` returns an unsubscribe, and a handler's hooks are
    dropped with its mod; `goats.command(name, handler)` extends the console.
  - Registration is open while the mods load and closed by `goats.freeze()`.
  - A throwing handler is logged against its mod and does not stop the frame or
    another mod's handlers.
  - `goats.api` is one integer, additive within a major.
  - Assets are opaque names read by the Rust host; JavaScript never sees a path.
  - `side`/compatibility digest and the tuning tree are Goat-specific and do not
    carry over.
- [Noctis](https://github.com/sdgoij/noctis) adds the two ideas this repository
  needs most: **one context (realm) per extension**, so one extension cannot see
  another's globals, and **a managed edit path** for commands that change state.
  It also has a manifest, a user enablement file, `--watch` reload, and WASM as a
  default-deny sandbox whose import list *is* the API. Its docked panels and
  project files are editor concepts a terminal does not have.

Winter differs from Goats in one place: the entry is an ES module rather than a
wrapped script, so a multi-file extension needs neither a bundler nor a global
wrapper. Goats parked module syntax as a possible v2; Slag already has source
text module records, so the cost here is one flat resolver rather than a syntax
design.

## The ergonomic difference

The Goat and Noctis globals are flat bags of members. The Winter global is
**namespaced into modules**, so an extension's surface reads like an API rather
than like a dump of functions:

```js
import { summarize } from "./util.js";   // a sibling file, resolved flat

winter.events.on("commandFinished", (event) => { ... });
winter.commands.add("git.status", "Git status", () => { ... });

winter.panes.focused()        // { id, title, program, columns, rows }
winter.panes.text()           // the focused pane's visible screen
winter.panes.send("ls\n")     // into the focused pty
winter.tabs.list()            // [{ index, title, active, panes }]
winter.tabs.select(2)

winter.app.notify("Winter", "done");   // a desktop notification
winter.app.clipboard.write(text);
winter.app.openUrl("https://...");
winter.app.log("...");                 // prefixed with the extension id
```

| Module | Surface |
|---|---|
| `winter.events` | `on(name, handler)` → unsubscribe; the names are the table below |
| `winter.commands` | `add(id, label, handler)` → a command palette row; `run(id)` |
| `winter.panes` | the focused pane's identity, text and pty writes |
| `winter.tabs` | the tab list, selection, and open/close |
| `winter.app` | notifications, clipboard, URLs and logging |
| `winter.assets` | a manifest-declared asset's opaque name |
| `winter.extension` | this extension's `{ id, name, version }`; `winter.api` |

The names, the shapes and the events are the contract; each module is independent
so a new one is an additive change within an `api` major.

## What Winter already has to hook

| Source | Event |
|---|---|
| `Control::take_events` (`TerminalEvent`) | `Bell`, `notification`, `paneExited` |
| `Control::scroll_facts` / `MarkTick` (ADR 059) | `commandFinished { row, error }` when a new mark appears |
| `Control::title` | `titleChanged { title }` |
| The shell's tab/pane commands | `tabOpened`, `tabClosed`, `paneFocused`, `paneSplit` |
| `Shell::poll_settings` | `settingsReloaded` |

Events are **observations**: handlers run after the shell has acted, so a slow or
broken extension cannot change the terminal's own behaviour. The parked "bugs"
layer is the opposite — it wants input before the terminal sees it, timing,
rendering and the cursor — and must not share this surface; it gets a separate
`Mischief` trait and its own ADR so the core stays deterministic.

## The crate

One crate, `crates/extensions`, with the engine behind a feature:

```
crates/extensions/src/
  manifest.rs   the manifest type, strict parsing, the API major
  discover.rs   search path, caps, enablement, id and loadAfter order
  host/         slag::Context per extension, the winter modules, dispatch
                (feature = "host")
```

`manifest` and `discover` are pure Rust: they parse, validate, cap and order
without an engine and without a window, and that is where the unit tests live.
`host` is the only place Slag appears and it is behind the crate's `host`
feature, so a build that only needs the loader (a list command, the settings
screen, a test) links no JavaScript engine. Cargo features are additive, so two
shells enabling `host` still build Slag once.

The core `winter` crate never depends on `extensions`; the embedding surface (ADR
082), the Node addon (ADR 084) and a shell that hosts no extensions stay free of
a JavaScript engine. `crates/app` is what enables `host` and hands the loaded set
to the chosen shell.

The shell implements a small Rust `Host` trait, which is the *internal* half of
the contract (never exposed to extensions). The trait sits at the crate root,
outside the feature, so a shell can implement it without enabling the engine:

```rust
pub trait Host {
    fn tabs(&self) -> Vec<TabInfo>;
    fn focused(&self) -> Option<PaneInfo>;
    fn visible_text(&self) -> String;
    fn select_tab(&mut self, index: usize);
    fn send_text(&mut self, text: &str);
    fn copy(&mut self, text: &str);
    fn notify(&mut self, title: &str, body: &str);
    fn open_url(&mut self, url: &str) -> bool;
}
```

The host translates each `winter` member into one method; a shell that cannot do
something (no URL opener, no clipboard) answers `false` and the extension sees a
normal failure rather than a crash.

## The manifest and the loader

`extension.json`, parsed strictly, in a directory or a zip with the manifest at
its root:

```json
{
  "id": "com.example.failed-command",
  "name": "Failed command",
  "version": "0.1.0",
  "api": 1,
  "description": "Says when a command above the prompt exited non-zero.",
  "author": "example",
  "loadAfter": ["com.example.gridkit"],
  "entry": "extension.js",
  "module": "extension.wasm",
  "assets": { "icon": "assets/icon.png" }
}
```

An extension may have `entry`, `module`, both, or neither (a data-only extension
of assets). There is no script/module switch: `entry` is always an ES module,
and every other `.js` or `.json` file under the directory is importable beside
it, at any depth. Discovery, in order, is the union of: an explicit
directory (CLI or `$WINTER_EXTENSIONS`), the user's config directory, and
`extensions/` beside the executable or in the working directory. Enablement
lives in one user config (`disabled = ["com.example.noisy"]`), written by an
extensions screen later. The loader validates ids and paths, caps the manifest,
each JavaScript file and their total, the wasm module and the assets, and
refuses a path that escapes the extension's directory; a broken extension is
reported and skipped, never fatal.

Ordering is id with `loadAfter` honoured topologically. Loading is all-or-nothing
per extension.

## Loading and isolation

One `slag::Context` per extension. The host first defines `winter` in that realm
(its modules backed by host functions), then compiles the entry with
`slag::api::Module::compile` and evaluates it. Module scope already keeps the
extension's top-level declarations private, so there is no wrapper to write and
no globals to leak; a reload drops the whole realm.

Imports are **local and resolved from the extension's root**. Every `.js` and
`.json` file anywhere under the extension's directory, other than the entry, is
registered with the agent under its root path with a `./` prefix — `./lib/util.js`
— so a tree of any depth is ordinary and that prefix is the only spelling that
resolves. The engine resolves a static specifier through one flat map and has no
referrer to resolve against, so `./util.js` written in `lib/a.js` is the root's
`util.js`, not its sibling.

The loader makes that rule visible rather than silently surprising. Before
evaluating, it compiles every module once and checks each declared specifier
against the registered set: a miss names the file and the specifier, and a
specifier whose sibling reading would be a different file from its root reading
is refused as ambiguous instead of being resolved one way. So `lib/a.js`
importing `./util.js` beside its own `lib/util.js` is an error that says to write
`./lib/util.js`. Re-exports take the same path.

A bare specifier (`lodash`), a `file:` or `node:` URL, or a path that escapes the
directory fails that extension and never the shell. Dynamic `import()` answers
from the same map, so it is root-relative too. `import data from "./data.json"
with { type: "json" }` is the way to ship structured data — `.json` files
register as JSON modules — and binary assets stay opaque `winter.assets` names.

The identity is `winter.extension` (`{ id, name, version }`) plus `winter.api`,
not `import.meta`. Slag's `initialize_import_meta_object` hook hands the host the
engine's internal value types, which the embedding surface does not export, so a
host cannot fill `import.meta` from outside; if a typed hook arrives,
`import.meta.extension` is an additive addition rather than a rename.

Top-level `await` is legal. The host drains the job queue after evaluation and
installs no timers, so a module can only await promises that are already
settled; an evaluation still stalled after the drain is reported and that
extension is dropped. A stuck extension cannot block a shell task.

A wasm-only extension ships no JavaScript: the host instantiates its module
through the engine's `WebAssembly` surface with an import object built from the
same `winter` functions. The ABI is the JS-API object rather than a second
protocol to design, so the Rust half is one implementation, tested once, for
both languages.

Failure isolation: `HostCallbacks` reports `console` output and unhandled
rejections against the extension's id; a throw inside a handler is caught by the
host and logged; a module that fails to instantiate fails that extension only.
Handlers run on the shell's thread, like the pty pump; the host is not `Send`, and
the API does not pretend a handler can block cheaply.

Registration freezes after every extension has been evaluated. A duplicate
command id is a load failure for the later extension, not a shadow. A command may
run another extension's command; the host keeps a small depth guard and logs a
cycle rather than recursing. This is the same shape as Goat's console.

## Commands in the palette

Each extension command is a palette row. The shared `palette::Command` grows one
variant:

```rust
pub enum Command {
    Action(settings::Action),
    Profile(String),
    Extension(String),   // "<extension id>:<command id>"
}
```

The macOS and GTK shells already dispatch `Action`/`Profile`; each gains the third
arm and the host lookup. Commands carry no chord: the settings file's action
vocabulary is strict, and an extension chord would need a schema change or a
second file. The palette is the extension's command surface.

## Safety

- **No filesystem and no network.** Slag's `fs` feature stays off; the host reads
  an extension's manifest, sources, wasm module and assets and hands back opaque
  names.
  A path is never the extension's.
- **No ambient capabilities beyond `Host`.** The `winter` modules are the whole
  surface; a new capability is a deliberate host method with a test on every
  shell.
- **Trusted code, controlled surface.** Extensions are not sandboxed from the
  terminal (they run in its process); the I/O wall and the per-extension realm are
  the containment. WASM is the sandbox for the compiled half, because its import
  list is default-deny by construction.
- **Additive versioning.** `winter.api` is one integer. Within a major, new events
  and modules are additive; a name that changes meaning is a new major.
- **Enablement is remembered**, and persisted in the user config, not session-only.

## What is here

1. `crates/extensions` — `manifest` and `discover`: parsing, caps, enablement
   and ordering, pure Rust and tested. No engine, no window.
2. Its `host` feature — the `winter` global, the module loader (registration,
   specifier validation), event and command dispatch, and a headless suite
   driving multi-file fixture extensions through Slag.
3. macOS — `Host for Shell`, the palette arm, `commandFinished`,
   `titleChanged`, `paneFocused`, `tabOpened`, `tabClosed` and
   `settingsReloaded`, all behind `WINTER_EXT=1`, with the fixture in
   `crates/extensions/fixtures/failed-command`.

WASM-only extensions (a manifest with a `module` and no `entry`), the persisted
`disabled` list and an extensions screen, the GTK and WinUI `Host`s, and a
`winter.panes.history` walk are not here. ADR 086 records the surface;
`cargo test -p extensions --features host` and `cargo test -p winter-cocoa` are
the suites.

## Open questions

- **Slag pin.** Use the revision Goat pins
  (`9fee0baf9897ae5d5ae16bf119b8fd3b8fe806d8`), or a newer one? Slag's `unicode`
  build script reads its `test262` submodule, and a git dependency is not
  recursive, so the pin only builds if that checkout carries the submodule or the
  script can be made to skip it. Confirm with a throwaway `cargo check` against
  the revision before the manifest is agreed.
- **Root-relative specifiers.** The engine resolves static imports through one
  map with no referrer, so an extension says `./lib/util.js` from anywhere and
  the loader refuses an import whose sibling reading would differ. File-relative
  imports need one of two things: a per-referrer resolution hook in Slag, or a
  loader that rewrites each specifier before compiling —
  `Module::requested_modules` already reports each specifier's offset, so the
  rewrite is possible, but it puts a source transformation in the loader's trust
  path.
- **WASM shape.** Instantiate through the engine's JS `WebAssembly` surface with a
  JS-import object (one host implementation for both languages), or a raw
  memory/record ABI like Noctis's (`noctis_update(ptr, count, dt)`)? The former is
  simpler and keeps one vocabulary; the latter avoids JS glue per event.
- **Namespace names.** `winter.panes` / `winter.tabs` / `winter.app` are the
  proposal; `winter.terminal` or `winter.ui` may read better for the first.
- **Visible text only, or scrollback too?** `winter.panes.text()` starts with the
  visible screen (the accessibility value); `winter.panes.history(lines)` is a
  buffer walk and can come later.
