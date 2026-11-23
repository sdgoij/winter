# ADR 053 — Ctrl+C copies a selection and otherwise interrupts

- Status: Accepted
- Date: 2026-11-22
- Related: `crates/settings/src/keybindings.rs`, `crates/winter/src/winui/app.rs`, `crates/winter/src/app.rs`, ADR 023, ADR 046

## Context

Windows Terminal binds `ctrl+c` to `copy` and `ctrl+v` to `paste` in
`userDefaults.json`. Its copy path is written so that the key can still serve
its console meaning: `ControlInteractivity::CopySelectionToClipboard` returns
false when there is no selection, `TerminalPage::_HandleCopyText` marks the
action handled with that result, and an unhandled key reaches the terminal as
`^C`. Copy with a selection consumes the chord; copy without one does not.

This port bound `copy` to `ctrl+shift+c` and `paste` to `ctrl+shift+v`. There
was no `ctrl+c` chord, so the key always went to the program, and selecting
text and pressing `ctrl+c` interrupted the shell instead of copying.

## Decision

- The default keybindings bind `ctrl+c` to `copy` and `ctrl+v` to `paste`, as
  upstream does. `ctrl+shift+c` and `ctrl+shift+v` leave the defaults.
- The shell's action dispatch reports whether it consumed the chord
  (`Shell::perform` returns `bool`, and the egui shell's event loop decides per
  action). Every action consumes its chord except copy.
- Copy consumes the chord only when the active view has a selection. With none
  it copies nothing, does not mark the key handled, and the key continues
  through the ordinary input path, which encodes `ctrl+c` as `ETX`.
- A single left click selects nothing: it clears the previous selection and
  leaves an anchor a drag can start from, as upstream's `LeftClickOnTerminal`
  does. Only a double-click, a triple-click or a drag creates a selection, so a
  stray click cannot arm a one-cell selection that swallows the next `ctrl+c`.
- The selection is left in place after a copy, as upstream's default
  `dismissSelection` is false.
- The rule lives in the shells, not in the settings schema: the key is a
  binding like any other, and a user who rebinds or unbinds `ctrl+c` still gets
  the decline rule for whatever chord names `copy`.

## Rationale

A terminal's `ctrl+c` has to stay available: interrupting a program is the most
common thing a user does with it. Windows Terminal solves that by letting the
copy action decline, so one chord means both things depending on whether there
is something to copy. Hard-coding `ctrl+c` in the shell would have fixed this
build but broken rebinding, and would have diverged from the file-driven chord
model (ADR 023).

## Consequences

- A settings file that already lists `ctrl+shift+c` was written by an earlier
  build. The loader merges user chords over the defaults, so such a file keeps
  `ctrl+shift+c` as an extra binding and gains the new `ctrl+c` default; no
  migration is needed and nothing silently changes meaning.
- A fresh file lists only `ctrl+c` and `ctrl+v`, matching upstream.
- `ctrl+v` no longer reaches a program. That is upstream's behavior too; a user
  who needs the literal key can unbind the chord.
- A click clears the selection; a drag starts one on the first move away from
  the press cell. The anchor is shell state, so a pointer that is captured and
  then lost without a release leaves an anchor that the next press replaces.
- Copy still requires a selection to have any effect, so a chord bound to copy
  that the user expects to consume the key always will not.
- The find bar and the profile menu own the keyboard while open, so a declined
  copy there produces neither a copy nor an interrupt; that matches the other
  keys they already swallow.

## Alternatives considered

- **Keep `ctrl+shift+c` and add `ctrl+c` as a hard-coded shell key.** It fixes
  this one symptom but makes `ctrl+c` unrebindable and puts a chord in code that
  belongs in the settings file.
- **Make copy always consume the chord.** Then `ctrl+c` stops interrupting a
  program unless the user also binds another key, which is a regression from
  every Windows console.
- **Dismiss the selection after copying.** Upstream's default keeps it; a user
  who wants it dismissed can ask for it later as a setting.
