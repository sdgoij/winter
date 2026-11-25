// The editor half of the embedding demo (ADR 083).
//
// VS Code owns the pane; this extension owns a process that owns the shell and
// the pixels. Commands go to the process's stdin as one JSON object per line, and
// the process answers on stdout with tagged messages: kind 0 is a frame (width,
// height, cell width, cell height, byte length, RGBA) and kind 1 is text for the
// clipboard. The webview paints frames with `putImageData`.

import * as cp from "child_process";
import * as fs from "fs";
import * as path from "path";
import * as vscode from "vscode";

/** A frame message: `u32 0` then five `u32`s. */
const FRAME_HEADER = 24;
/** A text message: `u32 1` and a length. */
const TEXT_HEADER = 8;
/** How often a frame is asked for while a panel is showing. */
const FRAME_INTERVAL = 33;

type Message =
  | { kind: "input"; text: string }
  | { kind: "resize"; cols: number; rows: number }
  | {
      kind: "pointer";
      action: "press" | "move" | "release";
      col: number;
      row: number;
      button: number;
      shift: boolean;
    }
  | { kind: "wheel"; col: number; row: number; lines: number; shift: boolean }
  | { kind: "copy" };

let server: cp.ChildProcessWithoutNullStreams | undefined;
let panel: vscode.WebviewPanel | undefined;
let timer: NodeJS.Timeout | undefined;
let pending = Buffer.alloc(0);

export function activate(context: vscode.ExtensionContext): void {
  context.subscriptions.push(vscode.commands.registerCommand("winter.open", () => open()));
  context.subscriptions.push(new vscode.Disposable(() => stop()));
}

export function deactivate(): void {
  stop();
}

function open(): void {
  if (panel) {
    panel.reveal();
    return;
  }

  panel = vscode.window.createWebviewPanel("winter", "Winter", vscode.ViewColumn.Active, {
    enableScripts: true,
    retainContextWhenHidden: true,
  });
  panel.webview.html = html();
  panel.onDidDispose(() => {
    panel = undefined;
    stop();
  });
  panel.webview.onDidReceiveMessage((message: Message) => {
    switch (message.kind) {
      case "input":
        send({ cmd: "write", text: message.text });
        break;
      case "resize":
        send({ cmd: "resize", cols: message.cols, rows: message.rows });
        break;
      case "pointer":
        send({
          cmd: message.action,
          col: message.col,
          row: message.row,
          button: message.button,
          shift: message.shift,
        });
        break;
      case "wheel":
        send({ cmd: "wheel", col: message.col, row: message.row, lines: message.lines, shift: message.shift });
        break;
      case "copy":
        send({ cmd: "copy" });
        break;
    }
  });

  start();
}

function start(): void {
  const binary = resolveBinary();
  if (!binary) {
    void vscode.window.showErrorMessage(
      "Winter: no frame server. Run `cargo build -p winter-demo` in the workspace, or set `winter.serverPath`.",
    );
    return;
  }

  server = cp.spawn(binary, [], { stdio: ["pipe", "pipe", "pipe"] });
  server.on("error", (error) => void vscode.window.showErrorMessage(`Winter: ${error.message}`));
  server.on("exit", (code) => {
    server = undefined;
    void vscode.window.showInformationMessage(`Winter: the frame server exited (${code ?? "signal"}).`);
  });
  server.stderr.on("data", (chunk: Buffer) => console.error(`winter-frame: ${chunk.toString()}`));
  server.stdout.on("data", (chunk: Buffer) => onData(chunk));

  timer = setInterval(() => send({ cmd: "render" }), FRAME_INTERVAL);
}

