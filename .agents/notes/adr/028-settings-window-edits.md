# ADR 028 — The settings window edits three fields and writes the file

- Status: Accepted
- Date: 2026-11-22
- Related: `crates/winter/src/ui.rs`, `crates/settings/src/parser.rs`; ADRs 022, 025–027

## Context

ADR 022 said the application never writes the file; the window reports it and
the poll reloads it, so the only editor is a text editor. That is right for a
scheme's twenty colours or a chord table, and poor for the two settings a user
changes on a whim: the font size and which profile or scheme is the default.

## Decision

- The window edits three fields — `font.size`, `default_profile` and
  `default_scheme` — with a drag value and two drop-downs built from the
  profiles and schemes the file already declared. It does not create, rename or
  delete profiles, schemes or bindings.
- **Save** writes the document with `settings::save` and then reads it back
  through the same path as startup, so what runs is the file's own settings,
  normalized, not the draft's idea of them. **Revert** re-reads without writing.
- A save writes a sibling temporary file and renames it over the target, so a
  crash or a concurrent reader never sees half a document.
- The window keeps a `Draft`. Whether it has unsaved edits is computed by
  comparing it with the settings in force, not flagged by each widget.
- If the file changes on disk while the draft has unsaved edits, the poll does
  not reload: it reports the conflict, and only Save or Revert resolve it.
- Closing the window discards the draft, and the window says so while dirty.

## Rationale

- **Three fields because a control beats a text editor for them.** A chord table
  with a drop-down per modifier is a project; a size and two choices are a row
  each.
- **Save reloads.** One path (read → normalize → apply) decides what is in
  force, so the window cannot save one document and run another.
- **Temporary file and rename.** The file is the source of truth and the watcher
  polls it; a truncated intermediate is exactly what the reload's "keep the
  session" rule is for, and it should not need to run on the user's own save.
- **A draft, not live editing.** Otherwise every nudge of the size drag would
  re-flow every pane; Save is the moment the user means it.
- **Absent stays absent.** Choosing the built-in palette writes no
  `default_scheme` key, and the drop-down offers only names that exist, so the
  editor cannot create the warnings ADR 027 reports.

## Consequences

Positive:

- The two settings people actually change no longer need a text editor or a
  restart, and a conflict with an external editor is visible instead of losing
  work silently.
- The writer exists for a later list editor, and it is tested without a window:
  directory creation, overwrite and no leftover temporary file.

Negative:

- Profiles, schemes and chords still need a text editor, and a user may expect
  the window to add a profile.
- The draft holds three fields; adding a fourth means extending `Draft`,
  `apply_to` and the editor together.
- A very narrow window cannot scroll the editor; the window is not resizable
  yet.
- The file has no lock, so two windows of this application editing at once is
  last-writer-wins. The conflict notice covers the common case (an external
  editor) on the second poll, not a simultaneous write.

## Alternatives considered

| Alternative | Why not |
|---|---|
| Edit the file live on every widget change | Each keystroke would reflow every pane and start the watcher's reload loop |
| Write directly over the file | A crash mid-write leaves the file truncated; the temp-and-rename costs three lines |
| Keep the draft in the settings crate | It is window state: it exists only while the window is open and is compared with what the window shows |
| Let the poll reload over a dirty draft | It silently discards the edit the user is in the middle of making |
| Full list editing now | The window has no scrolling or list model; it would dwarf the rest of the shell |
