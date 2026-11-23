# A native Linux shell — scope

Status: proposal, waiting on a toolkit decision.
Related: ADR 018 (the egui shell), ADR 038 (crate split), ADRs 040–053 (the
WinUI shell this mirrors), `crates/winterm/src/winui/`.

## Goal

Replace the egui shell on Linux with one built from the desktop's own toolkit,
so a Linux user gets the window chrome, tabs, menus, dialogs and text input
their desktop expects — the same step Windows took with WinUI (ADR 040).

Definition of done is parity with the WinUI shell on the features the project
already ships: tabs in the title bar, a profile menu, chords, pointer
selection, scrollback with prompt marks, the find bar, the settings window,
clipboard and a bell. Plus two things Linux does better and should not lose:
primary selection and IME.

## What must not change

- `renderer` stays the only painter. Both native shells blit the software
  backend's pixels; a GTK/Qt canvas draws our buffer, not its own.
- `input`, `view`, `tabs`, `term_control`, `session`, `shell_integration` stay
  shared. A rule that is not Linux-specific belongs there, never in the shell.
- The emulation crates stay platform-independent. `cfg(target_os = "linux")`
  appears in `main.rs` and the shell module and nowhere else.
- No new `unsafe`. gtk-rs is safe to use, so the Linux shell should not need an
  exemption in the workspace's deny list (unlike `winui::surface`).

## What this machine can already do

Measured on the Fedora 44 WSL distro here, not assumed:

| Thing | State |
|---|---|
| Guest | Fedora Linux 44, WSL 3.0.5, kernel 6.18, 32 cores, 30 GB |
| GPU | `/dev/dri/card0` and `renderD128`; Mesa 26.2.5 with the `dzn` Vulkan driver over D3D12 |
| KDE | Plasma 6.7.4: `kwin_wayland`, `plasmashell`, `sddm`-derived `plasmalogin`, `krdp` |
| Toolkits (runtime) | GTK 4.22.4, libadwaita 1.9.3, Qt 6.11.1 |
| Toolkits (`-devel`) | **missing** for all three; `pkg-config gtk4` and `Qt6Widgets` both say no |
| Rust | cargo 1.98.0, matching `rust-toolchain.toml` |
| Sudo | passwordless |
| WSLg | present (WSLg 1.0.79) but not usable from this shell: `DISPLAY`/`WAYLAND_DISPLAY` unset, `/mnt/wslg/run/user/1000` is `root:root 0700` and empty, no X socket |
| Nested KWin | **works**: `kwin_wayland --virtual --socket wayland-wm --width 1280 --height 800` accepts Wayland clients (kcalc started cleanly) and registers `org.kde.KWin.ScreenShot2` on the session bus |

So: real KDE compositing and real Qt/GTK clients already run here; what is
missing is (a) the `-devel` packages to build against, (b) a screenshot path
(details under *Verification*), and (c) any WSLg wiring — a nested compositor
is the practical route, not WSLg's own.

## The decision: which toolkit is "native"

Linux has no single native toolkit; it has two. The honest framing is:

| | GTK 4 + libadwaita | Qt 6 + KDE frameworks |
|---|---|---|
| Looks native on | GNOME, and Plasma with Breeze-GTK | Plasma, exactly |
| Rust bindings | `gtk4` 0.11.5, `libadwaita` 0.9.2 — mature, maintained, heavily used | `cxx-qt` 0.10.0 (KDAB, maintained) or `qmetaobject` 0.2.10 (stale since 2023) |
| Build cost | `pkg-config` + `gtk4-devel` mated to the crate's feature level | Qt 6 dev + CMake + a C++ compiler + moc, per platform |
| Ready-made widgets | `AdwTabView`/`AdwTabBar` (tabs), `AdwHeaderBar`, `AdwSearchBar`, `AdwPreferencesWindow`, `GtkPopoverMenuBar` | `QTabWidget`, `QToolBar`, `QDialog`, `QMenuBar` |
| Our code in the shell | Rust throughout | a Rust/C++ boundary, and every widget interaction crosses it |
| IME / a11y | first class, AT-SPI over the toolkit | first class, also AT-SPI |
| Risk | looks like a GNOME app to a Plasma user until Breeze-GTK is applied | more moving parts in the build, less Rust-side API surface |

**Recommendation: GTK 4 + libadwaita.** It is the toolkit whose Rust bindings
are mature enough that the shell can be unsafe-free Rust end to end, it maps
almost every WinUI feature onto a widget that already exists (tabs, header bar,
search bar, preferences window), and Breeze-GTK is what Plasma itself ships to
make GTK apps look at home. The Qt route buys exact Breeze chrome at the cost of
a C++ bridge in the hottest part of the UI code.

If Plasma-identical chrome matters more than build simplicity, Qt via `cxx-qt`
is the alternative, and the rest of this document holds either way apart from
the widget names.

## Architecture

One directory, `crates/winterm/src/linux/`, chosen in `main.rs` the way
`winui` is (ADR 040):

