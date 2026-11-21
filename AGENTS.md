# Working in winterm

Winterm is a cross-platform terminal emulator in Rust, ported module by module
from Microsoft's Windows Terminal C++. The porting plan is
[`.agents/notes/plan.md`](.agents/notes/plan.md); accepted decisions are in
[`.agents/notes/adr/`](.agents/notes/adr/README.md). This file is the working
agreement: how a change is made, tested, gated, and committed.

Procedures live in skills under `.agents/skills/`; this file stays short and
points at them.

## The iron rule

**A passing test is not a passing suite.** The cases that name your change can
pass while one in a crate you never opened fails. Run what CI runs, all of it,
before claiming anything is green.

The `Justfile` is the one place to look for commands. `.github/workflows/ci.yml`
is the truth; if the two disagree, CI wins. A pre-commit hook is a floor, not a
substitute.

## Change workflow

1. Make one focused change.
2. Add or update a test that fails without it. Prove it: revert the change (or
   mutate it), watch the new test fail, restore it. A test that passes on the old
   code proves nothing.
3. Refresh editor diagnostics project-wide. A diagnostic is a failure like a
   compiler warning.
4. Run the whole gate.
5. Commit green, tested changes without asking.
6. Stop and ask only when a decision, a spec ambiguity, or a missing environment
   genuinely blocks progress.

Do not claim a gate passed unless the command was run in this session and its
result was observed. A gate that cannot be run is reported by name, never turned
into a silent pass.

## Verification gates

```sh
just gate       # fmt, clippy, deny, test, doc — the whole contract
just fast       # fmt, clippy, test — the inner loop, not the gate
just test-crate vt
```

- `just fmt` — `cargo fmt --all --check` and `git --no-pager diff --check`. This
  is the whole gate for a docs-only change.
- `just clippy` — `cargo clippy --workspace --all-targets -- -D warnings`.
- `just deny` — licences, advisories, yanked crates, sources. Needs
  `cargo install cargo-deny --locked`; Cargo does not ship it. If the tool is
  missing the pre-commit hook skips it with a warning and CI still runs it.
- `just test` — the workspace suite.
- `just doc` — doc tests. There are none yet; the recipe exists so the first
  example that stops compiling fails the gate.

Enable the pre-commit hook once per clone:

```sh
just hooks
```

It runs formatting, lints, and `cargo-deny` when installed. Set
`WINTERM_PRECOMMIT_TESTS=1` to add the suite.

## Commit rules

Subject and body rules are in `.agents/skills/git-commit-messages`; load it when
writing one. In short: `<type>(<scope>): <imperative subject>`, no trailing
period, body wrapped at 72 columns explaining what changed and why.

- Code, tests, and the documentation that describes them are **one** commit.
- Never commit build output, `target/`, `/dist`, or a local `.db`.
- Tracked text is LF in the index and in the working tree. `.gitattributes`
  pins it; `just lf` puts the working tree back after an editor rewrites it.
- Keep the tree clean after committing.

## Project invariants

These are not style preferences. A change that breaks one is wrong even if its
tests pass.

### Porting

- **Windows Terminal C++ is the oracle.** Where this code and upstream disagree,
  upstream wins. A deliberate divergence is an ADR naming the program that needs
  it.
- **One Rust module per C++ file, same name**, with the upstream file named in
  the module doc comment. The method is in
  `.agents/skills/winterm-porting`.
- **A module lands working or not at all.** No `todo!()`, `unimplemented!()`,
  placeholder success, or module written but undeclared in `lib.rs`. A module
  that cannot be finished is deleted.
- **Port the upstream tests with the code.** A VT handler with no test naming its
  sequence is not ported.

### Emulation

- **The parser never panics**, on any byte, in any state, at any length. It
  consumes whatever a program writes to the pty: truncated sequences, invalid
  UTF-8, absurd parameter counts.
- **Terminal data is bytes.** `char` is a rendering concept; text becomes `char`
  through one decoding path with an explicit replacement policy.
- **Cell width comes from `types::codepoint_width`.** Nowhere else derives it.
- **Emulation is platform-independent.** `cfg(windows)` / `cfg(unix)` belong to
  the host, pty, and input layers — never to parsing, buffer, or dispatch.
- **The buffer is the single source of truth for screen state.** The adapter
  mutates it through the public API, never by reaching into row internals.
- **Writing a cell does not allocate.** Row storage is allocated when the row is.

### Workspace

- **The dependency ring is the architecture** (ADR 001):
  `utils → types → {buffer, vt} → adapter → …`. An edge that points left is a
  design error, not a `Cargo.toml` edit.
- **One home per type** (ADR 002). A type defined twice is a bug even when both
  copies compile.
- **`unsafe` is denied workspace-wide.** `utils` is the only crate that opts out,
  at its root, because it owns the lock-free primitives. The two
  self-referential buffer iterators carry an item-level `#[allow(unsafe_code)]`
  and a `SAFETY:` note. New `unsafe` anywhere is a review event.
- **No new dependency without a reason in the commit.** Prefer what the
  workspace already has.
- **No decorative comments.** No banners, box-drawing separators, or section
  headings whose only job is visual grouping. Existing ones are removed when
  their module is next edited; do not add new ones.
- **Comments explain why.** A comment that restates the code is deleted; a
  comment that names a constant's origin, a non-obvious constraint, or a
  trade-off stays and is updated when the rule moves.

### Tests

- **Every bug fix gets a regression test** that fails before the fix.
- **Every new error path gets a test** asserting the error, not just `is_err()`.
- **No sleeps, no network, no shared state.** Each test owns its fixtures.
- **No retries to hide flakiness.** A flaky test is a failure to fix.
- **No golden re-baseline to make a failure green.** Expected values come from
  upstream, a specification, or a hand-computed example.

### Ratchets

Two lints are allowed today and are the target of a cleanup, not a resting place:

| Lint | Status | Target |
|---|---|---|
| `clippy::unwrap_used`, `clippy::expect_used` | allowed | `warn` once the ~56 production call sites (46 of them in `utils`) are converted |

Do not add call sites under an allowance.

## Where things live

| Path | Holds |
|---|---|
| `.agents/notes/plan.md` | The porting plan, milestone status, and the cleanup list before Phase 5 |
| `.agents/notes/adr/` | Decisions that constrain future code |
| `.agents/skills/` | Procedures loaded on demand |
| `crates/utils` | Geometry, colour, containers, locks — the only crate with `unsafe` |
| `crates/types` | Terminal vocabulary: colours, attributes, enums, parameters |
| `crates/buffer` | `OutputCell`, `Row`, `TextBuffer`, selection, marks, search |
| `crates/vt` | The DFA parser and its dispatch actions |
| `crates/adapter` | `ITermDispatch`, page management, sixel, macros, charsets |

## Before starting Phase 5

The adapter layer is not connected yet: `AdaptDispatch` is not declared in
`lib.rs` and does not compile, `ITermDispatch` has no implementation, and three
types exist twice. The list is in `plan.md` §"Pre-Phase-5 cleanup". Do not build
a renderer on top of it until that list is empty.
