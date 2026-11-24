# ADR 072 — The macOS shell is AppKit, with our renderer for the chrome

- Status: Accepted
- Date: 2026-11-24
- Related: `crates/winter/src/macos/`, `crates/winter/Cargo.toml`,
  `docs/macos-shell-scope.md`; ADRs 040, 041, 051, 061, 062, 064, 068, 069

## Context

ADR 040 gave Windows a native shell and left macOS on egui. ADR 062 parked the
GTK shell and named macOS as the next native shell, scoped before it was built.
The macOS requirement is the same as Windows': the window, its menus, its
dialogs and its text fields should be the platform's controls, not drawn
imitations, while the terminal stays `renderer`'s pixels.

AppKit is the only native macOS toolkit. Rust reaches it through the `objc2`
family, which `eframe` and `winit` already carry on this host. The split the GTK
ADR argued for applies unchanged: the host layer is the platform's; the tab
strip, the scrollbar and the grid are ours, so the terminal looks the same as it
does on Windows and a colour scheme reaches the chrome.

The public surface is a good match for the shell's existing modules. `NSWindow`
with `FullSizeContentView` can put the tab strip in the title bar beside the
traffic lights; `NSMenu` gives the application a real menu bar and a profile
dropdown; `NSSearchField` is the find bar; a second `NSWindow` with AppKit
controls is the settings window; `NSPasteboard` (through `arboard`) is the
clipboard; `NSTimer` on the main run loop is the pty poll.

## Decision

1. **The macOS shell is AppKit**, through `objc2` 0.6, `objc2-app-kit` 0.3 and
   `objc2-foundation` 0.3, all in `[target.'cfg(target_os = "macos")'.dependencies]`
   and named only inside `crates/winter/src/macos/`. `main.rs` selects it under
   `cfg(target_os = "macos")`; Linux keeps egui (ADR 062).
2. **The host layer is AppKit's.** The window, the menu bar, the profile dropdown,
   the find field, the settings window, the About alert and the clipboard are
   native controls. `NSWindow` is full-size content with a transparent title bar;
   the strip leaves room for the traffic lights on the left.
3. **The terminal content is painted by `renderer`.** `MetalBackend` draws the
   payload's backgrounds, images, glyphs, grid lines and cursor on the device and
   reads the frame back for the `NSBitmapImageRep` blit (ADR 073); the software
   backend remains the fallback when no device is available. This is ADR 063's
   split with Metal instead of Direct2D.
4. **The tab strip and the scrollbar are ours**, painted with AppKit paths and
   attributed strings from the shared `tabs` and `scrollbar` geometry. They are
   not `NSTabView`/`NSScrollView` because those controls cannot carry the
   terminal's own fills, the close box beside the title and the prompt ticks
   (the reasons ADR 064 gives for Windows).
5. **`unsafe` is confined to the AppKit and Metal boundary.** `macos/mod.rs`,
   `view`, `chrome`, `frame`, `icon`, `metal` and `settings` carry one
   module-level `#[allow(unsafe_code)]`; every call has a `SAFETY:` note where the
   invariant is not obvious. `keys` and `shell` contain no `unsafe`, and no other
   crate gains an exemption.
6. **The shell shares every rule it can.** `input`, `view`, `tabs`,
   `term_control`, `session`, `find`, `scrollbar`, `grid` and `terminal_render`
   are used unchanged. Windows-only logic stays in `winui`; macOS-only logic
   stays here.
7. **The settings window edits the whole settings document.** The window is an
   editable `NSTextView` over the TOML file, so font, profiles, keybindings,
   schemes, themes and the defaults are all reachable, and Save parses the text
   with the reader startup uses before writing it. This is a deliberate
   divergence from WinUI's structured rows (ADRs 028–032): a native document
   editor gives every section at once without a second, drifting schema UI.
8. **The menu bar is the platform's.** A Shell menu carries new/close/next/
   previous tab and Find; the Edit menu carries the standard `copy:`/`paste:`
   selectors with a nil target, so the responder chain gives them to the terminal
   view or to the find field's editor, whichever has focus.
9. **A tab drag reorders the strip.** The press arms a drag on the tab body
   (not its close box), a move maps the pointer to a slot by the neighbours'
   midpoints, and `tabs::Tabs::move_to` applies the permutation. The rule is
   shared with a future drag on another toolkit; the close box, the `+` button
   and the empty strip never start a drag.
