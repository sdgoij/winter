# ADR 078 — The Linux release also ships an AppImage

- Status: Accepted
- Date: 2026-11-24
- Related: `scripts/package.sh`, `packaging/linux/AppRun`,
  `.github/workflows/ci.yml`; ADRs 055, 074, 077

## Context

ADR 077 turned the Linux archive into an app tree with a rootless installer.
That still asks a user to extract a tarball and run a script, and it leaves the
files spread across `~/.local`. The macOS answer is one bundle a user runs; the
Linux answer with the same property is an AppImage: one executable file with the
menu entry and the icon inside it, and no installation step.

Two facts constrain the tooling. `appimagetool` is not on the hosted runners,
and it is itself distributed as an AppImage, so CI has to run it extracted — a
runner has no FUSE. The tree keeps no binary assets (ADR 058), so the image must
be assembled from the same generated icon and desktop entry the tarball carries.

## Decision

1. **`scripts/package.sh` builds `dist/winter-<version>-<target>.AppImage` from
   the tree it already staged.** There is no second copy of the entry or the
   icon: the AppDir is filled from `dist/<name>/` plus
   `packaging/linux/AppRun`.
2. **The AppImage is optional to the script.** A tool comes from `APPIMAGETOOL`
   or `PATH`; when neither names one, the script says so and still produces the
   tarball. A missing tool must not fail a release, and a contributor's machine
   is not required to have one.
3. **The tag job supplies the tool.** The package job's Linux legs download
   `appimagetool-<arch>.AppImage` from the project's `continuous` release, run it
   with `APPIMAGE_EXTRACT_AND_RUN=1`, and export `APPIMAGETOOL` for
   `just package`. `ARCH` comes from `uname -m`, so the arm64 leg builds an arm64
   image.
4. **The AppDir is the conventional one.** `usr/bin/winter`; the entry in
   `usr/share/applications` and at the AppDir's root; the icon in
   `usr/share/icons/hicolor/256x256/apps` and at the root; the licences and the
   readme in `usr/share/doc/<name>`; and an `AppRun` that execs the binary beside
   it rather than a fixed prefix.
5. **The image is published with everything else.** The upload glob gains
   `dist/*.AppImage`, and the publish job's `sha256sum winter-*` covers it
   without a change.

## Rationale

- **It is the closest thing Linux has to the macOS bundle.** One file that runs
  from anywhere, with the entry and the icon inside.
- **Nothing new is invented.** The entry, the icon and the application id are
  ADR 077's; an AppImage adds a layout and a launcher script.
- **The release stays honest.** With no tool the archive is still produced, and
  the log says why the image is not; the tag job is the only place that has to
  provide one.

## Consequences

Positive:

- A user downloads one file, marks it executable and runs it: no install step,
  no root, no unpacking.
- The same AppDir is what a Flatpak or a deb recipe consumes later.
- The image is built by the runner that tested the code, so an AppImage and its
  tarball always come from the same build.

Negative:

- **The image carries no GTK.** It relies on the host's GTK 4.10 exactly as the
  tarball does; bundling, or a Flatpak runtime, remains open, and a host without
  the toolkit fails at load time.
- **The tag job now downloads a tool**, pinned only to the project's
  `continuous` build. A version pin is the follow-up if that build ever moves
  under us.
- **No AppStream metadata.** `appimagetool` accepts the image without it, but a
  software centre will show no description until a metainfo file exists.

## Alternatives considered

| Alternative | Why not |
|---|---|
| Build the AppImage only on a developer's machine | The release would have no single-file artefact, which is the point |
| `linuxdeploy` and its GTK plugin | The same download with more moving parts, when our script already builds the AppDir |
| Ship the `.AppDir` instead of an image | That is the tarball with an extra directory |
| Bundle GTK inside the image | Needs a bundling tool and tens of megabytes of payload; a Flatpak runtime is the principled form of it |