| Module | Owns |
|---|---|
| `mod.rs` | `run()`, the `gtk::Application` and the entry point's contract |
| `window.rs` | `AdwApplicationWindow`; the tab strip is the header bar (`AdwTabView` + `AdwTabBar`), which is the Linux spelling of ADR 051 |
| `surface.rs` | `gtk::DrawingArea`: `TerminalData::refresh` → `Renderer::paint_frame` → pixels in a `gdk::MemoryTexture` appended to the snapshot |
| `input.rs` | `EventControllerKey`, `GestureClick`, `EventControllerScroll`, `EventControllerMotion` → `crate::input::Event` |
| `menu.rs` | the profile drop-down as a real `GtkPopoverMenu` off a `GtkMenuButton`, replacing WinUI's hand-drawn overlay (ADR 052) |
| `find.rs` | `GtkSearchBar` + `GtkSearchEntry` over the shared `find::FindBar` |
| `settings.rs` | `AdwPreferencesWindow` over `config::Draft`: spin rows, entry rows, combo rows |
| `clipboard.rs` | `gdk::Clipboard` for `CLIPBOARD`, plus `PRIMARY` for middle-click paste |

Rendering path, matching ADR 041's Direct2D blit: paint into the renderer's
software target, wrap the pixels (`gdk::MemoryTexture` borrows a `GBytes`), and
draw that texture into the drawing area's snapshot. Full-frame uploads are the
first cut — the same thing WinUI does — with dirty-row uploads as a later
optimisation if a 1080p frame at 8 MB per paint proves too costly.

## Wayland rules that will bite

- **No global grab, no window positioning.** Everything is per-window:
  `GtkShortcutController` for chords, no cursor warping, no `move()`.
- **Scale is per-surface and can be fractional.** The face must be rasterised at
  the surface's scale, exactly as ADR 043 does for the composition scale on
  Windows; a cached 1.0 face is the classic blurry-terminal bug.
- **CSD is the default**, so the header bar *is* the title bar — which is what
  the WinUI shell already looks like. Do not add a server-side decoration.
- **IME is asynchronous.** `preedit-changed` must drive the renderer's existing
  `Composition` preview; committing text goes through the same `input` path as
  typed keys.
- **Primary selection** (`PRIMARY`) is separate from `CLIPBOARD`: middle-click
  pastes it, and it must not disturb the clipboard.
- **Accessibility is expected, not optional.** A custom grid widget needs
  `GTK_ACCESSIBLE_ROLE_*` and a text abstraction, or Orca sees an empty window;
  the WinUI shell has the same gap with UIA, so treat it as a named milestone
  rather than a surprise.

## Verification

Two rigs, both real:

1. **Nested KWin on a virtual output** (works today):
   `kwin_wayland --virtual --socket wayland-wm --width 1280 --height 800`, then
   the app with `WAYLAND_DISPLAY=wayland-wm`. This is a genuine KWin session —
   real decoration, real input routing — with no display attached.
2. **RDP into a Plasma session** (`krdp` 6.7.4 is installed): closer to a user's
   machine, slower to script.

The one unsolved piece is **capture**: `grim` fails (KWin does not implement
`zwlr_screencopy`), `spectacle` fails (it wants X11 or the screenshot portal),
and KWin's `org.kde.KWin.ScreenShot2` needs an fd-carrying call — a first
attempt through `python3-gobject` stalled and is the first task of milestone 1.
Two fallbacks if it stays awkward: `weston --backend=headless-backend.so` with
its screenshooter (a Wayland client still runs, just not under KWin), or driving
`krfb`/`krdp` and grabbing the stream.

To build at all, the guest needs: `sudo dnf install gtk4-devel libadwaita-devel`
(Qt route: `qt6-qtbase-devel qt6-qtdeclarative-devel cmake gcc-c++`). `just` is
not installed there; `cargo` alone is enough.

## Milestones

1. **Rig.** Capture a screenshot of `kcalc` under nested KWin, and a script that
   starts it, runs an app, shoots it and tears down.
2. **Toolchain.** Install the `-devel` packages; build and run a hello-world
   window in the guest.
3. **Skeleton.** Window, header bar, drawing area; paint one static frame from
   `renderer` into it; screenshot it.
4. **Live terminal.** `Session` + `Control` wiring, key input, resize, scroll;
   `just package`-style parity is not needed yet, just `cargo run`.
5. **Chrome.** Tabs, profile menu (native this time), find bar, settings window.
6. **Linux extras.** Primary selection, IME preview, bell, notifications.
7. **Cut-over.** `main.rs` prefers the native shell on Linux, egui stays for
   macOS until its own shell exists; `just test` must stay green on a headless
   runner with no display.

## Open questions

- GTK 4 + libadwaita, or Qt 6 for exact Breeze chrome? (Recommendation above.)
- Do we keep egui buildable on Linux as a fallback (`--features egui`) during
  the transition, or cut over in one step?
- Should the settings window be the native preferences dialog only, or also stay
  available as the shared `config::Draft` form for parity testing?