function onData(chunk: Buffer): void {
  pending = Buffer.concat([pending, chunk]);

  while (pending.length >= 4) {
    const kind = pending.readUInt32BE(0);

    if (kind === 1) {
      if (pending.length < TEXT_HEADER) {
        return;
      }
      const length = pending.readUInt32BE(4);
      if (pending.length < TEXT_HEADER + length) {
        return;
      }
      const text = pending.subarray(TEXT_HEADER, TEXT_HEADER + length).toString("utf8");
      pending = pending.subarray(TEXT_HEADER + length);
      if (text.length > 0) {
        void vscode.env.clipboard.writeText(text);
      }
      continue;
    }

    if (pending.length < FRAME_HEADER) {
      return;
    }
    const width = pending.readUInt32BE(4);
    const height = pending.readUInt32BE(8);
    const cellWidth = pending.readUInt32BE(12);
    const cellHeight = pending.readUInt32BE(16);
    const length = pending.readUInt32BE(20);
    if (pending.length < FRAME_HEADER + length) {
      return;
    }
    const pixels = pending.subarray(FRAME_HEADER, FRAME_HEADER + length);
    pending = pending.subarray(FRAME_HEADER + length);
    void panel?.webview.postMessage({ kind: "frame", width, height, cellWidth, cellHeight, pixels });
  }
}

function send(command: unknown): void {
  server?.stdin.write(`${JSON.stringify(command)}\n`);
}

function stop(): void {
  if (timer) {
    clearInterval(timer);
    timer = undefined;
  }
  send({ cmd: "quit" });
  server?.stdin.end();
  server = undefined;
  pending = Buffer.alloc(0);
}

function resolveBinary(): string | undefined {
  const configured = vscode.workspace.getConfiguration("winter").get<string>("serverPath");
  if (configured) {
    return configured;
  }

  const name = process.platform === "win32" ? "winter-frame.exe" : "winter-frame";
  // This extension lives in `editors/vscode`, below the Cargo root, so walk up
  // from each workspace folder instead of only looking at the folder itself.
  for (const folder of vscode.workspace.workspaceFolders ?? []) {
    let directory = folder.uri.fsPath;
    for (let depth = 0; depth < 4; depth += 1) {
      for (const profile of ["debug", "release"]) {
        const candidate = path.join(directory, "target", profile, name);
        if (fs.existsSync(candidate)) {
          return candidate;
        }
      }
      const parent = path.dirname(directory);
      if (parent === directory) {
        break;
      }
      directory = parent;
    }
  }

  return undefined;
}

