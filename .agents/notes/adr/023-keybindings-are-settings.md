# ADR 023 — Keybindings are settings data

- Status: Accepted
- Date: 2026-11-22
- Related: `crates/settings/src/keybindings.rs`, `actions.rs`; ADR 021, ADR 022

## Context

The first shell hard-codes the chords the window owns in `ui.rs`: a match on
`(key, ctrl, shift, alt)` whose arms name `TabAction`s. ADR 021 put those chords
in the window because they must be consumed before the shell sees the key, but
the table itself was never meant to be the final answer. Phase 10 names
keybindings and actions as settings.

Upstream has two pieces: `KeyChordSerialization.cpp` parses the `keys` string
into a virtual key plus flags, and `AppKeyBindings.cpp` maps a command name to an
action. Both are Win32-bound (`VK_*`, `VkKeyScanW`), so the vocabulary has to be
rebuilt against the keys this shell can observe.

## Decision

`crates/settings` grows two modules: `actions.rs` holds `Action`, and
`keybindings.rs` holds `Modifiers`, `Key`, `Chord`, `Keybinding`, the defaults
and the merge.

- The file grows `keybindings`:
  ```json
  {
    "keybindings": [
      { "keys": "ctrl+alt+t", "action": "newTab" },
      { "keys": "ctrl+shift+c", "action": "unbound" }
    ]
  }
  ```
- **Chord syntax is upstream's.** `[ctrl+][alt+][shift+]<key>`, split on `+`,
  case-insensitive, modifier order free, parts trimmed. A second key part is an
  error, as it is upstream. Repeating a modifier is the same flag, not an error.
- **The vocabulary is what an egui key event reports.** Key names are `0-9`,
  `a-z` (uppercase accepted), a single printable character for punctuation, the
  named keys upstream lists (`enter`, `tab`, `space`, `backspace`, `insert`,
  `delete`, `home`, `end`, `pgup`/`pageup`, `pgdn`/`pagedown`, `esc`/`escape`,
  the four arrows, `f1`–`f24`), and `plus`, `comma`, `minus`, `period`.
  `vk()`/`sc()`, numpad, browser and media keys, and `win` are **not** accepted:
  the window cannot see them, so a binding for them would never match.
- **Actions are flat names.** Upstream passes an argument in a sibling field
  (`{ "action": "moveFocus", "direction": "left" }`); here the name carries it
  (`focusLeft`). Every command has no argument, so one shape needs one
  validation pass.
- **A chord matches only with the exact modifiers.** `ctrl+shift+t` is not
  `ctrl+t`, and `ctrl+alt+arrow` stays with the shell.
- **User bindings are applied over the defaults.** A chord the file names
  replaces the default with that chord; a new chord is appended; a repeated
  chord in the file keeps the last entry. `unbound` removes the chord and is not
  kept, so the key falls through to the shell, as it did before it was bound.
- **An empty list gets the defaults**, so the file can only unbind one chord at a
  time.
- **A broken chord or unknown action rejects the file**, and `load` falls back to
  the defaults. This is stricter than the unknown-field rule (ADR 022): an
  unknown *field* can be ignored, but a chord that cannot be parsed has no safe
  reading, and silently dropping it would leave a key doing nothing for no
  stated reason.

## Rationale

- **A chord is a typed value, not a string.** Parsing happens once, at load, so
  the event path is a comparison, and a typo fails loudly with the bad text in
  the error.
- **The default table has one home.** `DEFAULT_KEYBINDINGS` is a `const` slice;
  the shell's behaviour with no file is exactly the table this crate ships, and
  a test reads the same constant the application does.
- **Merge, not replace.** Replacing the whole list would force a user to restate
  sixteen defaults to add one chord; unbinding one chord is the operation users
  actually want (`ctrl+shift+c` for the shell).
- **Exact modifiers keep the shell's chords unreachable.** The application's
  rule that `Ctrl+Alt+Arrow` and `Ctrl+C` belong to the shell survives because no
  chord can match a prefix of another.

## Consequences

Positive:

- Which chords the window claims is user data, and the table is testable
  without a window.
- The settings UI has a schema to edit, and the chart of defaults is one
  constant.
- The action names are the vocabulary a command palette will use next.

Negative:

- The vocabulary is narrower than upstream's: no `win`, numpad, browser or
  `vk()`/`sc()` chords.
- Forward compatibility covers unknown *fields* only. A newer file that names a
  new action is rejected, and the defaults are loaded whole.
- No chord can be bound to a key egui does not name, and `Command` on macOS is
  not a modifier in the schema yet.
- There is still no settings UI, so a rejected file is visible only as the
  defaults.

## Alternatives considered

| Alternative | Why not |
|---|---|
| Keep the `command_for` match and read a configurable table on top | Two sources for one decision; the file could not override a default without the match deferring first |
| Store chords as normalised strings and compare strings in the event path | The event path allocates, and the type cannot say what a valid chord is |
| Upstream's object form (`action` + `direction`) | A second field to validate for commands that take no argument |
| Rejecting a file whose chords are unknown instead of loading defaults | `load` must never fail to start the shell (ADR 022) |
| Merging defaults by action rather than by chord | The user rebinds a key, not a command; the chord is the identity |
