# ADR 069 — A tab's parts are grid columns, and the header sits at the bottom

- Status: Accepted
- Date: 2026-11-24
- Related: `crates/winter/src/winui/app.rs`, ADR 067, ADR 068

## Context

Two follow-ups from the strip in a real window:

- A tab's icon, title and close button were a horizontal `StackPanel`. A
  horizontal stack measures its children with an unbounded width, so the close
  button sat right after the title instead of at the tab's right edge — with a
  short title it floated in the middle of the tab — and a title could only be
  kept inside its budget by an explicit `max_width` on the `TextBlock`.
- The close button had no hover of its own, so the only feedback was the tab's.
- The new-tab group was 24 DIP in a 28-DIP row and read as a small control
  floating high in the title bar, because the header `Grid` — a single `Auto`
  row — sized to its content and sat at the top of the slot rather than filling
  it.

## Decision

- A tab's body is a `Grid` with `[Auto, STAR, Auto]` columns and `TAB_GAP`
  between them: the icon, the title (the star) and the close button. A short
  title leaves the star column's space before the close button, so the close
  button is always at the tab's right edge; a long one is constrained by the
  column, so the trimming it already asks for actually happens.
- The close button tints `CLOSE_HOVER_MIX` (0.18) of the way from the tab's fill
  towards its label colour while the pointer is over it. The state is
  `Shell::hovered_close`, a tab id, fed by `Message::HoverClose` — the same
  shape as `hovered_tab`.
- The new-tab group is `BUTTON_GROUP_HEIGHT` (26 DIP) with 32-DIP halves, and
  its divider is 16 DIP. It is centred in the row.
- The header `Grid` is bottom-aligned in the title bar's slot, so the row it
  holds sits at the pane whatever height the control's slot turns out to be.

## Rationale

A star column is the only way to say "the title takes what is left" in a layout
that does not measure text. The `max_width` it replaces was a guess at the same
number computed from the wrong direction: it constrained the title from outside
instead of letting the row distribute its own width.

Bottom-aligning the header is the same move as ADR 067's overhang, from the
other side: the slot's height is the control's business, so the row is pinned to
the edge we can see rather than trusted to a stretch.

## Consequences

- `title_room` still exists — it caps the title — but the Grid now enforces the
  same budget. Its gaps are counted twice, because the grid has three columns
  whether or not there is an icon.
- The close button's hover is a fill, not a separate hover state on the glyph;
  the cross keeps the tab's label colour. On a dark tab the tint is a lighter
  square, on a light one a darker one.
- A stale `hovered_close` id after a close is harmless: the id names no tab, and
  the next pointer move clears it.
- The group is 26 DIP rather than upstream's 24, so it is 1 DIP clear of the
  row's edge on each side rather than 2. A taller group than the row would clip.

## Alternatives considered

- **Keep the stack and give the title a `min_width` so the close button reaches
  the right edge.** It is the same arithmetic as the grid with an extra failure
  mode: a title wider than the budget pushes the close button out of the tab
  instead of being trimmed.
- **Draw the close button outside the tab, in a column of the strip.** Then every
  tab would need a matching spacer and the strip's spacing arithmetic would
  change twice.
- **Hover the close button by swapping the glyph for a filled circle.** It is a
  different shape on hover, which moves the eye; a fill behind the same cross
  does not.
- **Give the header `Grid` a fixed height.** The slot's height is not something
  the shell can read, which is the same trap ADR 067 records.
