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
server, and opens an **Extension Development Host** — a second VS Code window
with this extension loaded.

In that second window, run **Winter: Open Terminal** from the command palette
(<kbd>Ctrl</kbd>+<kbd>Shift</kbd>+<kbd>P</kbd>).

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
- Sends `{"cmd":"write","text":…}`, `{"cmd":"resize","cols":…,"rows":…}`,
  `{"cmd":"render","cursor":true}` and `{"cmd":"quit"}` as JSON lines.
- Reads frames: `u32` width, height, cell width, cell height and byte length,
  then RGBA bytes, painted with `putImageData`.

The server is found by walking up from the workspace folder to
`target/debug/winter-frame`, or wherever `winter.serverPath` points.

## Troubleshooting

- **No command in the palette** — you are in the wrong window. Press F5 and use
  the window titled `[Extension Development Host]`.
- **"no frame server"** — build it (`cargo build -p winter-demo`) or set
  `winter.serverPath`.
- **Blank canvas** — look at the `winter-frame` stderr lines in the extension
  host log (`Help → Toggle Developer Tools`).

## What it does not do

- No selection, copy, scrollback or mouse reporting: the pointer belongs to the
  host (ADR 019), and this host does not implement it.
- No dirty-region or blink optimisation: it asks for a frame every 33 ms.
- No IME: the webview's `keydown` is turned into text or an escape sequence.
