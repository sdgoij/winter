# A native macOS shell — scope and state

Status: shipping first cut (ADR 072). The shell runs a real pty, paints the
terminal through `renderer` — on Metal by default (ADR 073) — and owns a native
window, menu bar, find field, settings window, input-method client and an
accessibility element. Panes and the command palette are named follow-ups
below, not hidden gaps.
Related: ADRs 072 and 073 (the decisions), ADR 062 (macOS is the next native
shell), ADR 040 (the WinUI shell this mirrors), ADR 061 (the host/chrome split),
ADRs 064, 068, 069 (the strip), `crates/winter/src/macos/`.

## What ships

- A native `NSWindow`: full-size content view, transparent title bar, the
  system's traffic lights and resize edges, the tab strip in the title bar.
- One pty per tab through the shared `Session`/`Control`, every tab pumped from
  one timer on the main thread (the connection is not `Send`).
- The tab strip: equal-width tabs, a title, a close box with hover, a `+` button
  and a `▾` profile dropdown, painted from `crate::tabs`'s geometry and the
  pane's palette. A theme's four fills are honoured.
- The grid, cursor, selection, find highlights and prompt ticks from the shared
  renderer: cell quads, glyphs from a coverage atlas, images, grid lines and the
  cursor are drawn by Metal into a `CAMetalLayer` drawable, so the window shows
  the GPU's frame without a copy (ADR 073). The offscreen readback runs only
  when the layer has no drawable, or for the one frame `WINTER_SCREENSHOT`
  forces before it captures the view; it is also the fallback. The software
  backend paints when no device is available or `WINTER_SOFTWARE=1` is set.
- A real menu bar: About, Settings… (⌘,), Quit (⌘Q), New Tab (⌘T), Close Tab
  (⌘W), Next/Previous Tab (⌘⇧] / ⌘⇧[), Find… (⌘F), and the standard Edit menu
  whose `copy:`/`paste:` go to the terminal or the focused text field.
- Settings from the TOML file at startup, one profile per row, a saved file
  noticed by the same one-second stamp poll as the other shells (ADR 025).
- Pointer selection with click/drag, double-click word, triple-click line, the
  scrollbar with its prompt ticks, the wheel into history (or to a tracking
  program unless Shift is held), copy/paste, right-click paste.
- Drag-reorder of tabs: a press on a tab body arms a drag, a move past a
  neighbour's midpoint swaps the two, and the close box and the buttons never
  start a drag.
- Input-method composition: `TerminalView` is an `NSTextInputClient`, marked
  text is drawn as the renderer's underlined preview at the cursor, a commit
  reaches the pty through the shared text path, and the candidate window is
  anchored at the caret.
- Program icons on tabs: Launch Services' icon for the profile's program, with
  the shell's drawn mark when the program is a script or does not resolve.
- An accessibility element: VoiceOver reads the active tab's title and the
  visible screen's text, with the pane's frame and an empty child list.
- DECPS notes through the host's sound thread: the sequence is rendered to a
  temporary WAV and played by `afplay`; the bell stays the system's `NSBeep`.
- DECANM, schemes, OSC titles, cursor shapes, the visual bell and `NSBeep`, and
  OSC 777 notifications through `notify-rust`.
- The shell's drawn icon (ADR 058) as the application icon.
- A native settings window that edits the whole TOML document — font,
  profiles, keybindings, schemes, themes and the defaults — and validates it
  with the same reader startup uses before saving.
- `WINTER_SCREENSHOT=<path>` writes the content view as a PNG and quits;
  when the settings window is open it also writes `<stem>-settings.png`.
  `WINTER_OPEN_SETTINGS=1` opens the settings window at startup and
  `WINTER_OPEN_FIND=1` opens the find bar, for a live check on a machine whose
  display cannot be photographed.

## What does not ship yet

- A per-cell accessibility text interface. The view exposes the visible screen
  as one value; character- and line-level navigation through the
  `NSAccessibility` text protocol is the next step for VoiceOver, and the same
  gap the WinUI shell has with UIA.
- Panes and the command palette.

## Architecture

| File | Owns |
|---|---|
| `mod.rs` | `run()`, the `Controller` object (timer, menus, find field, window delegate), and the application menu |
| `view.rs` | `TerminalView`, the flipped content view: key, mouse, wheel, tracking area, `drawRect:` and the `NSTextInputClient` methods |
| `shell.rs` | Tabs, the renderer, chords, pointer rules, the settings load/reload, the find state, the IME composition. No AppKit except the `Frame` it owns |
| `chrome.rs` | Strip layout and hit-testing, the strip/scrollbar/bell paint, the pane's grid geometry |
| `frame.rs` | The software backend's pixels as an `NSBitmapImageRep`, and the byte order |
| `metal.rs` | The Metal content backend: pipelines, glyph atlas, image textures, drawable presentation and the offscreen readback (ADR 073) |
| `icon.rs` | A tab's program icon from Launch Services, and the drawn application mark (ADR 058), both cached |
| `settings.rs` | The native settings window and its action methods |
| `keys.rs` | Virtual-key codes and layout-character selection |

