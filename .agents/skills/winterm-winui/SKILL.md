---
name: winterm-winui
description: Build, run and live-check the WinUI 3 shell in crates/winterm/src/winui. Use when changing the Windows window, tabs, pointer or keyboard input, the Direct2D swap-chain surface, or the mouse-wheel hook, or when a Windows build or synthetic input behaves differently from the egui shell.
---

# The WinUI 3 shell

`crates/winterm/src/winui/` is the Windows shell; `main.rs` chooses it under
`cfg(windows)` and the egui shell everywhere else. The decisions are ADRs
040–048. This is how to change it without rediscovering the traps.

## Shape

| File | Owns |
|---|---|
| `app.rs` | `Shell` component, one `Control` and `View` per tab, the timer, chords, pointer and wheel dispatch |
| `surface.rs` | D3D11 device, composition swap chain, Direct2D blit (permitted `unsafe`) |
| `wheel.rs` | `WH_MOUSE_LL` hook and Shift state (permitted `unsafe`) |
| `settings.rs` | The settings `ContentDialog` over `config::Draft` |
| `../find.rs` | The query, matches and screen spans, shared with the egui pane |
| `mod.rs` | Module boundary and `run()` |

`app.rs` owns the only `Renderer` and swap chain. Both shells share `input.rs`,
`tabs.rs`, `view.rs`, `term_control.rs` and `term_connection.rs`; a rule that is
not Windows-specific belongs there, never in `winui`.

## Before a build or a live check

- Kill a running session first. `cargo build -p winterm` fails with
  `Access is denied` while `winterm.exe` is alive:
  `taskkill //F //IM winterm.exe`.
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
  type the letter. AltGr arrives as Control+Alt; do not suppress it.
- One mouse rule. `input::forwards_to_application(tracking, shift)` decides who
  owns a press, move, release and wheel. Shift means the user, not the program.
- The pane captures the pointer on press (`.capture_pointer_on_press(true)`), or
  the swap chain swallows moves and a drag never starts.
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
