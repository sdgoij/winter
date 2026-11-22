---
name: winterm-winui
description: Build, run and live-check the WinUI 3 shell in crates/winterm/src/winui. Use when changing the Windows window, tabs, pointer or keyboard input, the Direct2D swap-chain surface, or the mouse-wheel hook, or when a Windows build or synthetic input behaves differently from the egui shell.
---

# The WinUI 3 shell

`crates/winterm/src/winui/` is the Windows shell; `main.rs` chooses it under
`cfg(windows)` and the egui shell everywhere else. The decisions are ADRs
040–052. This is how to change it without rediscovering the traps.

## Shape

| File | Owns |
|---|---|
| `app.rs` | `Shell` component, one `Control` and `View` per tab, the timer, chords, pointer and wheel dispatch |
| `surface.rs` | D3D11 device, composition swap chain, Direct2D blit (permitted `unsafe`) |
| `wheel.rs` | `WH_MOUSE_LL` hook and Shift state (permitted `unsafe`) |
| `settings.rs` | The settings `ContentDialog` over `config::Draft` |
| `menu.rs` | The profile drop-down's rows: profiles from the file, then `Settings` and `About` |
| `../find.rs` | The query, matches and screen spans, shared with the egui pane |
| `mod.rs` | Module boundary and `run()` |

`app.rs` owns the only `Renderer` and swap chain. Both shells share `input.rs`,
`tabs.rs`, `view.rs`, `term_control.rs` and `term_connection.rs`; a rule that is
not Windows-specific belongs there, never in `winui`.

## Before a build or a live check

- Kill a running session first. `cargo build -p winterm` fails with
  `Access is denied` while `winterm.exe` is alive:
  `taskkill //F //IM winterm.exe`.
- Image sequences (sixel, kitty) need a console host that forwards them. Put an
  `OpenConsole.exe` beside the built binary — `just console-host` copies the one
  Windows Terminal ships — or set `WINTERM_CONSOLE_HOST`. Without one the
  session runs on the inbox ConPTY and silently drops images (ADR 055).
  `just dist` assembles the release folder with the host, its `NOTICE.html` and
  the Windows Terminal MIT licence beside `winterm.exe`.
- The app's `%TEMP%` is not bash's. MSYS `$TEMP` is `/tmp`; the app and
  PowerShell use `%LOCALAPPDATA%\Temp`. A probe file written by the app is not
  in `$TEMP`.
- The binding is `windows-reactor-pre` 0.108 (ADR 042). 0.100 has no input
  events; do not "upgrade" to it.

## Live-check procedure

1. Build and run; the window opens on the settings file's profile.
2. `SetForegroundWindow`, then click the pane. Focus arrives on pointer release,
   not on activation.
3. Re-click after long output. Focus drifts and chords then silently miss.
4. `SendKeys` cannot type `{` or `}` ("repeat count is not valid"). Keep the
   command simple.
5. A synthetic wheel is `mouse_event(0x0800, 0, 0, +/-120, ...)`; a
   `WH_MOUSE_LL` hook sees it. `SendInput` with a hand-built C# `INPUT` did not
   produce one, and `WH_GETMESSAGE` never sees `WM_MOUSEWHEEL`.
6. Synthetic drags never report `PointerMoved` with the left button held. A
   double-click (presses only) is the reliable selection check.

A tracking-program check, in the pane:

