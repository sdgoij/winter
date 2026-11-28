---
name: winter-linux
description: Build, run, live-check and package the GTK 4 shell in crates/gtk4/src. Use when changing the Linux window, tab strip, find bar, settings editor, pointer or keyboard input, IME, scrollbar, the OpenGL content path, the Cairo blit or the Linux package, or when a Linux build or headless screenshot behaves differently.
---

# The Linux shell

`crates/gtk4/src/lib.rs` is the Linux shell; `main.rs` chooses it when
the `gtk` feature is on, which it is by default (ADR 075). GTK owns the window,
the profile menu, the find bar, the command palette and the settings editor;
the tab strip, scrollbar and grid are our pixels, the split ADR 061 takes.
`winui`, `macos` and this module share `input`, `view`, `tabs`, `term_control`,
`session`, `find`, `palette`, `scrollbar`, `grid` and `terminal_render`; a rule
that is not GTK-specific belongs there.

## Shape

One module owns:

| Part | Owns |
|---|---|
| entry | `run()`, the window, the `GtkWindowHandle` title bar |
| chrome | the desktop's window palette (`kdeglobals`), the title-bar CSS and the strip's UI font |
| strip | layout, hit testing, painting, drag-reorder over `crate::tabs` |
| icons | each tab's program icon from the desktop icon theme, rendered through GSK once and cached as a Cairo surface |
| pane | the base `DrawingArea`: pane fills, the software blit, and the input controllers over the GL area |
| chrome | the `DrawingArea` above the GL area: dividers, one scrollbar per pane, bells and the focused pane border (ADR 087) |
| gl | the OpenGL content path (ADR 076): one `GtkGLArea` drawing every pane into its own scissored viewport; the two pipelines, atlas, image textures, fallback |
| input | key, IME, click, drag, scroll and motion controllers → `crate::input` |
| find | `GtkSearchBar` + `GtkSearchEntry` over `find::FindBar` |
| palette | a card over the pane: a filter field and clickable rows over `palette::Palette` |
| settings | a `GtkTextView` over the TOML document; Save/Revert/Close |
| clipboard | `gdk::Clipboard`; the read is asynchronous |
| a11y | the visible screen as the pane's accessible description |

## Build

The shell needs GTK 4.10 development headers. On this repository's Windows host
they live in the Fedora guest:

```sh
wsl.exe -d FedoraLinux-44 -- bash -lc 'cd /mnt/c/.../winterm && \
  cargo check -p winter --target-dir /home/t/winterm-target'
```

Use `/home/t/winterm-target` when the checkout is under `/mnt`: nine-mount I/O
makes an in-tree `target/` far slower. There is no headers-free build any
more: the GTK shell is the Linux shell (ADR 079), and the gate builds it.

The grid is drawn through GTK's OpenGL context when libGL/libEGL exists;
`WINTER_SOFTWARE=1` forces the CPU blit. Both paths are built either way.

## Package

`just package` builds the release and writes `dist/winter-<version>-<target>`
with the binary, `org.winter.Winter.desktop`, `winter.png` and `install.sh`
(ADR 077). Windows and macOS keep their own archives; only Linux gets the app
tree.

```sh
just package
tar -xzf dist/winter-*-linux-gnu.tar.gz -C /tmp/app
BINDIR=/tmp/prefix/bin DATADIR=/tmp/prefix/share /tmp/app/winter-*/install.sh
```

`BINDIR`/`DATADIR` keep a test out of `~/.local`; with neither set, the script
installs to `~/.local/bin` and `${XDG_DATA_HOME:-~/.local/share}`. Launch the
installed entry the way a desktop would — this needs the prefix on
`XDG_DATA_DIRS` *and* on `PATH`, since the entry's `Exec=winter` is a name:

```sh
setsid dbus-run-session -- env DISPLAY=:98 GDK_BACKEND=x11 \
  XDG_DATA_DIRS=/tmp/prefix/share:/usr/share \
  PATH=/tmp/prefix/bin:/usr/bin:/bin \
  sh -c 'gtk-launch org.winter.Winter & sleep 8; import -window root /tmp/menu.png'
```

