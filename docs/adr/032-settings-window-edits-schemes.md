# ADR 032 — The settings window edits colour schemes

- Status: Accepted
- Date: 2026-11-22
- Related: `crates/winter/src/ui.rs`, `crates/settings/src/palette.rs`; ADRs 024, 027, 031

## Context

The window could choose a scheme from the file and edit profiles and
keybindings; the twenty colours of a scheme still needed a text editor. The
schema already parses a spec (`#RGB`, `#RRGGBB`, `rgb:r/g/b`), so a text field
per colour is the honest editor.

## Decision

- The window edits schemes: a name and the twenty colour fields per scheme,
  Add scheme and Remove scheme.
- Each field is a text field with a swatch of the colour it parses to. **Empty
  means "leave the built-in colour"**, which is different from a text that does
  not parse: that one names the field and keeps Save disabled.
- The twenty field names live in one list (`SCHEME_COLORS`) used by the form,
  the reader and the writer. The round-trip test builds a scheme with every
  field set, so a missing arm in the reader or the writer cannot hide.
- The default-scheme drop-down lists the draft's names.
- Removing the scheme that `default_scheme` names leaves the name dangling; the
  reload reports it and the built-in palette paints (ADR 024).

## Rationale

- **The file's syntax is the editor's syntax.** A field holds what the file
  holds, so there is no conversion layer and no way for the window to write a
  colour the file cannot read.
- **Unset is a value.** A scheme that names only a background is the documented
  way to keep the rest of the palette (ADR 024); an empty field is how the
  window says that, and a colour picker would have to invent a value.
- **A swatch answers "did I type that right"** without a preview pane: the
  colour of the field is the colour the terminal will paint.
- **One list, three uses.** The form order, `scheme_color` and
  `set_scheme_color` all iterate `SCHEME_COLORS`, so the three cannot drift
  apart silently.

## Consequences

Positive:

- Every setting the schema holds is now editable in the window; the file is
  only needed for comments, quoting and `unbound`.
- Proving that the twenty names cover the type is a test, not a review habit.

Negative:

- Typing hex is the only way to pick a colour; there is no picker and no
  palette editor.
- Twenty rows per scheme make the window tall; the scheme list scrolls, but a
  scheme with many rivals is a long scroll.
- The swatch shows what the text parses to, so a half-typed spec has none.
- `rgb:` and the short `#RGB` forms are accepted but written back in canonical
  `#RRGGBB` after a save.

## Alternatives considered

| Alternative | Why not |
|---|---|
| A colour picker per field | It needs a colour to start from, so "unset" would become a value, and the file's spec syntax would be lost |
| A scheme preview pane (a mock terminal) | A useful second step, but the swatches answer the question the form raises |
| A table editor generated from the file | That is the text editor with extra steps |
| Keep schemes file-only | They are the setting with the most fields and therefore the most typos |
