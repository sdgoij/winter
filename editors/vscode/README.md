# Winter in a VS Code webview

A demo host for `winter::embed::Surface` (ADR 083). VS Code owns a webview; a
process owns the shell and paints the pixels with Winter's own renderer.

## Run it

```sh
cargo build --release -p winter-demo   # target/release/winter-frame
cd editors/vscode
npm install                    # once
```

Then open **`editors/vscode`** in VS Code and press <kbd>F5</kbd>. That runs the
`Run Winter demo` launch configuration, which compiles the extension, builds the
server, and opens an **Extension Development Host** — a second VS Code window,
opened at the repository root, with this extension loaded.

In that second window, run **Winter: Open Terminal** from the command palette
(<kbd>Ctrl</kbd>+<kbd>Shift</kbd>+<kbd>P</kbd>).

The shell is PowerShell on Windows — 7 (`pwsh`) when it is on `PATH`, then
Windows PowerShell, then `cmd.exe` — and `$SHELL` on macOS and Linux, with
`/bin/sh` as the last resort. `winter.shell` names a program instead.

> Opening `editors/vscode` in a normal window is not enough: VS Code only loads
> an extension when it is installed or started as a development extension
> (F5 / `--extensionDevelopmentPath`). Without that, the command does not exist.

### Without F5

```sh
cargo build --release -p winter-demo
cd editors/vscode && npm install && npm run compile
code --extensionDevelopmentPath="$PWD"
```

(`codium --extensionDevelopmentPath="$PWD"` for VSCodium.)

## What it does

- Spawns `winter-frame`, which starts your shell on a pty.
- Sends `key`, `paste`, `write`, `resize`, `render`, `press`, `move`, `release`,
  `wheel`, `copy`, `modes` and `quit` as JSON lines. A key travels as its
  **name, its physical `code` and its modifiers**, not as the characters a
  webview guessed: the terminal applies the input modes and the keyboard
  protocol the program asked for, so function keys, Shift+Tab and modified
  arrows work. A ConPTY asks for Win32 input records, and the record's virtual
  key and scan code come from the physical code — a record without them is one
  text byte, which the console answers with a press and a release together, so
  a held key would move for a single frame.
- Reads tagged messages: kind 0 is a whole frame (`u32` width, height, cell
  width, cell height, byte length, then RGBA), painted with `putImageData`; kind
  1 is text for the clipboard; kind 2 is a **band** — the rows that changed, with
  their top edge — so a keystroke sends about 60 KB instead of 1.8 MB; kind 3 is
  a diagnostic line, logged to the **Winter** output channel. A frame with a
  **zero width** means nothing changed, and the host keeps what it has. `modes`
  asks for kind 3: it is what a program asked of the input path, which is how to
  answer "why did that key do that".
- Keeps **one frame in flight**: it asks for a frame, waits for the canvas to
  report it painted, and only then asks again, no sooner than 33 ms later. An
  idle pane therefore costs 24 bytes a poll, a full-screen program sets the
  pace, and a tab that is not the active one stops asking altogether. Asking on
  a timer without waiting is a queue that never drains — a 900 by 510 frame
  costs about 5 ms in a release build and 75 ms in a debug one, so a
  30-per-second timer outruns a debug server, and because the server answers
  commands in order, keystrokes end up behind every frame already asked for.

## Controls

- **Type** — keys, `Ctrl+<letter>` as control codes, paste from the clipboard
  through the terminal's bracketed-paste filter.
- **Select** — click a cell, drag to extend, double-click for a word, triple
  click for the row. The server decides what a click means (`Surface::press`).
- **Copy** — <kbd>Ctrl</kbd>+<kbd>Shift</kbd>+<kbd>C</kbd>, or
  <kbd>Cmd</kbd>+<kbd>C</kbd>. Plain `Ctrl+C` still interrupts.
- **Scroll** — the wheel. A program that tracks the mouse gets the wheel
  instead, exactly as `winter`'s shells behave; hold Shift to keep it here.
- **Click in a tracking program** — forwarded as a mouse report, unless Shift is
  held, which keeps the pointer for selection.

The server is found by walking up from the extension's own folder (and from any
workspace folder) to `target/release/winter-frame`, then
`target/debug/winter-frame`, or wherever `winter.serverPath` points. Prefer the
release build: a debug server paints a game at about 13 frames a second.

## Troubleshooting

- **No command in the palette** — you are in the wrong window. Press F5 and use
  the window titled `[Extension Development Host]`.
- **"no frame server"** — build it
  (`cargo build --release -p winter-demo`) or set `winter.serverPath`. The lookup
  starts at the extension's own folder, so a development host with no workspace
  folder still finds `target/release`.
- **`cargo build` says "Access is denied"** — a server from an earlier session is
  still running and Windows will not replace a running `.exe`. Close the Winter
  tab (or end `winter-frame.exe`) and build again.
- **Blank canvas** — look at the `winter-frame` stderr lines in the extension
  host log (`Help → Toggle Developer Tools`), or the **Winter** output channel.

## What it does not do

- No IME: the webview's `keydown` is turned into text or an escape sequence.
- No drag-autoscroll: a drag clamps to the grid, so dragging past the edge does
  not scroll the history.
