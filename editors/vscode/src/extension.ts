// The editor half of the embedding demo (ADR 084).
//
// VS Code owns the pane; the extension's own process owns the terminal. The
// terminal is `winter`'s surface, reached through the addon in `winter.node`:
// input is a method call, and `render` returns the pixels that changed — a band
// of the pane, or the whole of it — or `null` when nothing did. There is no
// second process and no protocol between the halves (ADR 083 had both).
//
// The pixels travel to the webview base64-encoded and deflated: `postMessage`
// does not promise that a `Buffer` survives as bytes, and 1.8 MB of RGBA per
// frame is more than a canvas can decode at speed.

import * as fs from "fs";
import * as path from "path";
import * as vscode from "vscode";

/** How often a frame is asked for while a panel is showing. */
const FRAME_INTERVAL = 16;
/** How long an unacknowledged frame is waited for before it is asked for again. */
const ACK_TIMEOUT = 500;
/** The grid a terminal starts at, before the pane says how big it is. */
const START_COLUMNS = 100;
/** The grid a terminal starts at. */
const START_ROWS = 30;

/** The pixels that changed, as the addon returns them. */
interface Frame {
  /** Whether it is a band of the pane rather than the whole of it. */
  patch: boolean;
  /** The first pixel row it covers. */
  top: number;
  /** Its width in pixels. */
  width: number;
  /** How many pixel rows it covers. */
  height: number;
  /** Deflated RGBA. */
  pixels: Buffer;
}

/** A grid size in cells. */
interface Grid {
  columns: number;
  rows: number;
}

/** A cell size in pixels. */
interface CellSize {
  width: number;
  height: number;
}

/** Something the program asked the terminal to report. */
interface EventReport {
  kind: string;
  title?: string | null;
  body?: string | null;
}

/** The surface, as `crates/napi` exposes it. */
interface Terminal {
  write(text: string): void;
  paste(text: string): void;
  key(
    name: string,
    text: string,
    code: string,
    shift: boolean,
    ctrl: boolean,
    alt: boolean,
    pressed: boolean,
    repeat: boolean,
  ): void;
  resize(columns: number, rows: number): void;
  press(column: number, row: number, button: number, shift: boolean): void;
  move(column: number, row: number, button: number, shift: boolean): void;
  release(column: number, row: number, button: number, shift: boolean): void;
  wheel(column: number, row: number, lines: number, shift: boolean): void;
  copy(): string | null;
  modes(): string | null;
  render(): Frame | null;
  size(): Grid;
  cellSize(): CellSize;
  cursorVisible(): boolean;
  title(): string | null;
  takeEvents(): EventReport[];
  close(): void;
}

/** The addon module. */
interface Addon {
  defaultShell(): string;
  Terminal: {
    spawn(shell: string, columns: number, rows: number, fontSize?: number): Terminal;
  };
}

