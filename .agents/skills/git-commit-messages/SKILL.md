---
name: git-commit-messages
description: Write a Git commit message with a subject line and a descriptive body. Load when asked to write a commit message, summarize staged changes, or produce a changelog entry.
---

# Git commit messages

Produce the whole message — subject and body — in one response. Read the staged
files or the diff first; do not ask which files changed.

## Format

```
<type>(<scope>): <subject>

<body: what changed and why, not hard-wrapped>
```

- **Subject**: imperative mood, no trailing period, 50 characters if it fits and
  72 at the outside. Capitalise the first word after the scope.
- **Type**: `feat`, `fix`, `test`, `refactor`, `perf`, `docs`, `build`, `ci`,
  `chore`. The repository's history uses `feat(adapter)` and `test(buffer, vt)`
  for scopes, so a scope is the crate or crates touched.
- **Body**: separate from the subject with a blank line. Explain *what* and
  *why*; the diff already shows *how*. Omit the body when the subject says it.
  Write each paragraph as one line; do not hard-wrap the body.
- Do not restate the subject in the body.

## One commit per change

Code, its tests, and its documentation are one commit. A revert must not leave a
comment describing a mechanism that no longer exists, or a test for a rule that
was removed. A commit that is *only* documentation is fine when documentation is
the deliverable (an ADR, a measurement record, this skill).

## Patterns

**Many files, one crate** — group by module, one line each:

```
feat: add vt crate and text search to buffer

- Create vt crate with 8 modules:
  - state_machine.rs: DFA parser with 19 states
  - state.rs: VtState enum
  - parameters.rs: VT parameter parsing (CSI/OSC/DCS)
- Add search.rs to buffer with literal and wildcard search
- Fix OutputCellIterator::new clamp bug that underflowed under proptest
```

**Categorical change** — bold headers, blank lines between groups:

```
feat(adapter): terminal adapter with VT dispatch infrastructure

**New modules:**
- dispatch_types.rs — VT parameter types and enums
- term_dispatch.rs — ITermDispatch with 100+ handlers

**Fixes:**
- Replace VTID bit packing with array storage
- Remove duplicate enum variant values in GraphicsParams
```

**Focused change** — three lines, no list:

```
fix(buffer): keep the cursor column after a wrapped row is reflowed

Clamp the column to the row's width when the shrink path runs, so a cursor
parked past the new width does not index the row out of bounds.
```

**Test-only change** — name what each case covers and where the suite stands:

```
test(buffer, vt): add snapshot and property cases for reflow

- row.rs: mixed-width rendering (ASCII + CJK + emoji)
- text_buffer.rs: full-buffer render grid
- state_machine.rs: deterministic 1000-iteration fuzz of process_char
```

## DOs and DON'Ts

| DO | DON'T |
|----|-------|
| Read the diff first | Ask what changed |
| Write the whole message at once | Send a subject and wait for permission |
| Explain why | Narrate the code |
| Use imperative mood | "Added", "Fixes", "Adds" |
| Group related files | List every file |
| Leave the body unwrapped | Hard-wrap at a fixed column |
| Close with test results and follow-ups | Leave the body empty when there is substance |

If someone says "more detail" or "descriptive", they want a body grouped by
purpose, not a longer subject.