10. **The view is the input method's text client.** `TerminalView` conforms to
    `NSTextInputClient`; `keyDown:` runs the event through `interpretKeyEvents:`
    first, marked text becomes the renderer's `Composition` at the cursor, a
    commit goes through the shared text path, and
    `firstRectForCharacterRange:actualRange:` anchors the candidate window at the
    caret. Plain typing still takes the shell's key path: `insertText:` only
    commits when marked text existed, so no character is sent twice.
11. **A tab wears its program's icon, or the shell's drawn mark.** macOS has no
    GDI icon extraction (ADR 066), so the icon is `NSWorkspace`'s
    `iconForFile:` for the program a profile runs, resolved on `PATH` when the
    command is a bare name. Both the Launch Services icon and the drawn mark
    (ADR 058) are cached per program for the session; a program that cannot be
    resolved falls back to the mark rather than leaving the tab blank (ADR 070).
12. **The view is an accessibility element, not a text tree.** Overriding
    `NSView`'s accessibility methods gives the pane the text-area role, the
    visible screen as its value, the active title as its label and an empty
    child list. VoiceOver therefore reads what the pane shows; character-level
    navigation through the `NSAccessibility` text protocol remains out of
    scope and is named as such.
13. **DECPS plays through the host's sound thread.** Every tab's requested notes
    are drained each tick and handed to `crate::sound`, whose macOS backend
    renders the whole sequence to a temporary WAV and plays it with `afplay` on
    the existing thread; a failed player still sleeps out the sequence. BEL stays
    the system's `NSBeep`, so the bell keeps the platform's own sound and volume
    settings (ADR 033).

## Rationale

- **It is the oracle's layer, not an imitation of it.** AppKit supplies the
  window, the menu bar, the text field, the pasteboard and the standard
  editing commands; a drawn egui approximation cannot.
- **The renderer already has the right shape.** The software backend and
  `TerminalData` are the same pair the WinUI shell uses, so the pane behaves the
  same and the macOS shell adds no emulation code.
- **The dependency is small and already present.** `eframe` uses these `objc2`
  versions on macOS; the shell names them directly so AppKit calls are typed
  rather than reached through a re-export.
- **The scope is honest.** A first shell that runs a real pty, paints the
  renderer's pixels and owns the tab strip, the menus, the find field and the
  settings window is useful without pretending to IME or accessibility parity.

## Consequences

Positive:

- macOS gets a native window and menu bar, a real search field, a native
  settings window and the standard editing shortcuts.
- The chrome follows the terminal's palette, as on Windows; a theme's fills
  reach the strip.
- The content path runs on the GPU through Metal, with a software fallback and
  pixel-identical output (ADR 073).
- The emulation crates, the pty and the console host are unchanged.

Negative:

- **A frame is read back from the GPU.** The offscreen texture is copied to the
  CPU so the existing blit can draw it; a `CAMetalLayer` drawable is the next
  presentation step (ADR 073).
- **VoiceOver reads the pane, not a text tree.** The view is an accessibility
  element whose role is a text area and whose value is the visible screen's
  text, with the active title as its label and an empty child list. A per-cell
  text interface — the `NSAccessibility` text methods — is the follow-up if
  navigation by character or line is needed; this is the same shape as the
  WinUI shell's UIA gap.
- **No panes and no command palette.** Both are unported on WinUI as well.
- **A preview crate.** `objc2` 0.6 is a stable release of the family, but the
  AppKit surface is generated and some generated APIs are deprecated; pin the
  versions and treat a move as deliberate.
- **macOS is now a shell the gate must keep compiling.** The module is
  `cfg(target_os = "macos")`, so Linux and Windows builds do not see it, but the
  macOS CI host does.

## Alternatives considered

| Alternative | Why not |
|---|---|
| Keep egui on macOS | Drawn controls are not native; this is the requirement ADRs 040 and 062 already settled |
| SwiftUI through a bridge | A Swift toolchain and a generated bridge on the UI's hottest path; AppKit is available from Rust directly and is what SwiftUI uses underneath |
| `naui` over WinUI, AppKit and GTK | Young and third-party; the port is taking one platform at a time (ADR 040) |
| A WebView shell | Web content is not native controls |
| A full `renderer` GPU pass first | ADR 007 defers it; the software path is the tested one, and the blit is the same first cut the WinUI shell shipped |
| Native `NSTabView` | No fill, no per-tab close box and no pointer semantics for a terminal's strip |
