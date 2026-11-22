# ADR 025 — A saved settings file is reloaded by polling

- Status: Accepted
- Date: 2026-11-22
- Related: `crates/settings/src/watch.rs`, `crates/winterm/src/app.rs`,
  `crates/winterm/src/winui/app.rs`; ADRs 022–024

## Context

ADR 022 left the file unwatched and the application never wrote it. A terminal
that needs a restart to pick up a colour change is a terminal people stop
configuring.

Upstream watches the file with a platform notification API and reloads its
settings model. This port has no notification dependency, and its window is
already a frame loop that can be woken on a timer.

## Decision

- The application polls a `Stamp` — the modification time and the length — once
  a second in the shell's frame loop, and reloads when it differs. A file that
  appears or disappears is a change; deleting the file restores the defaults.
- `settings::watch` owns the comparison. There is no watcher thread and no new
  dependency.
- A reload applies:
  - **keybindings** at once, because the event loop reads the list per event;
  - the **font size** at once, because the cell metrics are recomputed each
    frame;
  - the **scheme** to every pane, whose palette is rebuilt from the built-in
    table and the new scheme. `Scheme::apply` resets the table first, so a
    colour the new scheme leaves out cannot keep the previous scheme's value or
    a program's OSC 4;
  - the **profile** to the next tab only. A running shell keeps its command.
- A reload that fails — the file is present but does not parse — leaves the
  session's settings in force and reports the error in the window. A save is
  not atomic here, and the watcher must not wipe a running session because it
  caught the file half-written. Startup keeps ADR 022's rule, where a broken
  file yields the defaults.

## Rationale

- **A poll is a watcher with no failure modes.** It cannot miss a directory
  event or hold a handle across a save; it costs one `metadata` call a second
  and one wakeup.
- **The stamp has two fields on purpose.** Time alone misses two saves inside
  the filesystem's timestamp granularity; length alone misses a same-length
  edit.
- **The rebuild is what applying a scheme means.** "The built-in palette plus
  this scheme" is also why a reload drops a program's OSC colours: the file is
  the user's later word.
- **Keep the session on a bad save.** The window is a view of a file the user
  is editing; the moment a save is half-written is exactly when it is most
  likely to be invalid.

## Consequences

Positive:

- Editing the file changes the terminal without a restart, and the window says
  when a save did not parse.
- The watcher is tested without a window: a missing file, a changed file and a
  deleted file.
- The same `Stamp` can serve a future settings editor's "modified on disk"
  warning.

Negative:

- A one-second poll means a one-second delay, and one wakeup a second while a
  settings path exists.
- A rewrite that changes neither the length nor the modification time is missed
  until the next change.
- A profile change does not restart running shells, so the window and the panes
  can disagree about the profile until a tab is opened.
- The application still does not write the file; editing stays in an editor.
- Both shells poll: `App::logic` on egui and `Shell::tick` on WinUI. A rejected
  save keeps the running settings; the settings dialog reports it (the egui
  window in its status row, the WinUI dialog above the editor).

## Alternatives considered

| Alternative | Why not |
|---|---|
| A platform notification API (`notify`) | A dependency and a thread for a file that changes rarely; the poll is one call |
| Comparing the file's contents every second | Reading and parsing the file every second to answer "did it change" |
| Loading the defaults when a reload fails | A half-written save would wipe the session's settings |
| Restarting every shell on a profile change | Loses a running program for a setting that only chooses the next one |
| Applying a scheme as a patch onto the live table | A colour dropped from the scheme would keep the old scheme's value |
