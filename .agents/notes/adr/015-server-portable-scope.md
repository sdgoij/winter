# ADR 015 — The server layer keeps the console API's portable halves

- Status: Accepted
- Date: 2026-11-22
- Related: `src/server/`; ADR 001, ADR 013

## Context

Upstream's `src/server` is the console driver's endpoint. `ConDrvDeviceComm`
exchanges messages with `condrv.sys`, `Entrypoints` receives them,
`WinNTControl` wraps NT object-manager calls, `ApiDispatchers` and
`IoDispatchers` are the routine tables the driver indexes, and `ApiMessage`
is the shared-memory layout both sides agree on.

A cross-platform terminal emulator has no console driver and no client calling
`ConsoleReadConsoleW`. It spawns a shell in a pty and feeds the bytes to the
parser, so a literal port of the driver plumbing would be dead code with no
caller.

## Decision

`crates/server` keeps only the modules whose behaviour a console API still
needs and that do not depend on the driver or the NT object manager:

- `ConsoleShimPolicy` is the first one (ported from `ConsoleShimPolicy.cpp`).
- Handle tables, wait queues, process lists and the API sorter's validation
  are candidates when a consumer exists.

Not ported: `ConDrvDeviceComm`, `Entrypoints`, `WinNTControl`, `IoThread`,
`ApiDispatchers`/`IoDispatchers`, `ApiMessage`/`ApiMessageState` (the driver
protocol), and `ObjectHeader`/`ProcessPolicy` (NT object metadata).

The pty layer is the port's own: ConPTY on Windows and `openpty` on Unix. It is
not a port of `ConDrvDeviceComm` — upstream has no pty because conhost *is* the
console. It lands when the application needs to spawn a shell. Both backends are
FFI, and the workspace denies `unsafe`; that change needs its own ADR.

## Rationale

- **No dead client.** A message format with no driver to speak it is not a
  port; it reimplements condrv.
- **The oracle still applies.** Shim policy, handle semantics and wait ordering
  are upstream rules, and they are ported file by file where they are reachable.
- **`unsafe` stays a review event.** Deferring the pty layer keeps the decision
  visible instead of smuggling an FFI module in under a port row.

## Consequences

Positive:

- The crate starts with content that runs and is tested.
- The pty decision, and the `unsafe` it will need, is named rather than implied.

Negative:

- Most of Phase 7's upstream file list is deliberately not ported, so the plan's
  rows are retired rather than ticked.
- A future console-API client (a compatibility layer for Win32 console apps)
  would bring the driver protocol back into scope; this ADR is reconsidered then.

## Alternatives considered

| Alternative | Why not |
|---|---|
| Port the driver protocol behind a trait | No client speaks it; the trait would be shaped by imagined callers |
| Skip the server crate until the app exists | The shim policy and handle bookkeeping are needed before the pty layer, and a crate for one module is a boundary the plan already names |
| Put the pty layer in `server` now | It needs `unsafe` and a platform split; that is a separate decision with its own ADR |
