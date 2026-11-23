# ADR 011 — The input engine synthesizes keys from a fixed layout

- Status: Accepted
- Date: 2026-11-22
- Related: `src/terminal/parser/InputStateMachineEngine.cpp`,
  `crates/host/src/input_state_machine_engine.rs`; ADR 003, ADR 010

## Context

`InputStateMachineEngine` turns characters back into key records. When it
receives a printable character, an `ESC`+key, or a control byte, it must decide
which virtual key produced it and which modifiers were held. Upstream asks
Win32:

- `VkKeyScanW` maps a character to a virtual key plus Shift/Ctrl/Alt, using the
  active keyboard layout.
- `MapVirtualKeyW` supplies a scan code from a virtual key.
- `GetDoubleClickTime` supplies the interval two clicks must fall within.

None of these is portable, and the emulation and state layers must not carry
`cfg(windows)` (CONTRIBUTING.md §Emulation). A layout-dependent answer would also make
the same input produce different records on two hosts running the same tests.

## Decision

The engine uses a fixed US-layout table in `key_for_character`.

- A letter, digit, or US punctuation character maps to its US virtual key, with
  Shift for the shifted form.
- `Ctrl+A` through `Ctrl+Z` map to the letter with `LEFT_CTRL_PRESSED`, which is
  how the control codes are typed.
- A character the table does not name keeps its character with virtual key 0.
- A synthesized record carries scan code 0. Win32-input-mode records keep the
  virtual key and scan code from the wire exactly.
- The double-click interval is a constructor parameter, defaulting to 500 ms,
  which is the value Windows normally reports.

## Rationale

- **Determinism.** Two hosts produce the same records for the same bytes, so a
  conformance case can assert them.
- **The consumers do not need a layout.** `InputBuffer` stores the records and
  the cooked read consults the virtual key only to classify editing keys; the
  character is what reaches the line.
- **Unknown characters are not lost.** Upstream drops a character `VkKeyScanW`
  cannot map; keeping it with virtual key 0 is closer to what the application
  wrote.
- **A layout belongs to the platform.** A future UI layer can translate physical
  keys before they reach this engine, or supply a table, without changing the
  parser.

## Consequences

Positive:

- The input engine is platform-independent and testable byte for byte.
- `VkKeyScanW`-style layout surprises cannot break an emulation test.

Negative:

- A client reading `ReadConsoleInput` under a non-US layout sees US virtual keys
  for characters this engine synthesizes. Win32-input-mode input, which carries
  its own keys, is unaffected.
- Synthesized records have no scan code, so a client that distinguishes keys by
  scan code sees 0.
- The table is data that must be extended deliberately; it is not a layout
  engine.

## Alternatives considered

| Alternative | Why not |
|---|---|
| Call `VkKeyScanW`/`MapVirtualKeyW` under `cfg(windows)` | Puts platform code in the emulation layer and makes records machine-dependent |
| Add a keyboard-layout crate | A dependency for a table whose only consumer is key synthesis; a full layout engine is larger than the input parser |
| Send only the character and no virtual key | Loses the key a client reads, and the cooked read's editing-key classification already depends on virtual keys |
| Drop characters the table cannot map | Loses input; upstream does this only because `VkKeyScanW` answers for it |
