# ADR 029 — The shell renders with the host's monospace family

- Status: Accepted
- Date: 2026-11-22
- Related: `crates/winterm/src/ui.rs`, `crates/renderer/src/font_cache.rs`; ADRs 008, 018

## Context

The egui shell painted every run with `FontId::monospace`: one embedded face
(Hack), upright only. SGR 3 therefore drew upright, and the port's own font
discovery — the renderer's `FontCollection` with its SFNT reader (ADR 008) — was
only used by the renderer's tests.

## Decision

- At startup the shell scans the platform's font directories, chooses a
  preferred fixed-pitch family, and registers its faces with egui under its own
  family names (`winterm-mono`, `winterm-mono-italic`).
- **A family that has an italic is preferred** over the first preferred family:
  the top choice can be installed upright-only — Cascadia Mono is on the machine
  this was written on — and choosing it would leave SGR 3 upright while a later
  preference has the face. The rule is
  `FontCollection::default_monospace_with_italic`.
- Every style comes from the one family, so the cell metrics stay consistent.
  The italic family lists the upright face after it as a fallback, and both
  families keep egui's own monospace list after that, for emoji and scripts the
  host face lacks.
- A style the family lacks falls back to the family's upright face; no host
  family at all falls back to egui's embedded monospace. Failure is silent: a
  machine with no discoverable fonts still opens a correct, upright terminal.
- Only italic is drawn from a face. Bold is not a rendering attribute here:
  SGR 1 is intense, `IntenseIsBright` is on and `IntenseIsBold` is off, which is
  the renderer's default. A bold face joins when a mode turns `IntenseIsBold`
  on.

## Rationale

- **A terminal should look like the host's terminal.** The family is the
  system's monospace, not a bundled one, which is also what makes italic
  possible.
- **One family per window.** Mixing a host italic with the embedded upright
  would give two advance widths for one grid.
- **The choice belongs in the renderer.** It is a rule over `FontCollection`'s
  metadata and is tested there with synthetic faces; the shell only reads files
  and registers them.
- **Silent fallback.** A missing font must not stop a terminal from opening.

## Consequences

Positive:

- SGR 3 draws in a real italic when the host has one, and the regular face
  becomes the system's monospace.
- The choice is tested without a window (synthetic faces in `font_cache`), and
  the registration is covered by an ignored host test that runs a headless egui
  pass and measures a glyph from each face.

Negative:

- The terminal's appearance now depends on installed fonts and differs between
  machines.
- Startup scans the font directories once; on a machine with many fonts that is
  a delay before the first frame.
- The chosen family may not be the preferred one when only a lower preference
  has an italic.
- A `.ttc` face is registered by index, so a host whose italic exists only
  inside a collection depends on egui honouring that index.

## Alternatives considered

| Alternative | Why not |
|---|---|
| Keep the embedded face | No italic, ever; the gap cannot close |
| Mix a host italic with the embedded upright | Two advance widths for one grid |
| Embed an italic font in the binary | A font licence in the repository and another bundled face to justify |
| Pick the first preferred family regardless of styles | On an upright-only install it silently loses italic, which the host test caught |
| Draw bold from SGR 1 now | It contradicts the `IntenseIsBright`/`IntenseIsBold` defaults; bright is what SGR 1 means here |
