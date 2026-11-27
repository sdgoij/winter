# ADR 086 — Extensions are Slag JavaScript and WebAssembly, hosted by the shell

- Status: Accepted
- Date: 2026-11-27
- Related: `crates/extensions` (`manifest`, `discover`, `host`),
  `crates/cocoa/src/extension.rs`, `crates/winter/src/palette.rs`;
  `docs/extensions-scope.md`; ADRs 080, 082

## Context

Two projects in this family already run user scripts: Goats evaluates one
`mods/*.js` per mod against a flat `goats` global and closes registration with
`freeze()`; Noctis gives each extension its own realm and treats a WASM
module's import list as its whole API. Both are good ideas, and neither reads
like an API: a flat bag of functions is what they both grew into.

The engine is already here. Slag is a Rust JavaScript engine with a V8-shaped
module surface and a `WebAssembly` implementation, used by both projects, and
Winter had no reason to add a second runtime or a compile-time Rust extension
path that would tie an extension to this build.

## Decision

1. **JavaScript and WebAssembly only, executed by Slag, one context per
   extension.** Each extension gets its own `slag::Context`, so its realm, its
   `winter` global and its top-level bindings are its own. The engine is a
   pinned git dependency (`28ec2f6`), and `Cargo.lock` is the lock.
2. **The global is namespaced into modules**: `winter.events`, `winter.commands`,
   `winter.panes`, `winter.tabs`, `winter.app`, `winter.assets`, plus
   `winter.extension` and `winter.api`. A module is added or a method is added
   within an `api` major; a name that changes meaning is a new major.
3. **The entry is an ES module, and a compiled module may stand beside it.**
   Module scope is the scope, so there is no wrapper to write and no globals to
   leak. The other `.js` and `.json` files under the extension directory are
   registered with the agent under `./<root path>`. The engine resolves a static
   specifier through one flat map with no referrer, so a specifier is
   root-relative, not file-relative; the loader compiles every module once
   before evaluating, reports a miss against the file that wrote it, and refuses
   a spelling whose sibling reading would be a different file instead of picking
   one.
4. **A compiled extension's import list is its API.** A manifest may name a
   `module` instead of or beside an `entry`; the host instantiates it in the same
   realm with `WebAssembly`, grants only the function imports in the `winter`
   namespace, and refuses anything else by name before any code runs. Strings
   cross as UTF-8 at `(pointer, length)` pairs into the module's own exported
   memory; registration happens in `winter_init`, because a start function runs
   before the host owns the memory. The module is an actor and not an observer:
   it can act and register, and it does not read the shell yet.
5. **The loader owns the filesystem, the engine never touches it.** `manifest`
   and `discover` parse `extension.json` strictly, validate the identifier and
   paths, cap the manifest, each module and their total, canonicalise every path
   and refuse one that escapes the directory, and read the text. Slag's `fs`
   feature stays off.
6. **Events are observations.** A handler runs after the shell has acted, and
   the shell hands it one snapshot taken before the handler (`tabs`, `focused`,
   `visible_text`). A handler's writes — `panes.send`, `tabs.select`,
   `app.notify`, `app.clipboard.write`, `app.openUrl` — are queued and applied
   after it returns, so an extension cannot re-enter the shell mid-frame. The
   parked "bugs" layer is the opposite shape and stays a separate surface.
7. **The shell implements `Host`.** `crates/extensions` names no window, no
   pty and no workspace crate; the trait is the internal half of the contract,
   and its reads are the snapshot and its writes are applied by the engine.
8. **A command is a palette row.** `palette::Command::Extension("<id>:<cmd>")`
   is dispatched by the shell; commands carry no chord, because the settings
   file's action vocabulary is strict and the palette is the command's surface.
9. **Off by default, switched on from the extensions window.** Loading needs
   `[extensions] enabled = true` in the settings file or `WINTER_EXT` set to
   anything but `0`; `WINTER_EXT=0` overrides an enabled file for one run, and
   `WINTER_EXTENSIONS` adds a discovery root beside the config directory's
   `extensions/` and the ones beside the executable and the working directory.
   The window (`⌘⇧E`) lists what discovery finds — including an extension that
   is switched off or failed, with the reason — and writes `enabled` and
   `disabled`. It reads discovery rather than the engine, so it works with the
   engine off, and a change takes effect on the next start.

## Consequences

- An extension is trusted code in the terminal's process: the I/O wall and the
  realm are the containment, not a sandbox. WASM is the sandbox for the compiled
  half when that path lands.
- No timer and no I/O exists in the realm, so a top-level `await` can only
  await promises that are already settled. The host drains the job queue and
  drops an extension whose evaluation promise is still pending.
- A broken extension is reported and skipped. Loading one never stops another
  and never stops the window.
- `import.meta` is not filled: Slag's hook hands the host engine-internal value
  types that the embedding surface does not export. `winter.extension` carries
  the identity instead.
- A failed handler is logged with its extension's identifier and the other
  handlers still run; a duplicate command id fails the later extension rather
  than shadowing.
- WASM is a sandbox for the compiled half by construction: the import list is
the capability list, and a module that asks for more does not link. A module's
trap is reported like a throwing handler.
- GTK and WinUI compile against the new palette variant and do not host
  extensions; each needs its own `Host`, which is a per-shell change.

## Alternatives considered

| Alternative | Why not |
|---|---|
| A second wasm runtime beside Slag | Two engines to build, pin and secure for one surface |
| A compile-time Rust extension path | Ties an extension to this build and this ABI; the two reference projects both rejected it |
| One flat `winter` global like Goats and Noctis | It is what both projects grew away from; modules keep the surface readable and let one module change without renaming another's members |
| Script entries in a wrapper, as Goats does | Slag has source text module records; the wrapper was only there to recover the scope a module gives for free |
| File-relative import specifiers | The engine has no per-referrer resolution hook, so it would need a source rewrite in the loader's trust path; root-relative plus a load-time check is smaller and cannot corrupt code |
| A second crate for the host behind a feature on the loader | Two crates, one contract; a feature on one crate keeps the loader engine-free and the shells' build single |
| Applying writes inside a handler | A handler would re-enter the shell mid-frame, and the window's own rules assume one actor per frame |
| Noctis's raw record ABI for the compiled half (`noctis_update(ptr, count, dt)`) | It is a second protocol to specify, and it puts a per-frame crossing in the way of a surface whose work is registration and one-off actions. The import list and the four exports carry the same capabilities with no record layout in the ABI |
| Granting a prelude of JS bindings to a wasm module instead of imports | The import list would stop being the capability list; a module that wants something the host refuses could no longer fail to link |
