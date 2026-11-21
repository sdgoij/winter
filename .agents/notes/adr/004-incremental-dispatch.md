# ADR 004 — Porting the dispatcher family by family

- Status: Accepted
- Date: 2026-11-22
- Related: ADR 002, ADR 003; `plan.md` §"Pre-Phase-5 cleanup"

## Context

`ITermDispatch` declares 123 callbacks and `adaptDispatch.cpp` implements them in
4,900 lines. A Rust trait requires every method in one `impl`, so the choice is
between porting all 123 in a single, unreviewable change and leaving the adapter
unconnected until someone does.

Upstream already has the two-layer answer: `ITermDispatch` is the interface, and
`TermDispatch` is an all-no-op implementation used by tests, so an implementor can
start from "everything ignored" and override what it needs.

The hazard with copying that into Rust is that a no-op default makes an unported
sequence *silently* do nothing — the failure mode `AGENTS.md` forbids, and the one
a terminal is least likely to survive, because the program that sent the sequence
cannot tell it was dropped.

## Decision

1. **`ITermDispatch` carries default implementations**, one per callback, as
   upstream's `TermDispatch` does. A dispatcher need only override the families
   it ports.
2. **Every default calls `unhandled(name)`** instead of returning silently.
   `AdaptDispatch` overrides the hook to count the call and remember the name, so
   an unported sequence is observable from a test.
3. **A ratchet test names the ported families.** `ported_families_do_not_report_and_unported_ones_do`
   exercises every override and asserts the counter stays zero; it also asserts
   that one known-unported callback does report. Porting a family means adding it
   to the first half of that test.
4. **`unknown_sequence` is not a missing handler.** It is overridden as a
   deliberate no-op, because "the parser did not recognise this" is a normal
   outcome, not a port gap.
5. **Handlers keep the C++ structure.** `_CursorMovePosition` becomes
   `move_cursor(Offset, Offset, bool)`; `Offset::absolute` subtracts one because
   VT coordinates are 1-based. A handler that is a direct translation of a named
   C++ function cites it.

## Rationale

- The port lands in reviewable slices, each with its own upstream reference.
- An omission is a test failure rather than a rendering bug reported months
  later.
- The trait stays one interface, so the application's dispatch loop does not
  change as families land.

## Consequences

Positive:

- The cursor-positioning family (CUU–CUP, DECSTBM, DECSLRM) is live now, driving
  a real `buffer::TextBuffer`.
- Each later family is an additive override plus a line in the ratchet test.

Negative:

- Until every family is ported, a terminal built on this adapter ignores most
  sequences. The counter makes that measurable, but it does not make it correct;
  the ratchet test is the thing that must reach zero.
- The defaults sit in the interface, which is one layer further from the caller
  than a per-implementation stub would be. Any new dispatcher inherits the same
  reporting obligation.

## Alternatives considered

| Alternative | Why not |
|---|---|
| Port all 123 callbacks in one change | 4,900 lines of C++ in a single commit, with no point at which the result is reviewable |
| No defaults; leave `AdaptDispatch` unconnected | Nothing is exercised and the adapter cannot be connected until the last family lands |
| Silent no-op defaults, as upstream's `TermDispatch` | Indistinguishable from success; the exact failure this repository has already paid for once |
| `todo!()` in unported handlers | Panics on a byte sequence a program is allowed to send |
