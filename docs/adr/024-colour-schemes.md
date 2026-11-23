# ADR 024 — Colour schemes are settings, and settings may use `types`

- Status: Accepted
- Date: 2026-11-22
- Related: `crates/settings/src/palette.rs`; ADR 002, ADR 022

## Context

ADR 022 decided the settings crate has no workspace dependencies, because the
profile schema needs nothing from the terminal: the application turns a profile
into a `PtyCommand`. Colour schemes are different. A scheme names terminal
colours, those colours live in `types::color_table::ColorTable`, and the grammar
a spec is written in is `Utils::ColorFromXParseColorSpec`, already ported in
`types`.

## Decision

- **`settings` depends on `types`, and only on `types`.** This amends ADR 022's
  no-workspace-dependency sentence; its profile and pty boundary stands.
- `palette.rs` owns `Color` (a `TextColor` read from a spec), `ColorError` and
  `Scheme` — the twenty colours upstream's scheme names.
- **A spec is read with the ported parser.** `#RGB`, `#RRGGBB`,
  `#RRRGGGBBB`, `#RRRRGGGGBBBB` and `rgb:r/g/b` are the same grammar Windows
  Terminal accepts, and a name the X11 table has not ported yet is rejected.
- The root grows `schemes` and `default_scheme`. The application applies the
  scheme to a pane's table when the pane is created, before the shell can write;
  a program's later OSC 10/11 then overrides it, as upstream's does.
- `foreground` and `background` fill the reserved slots
  (`DEFAULT_FOREGROUND`, `DEFAULT_BACKGROUND`) and move the aliases to them —
  the same slots OSC 10/11 use. `cursorColor` and `selectionBackground` fill
  `CURSOR_COLOR` and `SELECTION_BACKGROUND`.
- A colour the scheme leaves out keeps the table's built-in value. A scheme
  with no name is dropped, because nothing can select it. A `default_scheme`
  that names nothing selects no scheme and the built-in palette paints, which
  is deliberately unlike profiles: a typo should show the known colours, not an
  arbitrary scheme.
- A spec that does not parse rejects the file, and the defaults load, matching
  chords (ADR 023).

## Rationale

- **One colour type.** A settings-layer RGB triple would be a second colour
  vocabulary beside `TextColor`, which is ADR 002's bug even when both compile.
- **One spec grammar.** The xparse parser is already the port's answer to "what
  colour is this string", with its tests citing upstream; a hex-only copy in
  settings would drift from it.
- **The scheme is applied where the table is.** Painting reads the table every
  frame, so a scheme that wrote its colours anywhere else would need a merge at
  paint time and would fight OSC 10/11.
- **Partial schemes work.** A file that names only a background keeps a usable
  terminal, which is the same rule the profiles follow (ADR 022).

## Consequences

Positive:

- Colour schemes are user data and are tested without a window: the parser, the
  table writes and the alias moves each have a failing case.
- The reserved slots now have two writers (settings and OSC 10/11) with one
  precedence rule: later wins, and the program is later.
- A future settings UI has the schema for schemes and the picker's vocabulary.

Negative:

- `settings` now sits on `types`, so a change to the colour vocabulary can
  reach it. `types` has no I/O and no consumers of its own, so the edge stays
  shallow.
- A scheme cannot be chosen per profile yet; `default_scheme` applies to every
  pane.
- No named scheme ships in the crate: the built-in Campbell table is the only
  palette until a user writes one.
- The X11 colour-name table is still unported, so `red` as a spec is rejected
  even though upstream accepts it.

## Alternatives considered

| Alternative | Why not |
|---|---|
| Keep the spec string in the schema and parse it at paint time | A bad colour would surface as a missing colour, not as a rejected file, and the paint path would allocate |
| A settings-local `Rgb` type and a hex parser | A second colour type and a second grammar (ADR 002) |
| Resolve schemes in the application instead of the crate | The crate is where the file is tested; the application would grow schema tests |
| Fall back to the first scheme when `default_scheme` is missing | A typo would paint an arbitrary palette instead of the known built-in one |
| Draw `cursorColor` opaque | The chip would hide the glyph under it; the built-in cursor keeps the default alpha for that reason |
