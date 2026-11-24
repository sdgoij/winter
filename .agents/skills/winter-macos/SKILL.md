---
name: winter-macos
description: Build, run and live-check the AppKit shell in crates/cocoa/src. Use when changing the macOS window, tab strip, menu bar, find field, settings window, pointer or keyboard input, or the NSBitmapImageRep blit, or when a macOS build or a headless screenshot behaves differently.
---

# The macOS shell

`crates/cocoa/src/` is the macOS shell; `main.rs` chooses it under
`cfg(target_os = "macos")`. AppKit owns the window, menu bar, text fields,
dialogs and pasteboard; the tab strip, scrollbar and grid are our pixels, the
split ADR 072 takes from ADR 061. `winui`, `linux` and this module share
`input`, `view`, `tabs`, `term_control`, `session`, `find`, `scrollbar`, `grid`
and `terminal_render`; a rule that is not AppKit-specific belongs there.

## Shape

| File | Owns |
|---|---|
| `lib.rs` | `run()`, `Controller` (timer, menus, find field, window delegate), the application menu, the app icon, the screenshot path |
| `view.rs` | `TerminalView`: flipped content view, key/mouse/wheel events, tracking area, `copy:`/`paste:` for the responder chain, `drawRect:` |
| `shell.rs` | Tabs, renderer, chords, pointer rules, settings reload, find state; no AppKit except the `Frame` |
| `chrome.rs` | Strip layout and hit testing, strip/scrollbar/bell painting, pane grid geometry |
| `frame.rs` | Software backend pixels to `NSBitmapImageRep` |
| `metal.rs` | The Metal content backend: drawable presentation, its glyph atlas and the offscreen readback |
| `icon.rs` | A tab's program icon from `NSWorkspace`, and the drawn app mark, both cached |
| `settings.rs` | Native settings window over the whole TOML document |
| `keys.rs` | Virtual-key codes and layout-character selection |

The module declares the workspace's `unsafe` exemption for the AppKit and Metal
boundary (ADRs 072, 073). `keys.rs` and `shell.rs` contain none. Do not add
`unsafe` outside `macos`.

## Build

```sh
cargo build -p winter --offline
```

The dependencies are `objc2` 0.6.5, `objc2-app-kit`, `objc2-foundation`,
`objc2-metal` and `objc2-quartz-core` 0.3.2, all target-specific. `eframe` and
`winit` also carry
`objc2` on this host: **`winit` 0.30 uses `objc2` 0.5 / `objc2-app-kit` 0.2**.
A class defined against the wrong copy fails with “the trait `NSObjectProtocol`
is not satisfied” and a diagnostic listing both versions. Import protocols from
the direct `objc2-foundation` 0.3 dependency, and implement `NSObjectProtocol`
on every `define_class!` class that conforms to an AppKit protocol.

## Package

```sh
just app        # dist/Winter.app, for Finder or the Dock
just package    # the release disk image, with Winter.app inside
```

`just app` copies the release binary into the bundle, runs it with
`WINTER_APPICON=<bundle>/Contents/Resources/Winter.iconset` to draw the ten
iconset PNGs, compiles them with `iconutil`, writes `Info.plist`, and ad-hoc
signs the result. The icon is the same drawing as the window icon and the
Windows resource (ADR 058); there is no checked-in image.

`just package` on macOS builds **both** architectures — the host and the other
`-apple-darwin` target — and writes one compressed disk image per target with
`Winter.app`, `LICENSE.md`, `README.md` and an `Applications` symlink at the
mount point. `WINTER_TARGET=<target>` packages just one. The other slice must
be installed (`rustup target add aarch64-apple-darwin`); CI's macOS gate
cross-checks it with `cargo check` and runs `just app`, so the bundle path is
exercised on every push. The Intel runner links the arm64 slice from the SDK, so
no arm64 macOS runner is involved.

```sh
hdiutil verify dist/winter-<version>-<target>.dmg
hdiutil attach -nobrowse -readonly -mountpoint /tmp/winter-dmg dist/winter-<version>-<target>.dmg
WINTER_SCREENSHOT=winter-shot.png /tmp/winter-dmg/Winter.app/Contents/MacOS/winter
hdiutil detach /tmp/winter-dmg
```

