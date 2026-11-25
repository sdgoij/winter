# Winter in a VS Code webview

A demo terminal for `winter::embed::Surface` (ADR 084). VS Code owns a webview;
the extension's own process owns the shell and paints the pixels with Winter's
own renderer, through a small Node addon.

## Run it

```sh
cd editors/vscode
npm install                    # once
npm run build:addon            # cargo build -p winter-node, copied in as winter.node
```

Then open **`editors/vscode`** in VS Code and press <kbd>F5</kbd>. That runs the
`Run Winter demo` launch configuration, which compiles the extension, builds the
addon, and opens an **Extension Development Host** — a second VS Code window,
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
cd editors/vscode && npm install && npm run build
code --extensionDevelopmentPath="$PWD"
```

(`codium --extensionDevelopmentPath="$PWD"` for VSCodium.)

## What it does

- Loads `winter.node` and starts your shell on a pty with `Terminal.spawn`. There
  is no second process and no protocol: the terminal is in the extension's own
  process, and a closed pane kills the shell.
- Drives it by calling it. A key travels as its **name, its physical `code` and
  its modifiers**, not as the characters a webview guessed: the terminal applies
  the input modes and the keyboard protocol the program asked for, so function
  keys, Shift+Tab and modified arrows work. A ConPTY asks for Win32 input
  records, and the record's virtual key and scan code come from the physical
  code — a record without them is one text byte, which the console answers with
  a press and a release together, so a held key would move for a single frame.
- Paints what came back. `render()` returns the pixels that changed — a **band**
  of the pane, or the whole of it — or `null` when nothing did; a keystroke is
  about 10 KB and a full game frame about 53 KB, because both are **deflated**
  and the webview inflates them with `DecompressionStream` before
  `putImageData`.
- Keeps **one frame in flight**: it renders, waits for the canvas to report it
  painted, and only then renders again, no sooner than 16 ms later. A full-screen
  program sets the pace, and input asks for the frame that shows it straight
  away. Rendering on a timer without waiting is a queue that never drains, and
  keystrokes end up behind every frame already asked for.
- Says how it is doing: every couple of seconds it logs `pane: N frames/s,
  render X ms, paint Y ms, Z bytes a frame` to the **Winter** output channel, so
  "it feels slow" has a number attached to it.

## Controls

- **Type** — keys, `Ctrl+<letter>` as control codes, paste from the clipboard
  through the terminal's bracketed-paste filter.
- **Select** — click a cell, drag to extend, double-click for a word, triple
  click for the row. The terminal decides what a click means (`Surface::press`).
- **Copy** — <kbd>Ctrl</kbd>+<kbd>Shift</kbd>+<kbd>C</kbd>, or
  <kbd>Cmd</kbd>+<kbd>C</kbd>. Plain `Ctrl+C` still interrupts.
- **Scroll** — the wheel. A program that tracks the mouse gets the wheel
  instead, exactly as `winter`'s shells behave; hold Shift to keep it here.
- **Click in a tracking program** — forwarded as a mouse report, unless Shift is
  held, which keeps the pointer for selection.
- **Winter: Show What the Program Asked For** — logs the terminal's input modes
  and kitty keyboard flags. When a key does something surprising, this is the
  first question: a program that pushes the kitty protocol asks for encodings a
  plain shell never sees.

The addon is loaded from `winter.node` beside the extension, which
`npm run build:addon` writes, or from wherever `winter.addonPath` points.

## Troubleshooting

- **No command in the palette** — you are in the wrong window. Press F5 and use
  the window titled `[Extension Development Host]`.
- **"no winter.node"** — run `npm run build:addon` in `editors/vscode`, or set
  `winter.addonPath`.
- **`build:addon` says "could not write ... winter.node"** — a running
  development host holds the addon open, and Windows will not replace an open
  file. Close the Winter tab (or the dev host) and run it again.
- **"could not load winter.node"** — the addon was built for another platform or
  arch, or by a different Node ABI. Rebuild it on the machine that runs it.
- **A "no monospace face was found" error** — the host has no font in the
  system's font directories that reports itself as monospace.
- **Blank canvas** — look at the **Winter** output channel, which carries the
  addon path, the shell, the first frame's size and the pane's pace.

## What it does not do

- No IME: the webview's `keydown` is turned into text or an escape sequence.
- No drag-autoscroll: a drag clamps to the grid, so dragging past the edge does
  not scroll the history.
- No packaging: `winter.node` is built for the machine you run it on. Shipping
  the extension would mean one prebuilt addon per platform and arch, picked at
  activation (ADR 084).
