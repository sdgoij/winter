# ADR 064 — The WinUI shell builds its own tab strip

- Status: Accepted
- Date: 2026-11-24
- Related: `crates/winter/src/winui/app.rs`, ADR 051, ADR 052

## Context

ADR 051 put a `TabView` in the `TitleBar` control's left header. The strip
worked, but the tabs could not look like Windows Terminal's:

- `TabViewItem` in the `windows-reactor-pre` 0.108 binding exposes
  `is_closable`, `header`, `tag` and `content` — and nothing else. No
  background, no icon, no pointer events.
- The selected tab therefore could not be painted in the terminal's own
  background, an inactive tab could not take the title bar's, and a pointer
  could not make a tab hover.
- The `+` button belonged to `TabView` itself, so it could not be grouped with
  the drop-down beside it.
- The tab item is 32 DIP tall in a 40-DIP row, so the selected tab ended 8 DIP
  above the pane. The band between them was the title bar's colour, which is the
  one thing a tab strip that merges with its pane cannot have.

Nothing in the binding supplies these; they are not settings on `TabViewItem`
that are being missed.

## Decision

- The strip is a `StackPanel` of our own `Border`s, one per tab, placed in the
  same `TitleBar` left header. Each `Border` carries the state's fill, a 6-DIP
  top corner radius, the title and a close button.
- A tab is given **no height**. The horizontal panel's children stretch, so a
  tab is as tall as the row, and the selected tab's square bottom edge meets the
  pane with no band between them. A `min_height` of 32 keeps the tab usable if a
  future binding stops stretching the header slot.
- The selected tab's fill is the active terminal's default background, read
  from `Control::default_colors()` in the same `tick` that paints the frame, so
  a scheme change or an OSC 11 reply moves the tab and the pane together. The
  pane's `Border` is given the same colour.
- An inactive tab is the title bar's fill; a hovered one is the two mixed
  halfway.
- A window theme's `tab_row` and `tab` fills, when the settings file names
  them, win over the fills above; a fill the theme leaves out falls back to the
  terminal's colour. This is the same precedence the egui shell applies
  (ADR 035).
- The `+` and the drop-down are two `Button`s in a zero-spacing `StackPanel`:
  one group, centred in the row.
- Tab elements are keyed by the tab's id (`tab-<id>`), so closing a tab in the
  middle updates the elements that remain rather than rebinding handlers that
  captured a shifted index.

## Rationale

The three things the shell needs — a fill that follows the terminal, a hover
state, and an icon — are all per-item, and the item type exposes none of them.
Rebuilding the item in our own primitives is the only way to reach Windows
Terminal's look without an upstream change to the binding.

Filling the row is what makes the strip and the pane read as one surface. It is
also why the tab cannot carry a fixed height: the row's height belongs to the
`TitleBar` control and the caption, not to us.

## Consequences

- **Drag-reorder is lost.** `TabView` reordered its items and reported the new
  order; our strip does not implement dragging yet. `Tabs::reorder` and its
  tests remain, so the strip can call it once the pointer drag lands.
- The hover fill is derived from two fills rather than a theme resource, so a
  future light theme has to revisit `HOVER_MIX`.
- The row is the theme's `tab_row` fill, or the shell's own when the theme names
  none. The shell's default is a fixed dark grey, not a system colour: this
  snapshot has no way to read the theme's `TabViewBackground` resource.
- The close button is inside the tab, so a tab's pointer press selects it; the
  button's own click closes it. Both events fire, in that order, which is also
  what Windows Terminal does.
- The row's background is the `Border` behind the `TitleBar`, so the Mica
  backdrop still does not show through the strip (ADR 051).
- The drop-down's overlay position still comes from the header panel's measured
  width minus the button width (ADR 052); the group does not change that.

## Alternatives considered

- **Keep `TabView` and paint behind it.** A `Border` under the control cannot
  follow the selected item, cannot hover, and cannot close the 8-DIP band,
  because the band is inside the control's own layout.
- **Restyle `TabViewItem` through resource overrides.** The binding exposes
  `resource_overrides` on `Button` only, and a `TabViewItem`'s template brushes
  are not reachable through it.
- **Re-implement drag-reorder now.** It needs a pointer capture across tabs, an
  insertion indicator and a hit test on the drop target; it is a change of its
  own, and the strip is usable without it.