`hdiutil` and a read-only mount work without a WindowServer, so the DMG's
contents can be checked headlessly.

- **Launch Services starts a bundle at `/`.** `start_in_home` moves the process
  to `$HOME` when the executable is inside `.app/Contents/MacOS/` and the cwd is
  `/`; the raw binary keeps its directory, which the live check relies on.
- **The iconset needs all ten files.** `iconutil` rejects a set missing one of
  16, 32, 64, 128, 256, 512, 1024 (with the `@2x` pairs); `WINTER_APPICON`
  writes them all.
- **A bundle is not distributable until it is signed.** `codesign --sign -` is
ad-hoc only; Gatekeeper wants a Developer ID and notarization (ADR 074).

To install locally, `cp -R dist/Winter.app /Applications/`.

## Live check

There is no interactive WindowServer in the agent sandbox; AppKit logs
`Connection invalid` and clamps the window to its minimum size. The shell's own
capture is the verification path:

```sh
rm -f winter-shot.png
WINTER_SCREENSHOT=winter-shot.png ./target/debug/winter
```

After `just app`, the bundle's own binary runs the same check:

```sh
WINTER_SCREENSHOT=winter-shot.png dist/Winter.app/Contents/MacOS/winter
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
- **Metal's vertex buffers are unretained by the encoder.** Keep them alive
  until `waitUntilCompleted`. Do not use `packed_float` in MSL: the shader
  compiler rejected it here, so the shaders use plain `float2`/`float4` and the
  Rust `SolidVertex` carries `_padding: [f32; 2]` so `color` lands at offset
  16. Read a BGRA8 target back as little-endian `u32` (`0xAARRGGBB`) for
  `Frame::copy`.
- **A layer never appears in `cacheDisplayInRect`.** The content and its two
  overlay layers are not part of the view's cached display. `write_screenshot`
  hides them, marks the view as capturing and calls `Shell::prepare_capture`,
  which forces one offscreen frame, so `drawRect:` paints the readback pixels
  and `chrome`'s overlays for that one draw. A presenting frame never reads
  back, so without the forced frame the capture would blit stale pixels.
- **A readback target is `Managed`, not `Shared`.** A `Shared` `BGRA8Unorm`
  render target works on Apple GPUs and reads back zeros on Intel ones. The
  offscreen target is `Managed`, and the rendering command buffer ends with a
  blit `synchronizeResource` before `getBytes`. `Shared` sampled textures (the
  atlas, an image) are fine.
- **The content layer is `geometryFlipped`.** The view is flipped and the
  drawable's first row is the grid's first row; without the flip the pane is
  upside down.
- **A layer animates its properties by default.** Every frame resize, find
  backdrop or bell change is wrapped in a `CATransaction` with actions
  disabled, or the content slides and the overlays fade instead of keeping
  time with the window.
- **A layer-backed view's own drawing is a sublayer.** AppKit inserts the
  view's `drawRect:` content at index 0 and the controls' layers after it. The
  content and its overlays are inserted at the bottom (`insertSublayerBelow:`
  with a nil sibling, in reverse order), so the find field and its buttons are
  above the find backdrop rather than hidden by it.
- **The content layer covers the grid, not the pane.** The scrollbar and the
  strip sit outside its frame, so AppKit still paints them and they stay
  visible above the layer's left/right edges; the find backdrop and the bell
  cross the grid, which is why they are layers too.
- **`WINTER_SOFTWARE=1` selects the software backend.** Use it to A/B a frame;
  the two renderers match for opaque glyphs, and antialiased text differs by at
  most one 8-bit step from the GPU's rounding.
- **Build the face at the window's backing scale and size the opening window
  to the initial grid.** Spawning the shell at 100×28 and then resizing it for
  the real view makes the pty change size after zsh has laid out its first
  prompt; zsh's `PROMPT_EOL_MARK` then survives as a stray `%` on the first
  line, because the bytes were authored for the old width. `default_size` must
  include the pane inset and the scrollbar, and `Shell::new` must take the
  screen's scale.
- **The settings editor's `NSTextView` needs its smart substitutions off.**
  macOS rewrites quotes, dashes and spellings as you type, which corrupts the
  TOML keys. Turn off quote, dash, replacement and spelling correction, or the
  file fails to parse after a save.
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
