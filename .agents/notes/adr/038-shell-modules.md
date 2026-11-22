# ADR 038 — The egui shell is four modules, not one file

- Status: Accepted
- Date: 2026-11-22
- Related: `crates/winterm/src/{app,config,terminal_widget,window,icon}.rs`;
  ADRs 018, 019, 035, 036

## Context

ADR 018 recorded that egui is the first shell and that "replacing egui with a
native backend means replacing this file" — `ui.rs`. The file then grew to four
thousand lines holding five jobs: the window and its tabs, the viewport widget,
the settings editor, the font registration, and the window commands. Phase 9's
remaining rows name the pieces (`app`, `config`, `window`, `icon`,
`terminal_widget`, `terminal_renderer`, `settings`, `theme`), and the split is
also the one upstream uses: `App.cpp`/`AppHost.cpp` for the window,
`TermControl.cpp` for the widget, `TerminalSettingsEditor/` for the editor.

## Decision

- **`terminal_widget.rs`** is `TermControl`: `Pane`, its painting, and the only
  place egui events become `term` vocabulary. The viewport and selection stay
  here (ADR 019), over the `Control` that owns the terminal (ADR 036).
- **`app.rs`** is the application: `App`, `Tab`, the tab strip, the palette, the
  chord map, the font registration and `run`.
- **`config.rs`** is the settings window: the draft, its per-list editors, the
  editor's form, and the load/save/reload path. The `App` methods it owns are
  `impl App` blocks in this module.
- **`window.rs`** names the three viewport commands the shell asks for — a
  title, an attention request and a close — so the toolkit's vocabulary stays in
  one place.
- **`icon.rs`** draws the window icon in code. There is no resource pipeline in
  this port and a binary asset would be the only one nobody can read.
- **`settings.rs` and `theme.rs` have no counterpart here**: the settings model
  is `crates/settings` and the per-terminal slice is `term_settings.rs`
  (ADR 036); themes are `crates/settings/src/themes.rs` (ADR 035).
- **`terminal_renderer.rs` has no counterpart here.** egui's painter *is* the
  render pass, on top of wgpu through eframe, and `renderer`'s software backend
  is the path that is tested. A second wgpu pass would duplicate it (ADR 018).
- **`ui.rs` is gone.** `main` calls `app::run()`.

## Rationale

- **A module per job is a module per decision.** A change to how a pane paints
  cannot now break the settings editor, and the file that names a toolkit is
  named after the thing it is.
- **The tests stay with the window.** The shell's tests drive several modules at
  once — a draft, a chord, a painted colour — so they live in `app.rs` beside
  the window they exercise, with the fields they touch crate-visible.
- **The icon is data.** Twenty lines of arithmetic produce a 64×64 RGBA icon
  that a test can inspect, and the repository gains no opaque file.

## Consequences

- Cross-module state is `pub(crate)`: `App`'s fields for `config.rs`, the widget's
  types for `app.rs`. It is the one place the crate is looser than before, and
  it is confined to the shell.
- ADR 018's "replace this file" becomes "replace these four": a native backend
  replaces `app.rs`, `terminal_widget.rs`, `config.rs` and `icon.rs`, and keeps
  `window.rs`'s interface if the platform has the same three commands.
- The window has an icon, drawn rather than shipped.

## Alternatives considered

| Alternative | Why not |
|---|---|
| Keep one 4,000-line file | Phase 9 names the pieces, and every change touches all five jobs |
| A folder per module (`app/mod.rs`) | One file each is enough at this size |
| Ship a `.ico` | No resource pipeline, and an unreadable binary in the tree |
| A custom wgpu terminal pass | egui already submits through wgpu; `renderer`'s software path is the tested one |
| Move the shell's tests with their modules | They drive the window as a whole; splitting them would duplicate the fixtures |
