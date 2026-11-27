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
winter.panes.history(200)     // rows above it, as far back as the host gathered
winter.panes.history(5000, (text) => { ... });   // the whole scrollback, continued
winter.panes.send("ls\n")     // into the focused pty
winter.tabs.list()            // [{ index, title, active, panes }]
winter.tabs.select(2)

winter.app.notify("Winter", "done");   // a desktop notification
winter.app.clipboard.write(text);
winter.app.openUrl("https://...");
winter.app.log("...");                 // prefixed with the extension id
winter.assets.get("icon")              // the opaque name of a declared asset
winter.assets.bytes("icon")            // its bytes, as a fresh Uint8Array
```

| Module | Surface |
|---|---|
| `winter.events` | `on(name, handler)` → unsubscribe; the names are the table below |
| `winter.commands` | `add(id, label, handler)` → a command palette row; `run(id)` |
| `winter.panes` | the focused pane's identity, text, scrollback and pty writes |
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

`extension.toml`, parsed strictly, in a directory with the manifest at its root.
TOML because the file is meant to be edited: it takes comments, and the settings
file beside it is TOML already. Unknown keys are errors, so a typo is reported
with its line rather than ignored.

```toml
# The fixture the live check runs.
id = "com.example.failed-command"
name = "Failed command"
version = "0.1.0"
api = 1
description = "Says when a command above the prompt exited non-zero."
author = "example"
loadAfter = ["com.example.gridkit"]
entry = "extension.js"
module = "extension.wasm"

