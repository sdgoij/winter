# Contributing to Winter

Winter is a cross-platform terminal emulator in Rust, ported module by module
from Microsoft's Windows Terminal C++. The live porting status is
[`docs/plan.md`](docs/plan.md); accepted decisions are in
[`docs/adr/`](docs/adr/README.md).

This is the working agreement: the directions that hold for every change,
whoever makes it. Procedures that have been walked live in `.agents/skills/`;
the history of a decision lives in `docs/`.

## The iron rule

**A passing test is not a passing suite.** The cases that name your change can
pass while one in a crate you never opened fails. Run what CI runs, all of it,
before claiming anything is green.

The `Justfile` is the one place to look for commands. `.github/workflows/ci.yml`
is the truth; one workflow file holds the gate and the release, so the gate CI
runs and the gate a tag runs are the same jobs. If the two disagree, CI wins. A
pre-commit hook is a floor, not a substitute.

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

Do not claim a gate passed unless the command was run and its result was
observed. A gate that cannot be run is reported by name, never turned into a
silent pass.

Once every requirement is met and the gate is green, committing is the next
step, including changes to the public surface or the architecture. Code, its
tests, and the documentation that describes them are one commit.

## Verification gates

```sh
just gate       # fmt, clippy, deny, test, doc — the whole contract
just fast       # fmt, clippy, test — the inner loop, not the gate
just test-crate vt
just test-long  # release mode, the ignored cases included
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
- `just test-long` — the `#[ignore]`d cases in release mode. Not in `gate`: it
  depends on the host's installed fonts and is slow. CI runs it as its own
  `Full tests` job on every push, and `package` waits for it; run it locally
  when touching a crate that has an ignored case.

Enable the pre-commit hook once per clone:

```sh
just hooks
```

It runs formatting, lints, and `cargo-deny` when installed. Set
`WINTER_PRECOMMIT_TESTS=1` to add the suite.

## Test layers

Coverage is a review property, not a percentage. A layer is required when it can
catch a failure the layer below cannot; do not add a conformance case for a pure
rule, and do not leave a ported handler without one.

| Layer | Home | Required for |
|---|---|---|
| Unit | `#[cfg(test)]` in the module that owns the rule | every ported type, parser state, buffer operation, dispatch handler and error path |
| Conformance | `crates/term/tests/conformance/` | every escape sequence a handler names; expectations from ECMA-48, DEC, xterm or upstream `ut_*`, never from this implementation |
| Property | `proptest` in the owning crate | open-ended invariants: reflow, selection bounds, hashing, parameter parsing |
| Long-running | `#[ignore]` in the owning module | host-environment dependence and astronomically rare paths |

The conformance corpus has a single entry point (`main.rs`, wired through
`[[test]]` in `crates/term/Cargo.toml`) so its modules can live in
subdirectories. A sequence whose expected bytes came from running this code is
not a test; it is a regression lock on a possible bug.

## Test rules

- **Every bug fix gets a regression test** that fails before the fix.
- **Every new error path gets a test** asserting the error, not just `is_err()`.
- **Port the upstream tests with the code.** A handler whose sequence no test
  names is not ported.
- **No sleeps, no network, no shared state.** Each test owns its fixtures.
- **No retries to hide flakiness.** A flaky test is a failure to fix.
- **No vacuous tests.** Prove the test fails without the change.
- **No golden re-baseline to make a failure green.** Expected values come from
  upstream, a specification, or a hand-computed example.

The procedure, the corpus layout, and the Windows memory trap are in the
`winter-testing` skill.

## Commit rules

Subject and body rules are in `.agents/skills/git-commit-messages`; read it when
writing one. In short: `<type>(<scope>): <imperative subject>`, no trailing
period, body wrapped at 72 columns explaining what changed and why.

- Code, tests, and the documentation that describes them are **one** commit.
- Never commit build output, `target/`, `/dist`, or a local `.db`.
- Tracked text is LF in the index and in the working tree. `.gitattributes`
  pins it; `just lf` puts the working tree back after an editor rewrites it.
- Stage explicit paths. `git add -A` has swept a scratch file into a commit
  here before; the message file is `.commit-msg-tmp` and it is ignored.
- Keep the tree clean after committing.

## Project invariants

These are not style preferences. A change that breaks one is wrong even if its
tests pass.

### Porting

- **Windows Terminal C++ is the oracle.** Where this code and upstream disagree,
  upstream wins. A deliberate divergence is an ADR naming the program that needs
  it. A local checkout lives in the gitignored `.design/refs/terminal/`.
- **One Rust module per C++ file, same name**, with the upstream file named in
  the module doc comment. The method is in
  `.agents/skills/winter-porting`.
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
  `utils → types → {buffer, vt} → adapter → term`. An edge that points left is a
  design error, not a `Cargo.toml` edit. `renderer` sits on `utils`, `types` and
  `buffer`; `host` sits on `types`, `vt` and `adapter`, because it consumes the
  parser and the console interfaces; `server` sits on `host`; `audio` sits on no
  workspace crate at all, because it is sound without a device (ADR 033).
- **One home per type** (ADR 002). A type defined twice is a bug even when both
  copies compile.
- **`unsafe` is denied workspace-wide.** The exemptions are named, not
  incidental: `utils` opts out at its root because it owns the lock-free
  primitives; `server`'s two pty backends carry a module-level
  `#[allow(unsafe_code)]` with a `SAFETY:` note per call (ADR 016);
  `winter`'s `sound` sink does the same for `Beep` (ADR 033), its
  `winui::surface`, `winui::wheel`, `winui::icon` and `winui::profile_icon`
  modules do for Direct2D, the wheel hook, the window icon and a program's icon
  (ADRs 041, 047, 058, 066), and `macos` declares one allowance for the AppKit
  and Metal boundary — its window, menu, bitmap and device calls (ADRs 072,
  073). The two
  self-referential buffer iterators carry an
  item-level `#[allow(unsafe_code)]` and a `SAFETY:` note. New `unsafe`
  anywhere is a review event.
