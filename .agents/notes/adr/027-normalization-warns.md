# ADR 027 — Normalization reports what it changed

- Status: Accepted
- Date: 2026-11-22
- Related: `crates/settings/src/validation.rs`, `keybindings.rs`, `lib.rs`; ADRs 022, 025, 026

## Context

ADR 022 makes every parse normalize: an empty profile list gets the default
profile, a blank command gets the platform shell, a `default_profile` that names
nothing falls back to the first profile, and so on. Those rules are what make a
hand-written file usable, but they are also guesses, and until now they were
silent: the window could show the *settings in force*, not that the file had
asked for something else.

## Decision

- `normalize` returns `(Settings, Vec<Warning>)`. A warning names what the file
  wrote and what was used instead.
- `parse_with_warnings` returns them; `parse` drops them, so existing callers are
  unchanged. `Loaded` carries them, and the settings window lists them under
  "Adjusted on load".
- A warning is **never** an error. The file loads and the shell starts; the
  warning explains a decision. Only a document that cannot be read at all is an
  error (ADR 022, ADR 026).
- serde leaves absent fields absent: `Settings` deserializes with an `empty()`
  default, and a `Profile`'s missing fields are empty rather than runnable.
  Otherwise a missing `default_profile` would look like a file that asked for
  `"default"`, and the window would report a guess the file never made.
- The warnings:
  - a profile with no name (it is called `default`) or no command (the platform
    shell runs);
  - `default_profile` naming nothing (the first profile is used);
  - a scheme with no name (dropped), or `default_scheme` naming nothing (the
    built-in palette paints);
  - two profiles or two schemes sharing a name (the first is selected);
  - a font size outside the range (clamped);
  - a chord bound twice (the last binding wins), or `unbound` naming a chord
    that was not bound.

## Rationale

- **A guess the user cannot see is a bug report waiting to happen.** "My
  profile is not starting" is answered by "default_profile names nothing; the
  first profile is used", and the window is where that sentence belongs.
- **Warnings, not errors, because ADR 022 said so.** A partial file is a feature;
  turning each correction into a rejection would undo it.
- **Typed warnings.** The window and the tests read the same values, so a new
  rule cannot be added with a string only one of them knows.
- **Absent is absent.** The serializer already writes only what is set; the
  reader has to match, or "missing" and "explicitly the default" become the same
  thing and the warnings start lying.

## Consequences

Positive:

- Every normalization rule already has a test for its effect; each now also has
  one for its warning.
- The window answers the two questions a hand-written file raises: what is in
  force, and what did the file get wrong.

Negative:

- Each normalization rule has to push its warning, so a new rule can be added
  without one. The tests are the guard, not the type system.
- The list is prose in the window: no line numbers, so a large file still needs
  the reader to find the profile or scheme by name.
- A very confused file produces a long list, and the window does not scroll its
  warnings separately from the settings rows.

## Alternatives considered

| Alternative | Why not |
|---|---|
| Reject a file whose values are odd | Undoes ADR 022's partial-file rule |
| Keep normalization silent and explain it in the README | The README cannot say which line of *this* file was odd |
| Store the warnings on `Settings` | They describe the parse, not the settings; a `Settings` built in a test would have to invent them |
| Normalize at each use site instead | The rules would be spread across the window, the profile lookup and the palette, and each would guess differently |
