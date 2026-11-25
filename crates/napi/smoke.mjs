// A manual check that the addon drives a shell, not a unit test: it needs a
// built `winter.node` and a console to run on.
//
//   cargo build -p winter-node
//   node crates/napi/smoke.mjs target/debug/winter_node.node
//
// It asserts the things a host depends on: the pane paints, a keystroke reaches
// the shell and the shell answers, a resize repaints at the new grid, and
// closing the terminal ends it.

import { createRequire } from "node:module";
import { resolve } from "node:path";
import { inflateRawSync } from "node:zlib";
import { strict as assert } from "node:assert";

const require = createRequire(import.meta.url);
const addon = require(resolve(process.argv[2]));
assert.ok(addon.Terminal, "the addon exposes Terminal");

const terminal = addon.Terminal.spawn(process.env.COMSPEC ?? "cmd.exe", 80, 24);

/// Renders until something changed, or the time runs out.
async function painted(target, millis) {
  const until = Date.now() + millis;
  while (Date.now() < until) {
    const frame = target.render();
    if (frame) {
      return frame;
    }
    await new Promise((resolve) => setTimeout(resolve, 16));
  }
  return null;
}

/// The pixels a frame carries, with the host's deflate undone.
function pixels(frame) {
  const bytes = inflateRawSync(frame.pixels);
  assert.equal(bytes.length, frame.width * frame.height * 4, "a frame is its own pixels");
  return bytes;
}

try {
  const cell = terminal.cellSize();
  console.log("cell", cell, "grid", terminal.size());
  assert.ok(cell.width > 0 && cell.height > 0, "the font was measured");

  const first = await painted(terminal, 5000);
  assert.ok(first, "the first frame arrived");
  assert.equal(first.patch, false, "the first frame is the whole pane");
  assert.equal(first.width, 80 * cell.width);
  assert.equal(first.height, 24 * cell.height);
  assert.ok(pixels(first).some((byte) => byte !== 0), "the pane has ink on it");

  // A command typed as text, then Enter as a key: the shell has to answer.
  terminal.write("echo smoke");
  terminal.key("Enter", "\r", "Enter", false, false, false, true, false);
  terminal.key("Enter", "\r", "Enter", false, false, false, false, false);
  const echo = await painted(terminal, 5000);
  assert.ok(echo, "the shell answered the Enter");
  pixels(echo);

  terminal.resize(100, 30);
  const resized = await painted(terminal, 5000);
  assert.ok(resized, "a resize paints");
  assert.equal(resized.width, 100 * cell.width);
  assert.equal(resized.height, 30 * cell.height);
  pixels(resized);

  console.log("modes", terminal.modes());
  console.log("title", terminal.title(), "cursor", terminal.cursorVisible());
  console.log("ok");
} finally {
  terminal.close();
}

assert.equal(terminal.render(), null, "a closed terminal paints nothing");
terminal.close();
console.log("closed");
