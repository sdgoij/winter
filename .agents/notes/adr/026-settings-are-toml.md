# ADR 026 — Settings are TOML with a version

- Status: Accepted
- Date: 2026-11-22
- Related: `crates/settings/src/parser.rs`, `lib.rs`; ADRs 022–025

## Context

ADR 022 chose JSON for `settings.json`, on the grounds that one reader means one
set of tests. The file is hand-written — the application has never written it —
so its worst property is that it cannot hold a comment. TOML is what Cargo and
most Rust tooling use for a hand-written file: comments, trailing commas, bare
keys, and no quotes around every key.

ADR 022 also noted that the schema would need a migration step "when a field
changes meaning", and that there is no version field.

## Decision

- The file is `settings.toml`, in the same directories
  (`%APPDATA%\winterm\...`, `$XDG_CONFIG_HOME/winterm/...`, else
  `$HOME/.config/winterm/...`). This supersedes ADR 022's format and file name;
  every other decision in ADR 022 stands.
- The root carries `version`, the schema version this build writes. A missing
  version is 1, and a version below 1 is read as 1. A version **above**
  `CURRENT_VERSION` is refused: `parse` returns
  `SettingsError::NewerVersion { found, supported }`, `load_with_report` reports
  it as `LoadError::Invalid`, and the defaults load.
- `parse` returns `SettingsError` — the TOML error, or the version refusal — and
  `to_toml` writes the document. The application still never writes it.
- Field names are snake_case throughout: `default_profile`, `cursor_color`,
  `selection_background`, `bright_black`. The `action` *values* stay camelCase
  (`newTab`), because they name upstream's commands rather than a table key.
- `font.size` accepts an integer or a float, because TOML distinguishes
  `size = 16` from `size = 16.0` and a hand-written file will use the first.

## Rationale

- **A hand-written file wants comments.** Every schema decision here is one a
  user may need to annotate in place.
- **A version is cheap now and expensive later.** The one thing a future
  migration cannot do without is knowing what it is migrating from.
- **Refusing a newer file is the version's job.** Fields an older build does not
  know are ignored, but a changed *meaning* is not ignorable, so the version,
  not the fields, is the gate.
- **serde is the schema.** The same derives read TOML; the tests moved, the
  normalization did not.

## Consequences

Positive:

- The example file is readable and commentable, and a schema change has a place
  to announce itself.
- A file from a newer build is refused with both versions in the message,
  instead of being half-read.

Negative:

- No migration from `settings.json`: there is no released build, so a file is
  renamed and converted by hand (or deleted; the defaults are the same).
- TOML has no null, so an empty `Option` field is omitted from the document.
  toml 1.x drops `None` itself; the round-trip test pins that.
- Integers are not floats in TOML, which is why `font.size` has a visitor.

## Alternatives considered

| Alternative | Why not |
|---|---|
| Keep JSON and add a version | The format's inability to hold a comment is the reason to move |
| JSON with comments (JSONC/JSON5) | Not a standard; serde needs another parser and other tools cannot read it |
| YAML | Indentation-sensitive around shell commands and argument lists; the plan's first draft offered both, and two readers meant two test sets |
| RON | Rust-shaped rather than configuration-shaped, with no external tooling |
| Require `version` in every file | A hand-written file should start with the thing it configures, not boilerplate |
