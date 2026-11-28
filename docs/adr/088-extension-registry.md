# 088 — Extension registries are static TOML, and archives install beside the settings file

Status: Accepted.

## Context

ADR 086 made an extension a directory under a discovery root. An author
distributes one by handing a user a directory or a `.zip` and a line of prose;
there is no index to search, no version to choose and no record of where an
installed extension came from. The settings file already names extensions that
are disabled, and discovery already treats the settings directory as a root, so
the missing pieces are a published index, a source adapter and an install that
uses both.

The load model is also fixed at start: `Host::start_extensions` runs once, and the
extensions screen says a switch takes effect on the next start. That rule was
acceptable while installing meant unpacking a directory by hand; it is not
acceptable once the screen can install one.

## Decision

1. **A registry is one static TOML document at an HTTPS URL.** `schema = 1`,
   `name`, an optional `updated`, and one row per extension. The schema is
   strict: an unknown key is an error, exactly as the manifest is.
2. **A row lists releases, not a single version.** Each `versions` entry carries
   `version` (semver), `api` (the host major it needs) and a `source`. A user
   picks a release; the default is the highest semver whose `api` this build
   hosts.
3. **The source is a tagged union.** This build implements `archive`: an `https`
   or `file` URL, a SHA-256 checksum and an optional `path` to the directory
   inside the archive that holds `extension.toml`. A `git` adapter and an
   archive format beyond `.zip` are later variants; a release whose `type` this
   build does not know is listed but not installable, and does not fail the
   registry.
4. **The manifest's version is semver too.** The registry release's version must
   equal the downloaded manifest's version byte for byte. Semver ordering is
   used only for "which release is newer".
5. **Registries are identified by URL.** The file's `name` is a display label;
   the URL is what a provenance record stores. Two registries that list the same
   extension id are two rows, and the user installs from one of them.
6. **Installs land beside the settings file**: `<settings>/extensions/<id>/`.
   Discovery is unchanged, so a registry install and a hand-copied directory are
   read by the same walk. `.zip` files are extracted, not read in place; reading
   archives directly would need a `Store` abstraction over discovery, which
   `noctis` has and this loader does not.
7. **Verify, then install.** A bounded download is hashed while it streams, the
   archive is extracted into a hidden staging directory with `..`, absolute
   paths and symlinks refused, the staged tree is read by `Manifest` and
   `discover`, and only then is it renamed into place. An update moves the old
   directory aside first. A manual install with the same id is never replaced.
8. **A registry install leaves a provenance file** inside its directory:
   registry URL, version, archive URL, checksum and the install time. A
   directory without one is a manual install, whatever its name.
9. **The network is lazy and outside `crates/extensions`.** The default registry
   (`https://sdgoij.github.io/winter/extensions/registry.toml`) is on in a new
   `[extensions] registries` list, but nothing is fetched until the Search panel
   is opened. The last good copy is cached beside the settings file; the
   Installed panel does not fetch and therefore does not check for updates until
   Search has.
10. **Extensions reload without a restart.** The engine is rebuilt — holder
    taken, engine dropped, discovery re-run, a fresh engine frozen and swapped in
    — after an install, uninstall or update, after an enable/disable switch,
    after the settings poll sees a changed `[extensions]` table, and after a
    watched file changes. The extension's `marks` survive so
    `commandFinished` history is not replayed; the focus and title reset so the
    new engine observes the current pane once.
11. **Dependencies are displayed, not resolved.** A registry row may list
    `dependencies`, which the Search panel shows. The installer verifies them
    against the manifest's `load_after` after download but does not pull them.

## Consequences

- `crates/registry` owns the schema, validation and the merged search index. It
  sits above `extensions` (it reuses `is_id`, `is_relative_path` and `API`) and
  names no engine, window or transport. Fetching, cache and installation are the
  next slice; they add `ureq` (rustls), `zip` and `sha2`, none of which
  `crates/extensions` sees.
- `Manifest::validate` now refuses a version that is not semver. Every fixture
  and test moved from `"1"` to `"1.0.0"`. This is a manifest-validation
  tightening, not an `api` bump: `api` describes the host contract, not the
  file's spelling.
- A reload re-evaluates every loaded extension, not one. `Loaded` is
  self-contained, so a rebuild re-registers entries, modules and assets together;
  a per-extension reload, as Goats does, would leave asset changes needing a
  restart.
- The "takes effect on the next start" sentence leaves all three screens.
  `WINTER_EXT=0` still wins for the run.
- There is no signing. Trust is TLS, the checksum, the curated default registry
  and the user's decision to add a URL. The install confirmation says that an
  extension runs with the terminal's privileges.
- The default registry is a published `registry.toml`; CI validates it and the
  GitHub Pages job copies it. A registry with a higher `schema` is refused whole,
  so the format can change without an older build misreading it.

## Alternatives considered

| Alternative | Why not |
|---|---|
| A Git source first | It needs a git client on every machine, a fetch of arbitrary commits and a working tree to strip; an archive is one bounded download with one checksum, and the registry can point at any host |
| Read `.zip` files in place, as `noctis` does | Discovery would need a `Store` abstraction over every file read; extraction keeps the existing loader and the existing root rules untouched |
| Reload one extension at a time | The Goats host re-evaluates an entry but cannot re-register assets, so an asset edit needs a restart; a rebuild of a self-contained loader has no such gap |
| Resolve `dependencies` during install | A transitive install needs a second confirmation surface and a conflict policy; display first, resolve once the pack has real users |
| Keep the registry in `crates/extensions` | The loader must stay engine-free and network-free so an embedding can use it; a remote document and a downloader are a different concern |
| Sign the registry and the archives | No key distribution, no revocation story; the checksum plus TLS is the honest v1 |
| Put the registry in `settings.toml` | A registry is a remote document, not the user's configuration; the settings file names the URLs |
