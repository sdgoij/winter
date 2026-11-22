# ADR 051 — The WinUI tab strip lives in the title bar

- Status: Accepted
- Date: 2026-11-22
- Related: `crates/winterm/src/winui/app.rs`, ADR 040, ADR 042

## Context

The WinUI shell put the `TabView` in the first row of the client area, below the
system title bar, with the pane in the second row. Windows Terminal instead
makes the tab strip the title bar: the tabs start at the left edge, the caption
buttons stay at the right, and the terminal begins directly below. The two rows
wasted about a title bar's height and looked like a window with two title bars.

The binding is the `windows-reactor-pre` 0.108 snapshot (ADR 042). It exposes the
WinUI 1.7 `TitleBar` control, which sets `ExtendsContentIntoTitleBar`, hands the
system a drag region and keeps the caption buttons, and can host arbitrary
content in its centre and left and right headers.

## Decision

- The WinUI shell places the `TabView` in a `TitleBar` control's `left_header`.
  The control is the first row of the root `Grid`; the pane is the second. The
  back and pane-toggle buttons are hidden, and no separate title is drawn: the
  tabs are the title.
- The title bar row has a solid background behind the control, so the Mica
  backdrop does not show through the strip.
- The window title stays the active tab's title, for the taskbar and Alt+Tab.
  The `TitleBar` control's own `Title` is left empty.
- The row is pulled up by 8 DIP with a negative top margin. This snapshot leaves
  an 8-DIP strip of the system caption title above the control's content; the
  nudge covers it. Re-check the margin when the binding is updated.

## Rationale

The `TitleBar` control is the binding's supported way to extend the client and
keep the caption buttons. The left header is the one slot that is left-aligned
and sized to its content, which is what a tab strip needs; the centre content
slot centres its child, which put the tabs in the middle of the window.

Keeping the window title as the active tab's title costs nothing and keeps the
shell's taskbar and window switcher entries meaningful.

## Consequences

- The pane loses the separate tab row and gains that height; the grid re-fits
  from the panel's metrics as it does for any resize.
- The tab items and the `+` button remain interactive. The rest of the bar is
  the title bar's drag region, so dragging the empty strip moves the window.
- The negative 8-DIP margin is a workaround for a pre-release control, not a
  layout rule. If it is removed without a binding fix the caption title shows
  through above the tabs.
- A title bar row cannot show a separate window title without giving the tabs
  less room; Windows Terminal makes the same trade.

## Alternatives considered

- **The client row below the system title bar.** The current layout; it wastes
  the title bar height and does not match Windows Terminal.
- **The `TitleBar` control's centre `content` slot.** It centres the TabView, so
  the tabs sit in the middle of the window, not at the left edge.
- **`SetTitleBar` on a hand-built Grid.** The binding does not expose
  `SetTitleBar` or the interaction-region API, and the `TitleBar` control
  already does the drag-region work.