[assets]
icon = "assets/icon.png"
```

An extension may have `entry`, `module`, both, or neither (a data-only extension
of assets). There is no script/module switch: `entry` is always an ES module,
and every other `.js` or `.json` file under the directory is importable beside
it, at any depth. Discovery, in order, is the union of: an explicit
directory (CLI or `$WINTER_EXTENSIONS`), the user's config directory, and
`extensions/` beside the executable or in the working directory. Enablement is
the settings file's `[extensions]` table: `enabled` turns the engine on at all,
and `disabled` lists the identifiers to skip. The table is read when the window
opens, so a change applies on the next start, and the extensions window writes
it. The loader validates ids and paths, caps the manifest,
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

A read that needs the whole buffer arrives as a continuation: `history(lines)`
answers from the snapshot up to what the host gathered, and `history(lines, cb)`
has the host walk for exactly `lines` rows and call `cb` before the dispatch ends.
The host serves the continuations after the handler returns, so the shell is still
never entered from inside a callback; a continuation that asks again is answered
again, up to a bounded number of rounds.

### The compiled half

A manifest may name a `module` beside or instead of an `entry`. The host reads the
bytes, hands them to the bootstrap as an `ArrayBuffer`, and the bootstrap builds
`WebAssembly.Module` and `WebAssembly.Instance` — so a compiled extension is
instantiated by the same realm, against the same registries, and frozen by the
same `freeze` as a script one. An extension may ship both halves.

**The import list is the API.** Every import must be a function from the `winter`
namespace in the table below; anything else is refused by name before
instantiation, so a module that wants a clock fails to load rather than being
policed at call time.

| Import | Signature | Meaning |
|---|---|---|
| `log` | `(ptr, len)` | a log line under the extension's id |
| `send` | `(ptr, len)` | text into the focused pty |
| `notify` | `(tptr, tlen, bptr, blen)` | a desktop notification |
| `clipboard_write` | `(ptr, len)` | the clipboard |
| `open_url` | `(ptr, len)` | the user's browser |
| `select_tab` | `(index)` | the tab strip |
| `register_command` | `(idptr, idlen, lptr, llen) -> i32` | a palette row; `0` accepted, `1` refused and logged |
| `register_event` | `(nameptr, namelen) -> i32` | a subscription; the same answer |
| `snapshot` | `(ptr, cap) -> i32` | the shell's one picture, as JSON; answers how many bytes it needs |
| `asset` | `(nptr, nlen, ptr, cap) -> i32` | a declared asset's bytes; `-1` when the name is not declared |
| `tabs_count`, `tabs_active` | `() -> i32` | how many tabs, and which is active (`-1` for none) |
| `tab_panes`, `tab_title` | `(index) -> i32`, `(index, ptr, cap) -> i32` | one tab's pane count and title; `-1` for no such tab |
| `pane_columns`, `pane_rows` | `() -> i32` | the focused pane's grid, or `0` |
| `pane_id`, `pane_title`, `pane_program` | `(ptr, cap) -> i32` | the focused pane's strings, or `-1` |
| `history` | `(ptr, cap) -> i32` | the gathered scrollback, newline-separated |

| Export | Signature | Meaning |
|---|---|---|
| `memory` | — | required: the host reads and writes it |
| `winter_abi` | `() -> i32` | required; must be `1`, the ABI this build implements |
| `winter_alloc` | `(len) -> i32` | required to receive a string; the module's own arena |
| `winter_init` | `() -> i32` | optional, called once; `0` is success. Registration goes here — a start function runs before the host owns the memory |
| `winter_command` | `(idptr, idlen) -> i32` | optional, called for each command the module registered |
| `winter_event` | `(nptr, nlen, pptr, plen) -> i32` | optional; the payload is the event's JSON |

Every string crosses as UTF-8 at a `(pointer, length)` pair into the module's own
memory: the module's strings arrive through the imports, and the host writes its
strings through `winter_alloc`. The host refuses a pointer outside the memory and
refuses invalid UTF-8, so a module cannot make it read out of bounds, and a
module that answers a non-zero status is reported like a throwing handler.

**A compiled module reads the same picture a script handler is handed.**
`snapshot` writes the JSON of `{ tabs, focused, text }` — the exact payload
`winter.panes`, `winter.tabs.list` and the event handlers see — and answers the
length it needs rather than the length it wrote, so a module asks with an empty
buffer, calls `winter_alloc(len)`, and asks again: the `snprintf` convention, and
no second record layout in the ABI. Before the first dispatch the snapshot is
`null`, which is how a module tells `winter_init` (no shell to read yet) from a
callback. The typed imports beside it are the same picture field by field, for a
module that would rather not carry a JSON parser: strings answer the bytes they
need, counts answer `0` or `-1` for "nothing there", and a tab or pane that does
not exist is `-1` rather than an empty answer.

**A compiled module reads its assets as bytes.** `asset` copies a declared
asset's bytes into the module's memory by the same convention, and answers `-1`
for a name the manifest does not declare, so an empty asset and a missing one
are different answers. The host reads the bytes once at load rather than inside a
callback, capped per asset and in total, so a file that grows or vanishes later
is an extension that does not load rather than a trap in a handler. A script
extension still sees only the opaque name from `winter.assets`: it has no use for
the bytes yet, and the name is what keeps a path — and so the filesystem — out of
extension code.

Failure isolation: `HostCallbacks` reports `console` output and unhandled
rejections against the extension's id; a throw inside a handler is caught by the
host and logged; a module that fails to instantiate fails that extension only.
Handlers run on the shell's thread, like the pty pump; the host is not `Send`, and
the API does not pretend a handler can block cheaply.

Registration freezes after every extension has been evaluated. A duplicate
command id is a load failure for the later extension, not a shadow. A command may
run another extension's command; the host keeps a small depth guard and logs a
cycle rather than recursing. This is the same shape as Goat's console.

## Commands in the palette, and on a chord

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
arm and the host lookup.

A command can also be bound to a chord, in the settings file rather than in the
command's own manifest:

```toml
[[extensions.commands]]
keys = "ctrl+g"
command = "winter.failed-command:where-am-i"
```

The window's own bindings are checked first, so an extension cannot shadow a chord
the window already runs, and the first matching entry in the file wins. The
settings screen's action vocabulary stays strict: this is a table of its own, not
a new `action` value, so a typo inside it is an error and `Action` stays a plain
copyable enum.

## Safety

- **No filesystem and no network.** Slag's `fs` feature stays off; the host reads
  an extension's manifest, sources, wasm module and assets and hands back opaque
  names — and, to a compiled module, the asset bytes it asked for by name.
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
- **Enablement is remembered**, in the settings file's `[extensions]` table
  rather than in a session: `enabled` and `disabled` are read when the window
  opens, and the extensions window edits them.

## What is here

1. `crates/extensions` — `manifest` and `discover`: parsing, caps, enablement
   and ordering, pure Rust and tested. No engine, no window.
2. Its `host` feature — the `winter` global, the module loader (registration,
   specifier validation), event and command dispatch, and a headless suite
   driving multi-file fixture extensions through Slag.
3. macOS — `Host for Shell`, the palette arm, `commandFinished`,
   `titleChanged`, `paneFocused`, `tabOpened`, `tabClosed` and
   `settingsReloaded`, with the fixtures in `crates/extensions/fixtures`: a
   script notifier and a compiled one.
4. The extensions window (`⌘⇧E`): every extension on disk with a switch, the
   master switch that turns the engine on, and the reason a row is not running.
   Saving writes the `[extensions]` table; loading changes on the next start.

What is not here: the GTK and WinUI `Host`s and their screens, which are
per-shell work, and a continuation form for a compiled module's history — it
reads the bounded walk through its typed accessor instead. ADR 086 records the
surface; `cargo test -p extensions --features host` and
`cargo test -p winter-cocoa` are the suites.

## Open questions

- **A live history walk for a compiled module.** A script continues through a
  callback the host calls after the handler; a module would need an export the
  host could call with a pointer, which is a second protocol for a read that
  `history(ptr, cap)` already covers up to the gathered bound.
- **Namespace names.** `winter.panes` / `winter.tabs` / `winter.app` are the
  names; `winter.terminal` or `winter.ui` would have read better for the first,
  and renaming one is a new `api` major rather than an edit.
- **Root-relative specifiers.** The engine resolves static imports through one
  map with no referrer, so an extension says `./lib/util.js` from anywhere and
  the loader refuses an import whose sibling reading would differ. File-relative
  imports need one of two things: a per-referrer resolution hook in Slag, or a
  loader that rewrites each specifier before compiling —
  `Module::requested_modules` already reports each specifier's offset, so the
  rewrite is possible, but it puts a source transformation in the loader's trust
  path. The hook is specified for the engine in
  `docs/slag-static-import-resolution.md`; the loader above is what changes when
  it lands.