`desktop-file-validate` is run by `just package` when the host has it, and
`install.sh --uninstall` should leave nothing behind but the regenerated
`mimeinfo.cache`.

### The AppImage

The same run also builds `dist/winter-<version>-<target>.AppImage` from the
staged tree, when it has a tool (ADR 078). Without one it prints a line and
still writes the tarball:

```sh
curl -fsSL -o /tmp/appimagetool \
  https://github.com/AppImage/appimagetool/releases/download/continuous/appimagetool-x86_64.AppImage
chmod +x /tmp/appimagetool
APPIMAGETOOL=/tmp/appimagetool just package

APPIMAGE_EXTRACT_AND_RUN=1 WINTER_SCREENSHOT=/tmp/image.png \
  ./dist/winter-*-linux-gnu.AppImage
```

The image needs the host's GTK like the tarball does — it bundles no toolkit —
and its `WINTER_SCREENSHOT` is identical to the binary's from the same build,
which is the check that the AppDir's `AppRun` reached the real shell.

## Live check

The shell photographs itself, so no display server is needed. A headless weston
is the rig:

```sh
mkdir -p /tmp/xdg-w && chmod 700 /tmp/xdg-w
XDG_RUNTIME_DIR=/tmp/xdg-w weston --backend=headless-backend.so \
  --width=1280 --height=800 --socket=winter-test --idle-time=0 &
sleep 2
XDG_RUNTIME_DIR=/tmp/xdg-w WAYLAND_DISPLAY=winter-test \
  WINTER_SCREENSHOT=/tmp/shot.png /home/t/winterm-target/debug/winter
```

`WINTER_OPEN_SETTINGS=1` also writes `/tmp/shot-settings.png` (the sibling of
the requested path with `-settings`), `WINTER_OPEN_FIND=1` opens the find bar,
`WINTER_OPEN_PALETTE=1` opens the command palette and `WINTER_SPLIT=1` splits
the first tab. A correct first frame with a split shows two prompts with a
one-pixel divider, a focus border and one scrollbar gutter per pane.

A HiDPI panel is the same run with `GDK_SCALE=2` in front of the command. The
strip, the grid and the scrollbar must scale together and fill the window; a
grid in the bottom-left quarter means the widget's scale factor was not
threaded through the cell and the renderer's target.

An installed Winter or a preview AppImage already owns `org.winter.Winter` on
the session bus, so a plain launch hands its activation to that process and the
new build never runs. `--new-instance` sets `NON_UNIQUE` and opens a separate
process; `dbus-run-session -- <binary>` does the same for one run.

On a machine with a display: typing reaches the shell; Ctrl+Shift+T/W; the
settings editor's Save writes and reloads; Ctrl+Shift+F opens the find bar and
Escape closes it; Ctrl+Shift+P opens the command palette, where typing filters
the rows and the arrows move the selection; Shift+wheel scrolls history while a tracking program still
gets the wheel; a saved `settings.toml` reaches the scheme and font within a
second.

### Driving pointer input without a display

A headless X server plus `xdotool` exercises selection and tab reordering for
real; Xvfb has no window manager, so the CSD window is the only client and
XTEST input lands in it. This is the check that caught the claimed-sequence
bugs, and it is worth re-running after any gesture change:

```sh
setsid Xvfb :98 -screen 0 1280x800x24 >/tmp/xvfb.log 2>&1 </dev/null &
setsid env DISPLAY=:98 GDK_BACKEND=x11 dbus-run-session -- \
  /home/t/winterm-target/debug/winter >/tmp/app.log 2>&1 </dev/null &
# `dbus-run-session` matters: GtkApplication forwards to an existing instance
# on the user bus and the new process exits at once.
export DISPLAY=:98
xdotool search --onlyvisible --name '^t@james'   # the toplevel's id
```

