// The editor half of the embedding demo (ADR 083).
//
// VS Code owns the pane; this extension owns a process that owns the shell and
// the pixels. Commands go to the process's stdin as one JSON object per line, and
// the process answers on stdout with tagged messages: kind 0 is a whole frame
// (width, height, cell width, cell height, byte length, RGBA), kind 1 is text for
// the clipboard, and kind 2 is a band — the rows that changed, with their top
// edge — which is what a keystroke costs instead of the whole pane.
//
// The pixels travel base64-encoded: a webview's `postMessage` does not promise
// that a `Buffer` or a typed array survives as bytes, and a frame that arrives
// as an object is a blank canvas with no error to show.

import * as cp from "child_process";
import * as fs from "fs";
import * as path from "path";
import * as vscode from "vscode";

/** A frame message: `u32 0` then five `u32`s, then base64 RGBA. */
const FRAME_HEADER = 24;
/** A text message: `u32 1` and a length. */
const TEXT_HEADER = 8;
/** How often a frame is asked for while a panel is showing. */
const FRAME_INTERVAL = 33;

type Message =
  | { kind: "input"; text: string }
  | { kind: "resize"; cols: number; rows: number }
  | {
      kind: "key";
      name: string;
      text: string;
      shift: boolean;
      ctrl: boolean;
      alt: boolean;
      pressed: boolean;
      repeat: boolean;
    }
  | { kind: "paste"; text: string }
  | {
      kind: "pointer";
      action: "press" | "move" | "release";
      col: number;
      row: number;
      button: number;
      shift: boolean;
    }
  | { kind: "wheel"; col: number; row: number; lines: number; shift: boolean }
  | { kind: "copy" }
  | { kind: "painted" }
  | { kind: "ready" };

let server: cp.ChildProcessWithoutNullStreams | undefined;
let panel: vscode.WebviewPanel | undefined;
let log: vscode.OutputChannel | undefined;
let pending = Buffer.alloc(0);
let announced = false;

// One frame in flight, and no more.
//
// The server answers commands in the order they arrive, so a host that asks for
// a frame on a timer while the server is slower than the timer queues renders
// without bound: the pane falls further and further behind real time, and a
// keystroke sits behind every frame already asked for. A full-screen game makes
// that happen immediately — a 900x510 frame costs a few milliseconds in a
// release build and tens of milliseconds in a debug one. Asking for the next
// frame only after the pane has painted the last one keeps the queue at length
// zero however slow either end is.
let pumpHandle: NodeJS.Timeout | undefined;
let awaitingPaint = false;
let requestedAt = 0;
/** The pane's script is listening: before that, a frame would be dropped. */
let ready = false;
/** How long an unacknowledged frame is waited for before it is asked for again. */
const ACK_TIMEOUT = 1000;

export function activate(context: vscode.ExtensionContext): void {
  log = vscode.window.createOutputChannel("Winter");
  context.subscriptions.push(log);
  context.subscriptions.push(vscode.commands.registerCommand("winter.open", () => open(context)));
  context.subscriptions.push(new vscode.Disposable(() => stop()));
}

export function deactivate(): void {
  stop();
}

function open(context: vscode.ExtensionContext): void {
  if (panel) {
    panel.reveal();
    return;
  }

  panel = vscode.window.createWebviewPanel("winter", "Winter", vscode.ViewColumn.Active, {
    enableScripts: true,
    retainContextWhenHidden: true,
  });
  panel.webview.html = html();
  ready = false;
  panel.onDidDispose(() => {
    panel = undefined;
    stop();
  });
  // A hidden pane paints nothing, so the loop stops on its own; showing it again
  // has to ask for a frame, and the frame it was sent may never have arrived.
  panel.onDidChangeViewState(() => {
    if (panel?.visible) {
      awaitingPaint = false;
      pump();
    }
  });
  panel.webview.onDidReceiveMessage((message: Message) => {
    switch (message.kind) {
      case "input":
        send({ cmd: "write", text: message.text });
        break;
      case "key":
        send({
          cmd: "key",
          name: message.name,
          text: message.text,
          shift: message.shift,
          ctrl: message.ctrl,
          alt: message.alt,
          pressed: message.pressed,
          repeat: message.repeat,
        });
        break;
      case "paste":
        send({ cmd: "paste", text: message.text });
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
      case "painted":
        painted();
        break;
      case "ready":
        ready = true;
        pump();
        break;
    }
  });

  start(context);
}

