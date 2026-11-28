# A native Linux shell — scope

Status: shipping — ADR 075. The GTK 4 shell is the Linux shell, and ADR 079
deleted the egui fallback, so the toolkit is required. What remains is named under
[Remaining gaps](#remaining-gaps): primary selection, a cell-addressable
accessibility tree, tab program icons and a sound sink.
Related: ADR 075 (the shipping decision), ADR 062 (the parking it supersedes),
ADR 061 (the GTK decision it amends), ADR 018 (the egui shell), ADR 038 (crate
split), ADRs 040–053 (the WinUI shell this mirrors),
`crates/gtk4/src/`.

## Goal

Replace the egui shell on Linux with one built from the desktop's own toolkit,
so a Linux user gets the window chrome, tabs, menus, dialogs and text input
their desktop expects — the same step Windows took with WinUI (ADR 040).

Definition of done is parity with the WinUI shell on the features the project
already ships: tabs in the title bar, a profile menu, chords, pointer
selection, scrollback with prompt marks, the find bar, the settings window,
clipboard and a bell. Plus two things Linux does better and should not lose:
primary selection and IME.

## Progress

| Milestone | State |
|---|---|
| 1. Rig | ✅ headless weston + `WINTER_SCREENSHOT` (see *Verification*) |
| 2. Toolchain | ✅ `gtk4-devel` 4.22.5 and `libadwaita-devel` 1.9.4 installed in the guest |
| 3. Skeleton | ✅ `crates/gtk4/src/` behind the `gtk` feature: a window, a drawing area, and a frame painted from `renderer` |
| 4. Live terminal | ✅ `Session` + `Control` over the profile's shell, keys through the shared `input` rules, resize on every frame, the wheel into history |
| 5. Host chrome | ✅ tabs and the profile menu, the find bar (`GtkSearchBar` + `GtkSearchEntry`), the command palette (±`WINTER_OPEN_PALETTE`), the settings editor (±`WINTER_OPEN_SETTINGS`), drag-reorder, the scrollbar with prompt marks, the pane inset, and splittable panes whose dividers drag (±`WINTER_SPLIT`) |
| 6. Linux extras | ✅ IME preedit with the candidate anchor, the screen as the pane's accessible description, bell flash and OSC 777 notifications; ⏳ primary selection |
| 7. Cut-over | ✅ The GTK 4 shell is the Linux build and CI installs `libgtk-4-dev` (ADRs 075, 079) |

`cargo run -p winter` builds the GTK shell on Linux, and the headers above are
required (ADR 079). `just linux` runs the GTK
build from either host.

## What must not change

- `renderer` stays the only painter. The pane is the payload `renderer`
  produces — the software backend's pixels or the GL backend's draws from the
  same payload; a GTK/Qt canvas never paints its own glyphs or rules.
- `input`, `view`, `tabs`, `term_control`, `session`, `shell_integration` stay
  shared. A rule that is not Linux-specific belongs there, never in the shell.
- The emulation crates stay platform-independent. `cfg(target_os = "linux")`
  appears in `main.rs` and the shell module and nowhere else.
- `unsafe` is confined to `crates/gtk4`'s `gl`, the OpenGL loader and its calls (ADR
  075); gtk-rs stays safe to use, and the rest of the shell needs no exemption.

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
| WSLg | 1.0.79; **off** in `%USERPROFILE%\.wslconfig` (`guiApplications=false`) at the time of these measurements, which is why `DISPLAY`/`WAYLAND_DISPLAY` were unset and `/mnt/wslg` was empty. Turned on later: `DISPLAY=:0`, `WAYLAND_DISPLAY=wayland-0`, a real window on the desktop. |
| Nested KWin | **works**: `kwin_wayland --virtual --socket wayland-wm --width 1280 --height 800` accepts Wayland clients (kcalc started cleanly) and registers `org.kde.KWin.ScreenShot2` on the session bus |

So: real Wayland clients and real KDE compositing already run here; what is
missing is (a) the `-devel` packages to build against and (b) a screenshot path,
which *Verification* below resolves as the app's own job. The capture rig is
headless weston; WSLg, once switched back on, is the interactive one.

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

One module, `crates/gtk4/src/lib.rs`, chosen in `main.rs` the way
`winui` is (ADR 040). It owns:

| Part | Owns |
|---|---|
| entry | `run()`, the `gtk::Application`, the window and the `GtkWindowHandle` title bar |
| strip | the tab strip's layout, hit testing, painting and drag-reorder over `crate::tabs` |
| pane | the base `DrawingArea`: every pane's fill, and the software blit |
| chrome | the `DrawingArea` above the GL area: the dividers, one scrollbar per pane with prompt marks, the bells and the focused pane's border (ADR 086) |
| gl | the OpenGL content path (ADR 076): one `GtkGLArea` above the pane drawing every pane into its own scissored viewport, the two pipelines, the glyph atlas, image textures, and the software fallback |
| input | `EventControllerKey`, the `IMContext`, `GestureClick`, `GestureDrag`, the scroll and motion controllers → `crate::input` |
| find | `GtkSearchBar` + `GtkSearchEntry` over the shared `find::FindBar` |
| settings | a `GtkTextView` over the whole TOML document, beside the profile `GtkPopoverMenu` |
| clipboard | `gdk::Clipboard` for `CLIPBOARD`; `PRIMARY` waits for the GTK 4.16 floor |
| a11y | the visible screen as the pane's accessible description; a cell-addressable `GtkAccessibleText` needs a custom widget class |

The module is about 3.5k lines. Splitting it along those rows is the next
cleanup, once the shell has been exercised interactively; the WinUI and macOS
shells are the shape to follow.

Rendering path: the engine produces one `RenderingPayload` per pane. The GL
backend (ADR 075) draws each payload into the one GL area's framebuffer at that
pane's grid rectangle, with a viewport and scissor switch between panes — solid
quads for backgrounds, grid lines and the cursor, a one-channel glyph atlas for
text, RGBA textures for images — and GTK composites the area into the window.
The software backend is the fallback: it rasterises the same payload, the pane
copies the pixels into a Cairo `ImageSurface`, and the same capture path reads the
window. The strip, scrollbars, dividers and margins are always Cairo, painted
above the content.

## Wayland rules that will bite

- **No global grab, no window positioning.** Everything is per-window:
  `GtkShortcutController` for chords, no cursor warping, no `move()`.
- **Scale is per-surface and can be fractional.** The face must be rasterised at
  the surface's scale, exactly as ADR 043 does for the composition scale on
  Windows; a cached 1.0 face is the classic blurry-terminal bug.
- **CSD is the default**, so the header bar *is* the title bar — which is what
  the WinUI shell already looks like. Do not add a server-side decoration.
  A `GtkHeaderBar` centres its title widget, though, and a start child never
  expands: the bar is a `GtkWindowHandle` around the strip with a
  `GtkWindowControls` at each end, so the tabs start at the left edge and the
  buttons still follow `gtk-decoration-layout`.
- **Cairo wants an A8 stride that is a multiple of four.** A glyph mask whose
  stride is the plain pixel width creates an error surface, and
  `cairo_mask_surface` then draws nothing without reporting one.
- **A GTK call can emit a signal before it returns.** `popup()` on the
  strip's menu emits `leave` on the motion controller, so a handler that
  borrows the shell while the menu is being popped aborts the process with
  `RefCell already borrowed`. Widget work happens after the borrow ends, and
  the cosmetic handlers (hover, the two draws) use `try_borrow_mut`, so a
  re-entrant signal costs one repaint instead of the window.
- **Claim presses in the strip, or GTK takes them as title-bar gestures.**
  `GtkWindowHandle` claims every primary double press to toggle maximise and
  a drag past the threshold to move the window; a press the strip leaves
  unclaimed becomes one of those. `GtkGesture::set_state` is the call —
  `set_sequence_state` needs an event sequence a mouse press does not have,
  which is how the buttons first came out dead.
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
headless output, so the shell snapshots its own toplevel behind
`WINTER_SCREENSHOT=<path>`, the way `WINTER_DEBUG_MARKS` drove a dump earlier.
That captures the client chrome and CSD as GTK draws them — what a compositor
would show — and needs no capture protocol. Popovers are separate surfaces and
are snapshotted individually when a test needs them.

Since GTK 4.22 `GtkWidgetPaintable` renders the **last frame GTK painted**
(`widget->priv->render_node`), not the live widget: it returns an empty node at
any moment a redraw is queued, because queueing one drops the cached node. The
shot therefore runs from the frame clock's `after-paint` signal, when the node
it reads is the one just drawn. A timer — milestone 3's first approach — races
the 16 ms pty tick and loses.

**WSLg is the interactive display, when it is on.** WSLg 1.0.79 provides
`DISPLAY=:0` and `WAYLAND_DISPLAY=wayland-0` with their sockets under
`/mnt/wslg/runtime-dir`. On this machine it was switched off in
`%USERPROFILE%\.wslconfig` (`[wsl2]` + `guiApplications=false`), which still
mounts `/mnt/wslg` but leaves it empty — no `wayland-0`, no `.X11-unix`, and
`/run/user/1000/wayland-0` a dangling symlink — and `wsl.exe --system` reports
`WSL_E_GUI_APPLICATIONS_DISABLED`. Setting the flag to `true` and restarting
WSL (`wsl --shutdown`) brings the sockets up; from then on a window opened in
the distro appears on the Windows desktop and needs no compositor of its own.
The headless weston rig stays for screenshots and for a runner with no display.

