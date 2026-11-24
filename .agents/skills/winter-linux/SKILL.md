---
name: winter-linux
description: Build, run and live-check the GTK 4 shell in crates/winter/src/linux. Use when changing the Linux window, tab strip, find bar, settings editor, pointer or keyboard input, IME, scrollbar, the OpenGL content path or the Cairo blit, or when a Linux build or headless screenshot behaves differently.
---

# The Linux shell

`crates/winter/src/linux/mod.rs` is the Linux shell; `main.rs` chooses it when
the `gtk` feature is on, which it is by default (ADR 074). GTK owns the window,
the profile menu, the find bar and the settings editor; the tab strip, scrollbar
and grid are our pixels, the split ADR 061 takes. `winui`, `macos` and this
module share `input`, `view`, `tabs`, `term_control`, `session`, `find`,
`scrollbar`, `grid` and `terminal_render`; a rule that is not GTK-specific
belongs there.

## Shape

One module owns:

| Part | Owns |
|---|---|
| entry | `run()`, the window, the `GtkWindowHandle` title bar |
| strip | layout, hit testing, painting, drag-reorder over `crate::tabs` |
| pane | the base `DrawingArea`: pane fill, scrollbar, marks, and the input controllers over the GL area |
| gl | the OpenGL content path (ADR 075): `GtkGLArea`, solid/glyph/image pipelines, atlas, fallback |
| input | key, IME, click, drag, scroll and motion controllers → `crate::input` |
| find | `GtkSearchBar` + `GtkSearchEntry` over `find::FindBar` |
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
makes an in-tree `target/` far slower. `--no-default-features` builds the egui
fallback and needs no headers; the gate checks it on Linux.

The grid is drawn through GTK's OpenGL context when libGL/libEGL exists;
`WINTER_SOFTWARE=1` forces the CPU blit. Both paths are built either way.

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
the requested path with `-settings`), and `WINTER_OPEN_FIND=1` opens the find
bar. A correct first frame shows the strip with one tab, `+` and `▾`, the
prompt and the cursor on a dark pane inset 8 px from the left, and the
scrollbar's 10 px gutter on the right. A white pane means the drawing area got
no height — see the traps. Do not leave the PNGs in the tree.

On a machine with a display: typing reaches the shell; Ctrl+Shift+T/W; the
settings editor's Save writes and reloads; Ctrl+Shift+F opens the find bar and
Escape closes it; Shift+wheel scrolls history while a tracking program still
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

## Traps

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
- **GDK exposes no entry-point resolver.** `linux::gl`'s loader tries
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
- **The find entry owns its keys while focused.** The window's key controller
  returns `Proceed` when `entry.has_focus()`, or typing goes to the terminal.
- **The search bar's close button turns search mode off without
  `stop-search`.** Follow `notify::search-mode-enabled` or the next tick reopens
  it.
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

## Do not

- Add `unsafe` outside `linux::gl`: gtk-rs is safe, and the GL loader's
  exemption is the only one the shell names (ADR 075).
- Add a second renderer, pty pump or timer. The connection is not `Send`; the
  timer polls every pty on the UI thread.
- Put GTK in a shared module, or Linux logic in `input`, `view`, `tabs`,
  `find` or `scrollbar`.
- Claim the accessibility tree is complete: the pane exposes the visible screen
  as its description, not a cell-addressable `GtkAccessibleText` (ADR 074).
- Claim Linux has sound: ADR 033 stands, and only the bell flash and
  notifications are implemented.
