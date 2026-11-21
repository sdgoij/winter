# ADR 002 — One home per type, and what the adapter may see

- Status: Accepted
- Date: 2026-11-22
- Related: ADR 001, ADR 003; `plan.md` §"Pre-Phase-5 cleanup"

## Context

The adapter layer was written before the crates below it settled, and it grew its
own copies of things that already existed or that the renderer will need:

| Concept | Where it was | Why it was a problem |
|---|---|---|
| `TextAttribute` | `buffer/src/text_attribute.rs` | `adapter` needs it, and `adapter` does not depend on `buffer` |
| `VTParameter`, `VTParameters`, `VTSubParameters`, `VTID`, dispatch enums | `adapter/src/dispatch_types.rs` | Terminal vocabulary with no storage, hidden inside the adapter, unavailable to anything else |
| 94/96-character translation tables | `adapter/src/charsets.rs` | Static data the renderer will need too |
| `TextBuffer` | `buffer/src/text_buffer.rs` (744 lines) **and** `adapter/src/page_manager.rs` (a three-field stub) | `PageManager` could not hold the real buffer |

`adapter/src/adapt_dispatch.rs` was the symptom: not declared in `lib.rs`, so
invisible to `cargo check`, and therefore no longer compiling.

Two things that looked like duplication were not:

- `vt::parameters::VtParameters` is the parser's operand buffer; `VTParameters`
  is the dispatch-side list. Same name, different stages. Unifying them is a
  semantic change (`0` and "unset" mean different things to the two), and is not
  part of this decision.
- `vt::charsets::Charsets` is a G0–G3 manager the parser owns; the tables above
  are data. Both cite `charsets.hpp`; they are not the same module.

## Decision

1. **One home per type.** A type is defined exactly once, in the lowest crate
   that needs it:

   | Kind of type | Crate | Examples |
   |---|---|---|
   | Geometry, arithmetic, containers, colour | `utils` | `Point`, `Rect`, `Size`, `Color`, `SmallVec` |
   | Terminal vocabulary with no storage | `types` | `TextColor`, `TextAttribute`, `CharacterAttributes`, `CursorType`, `Viewport`, `ColorTable`, `LineRendition`, `MarkKind` |
   | VT parameter and dispatch vocabulary | `types` | `VTParameter(s)`, `VTSubParameters`, `VTID`, `EraseType`, `ModeParams`, `CursorStyle`, `GraphicsOptions`, charset tables |
   | Stored cells and rows | `buffer` | `OutputCell`, `Row`, `TextBuffer`, `Selection`, `Marks` |
   | Parser state and output | `vt` | `VtState`, `StateMachine`, `DispatchAction`, `VtParameters`, `Charsets` |

   A crate that does not allocate owns vocabulary; a crate that stores rows owns
   storage.

2. **The adapter depends on `buffer`, not on `vt`.** It applies parsed state to
   stored rows, so it must see the buffer. The loop that reads bytes, drives the
   state machine and calls a handler (`StateMachine` → `DispatchAction` →
   `ITermDispatch`) belongs to whatever owns both — the application crate — so
   `adapter` has no reason to name a parser type.

3. **A module is declared in `lib.rs` only when it compiles.** A module that is
   written but not declared drifts and then stops compiling, which is exactly
   what happened to `adapt_dispatch.rs`.

4. **Unused state is deleted, not annotated.** Fields and helpers kept for
   handlers that do not exist yet are removed with the module and come back with
   the handlers that read them.

## Rationale

- A duplicate type is worse than a wrong one: it compiles, so nothing points at
  the mistake until two halves of the system cannot talk to each other.
- The port's value is that a C++ header maps onto one Rust module. Two modules
  claiming the same header makes that mapping worthless.
- Putting the translation tables in `types` costs nothing today and saves the
  renderer from re-deriving them.
- Keeping `vt` out of `adapter`'s manifest keeps the edge honest: an unused
  dependency is a claim the code does not make.

## Consequences

Positive:

- `AdaptDispatch` is compiled, reachable, and tested for the state it holds.
- `PageManager` holds the real buffer, so the alternate-screen page is a real
  alternate `TextBuffer` rather than a flag on a stub.
- The vocabulary is available to the renderer and the console host, which both
  need `TextAttribute` and the charset tables.

Negative:

- Moving `TextAttribute` and the dispatch vocabulary touched every file that
  names them, and split the tests across crates. It is cheapest while the
  codebase is 18k lines.
- `types` now holds two things that are not obvious neighbours: cell attributes
  and VT dispatch enums. The shared property is that neither allocates.

## Alternatives considered

| Alternative | Why not |
|---|---|
| Have `adapter` depend on `buffer` and keep its own dispatch types | The vocabulary stays private to the adapter; the renderer would re-declare it |
| Move the vocabulary up into `adapter` | `buffer` would then have to depend on `adapter` to name `TextAttribute` — a cycle |
| Unify `VtParameters` with `VTParameters` in this pass | The parser's `0` means "default" where dispatch's `None` means "unset"; merging them changes parsing semantics and needs its own decision |
| Keep the `PageManager` stub and swap later | The renderer and the console host both need pages before the swap, and a stub that grows into a second buffer implementation is the expensive outcome |
