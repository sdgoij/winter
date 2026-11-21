# ADR 002 — One home per type, and adapter consumes buffer and vt

- Status: Accepted
- Date: 2026-11-22
- Related: ADR 001, ADR 003; `plan.md` §"Pre-Phase-5 cleanup"

## Context

The adapter layer was written as a self-contained crate before the crates below it
settled. The result is three parallel definitions of the same concepts:

| Concept | Copies | Consequence |
|---------|--------|-------------|
| `VTParameter`, `VTParameters`, `VTID` | `vt/src/parameters.rs`, `vt/src/dispatch_types.rs`, `adapter/src/dispatch_types.rs` | A `vt` parameter cannot be passed to an `adapter` handler |
| Character set tables | `vt/src/charsets.rs`, `adapter/src/charsets.rs` | A fix in one does not reach the other |
| `TextBuffer` | `buffer/src/text_buffer.rs` (744 lines), `adapter/src/page_manager.rs` (a three-field stub) | `PageManager` cannot hold the real buffer |
| `TextAttribute` | `buffer/src/text_attribute.rs` | `adapter` needs it and does not depend on `buffer` |

`adapter/src/adapt_dispatch.rs` is the clearest symptom: it is not declared in
`adapter/src/lib.rs`, so it is dead code, and it does not compile — it imports
`types::TextAttribute` and `crate::{FontBuffer, MacroBuffer}`, none of which
exist. The layer that is supposed to consume the parser has never been connected
to it.

This is a placement problem, not a design problem. The types are right; their
addresses are not.

## Decision

1. **One home per type.** A type is defined exactly once in the workspace, in the
   lowest crate that needs it:

   | Kind of type | Crate | Examples |
   |---|---|---|
   | Geometry, arithmetic, containers, colour | `utils` | `Point`, `Rect`, `Size`, `Color`, `SmallVec` |
   | Terminal vocabulary with no storage | `types` | `TextColor`, `CharacterAttributes`, `UnderlineStyle`, `CursorType`, `Viewport`, `ColorTable`, `LineRendition`, `MarkKind` |
   | VT parameter and sequence vocabulary | `types` (new module) | `VTParameter(s)`, `VTSubParameters`, `VTID`, `EraseType`, `ModeParams`, `CursorStyle`, `GraphicsOptions` |
   | Stored cells and rows | `buffer` | `OutputCell`, `Row`, `TextBuffer`, `TextAttribute`, `Selection`, `Marks` |
   | Parser and its output | `vt` | `VtState`, `StateMachine`, `DispatchAction`, charset translation |

   `TextAttribute` and the VT parameter vocabulary move *down* to `types`: both
   the parser and the adapter need them, and `buffer` already depends on `types`.

2. **`adapter` depends on `buffer` and `vt`.** It is the layer that applies parsed
   sequences to stored state, so it must see both. The duplicates in
   `adapter/src/dispatch_types.rs` and `adapter/src/charsets.rs` are deleted, and
   `adapter/src/page_manager.rs` holds `buffer::TextBuffer`, not a stub.

3. **A module is declared in `lib.rs` only when it compiles.** Dead or
   non-compiling modules are deleted or finished, never left beside working ones.

## Rationale

- A duplicate type is worse than a wrong one: it compiles, so nothing points at
  the mistake until two halves of the system cannot talk to each other.
- The port's value is that a C++ header maps onto one Rust module. Two modules
  claiming the same header makes that mapping worthless.
- `adapter` depending on `buffer` and `vt` is the dependency order the plan
  already assumes; it is only the manifests that disagree.

## Consequences

Positive:

- `AdaptDispatch` can implement `ITermDispatch` and actually drive a
  `TextBuffer`.
- The VT conformance tests can run parser → adapter → buffer end to end, which is
  the first test that proves the system rather than its parts.

Negative:

- Moving `TextAttribute` and the VT vocabulary into `types` touches nearly every
  file that names them. It is mechanical and it is cheapest now.
- `types` grows a module that is not "types with no storage" in spirit. The
  distinguishing property is that nothing in `types` allocates.

## Alternatives considered

| Alternative | Why not |
|---|---|
| Have `adapter` depend on `buffer` and keep its own VT types | Two parameter types still cannot be exchanged; the duplication survives |
| Move `TextAttribute` up into `adapter` | `buffer` owns the cells that carry it, and would then depend on `adapter` — a cycle |
| Keep the `PageManager` stub and swap later | The renderer and the console host both need pages before the swap, and a stub that grows into a second buffer implementation is the expensive outcome |
