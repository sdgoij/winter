# ADR 010 — The input parser belongs to the host layer

- Status: Accepted
- Date: 2026-11-22
- Related: `src/terminal/parser/InputStateMachineEngine.cpp`,
  `crates/adapter/src/interact_dispatch.rs`; ADR 001, ADR 009

## Context

`InputStateMachineEngine` is the parser's other grammar: it reads VT sequences
*from* an application and turns them into console input records — virtual-key
codes, `dwMousePosition`, `dwButtonState`, `dwEventFlags` — which it hands to
`IInteractDispatch::write_input`. Upstream uses it in the console host, where a
client can write sequences that conhost must translate back into input events.

The plan placed it in `crates/term` beside the output engine. That is the wrong
layer for three reasons:

- **Its consumer is the host, not the terminal.** `term::Terminal` renders what
  an application writes. It has no console input buffer to fill. The host
  (Phase 6) owns `ConsoleInformation`, `InputBuffer` and `ScreenBuffer`, which
  is what these records are for.
- **The record vocabulary is not portable yet.** `adapter`'s `InputRecord` has
  keyboard fields but no repeat count, key-down flag, mouse position, button
  state, event flags or focus flag — everything this engine fills. Deciding
  that shape is a host-layer decision, because the host is what consumes it.
- **Nothing can test it end to end.** Without a host, a port would assert
  against its own record builder, which proves the transcription and not the
  behaviour.

Meanwhile the input *encoder* (ADR 009) is complete and live: it turns host
events into the bytes an application reads, which is the half a terminal needs.

## Decision

The input parser is ported with the host, in Phase 6, and lives in `crates/host`.

- `crates/term` keeps `TerminalInput`: events in, bytes out.
- `vt` keeps the `IS_INPUT` grammar and the state machine; only the engine is
  deferred.
- The `InputRecord` vocabulary is extended when the host needs it, in the same
  commit as its first consumer, so the fields are shaped by use.

## Rationale

- **Layering is the architecture.** A module belongs where its consumer is;
  `term` builds sequences, the host consumes them.
- **No untested interface.** Extending `InputRecord` now would fix a public
  shape with no caller to validate it, and the host will want fields the parser
  alone cannot justify.
- **The gap is visible.** `plan.md` lists the module under Phase 6 and this ADR
  says why, so a contributor does not re-derive the decision.

## Consequences

Positive:

- `crates/term` stays the small, complete "bytes to screen, events to bytes"
  core.
- The host can shape its input records while writing the code that reads them.

Negative:

- A ConPTY-style server cannot yet parse input until Phase 6 lands.
- The parser's input grammar is exercised only by its own unit tests until
  then.

## Alternatives considered

| Alternative | Why not |
|-------------|---------|
| Port it into `term` now | Consumer is the host; the records would be invented twice |
| Extend `InputRecord` and port it now | Fixes a public vocabulary before its user exists |
| Skip the module entirely | The host needs it for ConPTY input; it is deferred, not dropped |
