# Winter in a VS Code webview

A demo host for `winter::embed::Surface` (ADR 083). VS Code owns a webview; a
process owns the shell and paints the pixels with Winter's own renderer.

## Run it

```sh
cargo build -p winter-demo          # target/debug/winter-frame
cd editors/vscode
npm install
npm run compile
```

Open `editors/vscode` in VS Code and press <kbd>F5</kbd> (Run Extension), then
run **Winter: Open Terminal** from the command palette. The extension finds
`target/debug/winter-frame` in the first workspace folder; set
`winter.serverPath` to point it somewhere else.

## What it does

- Spawns `winter-frame`, which starts your shell on a pty.
- Sends `{"cmd":"write","text":…}`, `{"cmd":"resize","cols":…,"rows":…}`,
  `{"cmd":"render","cursor":true}` and `{"cmd":"quit"}` as JSON lines.
- Reads frames: `u32` width, height, cell width, cell height and byte length,
  then RGBA bytes, painted with `putImageData`.

## What it does not do

- No selection, copy, scrollback or mouse reporting: the pointer belongs to the
  host (ADR 019), and this host does not implement it.
- No dirty-region or blink optimisation: it asks for a frame every 33 ms.
- No IME: the webview's `keydown` is turned into text or an escape sequence.
