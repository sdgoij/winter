---
name: winter-macos
description: Build, run and live-check the AppKit shell in crates/winter/src/macos. Use when changing the macOS window, tab strip, menu bar, find field, settings window, pointer or keyboard input, or the NSBitmapImageRep blit, or when a macOS build or a headless screenshot behaves differently.
---

# The macOS shell

`crates/winter/src/macos/` is the macOS shell; `main.rs` chooses it under
`cfg(target_os = "macos")`. AppKit owns the window, menu bar, text fields,
dialogs and pasteboard; the tab strip, scrollbar and grid are our pixels, the
split ADR 072 takes from ADR 061. `winui`, `linux` and this module share
`input`, `view`, `tabs`, `term_control`, `session`, `find`, `scrollbar`, `grid`
and `terminal_render`; a rule that is not AppKit-specific belongs there.

## Shape

| File | Owns |
|---|---|
| `mod.rs` | `run()`, `Controller` (timer, menus, find field, window delegate), the application menu, the app icon, the screenshot path |
| `view.rs` | `TerminalView`: flipped content view, key/mouse/wheel events, tracking area, `copy:`/`paste:` for the responder chain, `drawRect:` |
| `shell.rs` | Tabs, renderer, chords, pointer rules, settings reload, find state; no AppKit except the `Frame` |
| `chrome.rs` | Strip layout and hit testing, strip/scrollbar/bell painting, pane grid geometry |
| `frame.rs` | Software backend pixels to `NSBitmapImageRep` |
| `icon.rs` | A tab's program icon from `NSWorkspace`, and the drawn app mark, both cached |
| `settings.rs` | Native settings window over `config::Draft` |
| `keys.rs` | Virtual-key codes and layout-character selection |

The module declares the workspace's `unsafe` exemption for the AppKit boundary
(ADR 072). `keys.rs` and `shell.rs` contain none. Do not add `unsafe` outside
`macos`.

## Build

```sh
cargo build -p winter --offline
```

The dependencies are `objc2` 0.6.5, `objc2-app-kit` 0.3.2 and
`objc2-foundation` 0.3.2, all target-specific. `eframe` and `winit` also carry
`objc2` on this host: **`winit` 0.30 uses `objc2` 0.5 / `objc2-app-kit` 0.2**.
A class defined against the wrong copy fails with “the trait `NSObjectProtocol`
is not satisfied” and a diagnostic listing both versions. Import protocols from
the direct `objc2-foundation` 0.3 dependency, and implement `NSObjectProtocol`
on every `define_class!` class that conforms to an AppKit protocol.

## Live check

There is no interactive WindowServer in the agent sandbox; AppKit logs
`Connection invalid` and clamps the window to its minimum size. The shell's own
capture is the verification path:

```sh
rm -f winter-shot.png
WINTER_SCREENSHOT=winter-shot.png ./target/debug/winter
```

Read `winter-shot.png`; a correct first frame shows the active tab in the pane's
fill, the strip's `+` and `▾`, the prompt and a cursor. `WINTER_OPEN_SETTINGS=1`
opens the settings window on the same run (written as
`winter-shot-settings.png`), and `WINTER_OPEN_FIND=1` opens the find bar over the
pane. A crash before the capture is the construction failure. Do not leave the
PNGs in the tree.

On a machine with a display, check: click focuses the pane and typing reaches the
shell; ⌘T/⌘W/⌘F/⌘,/⌘Q; Escape closes the find field; ⇧wheel scrolls history
while a tracking program still gets the wheel; ⌘C copies a selection and plain
⌃C interrupts; a saved `settings.toml` reaches the scheme and font within a
second.

## Traps

- **`NSBitmapFormat::AlphaFirst` is big-endian `A,R,G,B`.** A `COLORREF` has no
  alpha. `to_ne_bytes` produces `B,G,R,A`, which the rep reads as a nearly
  transparent image; the pane looks white over the window. Use
  `(pixel | 0xFF00_0000).to_be_bytes()` and `SourceOver`.
- **The strip starts after the traffic lights.** The window is
  `FullSizeContentView` with a transparent hidden title bar; `LEFT_INSET` is the
  gap. A press on the empty strip drags the window; a press on a tab selects it
  and arms a drag, which reorders past a neighbour's midpoint.
- **IME needs `interpretKeyEvents:` in `keyDown:` and a commit-only-once rule.**
  `interpretKeyEvents:` calls `setMarkedText:`/`insertText:`/`doCommandBySelector:`
  synchronously. `insertText:` must commit only when marked text existed, because
  plain typing reaches the shell through `keyDown:`'s own mapping; committing
  unconditionally types every character twice.
- **`method_id` is for a retained return.** `#[unsafe(method(...))]` in
  `define_class!` wants a type that implements `ConvertReturn<()>`; a
  `Retained<T>` return needs `#[unsafe(method_id(...))]`. The error names
  `Retained<NSAttributedString>: Encode` and both `objc2` versions.
- **The timer must be in the common run-loop modes.** A timer only in the default
  mode freezes while a window is dragged or a menu tracks.
- **The window's appearance follows the pane, not the system.** A light
  `NSSearchField` and black count label are invisible on a dark terminal. The
  controller sets `NSAppearanceNameDarkAqua` or `NSAppearanceNameAqua` from the
  active background's luminance whenever it changes.
- **The Edit menu's `copy:`/`paste:` have a nil target.** `TerminalView`
  implements them, so the responder chain gives the command to the terminal or to
  the find field's editor. Pointing them at the controller steals ⌘C from the
  field.
- **The find field needs `NSSearchFieldDelegate`.** Escape arrives in
  `control:textView:doCommandBySelector:` as `cancelOperation:`.
- **The shell raises requests; it does not call AppKit.** The menu and the
  settings window are opened from the controller on the next tick, because the
  view holds the same `Rc<RefCell<Shell>>` and a callback would re-enter it.

## Do not

- Add a second renderer, pty pump or timer. The connection is not `Send`; the
  timer polls every pty on the UI thread.
- Put Windows-only logic in the shared modules; the WinUI shell has its own.
- Reset a frame's alpha to zero or drop the `| 0xFF00_0000`: the software
  backend writes `COLORREF`s, not opaque `ARGB`.
- Claim per-cell accessibility: the view exposes the visible screen as one
  value for VoiceOver, not an `NSAccessibility` text tree. The remaining gap is
  named in ADR 072 and `docs/macos-shell-scope.md`.
- Claim DECPS is silent: the macOS `sound` backend renders the sequence to a
  WAV and plays it with `afplay`; only BEL stays `NSBeep`.