function start(context: vscode.ExtensionContext): void {
  const binary = resolveBinary(context);
  if (!binary) {
    void vscode.window.showErrorMessage(
      `Winter: no ${serverName()}. Run \`cargo build --release -p winter-demo\` in the repository, or set ` +
        `\`winter.serverPath\`. Searched upward from ${context.extensionPath}.`,
    );
    return;
  }

  const shell = vscode.workspace.getConfiguration("winter").get<string>("shell");
  const args = shell ? ["--shell", shell] : [];
  log?.appendLine(`server: ${binary} ${args.join(" ")}`.trimEnd());
  server = cp.spawn(binary, args, { stdio: ["pipe", "pipe", "pipe"] });
  server.on("error", (error) => {
    log?.appendLine(`spawn failed: ${error.message}`);
    void vscode.window.showErrorMessage(`Winter: ${error.message}`);
  });
  server.on("exit", (code, signal) => {
    log?.appendLine(`server exited: ${code ?? signal}`);
    server = undefined;
  });
  server.stderr.on("data", (chunk: Buffer) => log?.append(chunk.toString()));
  server.stdout.on("data", (chunk: Buffer) => onData(chunk));

  pump();
}

/** Asks for the next frame, unless one is still outstanding. */
function pump(): void {
  if (pumpHandle) {
    clearTimeout(pumpHandle);
    pumpHandle = undefined;
  }
  if (!server || !ready || awaitingPaint || !panel?.visible) {
    return;
  }
  awaitingPaint = true;
  requestedAt = Date.now();
  send({ cmd: "render" });
  // A frame that is never painted — a reloaded webview, a dropped message —
  // would otherwise stop the loop for good.
  pumpHandle = setTimeout(askAgain, ACK_TIMEOUT);
}

function askAgain(): void {
  pumpHandle = undefined;
  if (!awaitingPaint) {
    return;
  }
  awaitingPaint = false;
  pump();
}

/** The pane has painted the frame it was sent, so the next one may be asked for. */
function painted(): void {
  if (!awaitingPaint) {
    return;
  }
  awaitingPaint = false;
  if (!server || !ready) {
    return;
  }
  // Pace the loop without adding the paint time to every frame: a host that is
  // slower than the interval simply asks again straight away.
  const spent = Date.now() - requestedAt;
  if (pumpHandle) {
    clearTimeout(pumpHandle);
  }
  pumpHandle = setTimeout(pump, Math.max(0, FRAME_INTERVAL - spent));
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
    const length = pending.readUInt32BE(20);
    if (pending.length < FRAME_HEADER + length) {
      return;
    }

    // Kind 2 is a band: the rows from `top`, `height` pixels of them.
    if (kind === 2) {
      const top = pending.readUInt32BE(4);
      const width = pending.readUInt32BE(8);
      const height = pending.readUInt32BE(12);
      const pixels = pending.subarray(FRAME_HEADER, FRAME_HEADER + length).toString("base64");
      pending = pending.subarray(FRAME_HEADER + length);
      void panel?.webview.postMessage({ kind: "patch", top, width, height, pixels });
      continue;
    }

    const width = pending.readUInt32BE(4);
    const height = pending.readUInt32BE(8);
    const cellWidth = pending.readUInt32BE(12);
    const cellHeight = pending.readUInt32BE(16);
    const pixels = pending.subarray(FRAME_HEADER, FRAME_HEADER + length).toString("base64");
    pending = pending.subarray(FRAME_HEADER + length);

    // A zero width is the server saying nothing changed: there is nothing for
    // the pane to paint, so the next frame may be asked for at once.
    if (width === 0) {
      painted();
      continue;
    }

    if (!announced) {
      announced = true;
      log?.appendLine(`first frame: ${width}x${height}, ${length} bytes`);
    }
    void panel?.webview.postMessage({ kind: "frame", width, height, cellWidth, cellHeight, pixels });
    continue;
  }
}

function send(command: unknown): void {
  server?.stdin.write(`${JSON.stringify(command)}\n`);
}

function stop(): void {
  if (pumpHandle) {
    clearTimeout(pumpHandle);
    pumpHandle = undefined;
  }
  awaitingPaint = false;
  ready = false;
  send({ cmd: "quit" });
  server?.stdin.end();
  server = undefined;
  pending = Buffer.alloc(0);
  announced = false;
}

function serverName(): string {
  return process.platform === "win32" ? "winter-frame.exe" : "winter-frame";
}

