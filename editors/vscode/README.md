# Winter in a VS Code webview

A demo host for `winter::embed::Surface` (ADR 083). VS Code owns a webview; a
process owns the shell and paints the pixels with Winter's own renderer.

## Run it

```sh
cargo build -p winter-demo     # target/debug/winter-frame
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
cargo build -p winter-demo
cd editors/vscode && npm install && npm run compile
code --extensionDevelopmentPath="$PWD"
```

(`codium --extensionDevelopmentPath="$PWD"` for VSCodium.)

## What it does

- Spawns `winter-frame`, which starts your shell on a pty.
- Sends `key`, `paste`, `write`, `resize`, `render`, `press`, `move`, `release`,
  `wheel`, `copy` and `quit` as JSON lines. A key travels as its **name and
  modifiers**, not as the characters a webview guessed: the terminal applies the
  input modes and the keyboard protocol the program asked for, so function keys,
  Shift+Tab and modified arrows work.
- Reads tagged messages: kind 0 is a whole frame (`u32` width, height, cell
  width, cell height, byte length, then RGBA), painted with `putImageData`; kind
  1 is text for the clipboard; kind 2 is a **band** — the rows that changed, with
  their top edge — so a keystroke sends about 60 KB instead of 1.8 MB. A frame
  with a **zero width** means nothing changed, and the host keeps what it has.
- Idles cheaply: it asks for a frame every 33 ms, but an unchanged pane answers
  with 24 bytes. The cursor blinks on a 500 ms half-period, and stops asking for
  frames at all while it is hidden.

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
workspace folder) to `target/debug/winter-frame`, or wherever
`winter.serverPath` points.

## Troubleshooting

- **No command in the palette** — you are in the wrong window. Press F5 and use
  the window titled `[Extension Development Host]`.
- **"no frame server"** — build it (`cargo build -p winter-demo`) or set
  `winter.serverPath`. The lookup starts at the extension's own folder, so a
  development host with no workspace folder still finds `target/debug`.
- **`cargo build` says "Access is denied"** — a server from an earlier session is
  still running and Windows will not replace a running `.exe`. Close the Winter
  tab (or end `winter-frame.exe`) and build again.
- **Blank canvas** — look at the `winter-frame` stderr lines in the extension
  host log (`Help → Toggle Developer Tools`), or the **Winter** output channel.

## What it does not do

- No IME: the webview's `keydown` is turned into text or an escape sequence.
- No drag-autoscroll: a drag clamps to the grid, so dragging past the edge does
  not scroll the history.
