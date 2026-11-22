# ADR 030 — The settings window edits profiles

- Status: Accepted
- Date: 2026-11-22
- Related: `crates/winterm/src/ui.rs`; ADRs 027, 028

## Context

ADR 028 gave the window three scalar fields and said it "does not create,
rename or delete profiles, schemes or bindings". Adding a second shell is the
next thing a user wants from that window, and a profile is a short form: name,
command, arguments, title.

## Decision

- The window edits the profile list: name, command, arguments and title per
  profile, a Remove button per row, and Add profile.
- A new profile starts named `profile N` with the platform shell, so it saves
  without a warning and without colliding with an existing name.
- Arguments are one text field, split on whitespace when the draft is applied.
  An argument that contains a space is the file's job; the window says so.
- Remove can empty the list. Save writes it, and normalization restores the
  default profile on the way back in, which the warnings report (ADR 027).
- The draft's dirty check is `apply_to(settings) != settings`, so a new editable
  field cannot forget to be compared.
- The default-profile drop-down lists the draft's names, not the file's, so a
  profile added in this session can be selected before it is saved.
- Schemes and keybindings stay file-only: twenty colour pickers and a chord
  editor are each their own project.

## Rationale

- **Profiles are a form; chords and colours are tables.** Four text fields fit a
  grid; a chord editor needs per-modifier validation and a key-capture widget.
- **No blocking validation.** ADR 027 already reports a blank name or command,
  and normalization makes both usable; a Save button that can be disabled for a
  reason the user cannot see would be worse.
- **One dirty rule.** Comparing the applied document with the settings in force
  covers every field the draft grows, including the list.

## Consequences

Positive:

- A second shell can be added, renamed or given arguments without leaving the
  window, and the default-profile choice follows the edited names.
- The draft's comparison now has one implementation rather than one per field.

Negative:

- Argument quoting is not supported in the window; a path with a space needs the
  file.
- Removing the profile a tab is running does not affect that tab, only the next
  one (ADR 025's rule for a profile change).
- Renaming a profile leaves `default_profile` naming the old name until the
  drop-down is changed; Save then normalizes it back to the first profile and
  reports the unknown name.
- Schemes and chords still need a text editor.

## Alternatives considered

| Alternative | Why not |
|---|---|
| One text field per argument with add/remove | More widgets and focus handling than a terminal settings window needs now |
| Validate names on save and refuse | Duplicates and blanks already have defined answers; refusing hides them |
| Keep the window scalar-only | Adding a shell is the most common profile operation |
| Edit schemes next | Forty colour fields need a picker and a preview, not a grid of text |
