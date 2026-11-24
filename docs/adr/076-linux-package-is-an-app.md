# ADR 076 — The Linux package is a desktop-integrated app, not a bare binary

- Status: Accepted
- Date: 2026-11-24
- Related: `scripts/package.sh`, `packaging/linux/`,
  `crates/winter/examples/write-icon.rs`, `crates/winter/src/icon_draw.rs`;
  ADRs 055, 058, 074, 075, 077

## Context

`just package` produced one archive per host: a folder with `winter.exe`, the
console host and the licences on Windows (ADR 055), and a bare release binary
with `LICENSE.md` and `README.md` elsewhere. On the two platforms with a
packaged shell that is enough: Windows puts `winter.exe` on a Start-menu tile
through its own installer, and macOS users are used to launching a binary from a
tarball or a shell.

Linux is different. The shell (ADR 074) is a GTK application whose only surface
is a window, and a bare ELF in a tarball gives the user nothing to click: no
application-menu entry, no icon in the dock, no install location. `docs/plan.md`
has carried "Linux packaging (AppImage, Flatpak, deb)" as unstarted work.

Two existing rules constrain the answer. The tree keeps no binary assets — the
icon is drawn by `icon_draw.rs` and embedded at build time on Windows (ADR 058).
And the release job builds the archives on stock runners, which carry no
app-packaging tooling.

## Decision

1. **The Linux archive is an app tree.** Beside `winter`, `just package` writes
   `org.winter.Winter.desktop`, `winter.png` and `install.sh`. The archive's
   name and location are unchanged, so the release job, the artifacts and
   `SHA256SUMS` do not move.
2. **The desktop file is named after the GTK application id.** The shell is
   `org.winter.Winter` (`linux::run`), and a Wayland compositor associates a
   surface with the `.desktop` file whose stem is that id; `Icon=winter` names
   the installed PNG instead.
3. **`StartupWMClass=winter`, measured rather than assumed.** Under Xvfb,
   `xprop -id <toplevel> WM_CLASS` reports `"winter", "winter"`: GTK 4 derives
   the class from the program name, not from the application id. Naming the id
   there would give the dock a second, icon-less entry beside the real one.
4. **The icon is generated, not committed.** `cargo run -p winter --example
   write-icon -- <path>` writes the same geometry the window shows, at the
   256 px a desktop entry installs. `png` is a dev-dependency, so the shipped
   binary links no encoder and the archive carries no asset nobody can read.
5. **Installing needs no root.** `install.sh` copies the binary to `$BINDIR`
   (default `~/.local/bin`), the entry to `$DATADIR/applications` and the icon
   to `$DATADIR/icons/hicolor/256x256/apps`, refreshes the desktop database and
   the icon cache when their tools exist, and `--uninstall` removes exactly
   those three files. `BINDIR` and `DATADIR` make the destinations inputs, so
   the same script installs into a temporary prefix in a test.
6. **The entry is validated when a validator exists.** `just package` runs
   `desktop-file-validate` on the staged file if the host has it; a typo in the
   entry is a broken install, and packaging is the last place to catch one.
7. **The tree is the substrate other formats consume.** An AppImage is built
   from it when a packaging tool is available (ADR 077); Flatpak and a deb stay
   in the plan, because each needs tooling the runners do not carry.

## Rationale

- **It is the smallest delta that gives Linux what the other shells have:** a
  launcher with the product's name and icon, installed per user, without a
  package manager or a daemon.
- **The icon cannot drift.** Rendering it from `icon_draw.rs` at package time
  keeps ADR 058's rule and removes the only binary asset the entry would need.
- **The archive keeps its contract.** Same name, same version in the name, same
  checksums; a Linux user gets an app where they got a binary and nothing else
  changes for the other hosts.

## Consequences

Positive:

- `./install.sh` puts Winter in the application menu with its icon, and
  `--uninstall` reverses it; `gtk-launch org.winter.Winter` starts the same app
  the menu does, which is the check the skill records.
- The tree is distro-neutral, so an AppImage or Flatpak recipe later reuses it
  without a second layout.
- `scripts/package.sh` honours `CARGO_TARGET_DIR`, so packaging works from a
  checkout whose build directory is elsewhere (the WSL rig).

Negative:

- **System GTK is the dependency.** The archive bundles neither GTK nor its
  themes; a host without GTK 4.10 runs nothing. Bundling is what a Flatpak
  runtime would add, and the AppImage of ADR 077 inherits the same gap.
- **The application id now names a file.** Changing it must move
  `org.winter.Winter.desktop` with it, or Wayland stops grouping the window
  under the entry.
- **Packaging builds the crate a second time** for the example — cheap after
  the release build, but the script now depends on a target that is not the
  binary.
- **One icon size.** 256 px is installed and desktops scale it; a size-tuned
  set (16/32/48) waits for a real complaint.

## Alternatives considered

| Alternative | Why not |
|---|---|
| An AppImage in `just package` | Adopted by ADR 077, conditionally: the image needs a downloaded tool, so the script builds it only when `APPIMAGETOOL` or `PATH` provides one |
| A `.deb` | Distribution- and version-specific, needs `dpkg-deb` and a Maintainer field, and CI packages two architectures of one distribution, not the distro family users run |
| A Flatpak manifest | Needs `flatpak-builder`, a runtime and a repository to publish to |
| Ship the icon as a committed PNG | Violates ADR 058's rule and puts the only unreadable file in the tree |
| Keep the bare binary | Leaves the Linux shell with nothing to click and keeps the plan item open |
