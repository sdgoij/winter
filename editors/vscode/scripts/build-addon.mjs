// Copies the built addon where the extension loads it from.
//
// `cargo build` names a shared library (`winter_node.dll` and friends); Node
// only loads an addon from a file named `.node`, so the extension gets the copy
// under that name (ADR 084).
//
// A running development host holds the file open, and Windows will not replace
// a file that is open: close the Winter tab before rebuilding.

import { execFileSync } from "node:child_process";
import { copyFileSync, mkdirSync } from "node:fs";
import { dirname, join, resolve } from "node:path";
import { fileURLToPath } from "node:url";

const here = dirname(fileURLToPath(import.meta.url));
const root = resolve(here, "..", "..", "..");
const profile = process.env.WINTER_PROFILE ?? "release";

const library =
  process.platform === "win32"
    ? "winter_node.dll"
    : process.platform === "darwin"
      ? "libwinter_node.dylib"
      : "libwinter_node.so";

execFileSync("cargo", ["build", "-p", "winter-node", ...(profile === "release" ? ["--release"] : [])], {
  cwd: root,
  stdio: "inherit",
});

const built = join(root, "target", profile, library);
const destination = join(here, "..", "winter.node");
mkdirSync(dirname(destination), { recursive: true });

try {
  copyFileSync(built, destination);
} catch (error) {
  console.error(`could not write ${destination}: ${error.message}`);
  console.error("close the Winter tab (or the development host) and run this again");
  process.exit(1);
}

console.log(`addon: ${destination}`);