Use `xdotool mousemove --window <id> X Y`, then `mousedown 1`, more `mousemove`,
`mouseup 1`; type with `xdotool type` and `xdotool key`. The pane's origin is
`(5, 51)` in toplevel coordinates (the CSD border and the strip sit above it),
so row 0 starts at Y=51 and a cell is about 10 x 18 px at the default face. A
capture is `import -window <id> /tmp/shot.png`. Select with a drag and check the
highlight spans the rows it crossed; reorder by dragging a titled tab past
a neighbour's midpoint.

## Panes

A tab is `winter::layout`'s tree plus a `Vec<Pane>` (ADR 087). Each pane owns its
`Control`, `View`, `FindBar`, renderer, software pixels, `Gl` handle and grid.
The one `GtkGLArea` spans the whole pane area; before each pane's
`paint_frame`, `Gl::set_viewport` selects its grid rectangle (and the first pane
clears the whole framebuffer). The chrome lives in a second `DrawingArea`
above the GL area: painting it on the base widget would hide it under the
opaque GL surface.

- The pointer maps through `pane_at`, `cell_in` and `pane_scrollbar_rect`; a
divider within three points is grabbed before a pane, and `held.left` must be
set on the press (the divider and scrollbar branches run before the normal
`hold`).
- The focused pane owns keys, IME, find and the wheel; the wheel falls back to
the focused pane, and selection stays in `press_pane`.
- A split spawns its new pane at `pane_grid(split_second(rect, axis))`, so the
new shell lays out its first prompt for the width it keeps.
- Ctrl+Shift+W, or the palette's **Close pane** row, closes the focused pane;
closing the last pane closes the tab. The strip's close box closes the whole
tab. `closeTab` is the action name, but its rule is the focused pane (the
shared action's doc says so).

## Traps

- **The GL area is above the base widget and opaque.** Do not paint dividers,
  scrollbars or the focus border on the base `DrawingArea`; they belong on
  `chrome_area` (ADR 087).
- **A claimed sequence stops everything in another group.** `GtkGestureClick`
  claims the sequence when the shell handles a press; GTK denies every other
  gesture group on the widget and stops propagating the sequence to the motion
  controller. Put a `GtkGestureDrag` in the same group with
  `click.group_with(&drag)` or it never receives a move after the press. The
  pane and the strip both do this.
- **A `Cell` cloned for two callbacks is two cells.** `Cell<T>` copies its value,
  so `let begin = start.clone()` hands the second callback the initial `(0, 0)`.
  Share an `Rc<Cell<_>>` between `drag-begin` and `drag-update`, or drag points
  are offset by the press position (a tab drag then lands one slot short).
- **Extend while the button is held, not while a press is pending.** The first
  drag update hands the press's anchor to the selection and clears `press`; a
  condition on `press.is_some()` selects one row and then stops.
- **The pane begins at `(5, 51)` in toplevel coordinates.** A click below the
  titlebar in window coordinates is `y - 51` in the drawing area; a driver that
  uses raw window coordinates presses the strip instead.
- **GL calls only run in the GL area's `render`.** That signal is the only
  place GTK makes the context current. The engine's backend is created lazily on
  the first frame for that reason; never call the backend from the tick or the
  base drawing area.
- **The engine's backend is fixed at creation.** A failed context or frame
  cannot swap it in place; the shell rebuilds the renderer with the software
  backend and hides the GL area.
- **The GL area is an overlay child above the base drawing area.** Its margins
  are the pane inset and the scrollbar's gutter, so its framebuffer is exactly
  the grid; the base widget paints the fill and scrollbar. Input controllers sit
  on the overlay, because the GL area would otherwise swallow the pointer.
- **A capture includes the GL area.** `GtkWidgetPaintable` composites it, so
  `WINTER_SCREENSHOT` needs no readback; do not add one without a case the
  composite misses.
- **GDK may give a GLES context.** Under Xvfb + llvmpipe this host reports
  `OpenGL ES 3.1`, so both shader variants matter; `gl.version().is_embedded`
  selects `#version 300 es` (or a desktop 330 core). A desktop context older
  than 3.3 or GLES older than 3.0 falls back.
- **GDK exposes no entry-point resolver.** `gl.rs`'s loader tries
  `glXGetProcAddressARB`, then `eglGetProcAddress`, then the libraries' own
  symbol tables; core 1.1 names are not guaranteed by the proc lookups.
- **The atlas is `R8`/`RED` with `UNPACK_ALIGNMENT` 1.** Image bands arrive as
  `0xAARRGGBB` and are swizzled to RGBA once per revision, because BGRA uploads
  are not portable to GLES.
- **A child of a `Box` gets zero height without `vexpand`.** A `DrawingArea`
  has no natural size; without `set_vexpand(true)` the box allocates it nothing
  and the pane paints no frame (a white window with a live strip).
- **`GtkWidgetPaintable` pictures the last frame GTK painted.** A queued redraw
  drops the cached node, so the shot runs from the frame clock's `after-paint`,
  never from a timer that can race the 16 ms tick.
- **`GtkWindowHandle` claims double presses and drags.** The strip must claim a
  press it handled with `Gesture::set_state`, or a click on a tab becomes a
  maximise gesture.
- **A GTK call can re-enter a borrowed shell.** A popover grabbing emits `leave`
  on the strip. Widget work happens after the borrow ends; cosmetic handlers use
  `try_borrow_mut` so a re-entrant signal costs a repaint, not the process.
- **Clipboard reads are asynchronous.** `read_text_future` runs on the main
  context; the shell raises `paste_requested` and the tick starts the read, so
  no `RefCell` borrow crosses the await.
- **The input method filters keys before the key controller.** Call
  `filter_keypress` before taking the shell's borrow: a commit is emitted
  synchronously from inside it. Marked text goes through the shared
  `terminal_render::composition`, and the candidate window follows
  `Shell::caret_rect`.
- **A search entry delegates focus to its inner text.** `entry.has_focus()` is
  false while someone types into a `GtkSearchEntry`, so the window's key
  controller asks the window for its focus widget and walks up to the entry
  (`focus_is_within`); `has_focus` sends the text to the terminal and lets the
  find tick overwrite the field.
- **The search bar's close button turns search mode off without
  `stop-search`.** Follow `notify::search-mode-enabled` or the next tick reopens
  it.
- **Opening the command palette clears its field while the shell is borrowed.**
  `changed` runs synchronously and its `try_borrow_mut` fails, which is fine:
  the open path rebuilds the rows itself. Typing later runs the same handler
  with no borrow held.
- **GTK measures widgets in logical pixels; the face is physical.**
  `Widget::scale_factor()` is the bridge. On a fractional desktop (KDE at 1.7x)
  it is `ceil(1.7) = 2`, which is also what `GtkGLArea` multiplies its
  framebuffer by; use it, not the fractional monitor scale, or the frame sits in
  a corner of a supersampled buffer. The face, the cell and the renderer's
  target are `size * scale`; `fit` and `fit_grid` multiply the pane's logical
  size by the scale before dividing by the cell; pointer, wheel and IME caret
  positions divide the cell by it; and the software blit draws the physical
  frame at `1/scale`. Forget any one of those and a HiDPI panel shows a quarter
  of the window, a stretched frame or clicks in the wrong cell. `sync_scale`
  rebuilds the face when the widget's factor changes (a window moved between
  monitors), so never cache the scale without comparing it on each frame.
