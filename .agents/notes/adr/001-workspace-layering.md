# ADR 001 — Workspace layering and the crate dependency ring

- Status: Accepted
- Date: 2026-11-22
- Related: `plan.md` §"Monorepo Workspace Layout"; ADR 002, ADR 003

## Context

The C++ source is one tree with an include-path convention: `til` and `types` sit
at the bottom, `buffer` and the VT parser sit on top of them, and `adapter`
reaches into all of it. Nothing in C++ stops a lower layer from including a
higher one, so the boundaries live in reviewers' heads.

The port starts over with boundaries the compiler enforces. The question is only
where they go: what a crate may see, and what happens when a change needs an edge
that is not in the ring.

## Decision

Five crates today, in this order:

```text
utils            geometry, colour, small-vec, locks, hash, unicode
  └── types      colour tables, character attributes, cursor type, viewport
        ├── buffer   cells, rows, TextBuffer, selection, marks, search
        └── vt       DFA parser, parameters, charsets, base64
              └── adapter   dispatch traits, page manager, sixel, macros
                    └── renderer, host, server, input, app    (later phases)
```

1. An edge points right and never left. `types` may not depend on `buffer`; `vt`
   and `buffer` do not depend on each other; `adapter` sees both.
2. The ring is declared once in `[workspace.dependencies]` and every crate
   inherits the entry. Adding an edge is a `Cargo.toml` diff, which is the moment
   a reviewer should think about it.
3. A crate is created when its first real content exists, not when the plan
   names it. A crate for one type is a directory, not a boundary.
4. Later phases follow the same rule: `renderer` and `host` sit beside `adapter`
   and see `buffer` and `vt`; only `app` sees everything.

## Rationale

- **Compile-enforced boundaries.** `cargo test -p vt` cannot touch a text buffer,
  and `-p types` cannot allocate a row, because neither has a path to one.
- **Fast inner loop.** Five small crates check in seconds, which is what makes
  `just gate` affordable before every commit.
- **The port survives translation.** Each C++ module maps onto exactly one Rust
  crate or module, so a change upstream can be diffed against a known place.
- **A visible review event.** The ring is architecture; an unannounced edge is the
  one change nobody notices in a large diff.

## Consequences

Positive:

- The parser and buffer can be fuzzed, benchmarked, and tested without a
  renderer, a window, or a pty.
- Platform code (`cfg(windows)`, `cfg(unix)`) is confined to the crates that must
  have it, so emulation stays identical on every platform.
- A future `renderer` has a small, stable surface to draw against.

Negative:

- Shared types must be placed deliberately. `TextAttribute` living in `buffer`
  while `adapter` needs it is exactly the mistake this ADR exists to prevent; see
  ADR 002 for the rule that fixes it.
- Some crates look thin for their size (`types`). Merging them for convenience is
  forbidden: the ring *is* the architecture.

## Alternatives considered

| Alternative | Why not |
|-------------|---------|
| One crate with modules | Nothing stops the parser from writing to a `TextBuffer` directly; the C++ failure mode reproduced in Rust |
| One crate per C++ module | Roughly forty crates, and the ring becomes unreadable |
| Merge `types` into `utils` | The dependency order stops being visible; `buffer` could then reach geometry helpers it should not need |
