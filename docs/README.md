# Notes

The historical knowledgebase for the port: what was decided, what was tried,
what it cost, and where the work stands. `CONTRIBUTING.md` holds the directions every
change obeys; `.agents/skills/` holds the procedures that have been walked. This
directory keeps the record — the *why* and the *when*.

| Document | Purpose |
|----------|---------|
| [plan.md](plan.md) | The fourteen-phase porting plan, milestone status, test targets, and what to build next. The reference for what to build next. |
| [adr/](adr/README.md) | Architecture decision records. A decision that constrains future code gets one; a decision that a test already enforces does not. |
| [linux-shell-scope.md](linux-shell-scope.md) | The scope the GTK host was built against, now the shipping Linux shell (ADR 074), with the Wayland traps and the named gaps (primary selection, an accessible text tree, tab icons, sound). |
| [macos-shell-scope.md](macos-shell-scope.md) | The macOS shell's scope, its module map, the AppKit traps that cost time, and the named follow-ups (ADR 072). |
| [../.agents/skills/](../.agents/skills/) | Not part of this record: procedures that have been walked — porting a module, testing, the WinUI, macOS and Linux shells, commit messages. |

The C++ source being ported is Microsoft's Windows Terminal. It is not vendored;
`/.design/refs` is the local checkout when one is present, and the module path in
each Rust file's doc comment names the upstream file it came from.

## Conventions

- A document here is either a plan (what to build, in what order), an ADR (a
  decision and its consequences), or a note (an investigation and its findings).
  If it is none of those, it belongs in a code comment or does not belong.
- An entry is a record, not an instruction. A step someone must follow is a
  skill; a line every change must obey is a rule in `CONTRIBUTING.md`.
- Status lines are dated. A status that has been overtaken by the code is a bug
  in the document, not in the code.
- Nothing here restates what a test already proves. The test is the record.