- **A tab icon is a GSK render, not a Cairo drawing.** The desktop theme resolves `utilities-terminal` (or an icon named after the program) as SVG, which Cairo cannot read. `icons::render` snapshots the paintable, calls `GtkNative`'s `GskRenderer::render_texture`, and copies the pixels into an `ARgb32` surface; the pixel copy swizzles GDK's memory order to Cairo's and premultiplies a straight format. Cache per program *and* pixel size, because the strip asks on every frame and the size changes with the scale.
- **The strip follows the desktop's window palette.** On KDE the row, the tab
  shades and the accent come from `kdeglobals` (`Colors:Header`,
  `Colors:Window`, `Colors:Selection`, with `WM` as the fallback), so a change
  of KDE colour scheme moves the strip too. Its height is 28 DIP, matching the
  Breeze decoration's computed title bar; `CHROME_CSS` zeroes the GTK header
  bar's 46-DIP padding and shrinks its window controls to Breeze's 20 px. The
  controls sit beside the strip, not inside it, so the palette is also on the
  `.winter-titlebar` background or a band of Adwaita shows to the right. Tabs
  are flat with a 2 px accent line on the selected one, and titles use the
  desktop's UI font, not the terminal's monospace face.
- **A session-leader host must open the pty slave with `O_NOCTTY`.** KIO starts a
  downloaded AppImage as a systemd user service, so Winter is a session leader
  before `main`. Without `O_NOCTTY` the parent's open adopts the slave as its
  own controlling terminal; the shell child's `setsid`/`TIOCSCTTY` then fails
  with `EPERM`, `Pty::spawn` errors, and the app exits (or closes its last tab).
  Launching from a terminal hides this because Winter is not a session leader;
  the regression test re-execs the test binary under `pre_exec` + `setsid`.
