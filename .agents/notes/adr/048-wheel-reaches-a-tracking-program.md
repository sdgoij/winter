# ADR 048 — The wheel reaches a mouse-tracking program

- Status: Accepted
- Date: 2026-11-22
- Related: `crates/winterm/src/winui/app.rs`, `crates/winterm/src/winui/wheel.rs`,
  `crates/winterm/src/input.rs`, ADR 019, ADR 046, ADR 047

## Context

ADR 047 gave the WinUI shell a wheel, but the hook always scrolled the
viewport. A full-screen program that tracks the mouse — `vim` with
`set mouse=a`, `htop`, `less --mouse` — therefore never saw the wheel, while
presses and moves already reached it under ADR 046's rule.

The shell already records `pointer_cell`: the cell a press or a move was last
over. The hook reports a screen point, so the cell is available without
repeating the pane's coordinate math.

## Decision

- A wheel over the window is forwarded to the active terminal when it tracks the
  mouse and Shift is not held, aimed at `pointer_cell`. It uses the same
  `input::forwards_to_application(tracking, shift)` rule as press, move and
  release, so there is one definition of who owns the mouse.
- Shift keeps the wheel for the viewport. A person can always scroll back out of
  a full-screen program, which is the override ADR 019 already gives the
  pointer.
- If the terminal does not take the event — it is not tracking, or no cell is
  known yet — the viewport scrolls. That is the behaviour ADR 047 described, and
  it remains the fallback rather than a second path.

## Rationale

The wheel is a mouse event. A program that asked for the mouse expects all of
it, and the press/move rule is the wrong place for an exception. Reusing the
rule also keeps Shift meaning the same thing everywhere: "the user, not the
program, gets this event".

## Consequences

- A wheel before the pointer has entered the pane scrolls, because there is no
  cell to report. The first move or press fixes that.
- The reported cell is the last hovered cell, not the wheel's own point. The hook
  fires off the UI thread's event loop, and converting its point would duplicate
  the pane's origin and scale math for a sub-cell difference.
- When a program tracks the mouse, a wheel over the pane no longer scrolls the
  viewport unless Shift is held. A program that enables alternate scroll still
  receives the cursor-key encoding, which its own mode asks for.
- The hook remains a read-only observer: the event is passed on to Windows as
  before (ADR 047).

## Alternatives considered

- **Keep scrolling always.** This was ADR 047's explicit limitation; it makes
  the wheel dead inside the programs that most often have scrollback of their
  own.
- **Convert the hook's screen point for every event.** It repeats the pane's
  origin and composition-scale math, and the shell already knows the hovered
  cell.
- **Test Shift separately from the forwarding rule.** A second rule drifts from
  the first; `forwards_to_application` is already the single answer.
