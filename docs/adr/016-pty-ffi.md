# ADR 016 — The pty layer is hand-written FFI, confined to two modules

- Status: Accepted
- Date: 2026-11-22
- Related: `crates/server/src/pty.rs`, `conpty.rs`, `posix_pty.rs`;
  ADR 001, ADR 015

## Context

The application has to run a shell attached to a pseudo terminal: ConPTY on
Windows, `openpty` on Unix. Upstream has no pty — conhost *is* the console —
so there is nothing to port. The workspace denies `unsafe` and has no FFI
dependency.

The options were: hand-written FFI, the `portable-pty` crate, or deferring
the layer. ADR 015 left the decision open; this ADR takes it.

## Decision

The pty layer is hand-written FFI:

- `windows-sys` on Windows for `CreatePseudoConsole`, `ResizePseudoConsole`,
  `ClosePseudoConsole`, `CreatePipe`, `CreateProcessW` and the process
  attribute list.
- `libc` on Unix for `posix_openpt`/`grantpt`/`unlockpt`/`ptsname_r` and
  `ioctl(TIOCSWINSZ)`.

`unsafe` is allowed **only** in the two backend modules: their declarations in
`lib.rs` carry `#[allow(unsafe_code)]`. The crate keeps the workspace deny
everywhere else. This is the review event ADR 001 anticipated; a third module
with `unsafe` is a new decision, not a copy of this one.

The public API is blocking: `Pty` implements `Read` and `Write` and offers
`resize`, `wait` and `kill`. Non-blocking I/O, an event-loop integration and
signal forwarding belong to the input and application layers, which can split
the handle there.

## Rationale

- **No dependency for a small surface.** The calls needed are a dozen; the
  `portable-pty` crate brings its own platform abstraction, signal handling and
  release cadence for the same calls.
- **Behaviour stays visible.** Resize, EOF and child-exit semantics are written
  down in this tree rather than inherited.
- **A single review point.** The `allow` sits on two module declarations, so
  `grep unsafe_code` still finds every exemption.

## Consequences

Positive:

- The pty layer is testable with real children (`echo`, `stty`) and no sleeps.
- The rest of `server` and the other crates stay `unsafe`-free.

Negative:

- The backends are platform code that must be kept working on three systems;
  CI is the only place the non-host platform runs.
- The command-line quoting for `CreateProcessW` is ours to get right, and a
  quoting bug is a security bug. It has its own unit tests.

## Alternatives considered

| Alternative | Why not |
|---|---|
| `portable-pty` | A large dependency with its own abstraction and policy for the same dozen calls |
| Defer until the app needs it | The server crate's purpose is to read the client; without a pty there is no client to read |
| A separate `pty` crate | Two crates for one abstraction; the boundary would exist only to hold the `allow` |