function html(): string {
  return `<!DOCTYPE html>
<html lang="en">
<head>
<meta charset="utf-8">
<style>
  html, body { margin: 0; padding: 0; height: 100%; background: #0c0c0c; overflow: hidden; }
  #screen { display: block; outline: none; user-select: none; -webkit-user-select: none; image-rendering: auto; }
</style>
</head>
<body>
<canvas id="screen" tabindex="0"></canvas>
<script>
  const vscode = acquireVsCodeApi();
  const canvas = document.getElementById("screen");
  const context = canvas.getContext("2d");
  let cell = { width: 9, height: 17 };
  let scale = 1;
  let last = { cols: 0, rows: 0 };

  window.addEventListener("message", (event) => {
    const message = event.data;
    // A zero width is the server saying nothing changed: keep the picture.
    if (message.kind !== "frame" || message.width === 0) {
      return;
    }

    cell = { width: message.cellWidth, height: message.cellHeight };
    if (canvas.width !== message.width || canvas.height !== message.height) {
      canvas.width = message.width;
      canvas.height = message.height;
    }

    // A typed array survives the message as one; anything else arrived as JSON.
    const bytes = message.pixels instanceof Uint8Array
      ? new Uint8ClampedArray(message.pixels.buffer, message.pixels.byteOffset, message.pixels.byteLength)
      : Uint8ClampedArray.from(Object.values(message.pixels));
    context.putImageData(new ImageData(bytes, message.width, message.height), 0, 0);
    fit();
  });

  // The pane is told a grid that fits it, so the server repaints at that size.
  function fit() {
    scale = Math.max(0.25, Math.min(2, (window.innerWidth - 8) / Math.max(1, canvas.width)));
    canvas.style.width = Math.floor(canvas.width * scale) + "px";
    canvas.style.height = Math.floor(canvas.height * scale) + "px";

    const cols = Math.max(20, Math.floor((window.innerWidth - 8) / (cell.width * scale)));
    const rows = Math.max(5, Math.floor((window.innerHeight - 8) / (cell.height * scale)));
    if (cols !== last.cols || rows !== last.rows) {
      last = { cols, rows };
      vscode.postMessage({ kind: "resize", cols, rows });
    }
  }

  window.addEventListener("resize", fit);

  // A pointer position in cells, clamped to the grid the canvas shows.
  function cellAt(event) {
    const rect = canvas.getBoundingClientRect();
    const columns = Math.max(1, Math.round(canvas.width / cell.width));
    const rows = Math.max(1, Math.round(canvas.height / cell.height));
    const col = Math.floor((event.clientX - rect.left) / scale / cell.width);
    const row = Math.floor((event.clientY - rect.top) / scale / cell.height);
    return {
      col: Math.min(columns - 1, Math.max(0, col)),
      row: Math.min(rows - 1, Math.max(0, row)),
    };
  }

  canvas.addEventListener("mousedown", (event) => {
    canvas.focus();
    event.preventDefault();
    vscode.postMessage({
      kind: "pointer",
      action: "press",
      button: event.button,
      shift: event.shiftKey,
      ...cellAt(event),
    });
  });

  window.addEventListener("mousemove", (event) => {
    // DOM: bit 0 is the left button, bit 1 the right, bit 2 the middle.
    const held = (event.buttons & 1) ? 0 : (event.buttons & 4) ? 1 : (event.buttons & 2) ? 2 : -1;
    if (held < 0) {
      return;
    }
    vscode.postMessage({
      kind: "pointer",
      action: "move",
      button: held,
      shift: event.shiftKey,
      ...cellAt(event),
    });
  });

  window.addEventListener("mouseup", (event) => {
    vscode.postMessage({
      kind: "pointer",
      action: "release",
      button: event.button,
      shift: event.shiftKey,
      ...cellAt(event),
    });
  });

  canvas.addEventListener(
    "wheel",
    (event) => {
      event.preventDefault();
      vscode.postMessage({
        kind: "wheel",
        lines: event.deltaY < 0 ? 3 : -3,
        shift: event.shiftKey,
        ...cellAt(event),
      });
    },
    { passive: false },
  );

  const escapes = {
    Enter: "\\r", Backspace: "\\x7f", Tab: "\\t", Escape: "\\x1b",
    ArrowUp: "\\x1b[A", ArrowDown: "\\x1b[B", ArrowRight: "\\x1b[C", ArrowLeft: "\\x1b[D",
    Home: "\\x1b[H", End: "\\x1b[F", Insert: "\\x1b[2~", Delete: "\\x1b[3~",
    PageUp: "\\x1b[5~", PageDown: "\\x1b[6~",
  };

  window.addEventListener("keydown", (event) => {
    // Ctrl+Shift+C and Cmd+C copy; the plain Ctrl+C must still interrupt.
    const copy =
      (event.ctrlKey && event.shiftKey && event.key.toLowerCase() === "c") ||
      (event.metaKey && event.key.toLowerCase() === "c");
    if (copy) {
      event.preventDefault();
      vscode.postMessage({ kind: "copy" });
      return;
    }

    let text = escapes[event.key];
    if (!text && event.ctrlKey && event.key.length === 1) {
      text = String.fromCharCode(event.key.toUpperCase().charCodeAt(0) & 0x1f);
    }
    if (!text && event.key.length === 1 && !event.metaKey) {
      text = event.key;
    }
    if (!text) {
      return;
    }
    event.preventDefault();
    vscode.postMessage({ kind: "input", text });
  });

  window.addEventListener("paste", (event) => {
    const text = event.clipboardData && event.clipboardData.getData("text");
    if (text) {
      event.preventDefault();
      vscode.postMessage({ kind: "input", text });
    }
  });

  canvas.focus();
</script>
</body>
</html>`;
}
