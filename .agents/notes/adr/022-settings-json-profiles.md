# ADR 022 — Settings are a JSON file of named profiles

- Status: Accepted
- Date: 2026-11-22
- Related: `crates/settings/`; ADR 001, ADR 018, `plan.md` Phase 10

## Context

Until now the application started `$SHELL` (Unix) or `%COMSPEC%` (Windows) and
had no way to change it. Phase 10 names a `crates/settings` crate, a JSON
schema, profiles, keybindings, themes and a settings UI.

Upstream's `settings.json` is a cascadia schema with profiles, colour schemes,
actions, keybindings and appearance. Most of it has no consumer in this port
yet, and a field no code reads is a field that drifts.

## Decision

`crates/settings` owns the schema, the parser and the path.

- The file is JSON at `%APPDATA%\winter\settings.json`, else
  `$XDG_CONFIG_HOME/winter/settings.json`, else
  `$HOME/.config/winter/settings.json`.
- The first schema is the profile:
  ```json
  {
    "profiles": [
      { "name": "default", "command": "cmd.exe", "args": [], "title": null }
    ],
    "default_profile": "default"
  }
  ```
- Every parse normalizes the result: an empty profile list gets the default
  profile, a blank command gets the platform shell, and a `default_profile`
  that names nothing falls back to the first profile. Unknown fields are
  ignored, so a newer file still loads in an older build.
- A missing, unreadable or invalid file yields the defaults silently. The
  difference is reported when a settings UI exists, not by failing to start.
- The crate has no workspace dependencies. The application converts a profile
  into `server`'s `PtyCommand`; the settings layer does not know a pty exists.
- Keybindings, themes, fonts and validation are **not** in the schema yet. The
  hand-written chord map in `ui.rs` stays until the settings that replace it
  are readable.

## Rationale

- **A schema only holds what is consumed.** A profile is the one setting the
  application acts on today; the rest would be dead configuration.
- **Normalization keeps a hand-written file usable.** A missing field should not
  need the UI to explain it, and a typo in `default_profile` should not leave
  the application with no profile.
- **A crate for the file, not for the app.** Parsing, defaults and the path are
  testable without a window or a pty, which is why the plan names a crate.

## Consequences

Positive:

- The shell is configurable, and the rules that make a partial file work are
  tested without a filesystem or a window.
- Later settings (colours, fonts, keybindings) have a home and a versioned shape
  to extend.

Negative:

- The file grows over the phase, so a migration step is needed when a field
  changes meaning. There is no version field yet.
- The application never writes the file, and does not watch it; both wait for
  the settings UI.
- A malformed file is invisible to the user until that UI exists.

## Alternatives considered

| Alternative | Why not |
|---|---|
| Adopt Windows Terminal's `settings.json` wholesale | Its profiles, schemes and actions target a host model with no counterpart here |
| YAML as well as JSON | The plan names both, but one reader means one set of tests and no ambiguity |
| Put settings in the `winter` crate | Parsing and defaults would be untestable without a window, and the plan already names the crate |
| Keep the shell hard-coded | The first user-facing setting is exactly this one |
