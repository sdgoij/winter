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
   `winter`'s. The shell is PowerShell on Windows — 7 before 5.1 — and `$SHELL`
   on macOS and Linux, with `--shell` naming one instead.
2. **Commands are JSON lines on stdin**: `write`, `key`, `paste`, `resize`,
   `render`, `press`, `move`, `release`, `wheel`, `copy`, `modes` and `quit`. A
   line this build does not know is skipped, not fatal — a host may be newer than
   the server, and dropping the session is worse than ignoring a line.
3. **Answers are tagged**: kind 0 is a whole frame — five big-endian `u32`s
   (width, height, cell width, cell height, compressed length) then the frame's
   RGBA, deflated; kind 1 is text, which is how a copy reaches the host's
   clipboard; kind 2 is a band — its top pixel row, width, height, an unused
   word, then the band's deflated RGBA — because a shell mostly changes one row
   and shipping the pane for it is what made the demo sluggish; kind 3 is a
   diagnostic line, which is how a host asks what the program requested of the
   input path. A frame with a zero width is the answer to a render that changed
   nothing: the host keeps the picture it has, and the answer is flushed so a
   host that waits for it gets it. The cell size is in the header because a host
   with a pane has pixels, not a grid; without it, turning a pane's size into
   `cols`/`rows` would mean guessing the font's metrics. The pixels are deflated
   because a full-screen program repaints its whole pane every frame and a
   webview cannot decode 1.8 MB of base64 per frame at 30 Hz.
4. **The pointer policy lives in the server.** A press, move, release or wheel
   is forwarded to a program that tracks the mouse, unless Shift is held; when it
   is not forwarded, a press starts a selection, a move extends it and the wheel
   scrolls. That is `forwards_to_application` and `Surface::press`/`drag` — the
   same rule the shells apply (ADR 019), written once here rather than in the
   webview.
5. **Keys go through the terminal's encoder.** The webview reports a key's name,
   its physical `code`, the text it typed and the modifiers; the server builds
   the event and calls `Surface::key`, which applies the input modes and the
   program's kitty-keyboard flags (ADRs 009, 056). Writing the characters
   instead loses function keys, modifier sides and both encodings — which is what
   a program that pushes `REPORT_ALL_KEYS_AS_ESCAPE_CODES` is asking for. A
   ConPTY asks for Win32 input records instead, and a record names the key by its
   virtual key and scan code: a record without them is a lone text byte, and the
   console answers a text byte with a press and a release together, so a held key
   moves for one frame. The physical `code` is what turns into both.
6. **`editors/vscode` is the other half**: a command, a webview with a canvas,
   and a child process. It keeps **one frame in flight** — it asks for the next
   frame only after the pane reports it painted the last, and no sooner than
   33 ms apart — and asks again after input.
7. **The demo uses one process per terminal.** Tabs, panes and remoting are not
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

- **An idle pane costs 24 bytes a poll.** `Surface::render` skips the paint when
  nothing changed — no bytes, no resize, no scroll, no selection and the same
  cursor phase — so the host's poll answers with a zero-width header. That
  header is written and **flushed** like any other answer: a buffered stdout
  that only flushed when the pane had changed leaves the host waiting for a
  reply that never leaves the process, which is a stall of one watchdog period
  per keystroke. The cursor phase is the server's: 500 ms, like every shell, and
  constant while the cursor is hidden, so a full-screen program stops asking for
  frames entirely.
- **No selection UI of its own.** Selection, copy, scrollback and mouse
  reporting are the server's, because they are the terminal's rules; the webview
  only turns pixels into cells. Dragging past the edge does not autoscroll —
  `Surface::drag_scroll` exists, but the webview clamps the pointer to the grid.
- **A keystroke costs 10 KB, a game frame 53 KB.** The server keeps the frame it
  last sent and diffs the new one, then sends the band of whole cell rows that
  changed; more than half the pane, a resize or the first frame goes whole. Both
  are deflated at level 1: on a 900 by 510 pane one typed character is a
  61,200-byte band that travels as about 10 KB, and a full-screen program's
  1,836,000-byte frame as about 53 KB — 1.4 MB/s of base64 instead of 9.9. The
  cost is about a millisecond of deflate per frame; output that scrolls still
  sends the frame, which is what the heuristic gives up.
- **A host that polls on a timer queues frames it will never show.** Answers
  come back in the order the commands arrived, so a render asked for while the
  previous one is still being painted is not dropped — it waits. A 900 by 510
  frame costs about 5 ms in a release build and 75 ms in a debug one, so a fixed
  30-per-second request against a debug server builds a backlog that grows
  without bound: the pane falls further behind real time, and a keystroke sits
  behind every frame already asked for. That is what the pane's
  acknowledgement is for — the host asks for the next frame only after the
  canvas has painted the previous one — and it is why the extension prefers
  `target/release`: a shell's prompt never made the difference visible, and a
  full-screen program does.
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