- **No new dependency without a reason in the commit.** Prefer what the
  workspace already has.
- **No decorative comments.** No banners, box-drawing separators, or section
  headings whose only job is visual grouping. Existing ones are removed when
  their module is next edited; do not add new ones.
- **Comments explain why.** A comment that restates the code is deleted; a
  comment that names a constant's origin, a non-obvious constraint, or a
  trade-off stays and is updated when the rule moves.

### Ratchets

Two lints are allowed today and are the target of a cleanup, not a resting place:

| Lint | Status | Target |
|---|---|---|
| `clippy::unwrap_used`, `clippy::expect_used` | allowed | `warn` once the ~56 production call sites (46 of them in `utils`) are converted |

Do not add call sites under an allowance.

## Where things live

| Path | Holds |
|---|---|
| `docs/plan.md` | The porting plan, milestone status, and what to build next |
| `docs/adr/` | Decisions that constrain future code |
| `.agents/skills/` | Proven procedures, one area per skill: porting, testing, the WinUI shell, the macOS shell, commit messages |
| `.design/refs/terminal/` | A local checkout of the upstream C++, gitignored, because it is the oracle and not source |
| `crates/utils` | Geometry, colour, containers, locks — the lock-free primitives, with the crate-root `unsafe` exemption |
| `crates/types` | Terminal vocabulary: colours, attributes, enums, parameters, input records |
| `crates/buffer` | `OutputCell`, `Row`, `TextBuffer`, marks, search, images |
| `crates/vt` | The DFA parser and its dispatch actions |
| `crates/adapter` | `ITermDispatch`, page management, sixel and kitty graphics, macros, charsets |
| `crates/term` | The parser-to-dispatch loop, the `Terminal` handle, key and mouse encoding |
| `crates/renderer` | Fonts, cell metrics, atlas, the software paint path |
| `crates/host` | The console side: history, input buffer, cooked read, input parser, selection and its input |
| `crates/server` | The console API surface: shim policy, and the pty layer (`unsafe` confined to its backends, ADR 016), with a headless OpenConsole host so image sequences reach the terminal (ADR 055) |
| `crates/settings` | The TOML settings file: profiles, keybindings, schemes, themes, normalization, migration |
| `crates/audio` | The bell and DECPS tones, computed without a device (ADR 033) |
| `crates/winter` | The application: the egui shell on Linux, `winui/` on Windows, and `macos/` on macOS |

## Where the port stands

The status table in `plan.md` is the live answer, and a row the code has
outgrown is a bug in the document. Do not restate the status here: a contributor
reads this file first, and a second copy drifts between the commits that move
it. `README.md` carries the one-paragraph user-facing version.

The plan's "Pre-Phase-5 cleanup" section is a record of what those passes
settled, not a to-do list.

## Rules, skills and notes

Three homes, each answering a different question:

| Home | Answers | Holds |
|---|---|---|
| `CONTRIBUTING.md` | What must every change obey? | The working agreement: the gates, the test rules, the invariants. Instructions, not explanations, and the same for a person and an agent. |
| `.agents/skills/<name>/SKILL.md` | How do I do this here? | A procedure that has been performed successfully at least once, with the traps that cost time while doing it. One area per skill. |
| `docs/` | Why is it like this, and what happened? | The historical knowledgebase: the plan and its status, the ADRs, investigations and measurements. |

`AGENTS.md` is a pointer to this file for the tools that look for that name; it
carries no rules of its own.

- A line every change must obey goes in this file. It is short and imperative
  because it is the first thing a contributor reads.
- A sequence of steps is a skill. Write it after the procedure works, not
  before: it records a path that has been walked. A proposal for a path is a
  note.
- A record of what was decided, tried, measured, or superseded is a note. A note
  never carries a step to follow; that is the skill's job.
- A new rule is non-obvious, repeatedly hit, and specific enough to act on. A
  one-off observation is a note or a test, not a rule.
- Do not add rules during feature work. A trap worth keeping goes in the commit
  or pull-request description under "Suggested `CONTRIBUTING.md` additions", and a
  reviewer decides.
- When a rule and a skill disagree, fix one of them in the same commit. Two
  homes for the same instruction is how they drift.

## Definition of done

A change is done when:

- the module doc names its upstream file, and every surface it claims is
  implemented — no stub, no placeholder success;
- the upstream cases for the change are ported, and a test that fails without it
  exists and was watched failing;
- the type lives in its ring crate (ADRs 001, 002), and no state crossed a layer
  it should not;
- a new parser state has a property in `generate.rs`, and a new sequence has an
  expectation from the specification or `ut_*`;
- the ADR, the `plan.md` row and the `README.md` status are updated in the same
  commit when a milestone or a user-visible capability moved;
- `just gate` was run and passed, with `just test-long` when an `#[ignore]`d
  case was touched;
- the tree is clean, LF-only, with no build output.

## Review checklist

- Is there a test that fails without this change, and did I watch it fail?
- Did I run `just gate`, not only the crate the change names?
- Does a new escape sequence have a conformance case, with its expectation from
  ECMA-48, DEC, xterm or upstream `ut_*` rather than from our output?
- Did `unsafe` appear outside the named exemptions?
- Is any type defined twice, or any `cfg(windows)` / `cfg(unix)` branch in
  parsing, buffer, or dispatch?
- Does writing a cell still avoid allocating, and does width still come from
  `types::codepoint_width`?
- Is the `plan.md` row still true, and is the documentation that describes the
  rule in this commit?
