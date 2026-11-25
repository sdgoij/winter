# ADR 081 — The Windows binary is a GUI-subsystem executable

- Status: Accepted
- Date: 2026-11-25
- Related: `crates/app/src/main.rs`, `crates/app/build.rs`,
  `crates/winter/src/trace.rs`; ADRs 055, 058, 080

## Context

Cargo links a Windows binary with the console subsystem unless the crate says
otherwise, so Explorer, the Start menu and a double-click each start
`conhost.exe` beside the shell. The console serves no purpose: the shell spawns
its own pty, and its diagnostics already go to the `WINTER_TRACE` file because a
GUI-subsystem process has no console to print to.

## Decision

`crates/app/src/main.rs` declares `windows_subsystem = "windows"`, so the linker
emits subsystem 2 (`Windows GUI`) instead of 3 (`Windows CUI`). It applies to
debug builds as well: the request is what a double-click does, and the trace file
is the diagnostic path for every profile.

## Rationale

- **Nothing in this process writes to its own console.** Output belongs to the
  pty; the app's messages are trace lines or UI.
- **The trace file is profile-independent.** Gating the attribute on
  `debug_assertions` would keep a console in the build a developer double-clicks
  and would make the two builds launch differently.

## Consequences

- A double-click, the Start menu and the taskbar open the window alone.
- `stderr` writes from the GPU path are discarded when there is no console;
  `Shell::tick` already reports a failed frame through `trace`.
- A startup error returned from `main` exits with status 1 and no message. A
  launch with `WINTER_TRACE` set is how that failure is diagnosed.
- A launch from an existing console can still inherit that console's handles;
  the shell does not attach to it.

## Alternatives considered

| Alternative | Why not |
|---|---|
| `cfg_attr(not(debug_assertions), windows_subsystem = "windows")` | A double-clicked debug build still opens a console, and the app has a trace file for the diagnostics it would show |
| `editbin /SUBSYSTEM:WINDOWS` after the build | A post-build step and a tool dependency for what a crate attribute does |
| `AttachConsole(ATTACH_PARENT_PROCESS)` at startup | Needs `unsafe` in the binary to attach to a console the shell does not use |
