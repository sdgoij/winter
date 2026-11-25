// The editor half of the embedding demo (ADR 083).
//
// VS Code owns the pane; this extension owns a process that owns the shell and
// the pixels. Commands go to the process's stdin as one JSON object per line, and
// frames come back as five u32s (width, height, cell width, cell height, length)
// then RGBA bytes, which the webview paints with `putImageData`.

import * as cp from "child_process";
import * as fs from "fs";
import * as path from "path";
import * as vscode from "vscode";

/** The frame header: five big-endian u32s. */
const HEADER = 20;
/** How often a frame is asked for while a panel is showing. */
const FRAME_INTERVAL = 33;

type Message = { kind: "input"; text: string } | { kind: "resize"; cols: number; rows: number };

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
    if (message.kind === "input") {
      send({ cmd: "write", text: message.text });
    } else if (message.kind === "resize") {
      send({ cmd: "resize", cols: message.cols, rows: message.rows });
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

  timer = setInterval(() => send({ cmd: "render", cursor: true }), FRAME_INTERVAL);
}

function onData(chunk: Buffer): void {
  pending = Buffer.concat([pending, chunk]);

  while (pending.length >= HEADER) {
    const width = pending.readUInt32BE(0);
    const height = pending.readUInt32BE(4);
    const cellWidth = pending.readUInt32BE(8);
    const cellHeight = pending.readUInt32BE(12);
    const length = pending.readUInt32BE(16);
    if (pending.length < HEADER + length) {
      return;
    }

    const pixels = pending.subarray(HEADER, HEADER + length);
    pending = pending.subarray(HEADER + length);
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
  #screen { display: block; outline: none; image-rendering: auto; }
</style>
</head>
<body>
<canvas id="screen" tabindex="0"></canvas>
<script>
  const vscode = acquireVsCodeApi();
  const canvas = document.getElementById("screen");
  const context = canvas.getContext("2d");
  let cell = { width: 9, height: 17 };
  let last = { cols: 0, rows: 0 };

  window.addEventListener("message", (event) => {
    const message = event.data;
    if (message.kind !== "frame") {
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
    const scale = Math.max(0.25, Math.min(2, (window.innerWidth - 8) / Math.max(1, canvas.width)));
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

  const escapes = {
    Enter: "\\r", Backspace: "\\x7f", Tab: "\\t", Escape: "\\x1b",
    ArrowUp: "\\x1b[A", ArrowDown: "\\x1b[B", ArrowRight: "\\x1b[C", ArrowLeft: "\\x1b[D",
    Home: "\\x1b[H", End: "\\x1b[F", Insert: "\\x1b[2~", Delete: "\\x1b[3~",
    PageUp: "\\x1b[5~", PageDown: "\\x1b[6~",
  };

  window.addEventListener("keydown", (event) => {
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
  canvas.addEventListener("click", () => canvas.focus());
</script>
</body>
</html>`;
}
