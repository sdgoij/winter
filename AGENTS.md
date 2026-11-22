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
  depends on the host's installed fonts and is slow. Run it when touching a
  crate that has an ignored case.

Enable the pre-commit hook once per clone:

```sh
just hooks
```

It runs formatting, lints, and `cargo-deny` when installed. Set
`WINTERM_PRECOMMIT_TESTS=1` to add the suite.

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
`winterm-testing` skill.

## Commit rules

Subject and body rules are in `.agents/skills/git-commit-messages`; load it when
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
  `utils → types → {buffer, vt} → adapter → term`. An edge that points left is a
  design error, not a `Cargo.toml` edit. `renderer` sits on `utils`, `types` and
  `buffer`; `host` sits on `types`, `vt` and `adapter`, because it consumes the
  parser and the console interfaces; `server` sits on `host`; `audio` sits on no
  workspace crate at all, because it is sound without a device (ADR 033).
- **One home per type** (ADR 002). A type defined twice is a bug even when both
  copies compile.
- **`unsafe` is denied workspace-wide.** `utils` opts out at its root because it
  owns the lock-free primitives, `server`'s two pty backends carry a
  module-level `#[allow(unsafe_code)]` with a `SAFETY:` note per call (ADR 016),
  and `winterm`'s sound sink does the same for `Beep` (ADR 033).
  The two self-referential buffer iterators carry an item-level
  `#[allow(unsafe_code)]` and a `SAFETY:` note. New `unsafe` anywhere is a
  review event.
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
| `.agents/notes/plan.md` | The porting plan, milestone status, and what to build next |
| `.agents/notes/adr/` | Decisions that constrain future code |
| `.agents/skills/` | Procedures loaded on demand: porting, testing, commit messages |
| `crates/utils` | Geometry, colour, containers, locks — the lock-free primitives, with the crate-root `unsafe` exemption |
| `crates/types` | Terminal vocabulary: colours, attributes, enums, parameters, input records |
| `crates/buffer` | `OutputCell`, `Row`, `TextBuffer`, marks, search, images |
| `crates/vt` | The DFA parser and its dispatch actions |
| `crates/adapter` | `ITermDispatch`, page management, sixel, macros, charsets |
| `crates/term` | The parser-to-dispatch loop, the `Terminal` handle, key and mouse encoding |
| `crates/renderer` | Fonts, cell metrics, atlas, the software paint path |
| `crates/host` | The console side: history, input buffer, cooked read, input parser, selection and its input |
| `crates/server` | The console API surface: shim policy, and the pty layer (`unsafe` confined to its two backends, ADR 016) |

## Where the port stands

Phases 1–5 are complete. Phase 6 (the console host) has its portable modules:
`history.cpp`, `inputBuffer.cpp`, the `readDataCooked.cpp` edit model,
`InputStateMachineEngine.cpp`, the portable halves of `consoleInformation.cpp`
and `screenInfo.cpp`, `settings.cpp`, `alias.cpp` (with its expansion wired into
the cooked read), `inputKeyInfo.cpp`, `scrolling.cpp`, `selection.cpp` and
`selectionInput.cpp` and `utils.cpp` are ported, so the input records have a
console, a screen, its settings, working aliases, a key description, scroll
handling, a selection moved by keys and the mouse, and the shared console
helpers. `directio.cpp` is the `ApiRoutines` surface and waits for the server
layer with the read-data wait path; `registry.cpp` is a portable schema that
layers a key over the settings, with the Win32 reads left to the server
(ADR 037); `handle.cpp` and `tracing.cpp` have no counterpart here — a lock
with no ctrl-event queue, and ETW. The read-data family's `IWaitRoutine` completion path,
the cooked read's pager, the render settings and the window metrics wait for
the server, renderer and window layers; `outputStream.cpp` has no counterpart
(ADR 013). Phase 7 has the server's portable policy (ADR 015) and its pty
layer (ADR 016: ConPTY and `openpty` behind two `unsafe` modules). Phase 8 has
the last of `Terminal.cpp`'s input surface — focus dedup, the viewport clamp and
AltGr aliasing — while the keyboard and mouse encoders stay in `term` (ADR 017).
It also has a first runnable shell: `crates/winterm` runs a shell on the pty and
paints the buffer through egui on winit and wgpu (ADR 018), as a `Control` over
a `Connection` trait with the viewport and selection in the widget and an event
queue for the bell, a notification and an exit — a shell that exits closes its
tab (ADR 036) — split into `app.rs`, `terminal_widget.rs`, `config.rs`,
`window.rs` and `icon.rs` (ADR 038). On Windows the shell is WinUI 3
(`winui/`, ADRs 040–047): a native window with a `TabView` whose tabs each own a
terminal, the ptys polled on a timer, and the active tab's pixels blitted by
Direct2D onto a `SwapChainPanel`, with the pane's grid fitted to the panel's
cells and the face rasterised at its composition scale, a start that runs the
settings file's profile and paints its font size and scheme, the file's chords
matched before the terminal sees them, a pointer selection that copies through
the clipboard, and scrollback reached by chord and by a low-level wheel hook
that a mouse-tracking program receives unless Shift is held (ADRs 047, 048);
egui remains the shell on Linux and macOS. The shell has wheel
scrollback, mouse selection that reaches into history and scrolls when a drag
leaves the pane, mouse forwarding to a
tracking application (Shift selects instead), copy/paste (ADRs 019, 020), OSC
titles, bell attention, cursor shapes, text decorations, reverse video,
intense brightening, faint text and italics from the host's monospace face
(ADR 029), tabs (ADR 021),
splittable panes (Ctrl+Shift+D/E, Alt+Arrow to focus, Alt+Shift+Arrow to
resize, or drag a divider), a shell chosen by the profile (or from the plus
menu), a command palette (Ctrl+Shift+P) over the profiles and the window's
actions, an in-terminal find bar (Ctrl+Shift+F), a scrollbar with prompt marks,
desktop notifications for a program's OSC 777, the window chords, a
colour scheme and the font size read from `%APPDATA%\winterm\settings.toml`
(ADRs 022–024, 034), a theme the tab strip paints (ADR 035), a settings window
(Ctrl+,) that reports the file it loaded, lists what normalization adjusted
(ADR 027), edits everything the file holds — font size, defaults, profiles,
keybindings, schemes and themes (ADRs 028, 030–032, 035) — and
previews the document a save would write, with a poll that reloads the file when
it is saved (ADR 025). Phase 11 has the bell and DECPS: `crates/audio` turns a
note into a tone with upstream's arithmetic, a bell flashes the pane and rings
the console host's 800 Hz tone, and a thread plays a DECPS sequence with the
Windows `Beep` API (ADR 033). The native window backends follow.

The plan's "Pre-Phase-5 cleanup" section is a record of what those passes
settled, not a to-do list. The status table in `plan.md` is the live answer, and
a row the code has outgrown is a bug in the document.

## Where a rule belongs

This file is read at the start of every session, so it stays high-signal.

- A rule is a trap to avoid, not a map to follow. If it needs a paragraph of
  explanation or a sequence of steps, it is a skill.
- A procedure (porting a module, running the suite, writing a commit message)
  lives in `.agents/skills/<name>/SKILL.md` and is referenced here in one line.
- A rule that names one crate belongs in that crate's `.rules`, not here.
- A new root rule must be non-obvious, repeatedly hit, and specific enough to
  act on. A one-off observation is a note or a test, not a rule.
