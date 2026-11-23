# ADR 012 — `SCREEN_INFORMATION` is a buffer, a viewport and console modes

- Status: Accepted
- Date: 2026-11-22
- Related: `src/host/screenInfo.cpp`, `src/host/screenInfo.hpp`,
  `crates/host/src/screen_info.rs`; ADR 001, ADR 005

## Context

`SCREEN_INFORMATION` is the host's screen object. Upstream it owns a
`TextBuffer`, a viewport, a cursor, the console's attributes, the VT state
machine, font metrics, scroll-bar state, a deferred-resize queue, an object
header, and links to the main and alternate buffers. Its 87 methods reach the
window, the renderer, the server's object table and the code pages.

Porting that whole graph would pull `renderer`, `server` and the window layer
into the host in one step, and three of them do not exist yet. Some of it would
also duplicate state that this workspace deliberately put elsewhere: pages and
alternate buffers are the adapter's `PageManager` (ADR 005), and fonts are the
renderer's (ADR 008).

## Decision

`host::ScreenInformation` is the portable record: a `buffer::TextBuffer`, the
viewport into it, the virtual bottom, the cursor's console-facing state, the
console's attributes, the output mode, and the ConPTY cursor-synchronization
flag.

This module ports:

- buffer access and `buffer_size` / `terminal_buffer_size`;
- the viewport: get, set with clamping, absolute or relative origin, virtual
  bottom, virtual viewport, VT page area;
- the cursor: position with bounds checking, make-visible scrolling,
  size/visibility/type/double-width, snap-on-input;
- attributes: current, popup, and the default pair;
- `clip_to_screen_buffer`, `clear_text_data`;
- `output_mode`, `scroll_scale`, terminal-scrolling mode, and the
  `ConptyCursorPositionMayBeWrong` flag.

It does not port: `CreateInstance`'s object list, `s_Insert`/`s_Remove`, the VT
state machine, `ConhostInternalGetSet`, the font getters and `UpdateFont`,
scroll-bar computation, window metrics and resize (`ProcessResizeWindow`,
`_AdjustScreenBuffer*`, `_deferredPtyResize`), `SendNotifyBeep`,
`PostUpdateWindowSize`, `GetScreenBufferInformation`, `AsCharInfo`, the word
boundary, or the read-side iterators.

## Rationale

- **One home per rule.** Page switching stays in `adapter::PageManager`; a second
  main/alternate pair in the host would create two answers to "which buffer is
  visible".
- **The host can be built without the server.** Everything above needs only
  `buffer`, `types` and `utils`, so it can be tested before any window exists.
- **A screen is not a window.** Font metrics and scroll bars are properties of
  the presentation, and this port's renderer is host-driven (ADR 006).

## Consequences

Positive:

- `ScreenInformation` is platform-independent and unit-testable.
- The adapter can later be handed a buffer from it without a new dependency
  edge.

Negative:

- The window-size helpers and the deferred resize path are unavailable until
  the window layer lands.
- Upstream keeps the current attributes in the `TextBuffer`; this workspace's
  adapter owns the VT write attributes, so the console keeps the legacy current
  attribute it writes with. The two are kept in step by the host's write path,
  not by the buffer.
- `_CommitViewport`'s window-buffer-size event is not posted here; the caller
  that owns the `InputBuffer` does it.

## Alternatives considered

| Alternative | Why not |
|---|---|
| Port the whole object graph now | Pulls in the renderer, server and window layers to compile one file |
| Put `ScreenInformation` in `buffer` | `buffer` is the storage layer; the viewport, cursor modes and console attributes are host state |
| Keep the alternate buffer inside `ScreenInformation` | Duplicates `PageManager`, and the adapter is the one that switches pages |
