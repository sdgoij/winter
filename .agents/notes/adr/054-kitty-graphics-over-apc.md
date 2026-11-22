# ADR 054 — Kitty graphics ride APC, and the pty host decides

- Status: Accepted
- Date: 2026-11-22
- Related: `crates/vt/src/state_machine.rs`, `crates/term/src/output_engine.rs`, `crates/adapter/src/kitty_graphics.rs`, `crates/server/src/pty.rs`, ADR 005, ADR 016

## Context

`reverie-amidst-ruins` draws every frame with the kitty graphics protocol: one
APC string per frame, `ESC _ G a=T,f=32,s=…,v=…,c=…,r=…,i=1,p=1,o=z,C=1,z=-1`
followed by a base64, zlib-compressed RGBA payload, split into 4096-byte chunks
with `m=1`. It deletes the image on exit with `a=d,d=I,i=1`. Windows Terminal
has no counterpart for it; the game fails earlier there anyway, because
`crossterm::terminal::window_size()` is a hard error on Windows.

The parser already had a `SosPmApcString` state, which — like upstream —
discarded SOS, PM and APC verbatim. The adapter already paints sixel into the
buffer's row image bands (ADR 005), and the renderer already scales those bands
into the frame, so the image path below the protocol existed and was unused.

## Decision

- The VT state machine keeps SOS and PM ignored but delivers an APC string to
  the engine: `action_apc_start`, `action_apc(byte)`, `action_apc_end`. A CAN/SUB
  still cancels the string, and a bare ESC ends it as upstream ends an OSC.
- `OutputStateMachineEngine` accumulates the APC bytes (capped at 32 MiB, so a
  malformed graphic cannot grow without bound) and passes the whole string to
  `ITermDispatch::apc`.
- `adapter::kitty_graphics` implements the subset a full-screen program uses:
  - `a=T` (transmit and display) and `a=d` (delete); `a=t` and `a=q` are parsed
    and ignored.
  - `f=32` RGBA and `f=24` RGB; `o=z` zlib; `s`/`v` source size; `c`/`r` display
    cells; `m` chunking; `i` image id.
  - Base64 is `vt::decode`; zlib is `miniz_oxide`'s inflate, already in the
    dependency graph through `flate2`.
- An image is painted at the buffer's cursor cell, scaled to the requested
  cells with the same 10x20 cell size the sixel path uses, written through
  `ImageSlice::mutable_pixels`. A retransmit of the same id erases the previous
  placement first; a delete erases the recorded cell rectangle.
- The pixel-size queries the game uses were already ported: `CSI 14 t` reports
  the visible page in pixels and `CSI 16 t` the cell size.

## Consequences

- On a Unix pty (`openpty`), the child's bytes reach the parser unchanged, so
  this renders on Linux and macOS today.
- **On Windows the system ConPTY consumes DCS and APC strings before the
  terminal sees them.** Verified with a pty-level test: a child that writes a
  sixel DCS or a kitty APC with `ENABLE_VIRTUAL_TERMINAL_PROCESSING` produces
  only conhost's rendered screen, and neither a duplex pipe nor the draft
  passthrough flag (`0x8`, which the SDK defines as a glyph-width flag) changes
  that. Windows Terminal 1.25 displays sixel because it does not use the system
  ConPTY: it launches the `OpenConsole.exe` bundled in its own package as the
  console host. Until this port does the same, a Windows client cannot deliver
  an image to it, so `reverie-amidst-ruins` cannot render here yet.
- The protocol support is not a stub: every layer is unit-tested, and the
  adapter and term tests paint a real RGBA payload into the buffer's bands.
- Only the listed subset is implemented. PNG payloads, z-index, unicode
  placeholders, animations, and kitty's query/reply handshake are ignored, and
  a program that waits for an `OK` reply will wait forever (the game asks for
  silence with `q=1`).
- The images occupy the same row bands as sixel, so text and images interact
  exactly as they already do for sixel: an image band is painted after the
  cell text for its row.

## Alternatives considered

- **Sixel only.** The game does not emit sixel.
- **Reimplement `winconpty`.** Launching a passthrough-capable console host is
  the real fix for Windows, but it is a pty-backend project of its own, not a
  parser feature; it belongs in `server` with ADR 016's `unsafe` budget.
- **Keep ignoring APC.** That is upstream's behavior, and it is why the game
  would show nothing even if the pty delivered it.
