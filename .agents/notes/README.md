# Notes

Design and planning material for the port. `AGENTS.md` is the working
agreement; this directory is the *why*.

| Document | Purpose |
|----------|---------|
| [plan.md](plan.md) | The fourteen-phase porting plan, milestone status, test targets, and what to build next. The reference for what to build next. |
| [adr/](adr/README.md) | Architecture decision records. A decision that constrains future code gets one; a decision that a test already enforces does not. |
| [../skills/](../skills/) | Procedures loaded on demand: porting a module, testing, the WinUI shell, commit messages. `AGENTS.md` points at them. |

The C++ source being ported is Microsoft's Windows Terminal. It is not vendored;
`/.design/refs` is the local checkout when one is present, and the module path in
each Rust file's doc comment names the upstream file it came from.

## Conventions

- A document here is either a plan (what to build, in what order), an ADR (a
  decision and its consequences), or a note (an investigation and its findings).
  If it is none of those, it belongs in a code comment or does not belong.
- A procedure belongs in `.agents/skills/`, not here. This directory holds what
  to build and why; a skill holds how to do it.
- Status lines are dated. A status that has been overtaken by the code is a bug
  in the document, not in the code.
- Nothing here restates what a test already proves. The test is the record.
