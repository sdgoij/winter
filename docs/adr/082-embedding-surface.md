# ADR 082 — An embedding surface composes the terminal without a window

- Status: Accepted
- Date: 2026-11-25
- Related: `crates/winter/src/{embed,term_connection,term_control}.rs`,
  `crates/winter/Cargo.toml`, `crates/renderer/src/atlas/shared.rs`; ADRs 002,
  006, 019, 029, 036, 040, 080, 081

## Context

The pieces a terminal is made of were already public and toolkit-free: `Control`
owns the parser and the buffer, `terminal_render` copies a frame into a
renderer's shape, `view` holds the viewport, `renderer` paints pixels. What a
host had to write for itself was the *composition*: pump, refresh, hand the
palette to the engine, invalidate, paint — five calls whose order matters — and
the input translation that turns a toolkit event into keys.

Two hosts outside this workspace were the reason to look: a browser or editor
panel that draws the terminal on a canvas, and an agent's tool that runs a
command and reads the screen. Both own their process and their pixels; neither
wants a window toolkit, a pty, or a notification daemon.

Three things put the pty layer in their way:

- `Connection::resize` took `server::PtySize`, so the backend trait — and every
  crate that implemented or used it — named the pty crate.
- The only connection with no pty behind it was `term_connection::tests::Buffer`,
  a test double that ADR 080 had to un-gate so another crate's tests could use it.
- `winter` linked `server` and `notify-rust` unconditionally, so even the parser
  and the renderer came with a pty and a desktop notification service.

## Decision

1. **`winter::embed::Surface` is the composition, in one place.** `new(Setup)`,
   `feed`, `take_written`, `key`, `paste`, `write`, `mouse`, `scroll_by`,
   `resize`, `render`, `pixels`, `frame_size`, `take_events`, `size`, `title`.
   It owns the viewport, so a scroll means something without the host tracking a
   document position. Not the selection: a host that draws its own chrome has
   its own pointer, and the selection is application state (ADR 019).
2. **The host measures the font.** `Setup` carries `FontInfo`, the atlas
   `Settings`, a shaper and a rasteriser, because font discovery is the host's
   (ADR 029) and the settings must already describe the grid the surface starts
   with. The surface does not guess a target size from a face it did not choose.
3. **`Connection::resize` takes cells, not a `PtySize`.** `Session`'s
   implementation builds the `PtySize`; `term_connection` no longer names
   `server`. ADR 036's five methods stand — only the fourth's argument changed.
4. **The pty-less double is a production type, `term_connection::Queue`.** A host
   feeds it what the program wrote (`feed`, `end`) and takes back what the
   terminal answered (`take_written`, or `written` to peek). The un-gated `tests`
   module ADR 080 left behind is gone: the double a test uses and the double an
   embedding uses are now the same object.
5. **Three features, all on by default.** `pty` gates `session`,
   `shell_integration`, `term_settings` and `impl Connection for Session`;
   `notifications` gates `notifications`; `settings` gates `config`,
   `term_settings`, `Control::apply_scheme` and the chord lookup in `input`. A
   host that turns them off owns the process, decides what a bell or a desktop
   notification means, and starts from a measured font and a grid rather than a
   settings file.
6. **`SharedBackend::pixels` returns the borrow.** `with_pixels` ends the borrow
   when its closure returns, which is useless to a host that copies a frame
   across an FFI boundary; the borrow is handed back instead.
7. **`utils`' temp-file helper carries `uuid`.** Nothing in the workspace calls
   `create_temp_file`, and `uuid`'s `v4` needs a randomness source on wasm, so the
   helper and the dependency sit behind `utils`' `temp-file` feature, off by
   default.

## Rationale

- **One composition rather than one per host.** The five per-frame steps are
  order-sensitive — pump before refresh, palette before paint, invalidate before
  paint — and a host that got the order wrong would present a stale frame or miss
  a program's response. The input rules are ADR 009's and are now reached by
  `Surface::key` instead of being re-derived.
- **Cells, because the pty is one backend.** The trait exists so a second backend
  can exist (ADR 036); a signature that names the first one's window geometry
  contradicts that.
- **A `Queue` with a name that says what it does.** The object is a byte queue in
  both uses; keeping a `tests::Buffer` beside an embedding's queue would be two
  homes for one thing (ADR 002).

## Consequences

- The shells are unchanged. `pty` and `notifications` are on by default, and
  `Control::resize` still takes cells, so no call site moved.
- **`render` and `pixels` are two calls.** Returning the frame's pixels from
  `render` would hold the surface's mutable borrow for as long as the borrow
  lived, because the pixels are a field of the surface.
- `term_settings` and `shell_integration` stay pty-only: they resolve a
  `PtyCommand`, which a host with no process has no use for.
- **The embedding build is wasm-capable.**
  `cargo check -p winter --no-default-features --target wasm32-unknown-unknown`
  passes. The default build does not, and fails in `notify-rust` and `settings` —
  the two crates the features exist to leave out. That is the split working, not
  a gap.
- **The pty-less build has no settings-free way to apply a scheme.**
  `Control::apply_scheme` takes a `settings::Scheme` and is gated with the
  feature; `Surface::color_table` hands a host the table to fill instead, and a
  program's OSC 10/11 still wins on the next frame.
- **Two gates fell out of adding a fourth target.** `find::highlight_spans` was
  gated to the three shell platforms although `terminal_render` — core code —
  calls it, so the gate went; `fonts::font_directories` needed its vector's type
  named, because on a target with no directory block nothing else says what it
  holds.
- The facade has its own tests: they feed bytes and assert painted ink, so the
  composition is covered without a pty and without a window.

## Alternatives considered

| Alternative | Why not |
|---|---|
| A new `crates/embed` crate | It would hold a second `Control`/`TerminalData` composition — those types are `winter`'s and ADR 002 keeps one home — and add a ring edge that skips the crate the core lives in |
| Bindings per host (napi, wasm-bindgen) in this workspace | Each binding would re-encode the same five steps and the same input rules; a binding should be a thin caller of `Surface` |
| Keep `Connection::resize(PtySize)` and gate modules only | The trait would still name `server`, so every pty-less host would build the pty crate |
| Return `&[u32]` from `render` | The pixels are a field of the surface, so every read of a frame would also hold the whole surface borrowed |

## Amendment

`Surface::new` takes the connection the host owns instead of building a `Queue`
of its own, and `feed`/`take_written` went with it. The first host written
against the surface was a frame server driving a real shell (ADR 083), and a
surface hardwired to a queue could not see a pty's output; once the host owns
the connection, feeding it and taking its answers are the connection's own
methods, and a `Queue` clone is all a test host needs.
