// The fixture the live check runs: a failed-command notifier.
//
// `commandFinished` is an observation, so nothing here can change what the
// terminal did. The palette row proves the command path and reads the snapshot
// the host handed over before the handler ran.
//
// Run it with:
//
//   WINTER_EXT=1 WINTER_EXTENSIONS=crates/extensions/fixtures \
//     cargo run -p app

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
  winter.app.log(`pane ${pane.id} in ${pane.program}, ${pane.columns}x${pane.rows}, ${winter.tabs.list().length} tab(s)`);
});
