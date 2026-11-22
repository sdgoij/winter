# ADR 052 — The profile menu is a custom overlay

- Status: Accepted
- Date: 2026-11-22
- Related: `crates/winterm/src/winui/app.rs`, `crates/winterm/src/winui/menu.rs`, ADR 042, ADR 050, ADR 051

## Context

Windows Terminal puts a drop-down next to the new-tab `+` that lists every
profile and, below them, `Settings` and `About`. Upstream builds it as a
`MenuFlyout` attached to a `DropDownButton`.

The WinUI shell uses the `windows-reactor-pre` 0.108 snapshot (ADR 042). That
snapshot exposes no flyout or menu it can use:

- `DropDownButton` has `content` and `on_click`, with no flyout relation.
- `MenuBar` and `MenuBarItem` exist, but `MenuBarItem` has only `title`, so a
  menu bar cannot be filled with rows.
- No `Flyout`, `MenuFlyout`, `MenuFlyoutItem`, `Popup` or `CommandBarFlyout`
  declaration is reachable; the underlying interfaces are in the crate's native
  bindings, not its public declarations.
- `TabView` has no header or footer slot that could host the drop-down beside
  its `+`.

## Decision

- A `Button` showing `▾` follows the `TabView` inside the title bar's
  `left_header`, as one horizontal `StackPanel`, so it sits immediately after
  the `+` (ADR 051).
- The menu itself is an overlay in the pane's `Grid`, like the find bar (ADR
  050): a `Border` with a solid fill, a 1-pixel border, a small corner radius
  and a vertical `StackPanel` of flat, left-aligned `Button`s. It hangs from
  the pane's top edge, under the drop-down button.
- The button and the strip share a `Grid` wrapper, and the shell observes that
  `Grid`'s composition host to learn its width. The menu's left margin is that
  width less the button's width, so its left edge lines up with the button's;
  it is clamped to stay inside a narrow pane. The `▾` and the strip's `+` are
  both 24 DIP high, and the `▾` carries the 13-DIP top offset the tab strip's
  own button has, so the two are even.
- Its rows come from `winui::menu::entries`: every profile in the settings file
  in file order, followed by `Settings` and `About winterm`. The list is a pure
  function of `settings::Settings` and is unit-tested on its own.
- A profile row opens a new tab through `TerminalSettings::for_profile` and the
  shell's shared `add_tab`; the new tab is active.
- `Settings` opens the existing dialog (ADR 049); `About` opens a small
  `ContentDialog` naming the build's version.
- The `▾` toggles the menu. `Escape` dismisses it, and so does a press in the
  pane. While it is open the pane forwards no key or character to the terminal;
  a bound chord still reaches the window, as with the find bar.
- Because the title bar is not under the pane's preview route, the root is a
  `Border` whose preview-key handler dismisses the menu when the focus is in the
  title bar. That is the only thing the root border does.

## Rationale

The overlay follows the two precedents the shell already has for a modal or
transient surface the binding cannot express: the find bar (ADR 050) and the
settings dialog (ADR 049). Both are ordinary views over shared models, so the
menu's contents stay testable without a window.

Keeping the list in `menu.rs` rather than inline in `view` means the ordering
rule — file order, then the two commands — is a unit test, not a screenshot.

## Consequences

- The menu hangs under the pane's top-left, not under the `▾`. The binding
  reports no control position, so the position comes from the header's measured
  width: that is the button's right edge, and its left edge anchors the menu.
- The 13-DIP offset and 32x24 button size mirror the tab strip's own button.
  They are not style resources, so a change to the TabView template would move
  the strip's button and leave the `▾` behind; re-measure if the binding's
  TabView changes.
- It is one flat list. There are no nested submenus and no arrow-key navigation
  beyond the buttons' own tab order.
- The root `Border` is load-bearing for `Escape`; removing it would leave the
  menu impossible to dismiss from the keyboard while the title bar has focus.
- When the binding gains a real flyout and `TabView` a footer slot, this overlay
  should be replaced by them: the model in `menu.rs` carries over unchanged.

## Alternatives considered

- **`TeachingTip`.** It is a teaching tip with a close button, not a menu; it
  has no row model and is dismissed by design.
- **`MenuBar` in the title bar.** `MenuBarItem` has no `items`, so it cannot
  hold the profile list.
- **The menu inside the title bar.** It would grow the bar's height and be
  clipped by the control's content area.
- **Placing the overlay in the root `Grid` at the button's position.** The
  binding exposes no way to read the button's bounds; the header's own measured
  width is the closest anchor it offers.