```powershell
[Console]::Write("`e[?1000h`e[?1006h")
while ($true) { $c = [Console]::In.Read(); [Console]::Write("[$c]") }
```

The wheel must print a report instead of scrolling; Shift+wheel must still
scroll.

## Rules the shell depends on

- Chords before the terminal. `on_preview_key_down` matches the settings file; a
  bound chord runs its action and returns handled so the toolkit does not also
  type the letter. AltGr arrives as Control+Alt; do not suppress it. Copy is the
  exception: with nothing selected `perform` returns false, so the key falls
  through and `ctrl+c` still interrupts the program (ADR 053).
- Key-ups are a separate subscription. `on_preview_key_down` never sees a
  release; `on_key_up` does, and without it a console client can never be told
  a key came up (ADR 056).
- A handled key raises no `CharacterReceived`. Printable keys must stay
  unhandled so their character arrives, then the press is held until it does
  and sent as one Win32 record with `Vk`, `Sc` and `Uc` (ADR 056).
- Space is the exception: XAML treats an unhandled space as an invocation and
  moves focus off the pane, so Space is handled, carries its own character
  (it is layout-independent), and the pane asks for focus back if XAML still
  takes it. The pane's key-up handler marks the release handled for the same
  reason.
- ConPTY asks for Win32 input mode as its first act: a new pty's output begins
  `ESC[?9001h`. Text-only input makes conhost answer with a key-down and a
  key-up together, which is what cuts a held jump to a few pixels.
- One mouse rule. `input::forwards_to_application(tracking, shift)` decides who
  owns a press, move, release and wheel. Shift means the user, not the program.
- The pane captures the pointer on press (`.capture_pointer_on_press(true)`), or
  the swap chain swallows moves and a drag never starts. A press itself selects
  nothing: it clears the selection and stores the anchor, and the first move
  away from that cell starts the drag (ADR 053). Double- and triple-click select
  the word and the line.
- `client_size` is the constant `initial_client` (120x30 cells at 16 pt).
  Declaring it from a value that changes snaps the window back on every
  re-render.

## Surface and renderer coupling

- A flip-model swap chain rotates its buffers: re-acquire the Direct2D target
  from buffer 0 every present. Drawing into the retained target leaves a frame on
  screen that was never painted.
- `set_scale` applies the inverse composition scale; the panel multiplies the
  content back.
- The atlas payload carries `Settings`; the shell hands the pane's colour table
  to the renderer every frame; `ShapedRow::clear` runs for every row at the
  start of a paint. These were real bugs; do not undo them.
- An image band is part of a row, not of its cells. `TerminalData::refresh`
  copies rows through `copy_visible_into`, so that copy has to carry the band
  (`Row::copy_from` is text only). A missing image after a correct sixel is
  almost always there (ADR 055).
- A kitty image is painted when the batch ends, not when its APC completes:
  `Terminal::process_bytes` flushes the deferred images, and a retransmission of
  the same id replaces the one waiting (ADR 057). A test that drives
  `StateMachine` directly must call `flush_graphics()` before reading the
  buffer.
- The debug build decodes images roughly ten times slower than release. A
  full-screen program that redraws every frame is a debug-build stress test; if
  the pane stops repainting, check the release build before blaming the code.
- The grid is the whole cells that fit the panel; the face is rasterised at the
  composition scale so a high-DPI monitor gets physical pixels.

## Title bar

The `TabView` is the `TitleBar` control's `left_header`, and the pane is the
second root row. The row carries a solid background and a `-8` DIP top margin:
this snapshot leaves a strip of the system caption title above the control's
content. Tabs and `+` stay interactive; the rest of the row drags the window.
The window title remains the active tab's title for the taskbar.

## Find bar

Ctrl+Shift+F toggles the active tab's bar (`find::FindBar`). It is an overlay in
the pane's `Grid`: a `TextBox`, previous/next and a count. While it is open the
pane forwards no key or character to the terminal; a bound chord still reaches
the window, Enter and Shift+Enter step, and Escape closes. Highlights come from
`TerminalData`'s `search_highlights` and the renderer's selection-brush pass; do
not paint them in the shell.

## Profile menu

The `▾` after the tab strip's `+` toggles a list built by `menu::entries`: every
profile in file order, then `Settings` and `About winterm`. The binding has no
flyout, so the list is an overlay in the pane's `Grid` that hangs under the
button (ADR 052).

- The button is 32x24 and carries a 13-DIP top offset, matching the tab strip's
  own add button; if you change one, measure the other from UI Automation.
- The list's left edge comes from the width of the `Grid` that wraps the strip
  and the button (`observe_composition_host`), not from a constant. The pane is
  a different ancestor, so no layout relationship exists without it.

- A profile row opens a tab through `TerminalSettings::for_profile` and the
  shell's `add_tab`; `open_tab` is the same path with the default template.
- While the menu is open the pane forwards no key or character, and a pane press
  dismisses it. Escape is handled by the pane and, when focus is in the title
  bar, by the root `Border`'s preview handler; that handler is the only reason
  the root is a Border.
- Keep the list in `menu.rs`. Its ordering rule is a unit test; do not build rows
  inline in `view`.

## Settings dialog

`settings.rs` draws the editor with a `ContentDialog` and sends
`SettingsMessage`s; the draft, validation and preview are `config::Draft`, shared
with the egui window (ADR 049). Save and Revert are in-content buttons so a save
that fails keeps the dialog open and shows the error; the dialog's own Close
button discards unsaved edits. A dirty draft is not overwritten by the poll —
the dialog says the file changed and Save or Revert resolves it.

While the dialog is open the wheel hook's copy is dropped: the dialog's
`ScrollViewer` scrolls, not the terminal behind it.

## Do not

- Add a second renderer, swap chain or pty pump thread. The connection is not
  `Send`; the timer polls every pty on the UI thread.
- Put `unsafe` outside `surface.rs` and `wheel.rs`; each call needs a `SAFETY:`
  note.
- Put Windows-only logic in `input`, `view`, `tabs` or `term_control`.
