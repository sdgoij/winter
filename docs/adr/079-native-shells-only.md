# ADR 079 — The native shells are the only shells

- Status: Accepted
- Date: 2026-11-24
- Related: `crates/winter/src/lib.rs`, `crates/winter/Cargo.toml`,
  `.github/workflows/ci.yml`; ADRs 018, 038, 040, 072, 075

## Context

ADR 018 made egui the first shell, and ADR 038 split it into `app`, `config`,
`terminal_widget`, `window` and `icon` so a native shell could replace it
module by module. The replacements arrived: WinUI 3 on Windows (ADR 040),
AppKit on macOS (ADR 072) and GTK 4 on Linux (ADR 075). egui stayed as the
fallback behind the `gtk` feature, so every build still carried `eframe`,
`winit` and `wgpu`, and the shell's rules — chord matching, the palette, the
settings window, the find bar, the tab strip — existed twice.

By the time the GTK shell shipped, the fallback was the only shell on no
platform: three native shells, one drawing imitation kept to prove the crate
still compiled without GTK.

## Decision

1. **egui and `eframe` leave the application.** `app.rs`, `layout.rs`,
   `terminal_widget.rs`, `window.rs` and `blink.rs` are deleted, `eframe` is
   dropped from the manifest and the lock, and the crate no longer names a
   widget toolkit outside a shell.
2. **The pieces the shells still share survive as toolkit-free modules.**
   `fonts.rs` takes `font_directories` out of the deleted `app.rs`, and
   `config.rs` keeps the settings draft (`Draft`, `ProfileDraft`,
   `BindingDraft`, `SchemeDraft`, `ThemeDraft`, `written`, `read`,
   `status_text`) that WinUI's editor is built on. Both are public API.
3. **The `gtk` feature is removed.** Linux always builds the GTK shell, so
   `gtk4`, `glow`, `libloading` and `bytemuck` are ordinary dependencies of the
   Linux build; `--no-default-features` no longer builds a GUI, and CI's
   "Check the egui fallback" step goes with the feature.
4. **The crate builds for the three platforms that have a shell.** On any
   other target `main` is a `compile_error!`, so a new platform starts by
   bringing a shell rather than by compiling a fallback nobody maintains.
5. **The records keep their history.** ADR 018 and ADR 038 are superseded, not
   rewritten; this ADR is why the modules they describe are gone.

## Rationale

- **One implementation per rule.** The chords, palette, tab strip, find bar and
  settings window now exist once each, in the shell of the platform, over
  shared state (`tabs`, `find`, `config`, `input`, `view`).
- **The dependency tree shrinks on every platform.** No `winit`, no `wgpu`
  render path, no second font stack, and the lock loses a family of crates that
  only the fallback used.
- **The fallback had stopped being a fallback.** It was not the shipped shell
  on any platform, so a build that only it kept alive tested nothing a user
  would run.

## Consequences

Positive:

- The gate is smaller and faster, and a `cfg` that only guarded the fallback
  is gone from `lib.rs`, `main.rs`, `find.rs` and `terminal_render.rs`.
- `config` and `fonts` are public, toolkit-free, and ready to serve the shells
  from a separate crate.
- 47 tests went with the egui modules: Linux now runs 154 of the crate's tests
  and Windows 186, all of them over code a shell uses.

Negative:

- **A toolkit is required to build the application.** Linux needs GTK 4.10
  headers; there is no headers-free build any more, which the README and
  `CONTRIBUTING.md` now say plainly.
- **The crate's boundaries are untested by a second consumer.** ADR 038's split
  was held honest by a shell compiled on every platform; the three shells do
  that job now, but only where they are built.
- **The deleted tests were real.** They covered chord matching, the palette and
  the settings draft through egui; the draft's `status_text` tests survive, the
  rest are covered by each shell's own tests or not at all.

## Alternatives considered

| Alternative | Why not |
|---|---|
| Keep egui behind a feature nobody builds | A second implementation that no user runs, tested only by the gate |
| Keep egui on an unshipped platform (BSD) | It would be the only shell there, unbuilt and untested in CI |
| Keep only the egui tests, without the crate | The tests drive `App`, which is the shell itself |
| Delete the shared `config` draft with the window | WinUI's editor is built on it, and the file it writes is the same document |
