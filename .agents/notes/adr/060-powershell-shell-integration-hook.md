# ADR 060 — A PowerShell profile gets a hook that sends FinalTerm marks

- Status: Accepted
- Date: 2026-11-23
- Related: `crates/winterm/src/shell_integration.rs`, `crates/winterm/shell-integration/winterm.ps1`, `crates/winterm/src/term_settings.rs`, `crates/buffer/src/marks.rs`, `crates/winterm/src/grid.rs`, ADR 059

## Context

ADR 059 makes Enter mark the line it submitted, so the scrollbar has ticks on
the default Windows profile. Those marks carry no exit code: only shell
integration can report one, and the tick's red state is exactly the exit code.
Windows PowerShell emits nothing, so without a hook every command looks
successful.

The mark pipeline itself also had a defect that only real shell integration
exposes. `Marks::start_prompt` and `start_command` merged a new mark into a
previous one of the same kind on the adjacent row, so a session's prompts and
commands collapsed into a single `Command` mark spanning everything, and the
last `D` overwrote the exit code that every tick reported. Measured live: one
`Command 3..95 Some(0)` for a session where the middle command failed.

Upstream does not have this shape: `TextBuffer` keeps one scrollbar record per
*prompt row*, and `EndCurrentCommand` walks up from the cursor to the nearest
prompt row to set that record's error. Two commands cannot share a record
because they are on different rows.

## Decision

- The app ships a PowerShell script and starts a PowerShell profile with it:
  `powershell.exe -NoExit -File <script>`. `-File` loads after the user's
  profile, so the hook wraps the prompt the user already has instead of
  replacing it, and `-NoExit` keeps the session.
- The script is embedded with `include_str!` and written to
  `%LOCALAPPDATA%\winterm\shell-integration.ps1`, so the hook does not depend on
  the install layout. A stale copy is rewritten, so an upgrade does not keep an
  old script.
- It emits `133;A` before the prompt, `133;B` after it, `133;C` from a
  PSReadLine Enter binding (skipped when the line editor is missing) and
  `133;D;<code>` at the next prompt. The exit code is `$?` plus
  `$LASTEXITCODE`; a failed cmdlet leaves the latter alone, so the number may
  be stale but the mark is still nonzero, which is what the tick needs.
- The hook is attached only to `powershell` and `pwsh`, and only when the
  profile's arguments do not already name what to run (`-Command`, `-File`,
  `-EncodedCommand`, `-NonInteractive`, or a bare script). `-File` takes the
  rest of the command line, so appending after one would either be ignored or
  steal the user's own.
- `WINTERM_SHELL_INTEGRATION=0` keeps the hook off a shell.
- Marks are now one per prompt row and one per command line, matching upstream:
  `start_prompt` and `start_command` create a mark unless one already covers
  the row, and `mark_prompt_row` (ADR 059) uses the same rule. `D` therefore
  reports its exit code to exactly one command.

## Consequences

- A failing command shows a red tick and a succeeding one a blue tick, in both
  shells, on the default Windows profile, with no user setup.
- A shell started outside winterm, or with its own `-Command`, keeps its own
  behaviour and gets only ADR 059's automatic marks.
- The script runs for every PowerShell tab, so a profile that sets its own
  `prompt` is wrapped rather than duplicated; the wrapper captures the original
  once, so sourcing the file twice does not nest.
- `bash` and `cmd.exe` are untouched. Bash can install the same marks from its
  own startup file; `cmd.exe` has no equivalent.
- Empty command lines now keep their own tick instead of merging into the
  previous prompt's.

## Alternatives considered

- **Ask users to edit `$PROFILE`.** That is what Windows Terminal does, and it
  means the feature does nothing until the user reads the docs.
- **Pass the marks with `-Command`.** It cannot be appended safely: `-Command`
  consumes the rest of the command line, so a profile with its own arguments
  breaks. `-File` needs no inner quoting and leaves user arguments intact.
- **Inject `PSModulePath` or a profile file.** Both write into the user's
  profile or module directories, which the terminal does not own.
- **Have the terminal guess the exit code.** It cannot: the shell is the only
  thing that knows whether the command failed.
- **Keep the range-list merge and attribute errors differently.** The merge
  itself is what loses the per-command record; upstream's row model is the
  reason its marks are per command.
