# A native macOS shell — scope and state

Status: shipping first cut (ADR 072). The shell runs a real pty, paints the
terminal through `renderer`, and owns a native window, menu bar, find field,
settings window and input-method client. Accessibility and a GPU content pass
are named follow-ups below, not hidden gaps.
Related: ADR 072 (the decision), ADR 062 (macOS is the next native shell), ADR
040 (the WinUI shell this mirrors), ADR 061 (the host/chrome split), ADRs 064,
068, 069 (the strip), `crates/winter/src/macos/`.

## What ships

- A native `NSWindow`: full-size content view, transparent title bar, the
  system's traffic lights and resize edges, the tab strip in the title bar.
- One pty per tab through the shared `Session`/`Control`, every tab pumped from
  one timer on the main thread (the connection is not `Send`).
- The tab strip: equal-width tabs, a title, a close box with hover, a `+` button
  and a `▾` profile dropdown, painted from `crate::tabs`'s geometry and the
  pane's palette. A theme's four fills are honoured.
- The grid, cursor, selection, find highlights and prompt ticks from the shared
  renderer, copied into an `NSBitmapImageRep`.
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
- DECANM, schemes, OSC titles, cursor shapes, the visual bell and `NSBeep`, and
  OSC 777 notifications through `notify-rust`.
- The shell's drawn icon (ADR 058) as the application icon.
- A native settings window for the font size and the three defaults; it writes
  the whole file through `config::Draft`, preserving sections it does not show.
- `WINTER_SCREENSHOT=<path>` writes the content view as a PNG and quits;
  when the settings window is open it also writes `<stem>-settings.png`.
  `WINTER_OPEN_SETTINGS=1` opens the settings window at startup and
  `WINTER_OPEN_FIND=1` opens the find bar, for a live check on a machine whose
  display cannot be photographed.

## What does not ship yet

- Accessibility: the custom view reports no accessibility tree. A terminal that
  draws its own text needs the AppKit text interface before VoiceOver can read
  it; this is the same gap the WinUI shell has with UIA.
- Panes, the command palette, DECPS tones and a GPU content path.
- The settings editor edits four values; profile, keybinding, scheme and theme
  rows stay file-only here.

## Architecture

| File | Owns |
|---|---|
| `mod.rs` | `run()`, the `Controller` object (timer, menus, find field, window delegate), and the application menu |
| `view.rs` | `TerminalView`, the flipped content view: key, mouse, wheel, tracking area, `drawRect:` and the `NSTextInputClient` methods |
| `shell.rs` | Tabs, the renderer, chords, pointer rules, the settings load/reload, the find state, the IME composition. No AppKit except the `Frame` it owns |
| `chrome.rs` | Strip layout and hit-testing, the strip/scrollbar/bell paint, the pane's grid geometry |
| `frame.rs` | The software backend's pixels as an `NSBitmapImageRep`, and the byte order |
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
