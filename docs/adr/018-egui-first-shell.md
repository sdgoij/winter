# ADR 018 — egui is the first shell; native backends come later

- Status: Superseded by ADR 079 (the native shells are the only shells)
- Date: 2026-11-22
- Related: `crates/winter/src/ui.rs`, `plan.md` Phases 8–9; ADR 001,
  ADR 006, ADR 007, ADR 017

## Context

The plan asks for four hand-written window backends (Win32, X11, Wayland,
Cocoa) and, in Phase 9, an "egui terminal viewport" and a "wgpu render pass".
Those are two different stacks, and the first delays anything runnable by four
platform FFI layers.

What the project needs now is a shell a person can type into on all three
desktop targets: a window, keyboard and mouse events, resize, DPI and
clipboard. `eframe` supplies exactly that on top of `winit` and `wgpu`, which
the plan already names for the renderer.

## Decision

The application crate (`crates/winter`) uses `eframe` (winit + wgpu + egui).

- egui is confined to `crates/winter/src/ui.rs`, the module that turns egui
  events into the core vocabulary and paints a frame. Nothing else names it.
- `session.rs` (the pty and its pump) and `grid.rs` (a buffer snapshot: row
  text, attribute runs, cursor) are toolkit-free and tested without a window.
- The terminal grid is painted from those snapshots. A GPU paint callback and
  the atlas path of ADR 007 stay deferred.
- The four hand-written backend rows in Phase 8 are retired for now. A native
  backend, when it is written, replaces `ui.rs`; `session.rs` and `grid.rs` are
  not touched.

## Rationale

- **Runnable first.** winit already handles the window, event loop, resize,
  DPI and clipboard on Windows, Linux and macOS; rewriting that four times
  buys look and feel, not behaviour.
- **A swap, not a rewrite.** The boundary is the point of this record. If egui
  leaked into selection, tabs or the session, the later native GUI would be a
  second rewrite of the application, not a replacement of its shell.
- **The renderer decision is unchanged.** The grid is data; how a backend
  rasterises it remains the renderer's problem (ADR 006, ADR 007).

## Consequences

Positive:

- A shell attached to a real pty is reachable without platform FFI for windows.
- The core of the app stays testable with `cargo test -p winter`.

Negative:

- No native look and feel and no OS-native accessibility yet.
- `eframe` brings a large dependency graph, which `cargo-deny` must keep clean
  on three target triples.
- egui is immediate-mode; a frame is rebuilt every time, so the grid must be
  painted as a surface rather than as one widget per cell.

## Alternatives considered

| Alternative | Why not |
|---|---|
| Hand-written Win32/X11/Wayland/Cocoa backends first | Four FFI layers before the first keystroke; each one is another review event |
| `winit` with a custom renderer, no egui | More code than `eframe` for the same first milestone, and the native-GUI question is unchanged |
| Defer the app until the native design is settled | The pty layer and `Terminal` have no consumer, so integration bugs stay hidden |
