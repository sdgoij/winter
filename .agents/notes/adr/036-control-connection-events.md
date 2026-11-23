# ADR 036 — A control is a terminal and a connection; the widget is the viewport

- Status: Accepted
- Date: 2026-11-22
- Related: `crates/winter/src/{term_connection,term_control,term_events,term_settings,
  session}.rs`, `crates/server/src/{conpty,posix_pty}.rs`; ADRs 016, 018, 019

## Context

Upstream splits a terminal control three ways: `ITerminalConnection`
(`src/cascadia/TerminalConnection/`) is the shell; `ControlCore` owns the
terminal and raises events; `TermControl` is the XAML that paints and takes
input. This port had one `Pane` in `ui.rs` doing all three, holding the pty
session, the terminal, the viewport, the parser's output queue and the events
in one struct.

That was not only untidy. `Pane.exited` was recorded and never read, so a shell
that exited left a dead tab and nothing noticed; and the pty layer had no way to
ask whether the client was still running, because a ConPTY keeps its output pipe
open after the client exits.

## Decision

- **`term_connection::Connection` is the backend.** Five methods: drain, write,
  resize, `exited`, close. `session::Session` implements it; the control holds a
  `Box<dyn Connection>`, so a test drives a whole terminal from a byte buffer
  with no pty.
- **`server::Pty::try_wait` exists because a pipe is not a process.** The
  pseudoconsole does not close its pipe when the client exits, so
  `Session::exited` asks the process (`WaitForSingleObject` with no wait on
  Windows, `Child::try_wait` on Unix).
- **`term_events::TerminalEvent` is what the terminal reports**: `Bell`,
  `Notification`, `Exited`. The window drains them once a frame; the bell is
  reported once per flash and the exit once per terminal. The title is *not* an
  event: upstream raises it for XAML, this shell reads the tab's title every
  frame.
- **`term_settings::TerminalSettings` is what a terminal starts with**: the
  command, the tab's title and the colour scheme, resolved from the profile when
  a tab opens, as upstream's `TerminalSettings` is.
- **`term_control::Control` owns the terminal, the connection and what they
  report.** It has no egui type in its signature.
- **The viewport and the selection stay in the widget** (ADR 019), which is why
  `Pane` keeps `view`, the find bar and the scrollbar and only *drives* the
  control.
- **An exit closes the tab, and the last tab closes the window** — upstream's
  `closeOnExit`, and the behaviour a terminal is expected to have.

## Rationale

- **A trait makes the control testable.** `Control`'s tests feed it bytes,
  assert on the snapshot, the events and what it wrote back; no pty is spawned
  and no shell runs.
- **Three names for three jobs.** "The terminal asked for a bell" (event), "the
  pane started flashing" (view state) and "the window wants attention" (policy)
  are now separate decisions. Before, the bell count and the flash start were
  returned as a tuple from a method named `take_bells`.
- **The exit bug was structural.** A field that is written and never read is the
  signature of a missing consumer; naming the event made the missing consumer
  obvious, and the live check showed the pty could not answer the question.

## Consequences

- Closing a shell closes its tab; closing the last tab closes the window. A
  program that exits immediately (a one-shot command in a profile) closes its
  tab as soon as it is gone.
- `Connection::exited` is polled once a frame per pane; on Windows that is one
  `WaitForSingleObject` with a zero timeout, on Unix one `waitpid(WNOHANG)`.
- A second backend (the remoting row) implements the trait and nothing in the
  control changes.
- `Pane` is now the widget: painting, egui event translation and the viewport.
  A native window backend replaces `Pane`, not `Control` (ADR 018).

## Alternatives considered

| Alternative | Why not |
|---|---|
| A watcher thread that blocks on `wait()` | It needs the pty shared across threads, and the pump thread already owns the reader |
| Treat the end of the output pipe as the exit | Wrong on Windows: the pseudoconsole keeps it open |
| Raise the title as an event too | The tab strip and the window title read it every frame; the event would have no consumer |
| Move the viewport into the control | ADR 019 makes it application state; the widget drives it and the control answers questions about the buffer |
| Keep `Pane` as it was and add the exit check | The three responsibilities would still be one struct, and the next event would be another field |
