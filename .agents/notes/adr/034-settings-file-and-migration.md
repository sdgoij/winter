# ADR 034 — A settings file always exists, and an older one is migrated once

- Status: Accepted
- Date: 2026-11-22
- Related: `crates/settings/src/{defaults,migration,parser,sgr_aliases}.rs`;
  ADRs 022, 026, 027, 032

## Context

Two kinds of installation reach this build with no `settings.toml`. A fresh one
has no file at all, so the window names a path that does not exist and the user
has nothing to edit. One that has been running since ADR 022 has a
`settings.json` — the JSON profile schema ADR 026 replaced — and its profiles
would be silently forgotten.

Neither case is an error, and neither is a reason to make the parser write
files: a load is a read.

## Decision

- **The defaults are one definition.** `defaults::settings()` is the base layer;
  `Settings::default` is it, and a test asserts that its TOML reads back through
  the normal path with no warnings.
- **`settings::ensure_file()` creates the file.** The application calls it once,
  at startup, before the first load. It writes the default document only when
  neither `settings.toml` nor the earlier `settings.json` exists. A file that
  exists — even an empty or broken one — is the user's and is left alone.
- **The earlier file is migrated, once.** When `settings.toml` is absent and a
  `settings.json` sits beside it, `load` reads version 0, normalizes it like any
  other document, and writes the result as TOML. The old file stays: deleting a
  user's file is not a migration. The window says which file was migrated.
- **Migration never guesses silently.** A profile with no command, or a default
  that names no profile, produces the same `Warning` a TOML file would (ADR 027).
- **A broken earlier file is reported and not overwritten.** The user still has
  it to fix, and no half-migrated TOML shadows it.
- **The scheme's colour names live in one table.** `SchemeColor` names the
  twenty colours, their palette slots and the two that move an alias; the
  scheme's getter and setter, applying a scheme and the editor's field list all
  read it. A test compares the table with what the file serialises, so a field
  added to `Scheme` and not to the table fails rather than drifts.

## Rationale

- **A file the window names should exist.** Upstream writes one on first run for
  the same reason: a settings window that says "no file" cannot be used.
- **Once, not every start.** Writing the TOML at migration time makes the second
  start read the current format, so the earlier path is not a second source of
  truth that has to keep agreeing with the first.
- **A read stays a read.** `load_with_report` keeps no side effects, which is
  what lets the parser tests load a path that does not exist without creating
  anything; the one write is a named call the application makes.
- **Keeping the old file costs nothing** and turns a mistaken migration into
  something the user can inspect instead of a loss.

## Consequences

- A fresh installation has a file to open, and the settings window stops naming
  a path that does not exist.
- An installation from ADR 022 keeps its profiles without the user doing
  anything, and can find out that it happened.
- The schema has one version to migrate *from* (0, the JSON file). A future 1 to
  2 change is a step in `migration.rs`, not a new module.
- `serde_json` becomes a direct dependency of `settings`; it was already in the
  tree for the GUI stack.
- `Scheme` has no remaining knowledge of its field names: they live in
  `sgr_aliases.rs`.

## Alternatives considered

| Alternative | Why not |
|---|---|
| Refuse to read `settings.json` | The profiles it holds are the user's, and ADR 022 promised the file would keep working |
| Delete `settings.json` after migrating | Destroys the only copy the user can inspect if the migration guessed wrong |
| Create the default file inside `load` | A read would write, and every test that loads a missing path would leave one behind |
| Write the file on every start where it is missing | Deleting the file is a legitimate way to ask for defaults; recreating it every second contradicts that |
| Keep the scheme's field names in the editor | That is what ADR 032 tried and it left four copies of the list |
