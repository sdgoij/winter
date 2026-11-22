# ADR 057 — Kitty images paint at the end of a batch

- Status: Accepted
- Date: 2026-11-22
- Related: `crates/adapter/src/kitty_graphics.rs`, `crates/term/src/terminal.rs`, ADR 054, ADR 056

## Context

A full-screen program that redraws every frame retransmits its image with the
same id and placement (`a=T,...,i=1,p=1`) each time. At 60 fps that is a
zlib-compressed, base64 payload of roughly 130 kB per frame. Decoding and
scaling each one on the UI thread costs more than a frame interval in a debug
build, so the host's event loop starves and key events queue up in bursts.

## Decision

- `KittyGraphics` stores a completed `a=T` payload as *deferred* instead of
  decoding it immediately, together with the cursor cell it was transmitted at.
- A retransmission of the same id replaces the deferred entry; only the newest
  frame can be seen.
- Any other kitty action flushes first, so a delete sees the frames that
  arrived before it.
- `Terminal::process_bytes` calls `AdaptDispatch::flush_graphics()` after each
  batch, which decodes and places what is deferred.

## Consequences

- A batch of N frames decodes once. The debug build keeps up with the game's
  60 fps image traffic instead of falling seconds behind.
- A test that drives the parser directly must call `flush_graphics()` before
  reading the buffer; the kitty paint tests do.
- Paint order is preserved for deletes and for distinct ids; only successive
  transmissions of one id are collapsed.