type Message =
  | { kind: "input"; text: string }
  | { kind: "resize"; cols: number; rows: number }
  | {
      kind: "key";
      name: string;
      code: string;
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

let terminal: Terminal | undefined;
let panel: vscode.WebviewPanel | undefined;
let log: vscode.OutputChannel | undefined;
/** The pane's script is listening: before that, a frame would be dropped. */
let ready = false;
let announced = false;

// One frame in flight, and no more.
//
// A render costs a few milliseconds for a shell and about six for a full-screen
// program, and the pane cannot paint faster than it can inflate and blit. Asking
// for the next frame only after the pane has painted the last one keeps the two
// in step: a slow end slows the protocol instead of building a queue of frames
// that are stale by the time they are shown.
let next: NodeJS.Timeout | undefined;
let watchdog: NodeJS.Timeout | undefined;
let awaitingPaint = false;
let requestedAt = 0;
/** What the pane is doing, logged every couple of seconds while it runs. */
let paintedSinceReport = 0;
let paintedAtReport = 0;
let paintedBytes = 0;
let renderMillis = 0;
let paintMillis = 0;
let measuredAtReport = 0;

export function activate(context: vscode.ExtensionContext): void {
  log = vscode.window.createOutputChannel("Winter");
  context.subscriptions.push(log);
  context.subscriptions.push(vscode.commands.registerCommand("winter.open", () => open(context)));
  context.subscriptions.push(vscode.commands.registerCommand("winter.modes", diagnose));
  context.subscriptions.push(new vscode.Disposable(() => stop()));
}

export function deactivate(): void {
  stop();
}

/** Logs what the program asked of the input path, which is where a key bug starts. */
function diagnose(): void {
  log?.show(true);
  log?.appendLine(terminal?.modes() ?? "no terminal is open");
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
    awaitingPaint = false;
    pump();
  });
  panel.webview.onDidReceiveMessage((message: Message) => {
    switch (message.kind) {
      case "input":
        input(() => terminal?.write(message.text));
        break;
      case "key":
        input(() =>
          terminal?.key(
            message.name,
            message.text,
            message.code,
            message.shift,
            message.ctrl,
            message.alt,
            message.pressed,
            message.repeat,
          ),
        );
        break;
      case "paste":
        input(() => terminal?.paste(message.text));
        break;
      case "resize":
        input(() => terminal?.resize(message.cols, message.rows));
        break;
      case "pointer":
        input(() => {
          if (message.action === "press") {
            terminal?.press(message.col, message.row, message.button, message.shift);
          } else if (message.action === "move") {
            terminal?.move(message.col, message.row, message.button, message.shift);
          } else {
            terminal?.release(message.col, message.row, message.button, message.shift);
          }
        });
        break;
      case "wheel":
        input(() => terminal?.wheel(message.col, message.row, message.lines, message.shift));
        break;
      case "copy": {
        const text = terminal?.copy();
        if (text) {
          void vscode.env.clipboard.writeText(text);
        }
        break;
      }
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
  const addonPath = resolveAddon(context);
  if (!addonPath) {
    void vscode.window.showErrorMessage(
      "Winter: no winter.node. Run `npm run build:addon` in editors/vscode, or set `winter.addonPath`.",
    );
    return;
  }

  let addon: Addon;
  try {
    addon = require(addonPath) as Addon;
  } catch (error) {
    log?.appendLine(`could not load ${addonPath}: ${error}`);
    void vscode.window.showErrorMessage(`Winter: could not load ${path.basename(addonPath)}: ${error}`);
    return;
  }

  const configured = vscode.workspace.getConfiguration("winter").get<string>("shell");
  const shell = configured && configured.length > 0 ? configured : addon.defaultShell();
  const fontSize = vscode.workspace.getConfiguration("winter").get<number>("fontSize");
  log?.appendLine(`addon: ${addonPath}`);
  log?.appendLine(`shell: ${shell}`);

  try {
    terminal = addon.Terminal.spawn(shell, START_COLUMNS, START_ROWS, fontSize);
  } catch (error) {
    log?.appendLine(`could not start ${shell}: ${error}`);
    void vscode.window.showErrorMessage(`Winter: could not start ${shell}: ${error}`);
    return;
  }

  pump();
}

/** Asks for the next frame, unless one is still with the pane. */
function pump(): void {
  if (next) {
    clearTimeout(next);
    next = undefined;
  }
  if (!terminal || !ready || awaitingPaint) {
    return;
  }
  awaitingPaint = true;
  requestedAt = Date.now();

  const started = Date.now();
  let frame: Frame | null;
  try {
    frame = terminal.render();
  } catch (error) {
    log?.appendLine(`render failed: ${error}`);
    stop();
    return;
  }
  renderMillis += Date.now() - started;

  for (const event of terminal.takeEvents()) {
    if (event.kind === "exited") {
      log?.appendLine("the shell exited");
      stop();
      return;
    }
    log?.appendLine(`${event.kind}${event.title ? `: ${event.title}` : ""}`);
  }

  if (!frame) {
    // Nothing changed: there is nothing to paint, so the next frame may be
    // asked for at once.
    painted();
    return;
  }

  if (!announced) {
    announced = true;
    log?.appendLine(`first frame: ${frame.width}x${frame.height}, ${frame.pixels.length} bytes`);
  }

  paintedBytes += frame.pixels.length;
  const pixels = frame.pixels.toString("base64");
  if (frame.patch) {
    void panel?.webview.postMessage({ kind: "patch", top: frame.top, width: frame.width, height: frame.height, pixels });
  } else {
    const cell = terminal.cellSize();
    void panel?.webview.postMessage({
      kind: "frame",
      width: frame.width,
      height: frame.height,
      cellWidth: cell.width,
      cellHeight: cell.height,
      pixels,
    });
  }

  // The pane answers when it has painted, and `painted` asks for the next one.
  watchdog = setTimeout(askAgain, ACK_TIMEOUT);
}

function askAgain(): void {
  watchdog = undefined;
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
  if (watchdog) {
    clearTimeout(watchdog);
    watchdog = undefined;
  }
  if (!terminal || !ready) {
    return;
  }

  const now = Date.now();
  paintedSinceReport += 1;
  paintMillis += now - requestedAt;
  measuredAtReport += 1;
  report(now);

  // Pace the loop without adding the paint to every frame: a pane slower than
  // the interval simply asks again straight away.
  const spent = now - requestedAt;
  next = setTimeout(pump, Math.max(0, FRAME_INTERVAL - spent));
}

/** Logs what the pane manages, so a host that feels slow can say why. */
function report(now: number): void {
  if (paintedAtReport === 0) {
    paintedAtReport = now;
    return;
  }
  const elapsed = now - paintedAtReport;
  if (elapsed < 2000) {
    return;
  }
  const frames = Math.max(1, paintedSinceReport);
  const measured = Math.max(1, measuredAtReport);
  log?.appendLine(
    `pane: ${Math.round((frames * 1000) / elapsed)} frames/s, ` +
      `render ${Math.round(renderMillis / measured)} ms, ` +
      `paint ${Math.round(paintMillis / measured)} ms, ` +
      `${Math.round(paintedBytes / frames)} bytes a frame`,
  );
  paintedSinceReport = 0;
  renderMillis = 0;
  paintMillis = 0;
  paintedBytes = 0;
  measuredAtReport = 0;
  paintedAtReport = now;
}

/** Runs an input action, then asks for the frame that shows it. */
function input(act: () => void): void {
  act();
  pump();
}

function stop(): void {
  if (next) {
    clearTimeout(next);
    next = undefined;
  }
  if (watchdog) {
    clearTimeout(watchdog);
    watchdog = undefined;
  }
  awaitingPaint = false;
  ready = false;
  announced = false;
  terminal?.close();
  terminal = undefined;
}

/** Where the addon is: the setting, or the copy the build script writes. */
function resolveAddon(context: vscode.ExtensionContext): string | undefined {
  const configured = vscode.workspace.getConfiguration("winter").get<string>("addonPath");
  if (configured) {
    return path.isAbsolute(configured) ? configured : path.join(context.extensionPath, configured);
  }
  const built = path.join(context.extensionPath, "winter.node");
  return fs.existsSync(built) ? built : undefined;
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

  // The chain every message is handled through, so paints stay in order.
  let painting = Promise.resolve();

  window.addEventListener("message", (event) => {
    // One at a time: inflating is asynchronous, and two bands painted out of
    // order would leave the canvas wrong.
    painting = painting.then(() => handle(event.data));
  });

  async function handle(message) {
    // A band is the rows from "top": the rest of the canvas is already right.
    if (message.kind === "patch") {
      try {
        await paintPatch(message);
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
      await paintFrame(message);
    } catch (error) {
      status.textContent = "frame error: " + error;
    }
    painted();
  }

  async function paintPatch(message) {
    const bytes = await pixels(message);
    if (bytes.length !== message.width * message.height * 4) {
      status.textContent =
        "patch: " + bytes.length + " bytes for " + message.width + "x" + message.height;
      return;
    }
    context.putImageData(new ImageData(bytes, message.width, message.height), 0, message.top);
    status.textContent = "";
  }

  async function paintFrame(message) {
    cell = { width: message.cellWidth, height: message.cellHeight };
    if (canvas.width !== message.width || canvas.height !== message.height) {
      canvas.width = message.width;
      canvas.height = message.height;
    }

    const bytes = await pixels(message);
    if (bytes.length !== message.width * message.height * 4) {
      status.textContent =
        "frame: " + bytes.length + " bytes for " + message.width + "x" + message.height;
      return;
    }

    context.putImageData(new ImageData(bytes, message.width, message.height), 0, 0);
    status.textContent = "";
    fit();
  }

  // The pixels arrive deflated. A full-screen program repaints 1.8 MB of RGBA
  // every frame, and decoding that from base64 a byte at a time is what a pane
  // cannot do at speed; inflating is native and the byte loop then runs over a
  // fraction of the bytes.
  async function pixels(message) {
    if (typeof message.pixels !== "string" || typeof DecompressionStream === "undefined") {
      throw new Error("cannot inflate a " + typeof message.pixels + " payload");
    }
    const raw = atob(message.pixels);
    const deflated = new Uint8Array(raw.length);
    for (let index = 0; index < raw.length; index += 1) {
      deflated[index] = raw.charCodeAt(index);
    }
    const stream = new Blob([deflated]).stream().pipeThrough(new DecompressionStream("deflate-raw"));
    return new Uint8ClampedArray(await new Response(stream).arrayBuffer());
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
      // The physical key, which is what a Win32 input record names; the name
      // above follows the layout, so it cannot say which key is held.
      code: event.code || "",
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
