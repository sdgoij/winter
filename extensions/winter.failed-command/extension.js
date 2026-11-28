// The fixture the live check runs: a failed-command notifier.
//
// `commandFinished` is an observation, so nothing here can change what the
// terminal did. The palette row proves the command path and reads the snapshot
// the host handed over before the handler ran.
//
// Run it with:
//
//   WINTER_EXT=1 WINTER_EXTENSIONS=extensions \\
//     cargo run -p app

// The bytes of a declared asset, read at load: the same bytes a compiled module
// would copy into its own memory.
const greeting = winter.assets.bytes("greeting");
let text = "";
for (let i = 0; i < greeting.length; i++) {
  text += String.fromCharCode(greeting[i]);
}
winter.app.log("asset: " + text.trim());

winter.events.on("commandFinished", (event) => {
  if (!event.error) {
    return;
  }
  const pane = winter.panes.focused();
  winter.app.notify("Failed command", pane ? `${pane.title} exited non-zero` : "a command exited non-zero");
});

winter.commands.add("where-am-i", "Failed command: where am I", () => {
  const pane = winter.panes.focused();
  if (!pane) {
    winter.app.log("no focused pane");
    return;
  }
  winter.app.log(
    `pane ${pane.id} in ${pane.program}, ${pane.columns}x${pane.rows}, ${winter.tabs.list().length} tab(s)`,
  );
  // The scrollback the host gathered, trimmed to the last two rows.
  const history = winter.panes.history(2);
  winter.app.log(history === "" ? "no history" : `history: ${history.split("\n").length} row(s)`);
});
