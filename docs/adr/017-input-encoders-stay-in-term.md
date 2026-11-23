# ADR 017 — The input encoders stay in `term`; there is no `input` crate

- Status: Accepted
- Date: 2026-11-22
- Related: `crates/term/src/input/`, `plan.md` Phase 8; ADR 001, ADR 009,
  ADR 011, ADR 015

## Context

Phase 8 of the plan names `crates/input` with `keyboard.rs`, `mouse.rs`,
`kitty.rs`, `ime.rs`, `input_thread.rs` and `pty_signal.rs`, mapped from
`src/terminal/input/`. When those files were ported, their only consumer was
`Terminal`: a key event goes in, a snapshot of `InputModes` comes from the
adapter, and the bytes that belong on the pty come out. The modules therefore
landed as `term::input::terminal_input` and `term::input::mouse_input`, with the
kitty protocol inside the former (ADR 009).

The plan rows now describe a crate whose content already exists two layers to
the left. Creating it would either duplicate the encoders or move them away
from the code that calls them.

## Decision

There is no `crates/input`.

- `terminalInput.cpp` is `crates/term/src/input/terminal_input.rs`; it owns the
  regular encodings, the kitty protocol, win32-input-mode and focus reporting.
- `mouseInput.cpp` is `crates/term/src/input/mouse_input.rs`; it owns the
  tracking modes, clicks, motion, wheel accumulation and the UTF-8/SGR
  encodings.
- IME has no portable module to port. The platform control gives the composed
  text to `Terminal::handle_key` as a character event.
- Platform message decoding (layout, scan codes, composed characters) belongs
  to the window backend that receives the messages, not to a shared crate.
- The input pump and pty signals are the application's composition of `server`
  (the pty) and `term` (the parser and the encoders). They land with the app
  crate, where both are visible.
- `ControlInteractivity.cpp`'s click counter and timer go with the rest of that
  file to the app crate (Phase 9); `src/cascadia` is not `src/terminal/input`.

## Rationale

- **ADR 001 rule 3.** A crate is created when its first real content exists. The
  content exists in `term`, where the consumer is.
- **The ring is unchanged.** `input` would need `adapter` for `InputModes`, the
  same dependency `term` already has; it would be a directory with a manifest.
- **One home.** Splitting the encoder from `Terminal` would let the two drift,
  and it would make the conformance corpus test a copy.

## Consequences

Positive:

- Phase 8's keyboard, mouse and kitty rows are retired rather than ticked, and
  the plan stops promising a crate that would duplicate working code.
- The pty layer gets its consumer in the app layer, next to the event loop that
  owns both halves.

Negative:

- A future non-`Terminal` consumer of key encoding (a harness, or the console
  side of a compatibility layer) has to reach into `term::input`. If that
  happens, this ADR is superseded rather than edited.

## Alternatives considered

| Alternative | Why not |
|---|---|
| Create `crates/input` and move `term::input` into it | Two crates would then be needed wherever one event is encoded; `term` cannot depend on a crate to its right |
| Create `crates/input` as a re-export of `term::input` | A façade with no rule of its own is a directory, not a boundary |
| Put the input pump in `server` | `server` would have to see `term`, which is a sibling edge that exists only to host a loop the app owns |