- **`--new-instance` skips GtkApplication's handoff.** A running shell owns
  `org.winter.Winter` on the session bus, so a plain launch hands the activation
  to it and prints its old window. The flag sets `NON_UNIQUE`; GApplication must
  not see it, so `take_new_instance` removes it from the arguments first.
- **The font size in the settings file changes the cell.** The opening window
  is sized for whole cells at that size, and a reload that changes it rebuilds
  the face and the renderer; `fit` must not skip the resize because the grid
  count did not change.
- **A plain `gtk::Window` needs the application.** Without it the settings
  window is finalized when the builder's local goes out of scope. Set
  `application` and `transient_for`.
- **GTK 4.10 is the floor.** `gtk::accessible` and the property API need
  `v4_10`; `PRIMARY` as a clipboard needs 4.16, which is why middle-click
  pastes `CLIPBOARD` today.
- **The window's `WM_CLASS` is `winter`, not the application id.** Measured with
  `xprop -id <toplevel> WM_CLASS` under Xvfb: GTK 4 derives the class from the
  program name, while `org.winter.Winter` is the Wayland app id. The desktop
  entry is named after the id and sets `StartupWMClass=winter`; swapping the
  two gives the dock a second, icon-less entry.
- **`gtk-launch` spawns the app and exits.** Wrapping it directly in
  `dbus-run-session` closes the session bus underneath the new process, which
  then exits after a run of `Cannot list activatable names` warnings; the
  Package section keeps the session alive with `sh -c 'gtk-launch … & sleep'`.
- **`appimagetool` is an AppImage itself.** Run it with
  `APPIMAGE_EXTRACT_AND_RUN=1` where FUSE is missing — a CI runner, or a
  container — and pass `ARCH=$(uname -m)`, or the image is labelled with the
  host's architecture instead of the build's.
- **An AppDir carries the entry and the icon twice.** `appimagetool` reads the
  desktop file and the icon at the AppDir's root; the copies under
  `usr/share` are what the mounted image's menu entry resolves through.

## Do not

- Add `unsafe` outside `crates/gtk4`: gtk-rs is safe, and the GL loader's
  exemption is the only one the shell names (ADR 076).
- Add a second renderer, pty pump or timer. The connection is not `Send`; the
  timer polls every pty on the UI thread.
- Put GTK in a shared module, or Linux logic in `input`, `view`, `tabs`,
  `find` or `scrollbar`.
- Claim the accessibility tree is complete: the pane exposes the visible screen
  as its description, not a cell-addressable `GtkAccessibleText` (ADR 075).
- Claim Linux has sound: ADR 033 stands, and only the bell flash and
  notifications are implemented.
