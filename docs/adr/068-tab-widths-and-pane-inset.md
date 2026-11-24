# ADR 068 — The pane's text is inset, and the strip never reaches the caption

- Status: Amended by 069 (grid columns, hover, group size)
- Date: 2026-11-24
- Related: `crates/winter/src/winui/app.rs`, ADR 064, ADR 067, ADR 059

## Context

Four things were wrong with the strip and the pane once a real window showed
them:

- The new-tab group sat flush against the last tab, with no gap.
- The terminal's first column started on the pane's left edge, so text touched
  the border.
- The terminal's last column ran under the scrollbar, which is an overlay on the
  pane's right edge (ADR 059).
- With enough tabs open, the new-tab group was pushed to the right, under the
  window's own caption buttons.

The fourth is the dangerous one: the caption buttons are drawn by the system on
top of the content, so a pushed control is a control the user cannot press.

## Decision

- The header's two children — the strip and the new-tab group — are separated by
  `TAB_ROW_GAP` (6 DIP).
- The pane's `Border` is padded on the left by `PANE_PADDING_LEFT` (8 DIP). The
  swap chain starts there, so the grid's first column does. The padding shows
  the pane's own background, which is the terminal's background.
- The grid reserves the scrollbar's width: `grid_width` is the panel less
  `bar::WIDTH`, and both `grid_for` call sites use it. The scrollbar stays where
  it is — an overlay on the pane's right edge — and the last column now stops
  before it, so text wraps instead of running underneath.
- The inset shifts pointer coordinates. `Shell::cell_at` subtracts
  `PANE_PADDING_LEFT` before mapping a position to a column, and
  `bar::contains` is given `pane_width` (the panel plus the inset) so a press in
  the strip is still recognised.
- Tabs are **equal width**: `tab_room` is the pane less the new-tab group, the
  caption buttons and their gap, and each tab takes an equal share of what is
  left, clamped to `MIN_TAB_WIDTH` and `MAX_TAB_WIDTH` (240, upstream's cap).
  The title ellipsises into `title_room`, so a squeezed tab loses characters
  rather than overlapping its own icon and close button.
- The strip is also capped with `max_width(tab_room)`.
- The tab row is `TAB_ROW_HEIGHT` (28 DIP) rather than 32, so 4 DIP of the
  title bar's slot stay above a tab instead of it touching the window's top
  edge.
- The new-tab group is centred in the row rather than bottom-aligned.

## Rationale

Equal widths are what upstream's `TabView` does (`TabWidthMode="Equal"`), and
they are the only way to fit an unknown number of tabs into a known width: a
per-tab natural width would need a text measurement the binding does not expose.

Two belts for the caption buttons: the widths promise the tabs fit, and the
`max_width` cap on the strip means the group cannot move even if they do not.
Tabs past the cap overflow their slot instead of pushing it, and the group —
declared after the strip — paints over them.

The left inset is on the pane's `Border` and not on the grid, because the
pane's background is already the terminal's background: the padding reads as a
margin of the terminal, exactly as upstream's `padding` setting does, and the
scrollbar keeps its own edge.

## Consequences

- Pointer positions in the pane shift by 8 DIP. Any new pane handler must go
  through `cell_at`, not `pointer_cell` directly.
- Reserving the scrollbar's width always, not only while it is visible, costs
  about half a column. The alternative is a grid that reflows when the history
  crosses the pane's height, which is worse.
- Below `MIN_TAB_WIDTH` per tab the tabs no longer fit `tab_room`; they overflow
  their slot and the strip's cap keeps the group in place, so the rightmost tabs
  paint under the caption buttons. Upstream scrolls its tab strip; a
  `ScrollViewer` here is the follow-up.
- The tab's title is one line with a character ellipsis at every width, not only
  when squeezed.
- At extreme magnification the 8-DIP inset is less than a cell; the grid simply
  fits fewer columns.

## Alternatives considered

- **Pad the pane on the right instead of shrinking the grid.** The pane's
  `ContentPresenter` insets its child, so the scrollbar — which is right-aligned
  in that child — would move left with it, leaving a bare gutter at the window's
  edge.
- **Hide the tabs that do not fit.** Fewer, unpredictable tabs is worse than
  narrow ones, and it hides the tab the user just opened.
- **Measure each tab's title and size tabs to content.** The binding exposes no
  text measurement.
- **Put the new-tab group first and the strip after it.** The group would drift
  left and right as tabs come and go, which is not what Windows Terminal does.