function resolveBinary(context: vscode.ExtensionContext): string | undefined {
  const configured = vscode.workspace.getConfiguration("winter").get<string>("serverPath");
  if (configured) {
    return configured;
  }

  const name = serverName();
  // The extension's own folder comes first: a development host can open with no
  // workspace folder at all, and `editors/vscode` always sits below the Cargo
  // root, so walking up from here finds `target/debug` either way.
  const roots = [
    context.extensionPath,
    process.cwd(),
    ...(vscode.workspace.workspaceFolders ?? []).map((folder) => folder.uri.fsPath),
  ];

  for (const root of roots) {
    let directory = root;
    for (let depth = 0; depth < 5; depth += 1) {
      for (const profile of ["release", "debug"]) {
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
  body { position: relative; }
  #screen { display: block; outline: none; user-select: none; -webkit-user-select: none; image-rendering: auto; }
  #status { position: absolute; left: 10px; top: 8px; margin: 0; color: #cccccc;
            font: 12px/1.4 monospace; white-space: pre-wrap; }
</style>
</head>
<body>
<canvas id="screen" tabindex="0"></canvas>
<pre id="status">waiting for the first frame…</pre>
<script>
  const vscode = acquireVsCodeApi();
  const canvas = document.getElementById("screen");
  const context = canvas.getContext("2d");
  const status = document.getElementById("status");
  let cell = { width: 9, height: 17 };
  let scale = 1;
  let last = { cols: 0, rows: 0 };

  // Every frame the pane is sent is acknowledged once it is on the canvas. The
  // host asks for the next one only then, so a pane that cannot keep up slows
  // the protocol down instead of building a queue of frames it will never show.
  function painted() {
    vscode.postMessage({ kind: "painted" });
  }

  window.addEventListener("message", (event) => {
    const message = event.data;

    // A band is the rows from "top": the rest of the canvas is already right.
    if (message.kind === "patch") {
      try {
        paintPatch(message);
      } catch (error) {
        status.textContent = "patch error: " + error;
      }
      painted();
      return;
    }

    // A zero width is the server saying nothing changed: keep the picture.
    if (message.kind !== "frame" || message.width === 0) {
      return;
    }

    try {
      paintFrame(message);
    } catch (error) {
      status.textContent = "frame error: " + error;
    }
    painted();
  });

  function paintPatch(message) {
    const bytes = decode(message.pixels);
    if (!bytes || bytes.length !== message.width * message.height * 4) {
      status.textContent =
        "patch: " + (bytes ? bytes.length : "no") + " bytes for " + message.width + "x" + message.height;
      return;
    }
    context.putImageData(new ImageData(bytes, message.width, message.height), 0, message.top);
    status.textContent = "";
  }

  function paintFrame(message) {
    cell = { width: message.cellWidth, height: message.cellHeight };
    if (canvas.width !== message.width || canvas.height !== message.height) {
      canvas.width = message.width;
      canvas.height = message.height;
    }

    const bytes = decode(message.pixels);
    if (!bytes) {
      status.textContent = "frame: unexpected pixel payload (" + typeof message.pixels + ")";
      return;
    }
    if (bytes.length !== message.width * message.height * 4) {
      status.textContent =
        "frame: " + bytes.length + " bytes for " + message.width + "x" + message.height;
      return;
    }

    context.putImageData(new ImageData(bytes, message.width, message.height), 0, 0);
    status.textContent = "";
    fit();
  }

  function decode(pixels) {
    if (typeof pixels === "string") {
      const raw = atob(pixels);
      const bytes = new Uint8ClampedArray(raw.length);
      for (let index = 0; index < raw.length; index += 1) {
        bytes[index] = raw.charCodeAt(index);
      }
      return bytes;
    }
    if (pixels instanceof Uint8Array) {
      return new Uint8ClampedArray(pixels.buffer, pixels.byteOffset, pixels.byteLength);
    }
    if (pixels && pixels.type === "Buffer" && Array.isArray(pixels.data)) {
      return Uint8ClampedArray.from(pixels.data);
    }
    return null;
  }

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

    // The command key belongs to the editor on macOS; sending it as text would
    // type the letter of a shortcut. (The copy chord above is the exception.)
    if (event.metaKey) {
      return;
    }

    event.preventDefault();
    sendKey(event, true);
  });

  window.addEventListener("keyup", (event) => {
    if (event.metaKey) {
      return;
    }
    sendKey(event, false);
  });

  // The terminal encodes a key, not the webview: it knows the input modes and
  // the keyboard protocol the program asked for.
  function sendKey(event, pressed) {
    const name = event.key;
    const typed = escapes[name] || (name.length === 1 && !event.ctrlKey ? name : "");
    vscode.postMessage({
      kind: "key",
      name,
      text: typed,
      shift: event.shiftKey,
      ctrl: event.ctrlKey,
      alt: event.altKey,
      pressed,
      repeat: pressed && event.repeat,
    });
  }

  window.addEventListener("paste", (event) => {
    const text = event.clipboardData && event.clipboardData.getData("text");
    if (text) {
      event.preventDefault();
      vscode.postMessage({ kind: "paste", text });
    }
  });

  canvas.focus();

  // The pane is listening: any frame sent before this would have no listener to
  // paint it and nothing would ever acknowledge it.
  vscode.postMessage({ kind: "ready" });
</script>
</body>
</html>`;
}
