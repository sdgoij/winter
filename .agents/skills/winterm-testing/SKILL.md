---
name: winterm-testing
description: Run and add tests in the winterm workspace. Use when running the suite or gate, adding a unit, property, conformance or ignored test, deciding where a test belongs, or reporting whether a change is green.
---

# Testing winterm

`AGENTS.md` states the rules; this is the procedure. The layers are in
`AGENTS.md` §Test layers — this file is how to work in them.

## Run the contract

```sh
just gate        # fmt, clippy, deny, test, doc — the whole contract
just fast        # the inner loop
just test-crate adapter
just test-long   # release, --include-ignored
```

Report the observed result, not an impression: the exit code and the
`test result:` line cargo prints, with its passed, failed and ignored counts. A
gate that was not run in this session is not green, however likely it is.

The gate is `fmt` → `clippy` → `deny` → `test` → `doc`. CI runs the same
`just gate` on Linux, Windows and macOS, so a green local run does not prove a
green CI run when a change touches `cfg(windows)` or `cfg(unix)`.

## On Windows: bound the test threads

`cargo test --workspace` at cargo's default parallelism can abort with
`0xc000012d` (`STATUS_COMMITMENT_LIMIT`) on a memory-constrained machine. That is
the linker and the harness exhausting commit, not a failing test. Run the same
cases with:

```sh
cargo test --workspace -- --test-threads=4
```

`just gate` has been reliable; a direct `cargo test --workspace` is the one to
bound.

## Prove the test is not vacuous

A test that passes on the old code proves nothing. Before claiming a new case
guards a change:

1. Run it against the change; it passes.
2. Revert the production change (or mutate the line the test should catch).
3. Run it again; it must fail, and fail for the reason you wrote it for.
4. Restore the change and confirm it passes.

If step 3 passes, the test is asserting something else. Fix the test, not the
narrative.

## Unit tests

In `#[cfg(test)] mod tests` in the module that owns the rule. No I/O, no temp
files, no clock. Name the behaviour, not the function:

```rust
#[test]
fn a_repeated_key_press_adds_to_the_repeat_count() { /* ... */ }
```

Exercise the public surface of the type. A test that reaches into a private
field to build or read its fixture is fine inside the owning module; a test in
another crate that does so is a missing accessor.

## Conformance cases

The corpus lives in `crates/term/tests/conformance/` and is one binary:

- `main.rs` declares every module; a new file needs its `mod` line there.
- `support.rs` holds `Screen`, which drives `term::Terminal` with bytes and reads
  back the visible screen, cursor and host responses.
- A case feeds the bytes a program would write and asserts what a user would see
  or what the host owes the program (DSR reply, clipboard, title).
- `generate.rs` is the one module that does not name a sequence. Its properties
  feed arbitrary bytes and assert only that the terminal survives them: the
  screen stays a rectangle, the cursor stays inside it, the same bytes twice
  give the same screen, and a sequence cut off by the end of the input does not
  swallow what follows. Extend it when a new parser gains a state the corpus
  cannot enumerate.

Expectations come from ECMA-48, DEC's manuals, xterm's documented behaviour, or
the upstream C++ `ut_*` case — never from what this implementation prints. When
they disagree, the reference wins and the implementation is the bug.

```rust
#[test]
fn csi_question_1049_h_enters_the_alternate_screen() {
    let mut screen = Screen::new(20, 5);
    screen.feed_str("main");
    screen.feed_str("\x1b[?1049h");
    assert_eq!(screen.line(0), "");
}
```

Run the corpus alone with `cargo test -p term --test conformance`.

## Property tests

`proptest` is a dev-dependency of the owning crate; the `proptest!` block lives
beside the code it constrains.

- State an invariant, not today's output: reflow preserves characters, a
  selection rect stays inside the buffer, `hash` is deterministic for equal
  values, a parsed parameter is never out of range, run-length encoding expands
  back to its values, a generational token is valid exactly while nothing was
  written after it.
- A failure is a bug in the code until proven otherwise. Do not weaken the
  generator, add `prop_assume!` filters, or shrink the domain to make it pass.
- Keep cases bounded (`1..=40` rows, not `0..=usize::MAX`); a property test that
  takes seconds per case is a test nobody runs.

## Ported upstream tests

Port the upstream case with the code, named after the sequence or the API it
exercises. A handler with no case naming its sequence is not ported, and a
`ut_adapter` case is evidence for the expected value where the specification is
tacit.

## Long-running and environment-dependent cases

Mark a case that depends on the machine or is astronomically rare:

```rust
#[ignore = "reads the host's installed fonts; run with \
            `cargo test -p renderer -- --ignored`"]
```

`just test-long` runs them all in release mode. It is not in `gate`: the font
case depends on what the host has installed, and release builds are slow.

## Failure triage

- Compile error in a crate the change never touched: the workspace ring was
  broken or a re-export moved. Fix the edge; do not duplicate the type.
- `clippy -D warnings` after a green test run: the gate is not green. Fix it in
  this change.
- A test that fails only on one platform: state the platform in the failure. A
  platform-specific pass is not a pass.
- A test that fails intermittently: the fix is determinism (owned fixtures,
  explicit ordering), never a retry.
