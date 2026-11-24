---
name: winter-linux
description: Build, run and live-check the GTK 4 shell in crates/winter/src/linux. Use when changing the Linux window, tab strip, find bar, settings editor, pointer or keyboard input, IME, scrollbar or the Cairo blit, or when a Linux build or headless screenshot behaves differently.
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
| pane | the `DrawingArea`: refresh → `paint_frame` → Cairo, scrollbar, marks, bell |
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

## Traps

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

- Add `unsafe`: gtk-rs is safe, and the workspace's exemption list does not
  name this shell.
- Add a second renderer, pty pump or timer. The connection is not `Send`; the
  timer polls every pty on the UI thread.
- Put GTK in a shared module, or Linux logic in `input`, `view`, `tabs`,
  `find` or `scrollbar`.
- Claim the accessibility tree is complete: the pane exposes the visible screen
  as its description, not a cell-addressable `GtkAccessibleText` (ADR 074).
- Claim Linux has sound: ADR 033 stands, and only the bell flash and
  notifications are implemented.
