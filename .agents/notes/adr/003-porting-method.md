# ADR 003 — Porting method

- Status: Accepted
- Date: 2026-11-22
- Related: ADR 001, ADR 002; `plan.md` Phase 12

## Context

Winter is a port, not a reimplementation. Microsoft's Windows Terminal is the
behavioural target, the test corpus, and the documentation. A port that invents
its own structure diverges silently: the same escape sequence then behaves
differently from the terminal every program in the world was written against, and
nobody finds out until a program draws garbage.

The port also has to run on Windows, Linux, and macOS, which upstream does not.

## Decision

1. **Upstream is the oracle.** When this code and `microsoft/terminal` disagree
   on what a sequence means, upstream wins. A deliberate divergence is recorded
   as an ADR with the program that needs it named.
2. **One Rust module per C++ file, same name.** `TextBuffer.hpp/cpp` becomes
   `text_buffer.rs`; `DispatchTypes.hpp` becomes `dispatch_types.rs`. The module
   doc comment names the upstream file it came from, so a reviewer can diff the
   two.
3. **Port the tests with the code.** Upstream's `ut_adapter` and `ut_parser`
   cases become Rust tests in the owning module, not a new test design invented
   for Rust. A handler with no case naming its sequence is not ported.
4. **A module lands working or not at all.** It is declared in `lib.rs` only when
   it compiles and its behaviour is tested. No `todo!()`, no `unimplemented!()`,
   no placeholder success, no module left beside the ones that work.
5. **The port copies signatures, not idioms.** Where a C++ signature is long or
   awkward (`ITermDispatch::CopyRectangularArea`, nine DECCRA operands), the Rust
   signature keeps its shape and carries `#[allow(clippy::too_many_arguments)]`
   with the reason, so the two can be compared line by line.
6. **`unsafe` is confined to `utils`.** It is the crate that implements the
   lock-free queue, the stack-allocated vector, and the manual mutex. Every other
   crate denies `unsafe_code`; the two self-referential buffer iterators carry an
   item-level allow and a `SAFETY:` note. Upstream's `til` is `unsafe`-heavy by
   design and that does not carry over.
7. **Emulation is platform-independent.** `cfg(windows)` and `cfg(unix)` belong
   to the host, pty, and input layers. The parser, the buffer, and the adapter
   must produce identical results on every platform, or the conformance suite
   means nothing.

## Rationale

- **A diffable port is maintainable.** "What changed upstream" has an answer when
  file names match.
- **Upstream tests encode a decade of bug reports.** Re-inventing them loses
  exactly the cases nobody thinks to write.
- **Stubs are the failure mode this repository has already paid for.** The
  adapter's `AdaptDispatch` was written, never wired, and silently drifted until
  it stopped compiling. The rule exists because of that.
- **Platform branching in emulation is untestable.** Two code paths mean two
  behaviours, and the conformance corpus only runs on one of them.

## Consequences

Positive:

- A new phase starts from a known file list and a known set of tests.
- Behavioural bugs are found against upstream, not against intuition.
- Cross-platform claims are checkable: the same tests run everywhere.

Negative:

- Rust-idiomatic shapes are sometimes left on the table, which costs readability.
  The reviewer decides case by case; the default is `allow` with a reason.
- Porting a module here is slower than writing a plausible one, which is the
  point.

## Alternatives considered

| Alternative | Why not |
|---|---|
| Reimplement from the VT specification | The spec is silent on the parts that matter in practice (Windows terminal modes, sixel, DECDRCS); upstream is the de-facto target |
| Port only what the plan lists, invent the rest | Produces a terminal that is subtly not the one programs were tested against |
| Keep upstream's file layout inside one crate | A single crate cannot enforce the ring in ADR 001 |
