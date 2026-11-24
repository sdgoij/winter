# ADR 074 — macOS ships as a `Winter.app` bundle

- Status: Accepted
- Date: 2026-11-24
- Related: `scripts/package.sh`, `Justfile`, `crates/winter/src/macos/icon.rs`,
  `crates/winter/src/macos/mod.rs`; ADRs 058, 072, 073

## Context

A release archive on macOS held a bare `winter` executable. Finder treats a
Mach-O file as something to hand to Terminal.app, so a double-click opened a
terminal window that ran a second terminal, with no icon in the Dock or the
app switcher. Launch Services also gives a bundle's process `/` as its working
directory, so the first tab's shell would start at the filesystem root.

The shell has an icon, but it is drawn rather than shipped (ADR 058): the same
`icon_draw` geometry feeds the egui window icon and the executable's Windows
resource. A macOS app wants an `.icns`, which is a set of PNGs at fixed pixel
sizes.

## Decision

1. **`just app` assembles `dist/Winter.app`.** `just package` puts the same
   bundle inside the release archive; Linux and Windows keep their archives.
   `scripts/package.sh` owns the layout for every host.
2. **The bundle is the platform's own document type.** `Contents/Info.plist`
   names the executable, the icon, the display name, `APPL` as the package type,
   the version from `Cargo.toml` or the tag, macOS 11 as the minimum,
   `NSHighResolutionCapable` and `NSPrincipalClass = NSApplication`. The
   identifier is `io.github.sdgoij.winter`.
3. **The icon is drawn by the binary, not checked in.** `WINTER_APPICON=<dir>`
   makes `winter` write the ten `.iconset` PNGs (16–1024, with the `@2x` pairs)
   from `crate::icon::rgba`, and exits before the shell starts; the packaging
   script compiles them with `iconutil`. The Dock image the shell sets at runtime
   and the bundle's icon are therefore the same drawing.
4. **The bundle is ad-hoc signed.** `codesign --sign -` gives a locally built
   app one identity. A Developer ID, notarization and a disk image are the
   release task below, not this one.
5. **A bundle starts its shell in the home directory.** When the executable is
   inside `.app/Contents/MacOS/` and the process was started at `/`, the shell
   changes to `$HOME` before the first pty. The raw binary keeps the directory it
   was started from, which is what a developer and `WINTER_SCREENSHOT` expect.
6. **Both macOS architectures are built on the Intel runner.** A macOS package
   builds the host target and the other `-apple-darwin` target; CI installs it
   with `rustup target add`, and each archive keeps its own target name. The
   Intel host links the arm64 slice out of the SDK, so a release never waits on
   an arm64 macOS runner. `WINTER_TARGET=<target>` packages one target, and the
   push gate cross-checks the other slice with `cargo check`.

## Rationale

- **Finder and the Dock work.** A bundle is how macOS names an application; the
  icon and the double-click follow from the document type rather than from a
  wrapper invented here.
- **One artwork, three homes.** The Windows resource, the Dock image and the
  bundle's `.icns` come from the same function, so they cannot drift and the
  tree still has no binary asset (ADR 058).
- **Packaging stays one script.** The Windows console host, the Linux archive
  and the macOS bundle are the same `package.sh`; CI's four hosts run the same
  recipe.

## Consequences

Positive:

- The macOS archive is an application a user can drag to `/Applications`,
  launch from Finder or the Dock, and recognise by its icon.
- An Apple silicon archive exists without an arm64 macOS runner, and the gate
  builds it on every push rather than discovering a break at a tag.
- The first tab opens in the user's home rather than `/`.
- `WINTER_APPICON` makes the icon reproducible and testable from the command
  line.

Negative:

- **An ad-hoc signature is not distributable.** A downloaded, quarantined copy
  fails Gatekeeper until the release is signed with a Developer ID and
  notarized; that needs an Apple identity the project does not have in CI yet.
- **The icon's edges are hard rather than antialiased.** The drawing is the
  binary-coverage geometry ADR 058 fixed; at 1024 it is accurate per pixel, and
  a supersampled version would be a change to every consumer of that drawing.
- **The other architecture is checked, not run.** CI's macOS tests execute the
  Intel slice; the arm64 slice is cross-built and `cargo check`ed, and the Intel
  runner cannot start it. A runtime failure specific to arm64 would surface for
  users, not in the gate.
- **A cross build needs its target installed.** `rustup target add` is a CI
  step and a one-time step for a local `just package`; the script says which
  target is missing.
- **`iconutil` and `codesign` are part of packaging.** Both ship with the
  developer tools; a host without them cannot produce a bundle.

## Alternatives considered

| Alternative | Why not |
|---|---|
| Ship the bare executable and a wrapper script | Finder still starts the script under Terminal.app, and the Dock shows a generic icon |
| Commit an `.icns` | A binary asset in the tree with no readable source, and a second copy of the drawing (ADR 058) |
| A disk image now | DMG layout and Gatekeeper acceptance are release work; a directory with the bundle is the part a user needs |
| A Developer ID signature in CI | No identity or notarization credentials are configured, and an unsigned-but-ad-hoc app is honest about what it is |
| Wait for an arm64 macOS runner | The pool is frequently unavailable, and a release must not depend on one being free; the Intel host can link both slices |
