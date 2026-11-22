# ADR 013 — The host drains queues; there is no `ITerminalApi` seam

- Status: Accepted
- Date: 2026-11-22
- Related: `src/host/outputStream.cpp`, `src/terminal/adapter/ITerminalApi.hpp`,
  `crates/adapter/src/adapt_dispatch.rs`; ADR 004, ADR 010

## Context

Upstream splits the adapter from its host with `ITerminalApi`. `AdaptDispatch`
calls it for everything the adapter does not own: the active buffer and its
viewport, window title, clipboard, system modes, code pages, resizing, the
window's visibility and bell, and the responses a client is owed.
`outputStream.cpp` is the conhost implementation, `ConhostInternalGetSet`, and it
is the only implementation upstream ships.

This port went the other way. `AdaptDispatch` owns the state it can — modes,
margins, the SGR stack, cursor style, saved cursors, tab stops, the colour
table — and queues what belongs to the host: `pending_responses`,
`pending_macro_input`, `clipboard_content`, `window_title`,
`current_working_directory`, `pending_window_visibility`,
`pending_window_resize`, `redraw_requested`, and MIDI notes. The host drains
those queues. That is what lets `term::Terminal` exist before the console does,
and it is why the adapter has no dependency on a host.

Porting `outputStream.cpp` as written would need `ITerminalApi` first, and then
moving the adapter's state back into the host and rewriting the families that
already own it — a change to the architecture, not a port of one file.

## Decision

`ITerminalApi` and `ConhostInternalGetSet` are not ported. The adapter stays the
owner of its state and the producer of explicit queues; the host drains them.

The plan's `output_stream.rs` row is removed. Its portable content is already
elsewhere:

- responses: `AdaptDispatch::take_responses`, drained by `term::Terminal`;
- clipboard, title, working directory, window requests: the same drain pattern;
- the screen and its modes: `host::ScreenInformation` (ADR 012) and
  `host::Settings`;
- code pages: not modelled; terminal data is UTF-8 at the boundary.

## Rationale

- **One adapter, two hosts.** The console host and a standalone terminal can
  drain the same queues; neither needs a second implementation of a 40-method
  seam.
- **The adapter is testable without a host.** ADR 004's ratchet test drives
  every dispatch callback against a real buffer with no console object.
- **The seam's synchronous queries are the thing this port cannot honour.**
  `GetBufferAndViewport` returns a live reference; a queued request returns
  later. Making that synchronous again would put the host back inside the
  adapter, which is the coupling ADR 001 forbids.

## Consequences

Positive:

- No duplicate state between adapter and host.
- The adapter's public surface stays data-in, queues-out, which is easy to test.

Negative:

- A sequence whose handling needs a synchronous answer from the host (a resize
  that must report success, a window query) is a request the host fulfils after
  the dispatch returns. A client that depends on the synchronous ordering sees
  the difference.
- Two drain loops exist: the terminal's and the console's. They share the queue
  types, not the code.

## Alternatives considered

| Alternative | Why not |
|---|---|
| Port `ITerminalApi`, then `ConhostInternalGetSet` | Requires moving state out of a complete adapter and rewriting its tests; contradicts ADR 004's ratchet |
| Implement `ConhostInternalGetSet` against a new host trait | A second seam with the same synchronous-query problem, and the adapter would ignore it |
| Keep the adapter state and also add the seam | Two homes for the same state, the failure ADR 002 names |
