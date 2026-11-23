# ADR 059 — Enter marks a submitted command line as a prompt

- Status: Accepted
- Date: 2026-11-23
- Related: `crates/term/src/terminal.rs`, `crates/adapter/src/adapt_dispatch.rs`, `crates/buffer/src/marks.rs`, `crates/winter/src/grid.rs`, `crates/winter/src/winui/scrollbar.rs`, `crates/winter/src/terminal_widget.rs`, ADR 018, ADR 055

## Context

The scrollbar's ticks come from shell-integration marks only: `OSC 133;A`,
`OSC 1337` and `OSC 633`. The default Windows profile is `powershell.exe`, and
neither Windows PowerShell 5.1 nor pwsh 7 emits `OSC 133` in our pty — measured
with a probe that counted zero marks with and without `WT_SESSION` set. The
bundled `OpenConsole.exe` cannot supply them either: upstream's
`ConhostInternalGetSet::NotifyShellIntegrationMark` is a no-op, because marks
are a Terminal-app feature.

So on the platform whose default shell sends nothing, the bar had no ticks at
all, and both shells looked like the feature was broken.

Upstream has the same gap and fills it with `autoMarkPrompts`
(`Terminal.cpp`): on an Enter key-down outside the alternate buffer it calls
`TextBuffer::StartOutput()` and then `ManuallyMarkRowAsPrompt` on the cursor
row, heuristically treating the line just submitted as a prompt.

## Decision

- `Terminal::handle_key` asks the adapter to mark a prompt when a pressed
  Enter or NumpadEnter arrives and the terminal is not in the alternate buffer.
  A full-screen program owns its screen and its Enter is not a prompt.
- `AdaptDispatch::mark_prompt` marks the cursor's **document** row, not the
  screen row. Marks are stored against document rows while the cursor reports a
  screen row, so the offset (`scrollback_len + cursor row`) now lives in one
  helper, `cursor_document_row`, shared with the `OSC 133` path.
- `Marks::mark_prompt_row` keeps adjacent rows as separate marks, unlike
  `start_prompt`, which merges them: one command line is one tick. It leaves a
  row alone when a prompt mark already covers it, so a shell that does send
  `OSC 133;A` is not marked twice.
- The tick carries no exit code, so it is never drawn red. Only a
  shell-integration `D` mark can report a failure.
- Prompt ticks paint over the scrollbar thumb in both shells. Upstream's mark
  image (`ScrollBarCanvas`) is declared after its `ScrollBar` and therefore
  draws above the thumb; an overlay that painted the ticks first hid the most
  recent ones, which are the ones under the thumb while the view is at the
  bottom.

## Consequences

- The bar has ticks for every shell, including the default Windows profile,
  with no shell integration installed and no new setting: upstream's
  `AutoMarkPrompts` defaults on, and so does this.
- Scrollback is not required for correctness but the bar only appears once the
  document is taller than the pane, so a fresh tab shows no strip.
- An automatic mark cannot know whether the command failed, so a session
  without shell integration shows ticks but never a red one.
- The document-row rule is now a single expression, so a future mark site
  cannot repeat the screen-row mistake that placed ticks near the top of the
  document.

## Alternatives considered

- **Have conhost forward the marks.** It explicitly does not implement them;
  verified in the reference source and live with the bundled host.
- **Persuade PowerShell to emit `OSC 133`.** Setting `WT_SESSION` or
  `TERM_PROGRAM` does not; verified by probe.
- **Merge the automatic mark with its neighbours.** A multi-line prompt or a
  command line continued across rows collapses into one tick, and the bar loses
  the per-command rhythm it is for.
- **Mark in the alternate buffer as well.** Every Enter inside a full-screen
  program would add a tick for something that was never a prompt.
- **Draw the ticks under the thumb.** This is what the overlay did first, and
  live it hid exactly the newest prompts.
