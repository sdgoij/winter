# ADR 083 — A frame server lets an editor host the terminal

- Status: Accepted
- Date: 2026-11-25
- Related: `crates/demo`, `editors/vscode`,
  `crates/winter/src/{embed,term_connection}.rs`; ADRs 009, 019, 029, 040, 082

## Context

ADR 082 gave the terminal a composition a host can call: feed bytes, translate
input, paint, read pixels. What it did not answer is how a host that is not a
Rust binary in this workspace reaches it.

The two editors worth checking both own their panes and cannot link Rust: Zed
extensions are `wasm32-wasip2` components with no view surface and no process
access, and VS Code extensions run in a Node host whose only custom surface is a
webview — an HTML canvas. Neither can be handed a native window, so both can only
be handed pixels.

That leaves a process boundary: the host owns the shell, and something outside
it owns the frame.

## Decision

1. **`crates/demo` (`winter-demo`, binary `winter-frame`) is a host, not a
   shell.** It measures a face (ADR 029), spawns the shell on a pty and builds
   one [`Surface`] over it. It holds no terminal logic: what the screen says is
   `winter`'s.
2. **Commands are JSON lines on stdin**: `write`, `resize`, `render`, `press`,
   `move`, `release`, `wheel`, `copy` and `quit`. A line this build does not know
   is skipped, not fatal — a host may be newer than the server, and dropping the
   session is worse than ignoring a line.
3. **Answers are tagged**: kind 0 is a frame — five big-endian `u32`s (width,
   height, cell width, cell height, byte length) then RGBA with alpha forced
   opaque — and kind 1 is text, which is how a copy reaches the host's clipboard.
   The cell size is in the header because a host with a pane has pixels, not a
   grid; without it, turning a pane's size into `cols`/`rows` would mean guessing
   the font's metrics.
4. **The pointer policy lives in the server.** A press, move, release or wheel
   is forwarded to a program that tracks the mouse, unless Shift is held; when it
   is not forwarded, a press starts a selection, a move extends it and the wheel
   scrolls. That is `forwards_to_application` and `Surface::press`/`drag` — the
   same rule the shells apply (ADR 019), written once here rather than in the
   webview.
5. **`editors/vscode` is the other half**: a command, a webview with a canvas,
   and a child process. It asks for a frame every 33 ms and after input.
6. **The demo uses one process per terminal.** Tabs, panes and remoting are not
   modelled; a second terminal is a second process.

## Rationale

- **A process boundary is what an editor extension actually has.** It needs no
  FFI, no per-platform packaging of a native addon, and no new dependency: the
  JSON parse uses `serde_json`, which the workspace already carries.
- **It exercises the surface rather than a subset of it.** Bytes in, keys
  translated, a frame out — the same four calls a shell makes, reached from
  outside the workspace.
- **The pixels are Winter's.** The host paints nothing; the frame is
  `renderer`'s software path, so a glyph, an emoji, a sixel image or a cursor
  shape arrives as the terminal drew it.

## Consequences

- **Continuous rendering.** The demo asks for a frame every 33 ms while a panel
  is showing; `Surface::requires_continuous_redraw` and the dirty regions are
  not used yet, and the cursor does not blink.
- **No selection UI of its own.** Selection, copy, scrollback and mouse
  reporting are the server's, because they are the terminal's rules; the webview
  only turns pixels into cells. Dragging past the edge does not autoscroll —
  `Surface::drag_scroll` exists, but the webview clamps the pointer to the grid.
- **Frames are large.** A 900 by 510 pane is about 1.8 MB per frame; that is
  nothing over a local pipe and wrong for a remote connection.
- **The editor half is outside the cargo gate.** `editors/vscode` is TypeScript
  and needs `npm install && npm run compile`; nothing in `just gate` compiles or
  tests it, and this ADR does not change that.
- **The binary is native and stays native.** The pty has to live outside a
  browser, so a wasm canvas would still need this process and a transport —
  two builds instead of one.

## Alternatives considered

| Alternative | Why not |
|---|---|
| A napi addon for the Node extension host | It needs a build per platform and arch and re-encodes the same four calls; the process boundary already exists |
| A wasm build of the core and a canvas painter | The pty cannot live in the browser, so the host process is still needed, and the frame conversion is the same |
| An MCP server so the agent drives the terminal | Useful, and a different demo: the agent reads text, a person reads pixels |
| Let the editor draw from `grid::Screen` runs | Its own text shaper would then decide glyph positions, so the demo would stop showing Winter's rendering |
