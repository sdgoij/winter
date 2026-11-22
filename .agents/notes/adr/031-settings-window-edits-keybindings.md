# ADR 031 — The settings window edits keybindings

- Status: Accepted
- Date: 2026-11-22
- Related: `crates/winterm/src/ui.rs`, `crates/settings/src/actions.rs`; ADRs 023, 027, 030

## Context

The window chose a profile, a scheme and a size, and edited the profile list;
chords still needed a text editor. A chord is a short value with a small
vocabulary (the actions) and a syntax ADR 023 already parses, so a row of a text
field and a drop-down is all the window needs.

## Decision

- The window edits the keybinding list: a chord text field, an action drop-down
  and Remove per row, plus Add binding.
- **The list shown is the effective one** — the defaults merged with the file —
  and Save writes it whole. A saved file therefore names every binding in use,
  including defaults that were not changed. That pins them: a later change to
  `DEFAULT_KEYBINDINGS` does not reach a file the window has saved.
- A chord stays text in the draft. A row whose text does not parse keeps Save
  disabled and is named in the window; the document is not built at all.
- Add binding starts with an empty chord and `newTab`. The empty chord is
  invalid, so Save stays disabled until it is typed — the same rule, not a
  special case.
- `Action::ALL` is the drop-down's vocabulary (everything but `Unbound`) and
  `Action::name()` is each action's file name. A test writes every name through
  the document and parses it back, so a rename cannot drift from the schema.
- The window becomes resizable: two lists and a form no longer fit a fixed
  height.

## Rationale

- **A row is the whole editor.** The chord syntax is three modifiers and a key;
  the action is a closed set. There is nothing a key-capture widget would add
  but focus handling.
- **Text until save.** A half-typed chord is a normal moment while editing;
  parsing on every keystroke would either drop the text or show a flash of
  error for a state the user is passing through.
- **Invalid blocks the save.** Writing a document the parser would reject turns
  the user's click into an error in the window; refusing the click with the bad
  row named is clearer.
- **Names from the schema's own vocabulary.** `Action::name()` matching
  `serde`'s output is a test, not a convention.

## Consequences

Positive:

- A chord can be added, changed or removed without a text editor, and a
  typo cannot reach the file.
- The action list is one constant, which a command palette can reuse.

Negative:

- Saving the effective list pins the defaults the user never touched.
- Removing every row is not "no bindings": an empty list means the defaults
  (ADR 023), so the defaults come back on reload. The editor cannot produce a
  window with no chords.
- Two rows with the same chord are saved as written; the merge keeps the last,
  and the warning (ADR 027) reports it after the reload.
- A chord the host layout cannot produce is still typed as text; the window
  does not capture keys.
- Schemes remain file-only.

## Alternatives considered

| Alternative | Why not |
|---|---|
| A key-capture widget (press the chord to record it) | It needs focus policy, a cancel path and modifier tracking, and the text field already matches the file |
| Edit the user's bindings rather than the merged list | The normalized `Settings` no longer separates them; that would need a second field carried through the schema |
| Drop rows that do not parse on save | A silent deletion of what the user typed is worse than a disabled button |
| Offer `unbound` in the drop-down | In the window, removing the row is the same edit with no dead action |
