# ADR 044 — The WinUI shell reads settings at startup

- Status: Accepted
- Date: 2026-11-22
- Related: `crates/winterm/src/winui/app.rs`, `crates/winterm/src/term_settings.rs`,
  `crates/renderer/src/render_settings.rs`, `crates/renderer/src/atlas/engine.rs`,
  ADR 025, ADR 026, ADR 034, ADR 043

## Context

The first WinUI shell ignored the settings file. It ran `powershell.exe`, painted
Consolas at 16 points and used the built-in palette, so a configured profile, a
font size or a colour scheme changed the egui shell and nothing else.

The egui shell already resolves a profile through `TerminalSettings` and applies
the scheme to the control's colour table. The renderer resolves a cell's colours
through its own `RenderSettings` table, which is a different copy from the one
`TerminalData` holds, so a palette applied to the control does not reach painted
cells by itself.

## Decision

The WinUI shell reads the same file through the same path.

- At startup it calls `settings::ensure_file()` and
  `settings::load_with_report()`, resolves `TerminalSettings` for the default
  profile, and uses that profile's command and arguments, its title, the file's
  font size and its scheme.
- The scheme is applied to the control's table. The default background and
  foreground are then read back from that table, given to the engine through
  `Renderer::set_default_colors`, and used as the surface's clear colour.
- Every frame the shell hands the control's table to
  `Renderer::settings_mut().set_color_table`, because an engine resolves a cell's
  colours through the renderer's settings, not through the data's copy.
- A profile command that cannot be spawned falls back to PowerShell, then `cmd`.

## Rationale

One resolution path for both shells means a file cannot mean different things on
different platforms. Reading the palette back from the control's table keeps a
single source of truth: whatever the program's OSC 10/11 wrote is what the pane
paints, and the shell does not reimplement colour resolution.

## Consequences

- The file is read once. Live reload stays the settings-watcher step (ADR 025)
  and is not wired into this shell yet.
- A DPI change rebuilds the renderer, so the shell keeps the resolved colours
  and re-applies them instead of falling back to the built-in palette.
- The window title is the profile's until the program sets one with OSC 0/2,
  which happens immediately for cmd and PowerShell.
- A broken file still starts a shell: `load_with_report` yields the defaults.

## Alternatives considered

- **Keep hardcoding the shell.** The Windows shell then contradicts the settings
  window that edits the same file.
- **Give the renderer the scheme directly.** It duplicates `Scheme::apply`, and
  the pane and the cells can disagree after a program sets OSC 10/11.
- **Resolve the title at each frame from the profile.** The program's OSC title
  is the later word and must win, so the profile's title is only the initial one.
