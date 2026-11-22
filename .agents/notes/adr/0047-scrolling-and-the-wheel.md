# ADR 047 — Scrolling is actions, and the WinUI wheel is a low-level hook

- Status: Amended by 048 (the wheel reaches a tracking program)
- Date: 2026-11-22
- Related: `crates/winterm/src/winui/wheel.rs`, `crates/winterm/src/winui/app.rs`,
  `crates/settings/src/actions.rs`, `crates/settings/src/keybindings.rs`,
  ADR 019, ADR 023, ADR 046

## Context

The Windows shell had no way to leave the live screen: the wheel did nothing and
`Shift+PageUp` reached the program. Scrolling was not a settings action at all,
so no chord could name it.

Two platform facts forced the shape of the fix:

- `windows-reactor-pre` 0.108 has no `PointerWheelChanged`, and a wheel aimed at
  the window does not appear in this thread's message queue either (`WH_GETMESSAGE`
  sees clicks and moves, never `WM_MOUSEWHEEL`). A message-queue hook cannot see
  it.
- A low-level mouse hook does see it, before the input stack routes it.

## Decision

- Scrolling is six settings actions — `scrollUp`, `scrollDown`, `scrollUpPage`,
  `scrollDownPage`, `scrollToTop`, `scrollToBottom` — with default chords:
  `ctrl+shift+up/down` for lines, `shift+pageUp/pageDown` and
  `ctrl+shift+pageUp/pageDown` for pages, `ctrl+shift+home/end` for the ends.
  Both shells run them against the focused or active viewport.
- The WinUI shell reads the wheel from a `WH_MOUSE_LL` hook. It accumulates
  notches for wheels whose point is inside the window, keeps a remainder smaller
  than one notch for the next frame, and passes the event on: XAML has nothing
  to do with it, and swallowing it would be a surprise for the rest of the desk.
- Typing or pressing a key returns the view to the live screen, so a scroll never
  hides the line being typed.

## Rationale

A chord that a file can rebind is the project's rule for window commands
(ADR 023), and scrolling is one. The wheel is host input, not XAML input, so
reading it where Windows offers it — before routing — is more honest than
pretending the binding reports it. Passing the event on keeps the hook a
read-only observer.

## Consequences

- The hook is global for the session and lives in one `#[allow(unsafe_code)]`
  module with a `SAFETY:` note per call, like the pty backends and the surface.
  Its filter is the window's own rectangle, so a wheel anywhere else is ignored
  and costs one rectangle test.
- A wheel over the tab strip scrolls the pane too, because the filter is the
  window, not the pane's inner rectangle.
- `Shift+PageUp`/`PageDown` no longer reach a program by default; a file that
  wants that back binds them to `unbound` (ADR 031).
- The wheel did not reach a mouse-tracking program: it always scrolled the
  viewport, because the hook has a screen point and not a cell. (Amended by
  ADR 048: a tracking program now receives it unless Shift is held.)

## Alternatives considered

- **Wait for the binding.** The wheel is the most-used way to reach scrollback,
  and there is no release that reports it.
- **Vendor a patched binding.** A path dependency on a fork of an unpublished
  pre-release is more to maintain than one hook.
- **Subclass the window procedure.** WinUI owns that procedure; a `WH_GETMESSAGE`
  hook would have been gentler but, as measured, never sees the wheel.
- **A `ScrollViewer` around the pane.** It would move the panel instead of the
  terminal's viewport, and its events are the same XAML ones that are missing.
