# A native Linux shell — scope

Status: decided — ADR 061. The milestones at the end are the plan of record.
Related: ADR 061 (the decision), ADR 018 (the egui shell), ADR 038 (crate
split), ADRs 040–053 (the WinUI shell this mirrors), `crates/winterm/src/winui/`.

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

So: real Wayland clients and real KDE compositing already run here; what is
missing is (a) the `-devel` packages to build against and (b) a screenshot path,
which *Verification* below resolves as the app's own job. The rig is headless
weston, not WSLg.

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

**Decided (ADR 061): GTK 4 for the host layer only.** The window, its menus,
its dialogs, IME and accessibility plumbing come from GTK; the tab strip, the
scrollbar and the grid are our renderer's pixels, as they already are on
Windows. Browsers split the UI the same way — the content and the chrome next to
it are theirs, the host integration is the platform's — and it is what settles
the dilemma: a Plasma user sees Breeze menus and dialogs around our tab strip,
not a GNOME application.

GTK over Qt for the host because the Rust bindings are mature enough that the
shell is unsafe-free end to end, because a widget exists for every feature the
WinUI shell has, and because `GtkAccessibleText` (GTK 4.14+) exists for a widget
that draws its own text. Qt keeps exact Breeze chrome as a later, optional
second shell rather than as a rejected idea.

## Architecture

One directory, `crates/winterm/src/linux/`, chosen in `main.rs` the way
`winui` is (ADR 040):

| Module | Owns |
|---|---|
| `mod.rs` | `run()`, the `gtk::Application` and the entry point's contract |
| `window.rs` | `GtkApplicationWindow`, its host menu (a `GtkPopoverMenu` off a menu button), and the drawing area that the chrome and the grid fill |
| `surface.rs` | `gtk::DrawingArea`: `TerminalData::refresh` → `Renderer::paint_frame` → pixels in a `gdk::MemoryTexture` appended to the snapshot; the tab strip and scrollbar are painted into the same target |
| `tabs.rs` | The tab strip's hit-testing and painting over the shared `crate::tabs`, so it looks the same as on Windows instead of like `AdwTabBar` |
| `input.rs` | `EventControllerKey`, `GestureClick`, `EventControllerScroll`, `EventControllerMotion` → `crate::input::Event` |
| `menu.rs` | The profile menu's rows, rendered by the host `GtkPopoverMenu` (a native menu, unlike WinUI's overlay — ADR 052) |
| `find.rs` | `GtkSearchBar` + `GtkSearchEntry` over the shared `find::FindBar` |
| `settings.rs` | `AdwPreferencesWindow` over `config::Draft`: spin rows, entry rows, combo rows |
| `clipboard.rs` | `gdk::Clipboard` for `CLIPBOARD`, plus `PRIMARY` for middle-click paste |
| `a11y.rs` | The grid as a `GtkAccessibleText`, so Orca can read the terminal |

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

Three things were measured on the guest, and they settle the rig:

- **A headless weston is the display.**
  `weston --backend=headless-backend.so --width=1280 --height=800
  --socket=<name>` creates a working Wayland socket; `kcalc` — a real Qt 6
  Wayland client — runs on it with no output and stays up, and weston enables
  its headless output at the requested size. No session, no seat, no WSLg.
- **A nested KWin runs clients but will not screenshot them.**
  `kwin_wayland --virtual` accepts clients and exposes
  `org.kde.KWin.ScreenShot2`, but the call returns
  `org.kde.KWin.ScreenShot2.Error.NoAuthorized`: the API is KAuth-gated and a
  nested compositor has no session to authorize it. `grim` cannot help (KWin
  implements no `zwlr-screencopy`) and `spectacle` wants X11 or the portal. So
  KWin stays useful for *interaction* checks — real decoration, real input
  routing — not for capture.
- **`labwc` headless is not usable here.** It exits before creating its socket
  (wlroots 0.18 has no pixman renderer, and its Xwayland fails on a dangling
  `/tmp/.X11-unix` WSLg symlink). Not worth pursuing when weston works.

**Capture is the app's own job.** `weston-screenshooter` produces nothing on a
headless output, so the shell snapshots its own toplevel with `GtkSnapshot` →
`GskRenderNode` → PNG behind `WINTERM_SCREENSHOT=<path>`, the way
`WINTERM_DEBUG_MARKS` drove a dump earlier. That captures the client chrome and
CSD as GTK draws them — what a compositor would show — and needs no capture
protocol. Popovers are separate surfaces and are snapshotted individually when
a test needs them.

WSLg is present (WSLg 1.0.79) but unusable from a plain shell: no `DISPLAY`, no
socket, and `/run/user/1000/wayland-0` is a stale symlink into
`/mnt/wslg/runtime-dir` — which is why every compositor that tried to bind
`wayland-0` failed until a private socket name was used.

To build at all, the guest needs: `sudo dnf install gtk4-devel libadwaita-devel`
(Qt route: `qt6-qtbase-devel cmake gcc-c++`). `just` is not installed there;
`cargo` alone is enough.

## Milestones

1. **Rig.** Headless weston up, a real client on it, and the shell's own
   `WINTERM_SCREENSHOT` snapshot producing a PNG — the first picture of our
   chrome on Linux.
2. **Toolchain.** Install the `-devel` packages; build and run a hello-world
   window in the guest.
3. **Skeleton.** Window, drawing area; paint one static frame from `renderer`
   into it, including the tab strip; screenshot it.
4. **Live terminal.** `Session` + `Control` wiring, key input, resize, scroll;
   `cargo run` in the guest is the bar, not packaging.
5. **Host chrome.** Native profile menu, find bar, settings window, native
   dialogs where they exist.
6. **Linux extras.** Primary selection, IME preview, bell, notifications.
7. **Cut-over.** `main.rs` prefers the native shell on Linux, egui stays for
   macOS until its own shell exists; `just test` must stay green on a headless
   runner with no display.

## Open questions

- Do we keep egui buildable on Linux as a fallback (`--shell egui`) during the
  transition, or cut over in one step? (Leaning: keep it selectable, ADR 061.)
- Should the settings window be the native preferences dialog only, or also stay
  available as the shared `config::Draft` form for parity testing?