A shell profile must not export `DISPLAY` itself. A leftover
`export DISPLAY=$(ip route show | grep default | awk '{print }'):0` handed Qt a
route line as the display name (`could not connect to display default via
172.27.160.1 dev eth0 proto kernel onlink :0`), and GTK would have failed the
same way; WSLg sets both variables, so the profile line is deleted.

**WSLg's keymap is `us`, and it is not configurable.** Its system distro ships
no `weston.ini`, and nothing in the guest reaches the compositor, so a custom
Windows layout (`0409:A0000409` on this machine) never arrives: every GUI app
in WSL types QWERTY, `konsole` included — which is how we know the shell is
not the one dropping it. The fix belongs on the Windows side, where a scancode
remapper makes that `us` keymap produce the right letters. X11 would take a
keymap of its own, but X11 is not a path this shell takes.

To build at all, the guest needs: `sudo dnf install gtk4-devel libadwaita-devel`
(Qt route: `qt6-qtbase-devel cmake gcc-c++`). `just` is not installed there;
`cargo` alone is enough.

## Milestones

1. **Rig.** ✅ Headless weston up, a real client on it, and a window's own
   `WINTER_SCREENSHOT` snapshot producing a PNG.
2. **Toolchain.** Install the `-devel` packages; build and run a hello-world
   window in the guest.
3. **Skeleton.** ✅ Window, drawing area; one frame from `renderer` painted
   into it from a scripted connection. The cell is the face's own ascent plus
   descent (`FontDueRasterizer::vertical_metrics`), which keeps row zero inside
   the widget.
4. **Live terminal.** ✅ `Session` + `Control` wiring, key input, resize; the
   bar was `cargo run` on a real prompt in the guest, photographed by the shot.
5. **Host chrome.** ✅ The tab strip (tabs, close boxes, the new-tab button
   and the profile menu's button) is painted by this shell inside a
   `GtkWindowHandle`, with the profile list, the settings command and About in
   a `GtkPopoverMenu`. The find bar is a `GtkSearchBar` whose previous/next
   buttons and match count read like the other shells', the settings editor is
   one `GtkTextView` over the TOML document (a second request raises the open
   one), and a tab reorders when it is dragged past a neighbour. A tab wears
   its program's icon from the desktop icon theme (`utilities-terminal` when
   the theme names none), rendered through GSK once and cached as a Cairo
   surface. Ctrl+Shift+P opens the command palette: a card over the pane whose
   field filters the profiles and the actions the shell performs, with the
   arrows moving the selection and the bound chord shown beside each row.
6. **Linux extras.** ✅ IME preedit and commits with the candidate window at the
   caret, the screen as the pane's accessible description, the bell flash and
   OSC 777 notifications. ⏳ Primary selection.
7. **Cut-over.** ✅ Taken by ADR 075: the GTK shell is the Linux build and the
   gate installs the headers. ADR 079 removed the egui fallback with it.
8. **Package.** ✅ Taken by ADR 077: the Linux archive is an app tree — the
   binary, a desktop entry named after the application id, the generated icon
   and a rootless installer for `~/.local` — and by ADR 078: the tag job also
   builds an AppImage from the same tree.

## Remaining gaps

- **Primary selection.** GDK exposes `PRIMARY` as a clipboard only from 4.16;
  middle-click pastes `CLIPBOARD` today. Raising the floor to 4.16 is the
  follow-up and drops the Ubuntu 24.04 runner without a PPA.
- **A cell-addressable accessibility tree.** The visible screen is the pane's
  description, which Orca can read, but there is no `GtkAccessibleText` tree.
  That needs a custom widget class; macOS has the same shape of gap (ADR 072).

- **Sound.** ADR 033 stands: the bell flashes and DECPS is silent on Linux.
- **HiDPI scale.** The face, the cell and the GL viewport are physical pixels
  while GTK allocates logical ones; both content paths assume scale 1. A
  fractional scale needs the same treatment ADR 043 gives Windows.
- **GL resources survive a renderer rebuild.** A font-size change drops the
  backend but the context owns its objects until it is destroyed; deleting per
  rebuild is the follow-up.
- **The module split.** `linux/mod.rs` carries entry, strip, pane, gl, input, find,
  settings, clipboard and accessibility; splitting it is the cleanup named in
  *Architecture*.

## Open questions

ADR 079 removed the fallback: the GTK shell is the Linux shell, so a GTK
problem is a Linux problem.

- Should the settings window be the native document editor only, or also stay
  available as the shared `config::Draft` form for parity testing?
- When the GTK floor reaches 4.16, should `PRIMARY` and middle-click paste be
  part of the same change that raises it?
