# ADR 006 — The renderer is host-driven, with no thread of its own

- Status: Accepted
- Date: 2026-11-22
- Related: `crates/renderer/src/render_engine.rs`, `renderer.cpp`,
  `RenderEngineBase.cpp`; ADR 001, ADR 003

## Context

Upstream's `Renderer` owns a dedicated render thread, a Win32 shutdown event, a
manual-reset enable event, two atomics (`_redraw`, `_threadKeepRunning`), a small
timer wheel, and a `WaitForMultipleObjects` loop. The paint orchestration is
interleaved with that machinery: `_renderThread`, `_waitUntilCanRender`,
`_waitUntilTimerOrRedraw`, `_tickTimers`, and the event handles are members of
the class.

None of that portability survives translation. A renderer that spawns its own
thread and waits on OS handles cannot be tested without timing, cannot be
cancelled portably, and couples the drawing code to the window backend. It also
conflicts with two project rules: emulation stays platform-independent, and the
test suite has no sleeps.

The renderer crate already exposes the engine surface (`RenderEngine`,
`RenderEngineBase`) and the read path (`TextBufferCellIterator`). The remaining
question is only who owns the loop.

## Decision

The renderer is a state machine that the host drives. It does not spawn a thread.

- **The host owns the loop.** A `Renderer` exposes `paint_frame()`, redraw
  requests and timer queries. The application decides when to call them, how to
  sleep, and on which thread.
- **Shutdown is an `AtomicBool`.** Every entry point that can block takes a
  `&AtomicBool` and returns without painting when it is set, matching the
  `HANDLE` shutdown parameter on `IRenderEngine`.
- **Time is `std::time::{Duration, Instant}`.** Upstream's `TimerRepr` is a
  100 ns tick; intervals and deadlines are `Duration` values here.
- **Timers are data, not threads.** A timer is a description (deadline,
  interval, callback) held in a list. The host ticks the list; the renderer never
  sleeps, spawns, or wakes itself. A callback that must run is a
  `Box<dyn FnMut(&mut Renderer)>`.
- **Throttling stays in the engine.** `RenderEngine::wait_until_can_render` is
  the ~60 FPS primitive, as upstream's `RenderEngineBase` implements it. The
  host may call it between frames.
- **Redraw is a flag plus the host's own wake mechanism.** A request sets a
  flag; the host's event loop is responsible for converting that into a wakeup
  (a window message, a `Condvar`, an async task). The renderer does not know
  which.

## Rationale

- **Testable without timing.** Every decision is a call with an explicit
  argument or a pure function of state, so a test asserts the result and never
  sleeps.
- **Platform code stays in the host.** `cfg(windows)` / `cfg(unix)` already
  belong to the host, pty and input layers; the renderer stays free of them.
- **One loop, not two.** A terminal already has an event loop for input and
  window events. A second scheduler in the renderer would race it for wakes and
  make shutdown ordering ambiguous.
- **The engine contract is unchanged.** `RenderEngine` still mirrors
  `IRenderEngine`, so a future backend is written against the same methods
  upstream implements.

## Consequences

Positive:

- The renderer can be exercised frame by frame in unit tests.
- Cancellation and shutdown are the host's normal control flow, not a special
  event object.
- A software backend and a GPU backend share the same loop.

Negative:

- The host must pump the timer list and the redraw flag; forgetting to does
  nothing rather than panicking, so the host needs a test for it.
- The "render thread" performance shape is gone. If a host needs painting off
  the UI thread, it puts `Renderer` on its own thread; the renderer does not
  decide that for it.

## Alternatives considered

| Alternative | Why not |
|-------------|---------|
| Spawn a render thread inside the renderer | Reintroduces OS waits, breaks deterministic tests, and duplicates the host's loop |
| Adopt an async runtime (tokio, smol) | A large dependency for one timer wheel; tests would need a runtime, and the no-network/no-sleep rules favour plain calls |
| Use `std::thread` + `Condvar` as the default | Still a hidden thread; the host's event loop is the right place to wake on input and window events |
| Port `WaitForMultipleObjects` behind a trait | The abstraction would be larger than the behaviour it buys, and every host already has a loop |
