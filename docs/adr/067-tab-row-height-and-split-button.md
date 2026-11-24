# ADR 067 — The tab row is a fixed 32 DIP and the tab paints past it

- Status: Accepted
- Date: 2026-11-24
- Related: `crates/winter/src/winui/app.rs`, ADR 064, ADR 051, ADR 052

## Context

ADR 064 assumed the `TitleBar` control's left-header slot would stretch its
child, so a tab with no height of its own would be exactly as tall as the row
and its square bottom edge would meet the pane. Run against a real window, two
things were wrong:

- A band of title-bar colour stayed between the strip and the pane. The slot
  does not end at the pane's top; the control's own layout leaves its content
  short of the bottom.
- The close button showed no cross, and `+` and `▾` read as two buttons.

The close button was a `Button` with a `TextBlock` for its content. The stock
button style's padding leaves a 22-DIP box a few DIPs of content, so whatever
was left was not a glyph; and a `Border` used in its place must be given an
explicit transparent background, because a border with a null brush is not
hit-testable and the press falls through to the tab behind it.

## Decision

- The strip is `TAB_ROW_HEIGHT` (32 DIP) tall and bottom-aligned in the header
  slot. A tab fills it, so its height no longer depends on how the control
  sizes its header.
- A tab paints `TAB_OVERHANG` (12 DIP) past its slot: a negative bottom margin
  with matching bottom padding, so the label stays where it was and only the
  fill grows. This is upstream's trick — `TabRowControl.xaml` pads its tab strip
  by -1 "to prevent a small 1px gap ... between the tab item and the content" —
  at the size this binding needs. Anything past the row is painted over by the
  pane, so the overhang cannot overshoot visibly.
- A tab's close button is a `Border`: 20 DIP square, a 4-DIP corner radius, an
  explicit transparent background, and upstream's `U+E711` cross in the symbol
  font at 10 points.
- `+` and `▾` are one split control, as upstream's `SplitButton` is: a 24-DIP
  pill with a 4-DIP radius and a hairline between two 31-DIP halves, bottom
  aligned 4 DIP above the row (upstream's `Margin="0,4"`). Each half keeps its
  own press; the binding has no `SplitButton`.
- All three glyphs come from `SYMBOL_FONT`, which names both
  `Segoe Fluent Icons` and `Segoe MDL2 Assets` — upstream's
  `SymbolThemeFontFamily`. The codepoints are the ones upstream uses
  (`U+E710` plus, `U+E70D` chevron, `U+E711` close).
- The strip selects a tab **by id**, not by index. A press on a tab's close
  button bubbles to the tab after the close has run, and an index would then
  name whatever slid into that place; an id names nothing and the press is
  ignored. Both handlers ignore presses that are not the left button.

## Rationale

Fixing the tab's height at the number upstream uses, and painting past it,
gives a result that does not depend on how this snapshot lays out a `TitleBar`
— which is undocumented and, per ADR 051, has already surprised this shell once.
A negative margin is the one mechanism that can close a gap whose size we cannot
read.

The glyph font matters more than it looks: `U+E711` is in the Private Use Area,
so a missing font produces nothing at all rather than a boxed fallback. Naming
both symbol faces covers Windows 10 and 11.

## Consequences

- The tab's height is a constant again; a future binding that sizes the header
  slot differently changes `TAB_ROW_HEIGHT` or `TAB_OVERHANG`, not the layout.
- The close button is our own `Border`, so it has no focus ring, no keyboard
  activation and no tooltip. Upstream's close affordance has all three; this one
  is pointer-only until the strip has its own automation peers.
- The close button has no hover fill. It inherits the tab's, which is what a
  press over it shows.
- `+` and `▾` are two buttons in a `Border` rather than one control: the
  divider is painted, but there is no single focusable element and no keyboard
  path to the chevron.
- `Message::Select` carries an id, so `Tabs::select`'s index is derived at the
  handler instead of captured in the element.

## Alternatives considered

- **Keep the stretch and give the tab a `min_height` big enough to reach the
  pane.** The height needed is the row's, which the shell cannot read; a guess
  that is too small leaves the band, and one that is too large pushes the tab's
  rounded top above the row and clips it.
- **Move the strip out of the `TitleBar` into its own row above the pane.** It
  would be exactly sized, and the window would lose its drag region and its
  space for the caption buttons (ADR 051).
- **Let the pane's background cover the band.** The pane's `Border` spans the
  full width, so the terminal's colour would appear to the right of the last
  tab, under the buttons, which is not what Windows Terminal shows.
- **Draw the cross as two `Path`s.** The binding exposes no `Path`, `Shape` or
  `FontIcon`; a `TextBlock` in the symbol font is the only glyph source there
  is.
