# ADR 084 — The editor's own process hosts the terminal

- Status: Accepted
- Date: 2026-11-25
- Related: `crates/napi`, `crates/demo`, `editors/vscode`,
  `crates/winter/src/{embed,term_control}.rs`; ADRs 016, 019, 029, 056, 082, 083

## Context

ADR 083 put the terminal behind a process boundary: `winter-frame` owns the pty
and the surface, and the editor talks to it in JSON lines over stdio. It chose
that because an editor extension has no FFI without shipping native code, and
because the transport was cheap to write and to keep honest: no per-platform
build, no ABI.

It works, and the demo it produced is a real terminal. What never sat right is
the process itself. Running a shell already costs a child process; the frame
server made it two. The protocol also became a thing of its own — framing,
flush rules, a version that has to match on both ends — and every frame crossed
two boundaries, from the pty to `winter-frame` and from there to the editor and
the webview.

## Decision

1. **`crates/napi` (`winter-node`) is the terminal as a Node addon.** It exposes
   one class: `Terminal.spawn(shell, columns, rows, fontSize?)`, input as method
   calls, and `render()` returning the pixels that changed — a band or the whole
   pane, deflated — or `null` when nothing did. Everything it does is a call into
   [`winter::embed::Surface`]; the addon translates Rust types to JavaScript's
   and nothing else.
2. **The extension loads it and owns the terminal.** No child process, no
   protocol, no framing. The pane's half is unchanged: the same webview, the
   same one-frame-in-flight pacing, the same deflated base64 frames.
3. **The policies stay in `crates/demo`,** which is now the shared host library:
   the font measurement (ADR 029), the pointer policy (ADR 019), the physical
   key to Win32 virtual key and scan code table (ADR 056), the cursor's blink
   phase, the banding, and the default shell. Both hosts — the binary and the
   addon — call the same functions, so the two cannot drift.
4. **`winter-frame` and ADR 083 stay.** The binary is the headless host: a CLI
   that can be run without an editor, and the thing the protocol's tests drive.
   ADR 083's decisions still describe it; what changed is which host the editor
   uses.
5. **`unsafe` appears in one place, named.** `napi-derive` generates `extern "C"`
   entry points and the class table, which cannot be written without it; the
   exemption is the crate root of `crates/napi`, like `utils`' lock-free
   primitives and `server`'s pty backends (ADR 016).

## Rationale

- **One process, one owner.** The extension's process is the editor's: it starts
  the shell, reads its output and paints it, and a closed pane kills it. There is
  no second thing to start, stop, or restart when the code changes.
- **The protocol cannot disagree with itself.** A call either type-checks or it
  does not; there is no framing to get wrong, no flush rule to remember, and no
  "the extension is newer than the server" state.
- **The measured cost is small.** A shell's frame band costs under a millisecond
  of the host's thread; a full-screen program's 900 by 510 frame costs about six
  in a release build, which is the price of co-location and is what the pane's
  own paint costs anyway.

## Consequences

- **The addon is built per platform and arch.** `npm run build:addon` builds it
  and copies it beside the extension as `winter.node`; a packaged extension would
  ship one per target and pick at activation. That is the cost ADR 083 refused,
  and it is now paid deliberately.
- **The pixels still leave the process.** `postMessage` takes strings, not
  binary, so frames are base64 — and stay deflated (ADR 083) because a full
  frame is 1.8 MB raw and about 53 KB deflated.
- **A shell is still a child process.** Nothing here removes that: `Session`
  spawns the pty's client exactly as before. What was removed is the second
  process, the one that was never the shell.
- **The host thread does the rendering.** A render blocks the extension host for
  its duration. An async `render` would move it off the thread at the cost of
  ownership and ordering; it is not needed at these numbers.
- **`winter-frame`'s protocol tests still describe the binary**, not the editor.
  The editor's half is TypeScript with no test harness in `just gate`, exactly as
  ADR 083 said.

## Alternatives considered

| Alternative | Why not |
|---|---|
| Keep the process boundary (ADR 083) | Two processes to run a shell, a protocol to keep in step, and a rebuild the user has to remember; the addon removes all three |
| A wasm build of the core in the webview | Pixels would never leave the webview, but the pty still needs native code, so a process stays; and the renderer would need an embedded font and a wasm rasterizer. The eventual second half, not the first |
| Call the binary over a socket instead of stdio | Same two processes, more moving parts |
| A napi build inside `crates/demo` | The demo is a host with a wire format and a `main`; the addon is an ABI. One crate each keeps `cargo build -p winter-demo` from needing a Node toolchain |