`main.rs` chooses this shell under `cfg(target_os = "macos")`; `lib.rs` keeps the
module out of the other builds. The workspace-ring rule is unchanged: nothing
below `winter` sees AppKit.

## Traps that cost time

- **The source tree carries two `objc2` versions.** `winit` 0.30 uses
  `objc2` 0.5/`objc2-app-kit` 0.2; `eframe` and this shell use 0.6/0.3. A class
  defined with the wrong one fails with "the trait `NSObjectProtocol` is not
  satisfied", and the diagnostic lists both versions. The fix is to import the
  protocols from the direct `objc2-foundation` 0.3 dependency and to implement
  `NSObjectProtocol` on every `define_class!` class that conforms to an AppKit
  protocol.
- **`NSBitmapFormat::AlphaFirst` is big-endian `A, R, G, B`.** A `COLORREF`
  (`0x00BBGGRR`) has no alpha. `to_ne_bytes` produces `B, G, R, A`, which the
  rep reads as a nearly transparent image whose alpha is the colour's low byte;
  the pane then looks white. The bytes are `(pixel | 0xFF00_0000).to_be_bytes()`
  and the compositing operation is `SourceOver`.
- **The window reports the minimum size without a WindowServer.** On a headless
  session AppKit logs `Connection invalid` and clamps the window; the content
  view still paints, so `WINTER_SCREENSHOT` is the verification path there.
- **A menu item's `copy:` with a nil target is the right design.** Pointing a
  Copy item at the controller would steal ⌘C from the find field's editor. The
  `TerminalView` implements `copy:`/`paste:` so the responder chain selects the
  right receiver.
- **The find field needs `NSSearchFieldDelegate`, not just
  `NSControlTextEditingDelegate`.** Escape is caught in
  `control:textView:doCommandBySelector:` with `cancelOperation:`.
- **The timer belongs to the common run-loop modes.** A timer only in the
  default mode stops while a window is dragged or a menu is tracking; the pty
  and the strip then freeze until the gesture ends.
- **`method_id` is for a retained return.** `#[unsafe(method(...))]` in
  `define_class!` requires a type that implements `ConvertReturn<()>`; a
  `Retained<T>` return is only accepted under `#[unsafe(method_id(...))]`. The
  symptom is "`Retained<NSAttributedString>: Encode` is not satisfied" with a
  note listing both `objc2` versions.
- **Metal's vertex buffers are unretained.** A buffer bound with
  `setVertexBuffer:offset:atIndex:` must outlive the command buffer; keep it
  alive until `waitUntilCompleted` or the draw silently reads freed memory.
- **MSL vertex structs need the Rust layout's padding.** The shader uses plain
  `float2`/`float4`, whose alignment differs from a bare `#[repr(C)]` struct
  with two `f32`s before a colour; the Rust `SolidVertex` carries
  `_padding: [f32; 2]` so `color` lands at offset 16. `packed_float` was tried
  and the shader compiler rejected it here.
- **A layer never appears in `cacheDisplayInRect`.** The screenshot path is a
  cached display of the content view, which draws the view and not its
  sublayers. Hide the content and overlay layers, force one offscreen frame so
  the view has the readback pixels to paint, and restore the layers after the
  capture.
- **A BGRA8 readback is already `0xAARRGGBB`.** The bytes are `B, G, R, A`;
  read little-endian and hand the `u32` to `Frame::copy`, which applies the
  `AlphaFirst` byte order (ADR 072's bitmap trap).
- **A layer-backed view's own drawing is a sublayer.** After `setWantsLayer(true)`
  AppKit inserts a layer for the view's `drawRect:` content at index 0 and adds
  each control's layer after it. The content and its two overlay layers are
  inserted at the bottom too, so the find field, its buttons and the count stay
  above the find backdrop instead of being covered by it.
- **A layer animates its properties by default.** Setting a frame or a
  background colour starts a 0.25-second action; a window resize would slide
  the content instead of fitting it. The controller wraps every change in a
  `CATransaction` with actions disabled.
- **The content layer's drawable is top-down.** The view is flipped and its
  drawable's first row is the grid's first row, so the `CAMetalLayer` is
  `geometryFlipped`; without it the pane is upside down.
- **The strip starts after the traffic lights.** `FullSizeContentView` puts the
  content under them; `chrome::LEFT_INSET` is the gap.

## Verification

The gate is the first check:

```sh
just gate
```

On this host there is no interactive WindowServer session, so the live check is
the shell's own capture:

```sh
cargo build -p winter
WINTER_SCREENSHOT=winter-shot.png ./target/debug/winter
```

A first frame shows the active tab in the pane's colour, the strip, the prompt
and the cursor. `WINTER_OPEN_SETTINGS=1` exercises the settings window's
construction on the same run; a crash there happens before the capture.

A machine with a display should also check: a click focuses the pane, typing
reaches `zsh`, ⌘T opens a tab, ⌘W closes it, ⌘F opens the find field and Escape
closes it, Shift+wheel scrolls history while a tracking program still receives
the wheel, ⌘C copies a selection and a plain ⌃C interrupts, and a saved
`settings.toml` reaches the scheme and the font.
