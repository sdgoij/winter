# ADR 009 — Input translation takes portable events and a mode snapshot

- Status: Accepted
- Date: 2026-11-22
- Related: `crates/term/src/input/terminal_input.rs`,
  `src/terminal/input/terminalInput.cpp`; ADR 001, ADR 002, ADR 006

## Context

Upstream's `TerminalInput::HandleKey` takes a Win32 `INPUT_RECORD`: virtual key
codes, scan codes, a UTF-16 `UnicodeChar`, and a `dwControlKeyState` bit mask.
It also owns the modes that change its encoding (`_inputMode`), the kitty
protocol stacks, and the alternate-buffer flag, and `AdaptDispatch` writes into
it directly through a reference.

None of that crosses the platform boundary intact: `INPUT_RECORD` does not exist
off Windows, and three of the encoder's inputs come from Windows-only services.
`ToUnicodeEx` and `GetKeyboardLayout` resolve a key against the active keyboard
layout; `wRepeatCount` and `bKeyDown` carry repeat and release; and the 50 ms
timestamps around Left-Ctrl and Right-Alt exist only to detect a synthetic
AltGr Ctrl.

The project rules already say the renderer and emulation stay free of
`cfg(windows)`, and that a type has one home. The adapter here already owns
every input-affecting mode (`ModeFlags`) because DECSET, DECRST and DECRQM need
them, and it owns the kitty flags and stack because `CSI > u` and `CSI < u`
dispatch to it.

## Decision

The input engine consumes a portable event and a snapshot of the modes.

- **`KeyEvent` is the portable `INPUT_RECORD`.** It carries a `Key`, the
  layout-translated `codepoint`, the layout's `base_codepoint` and
  `shifted_codepoint` when the platform knows them, per-side modifiers, caps and
  num lock, the enhanced-key bit, `pressed`, `repeat`, and `alt_gr`.
- **The platform supplies what only it knows.** Repeat, release and AltGr are
  facts the host reports, not things the engine infers from repeated keys or
  from timestamps. The 50 ms Ctrl/Alt timing heuristic is therefore not ported;
  `alt_gr` is explicit instead.
- **`Key` names the key, not the layout.** `Key::Char(c)` means "the physical
  key whose US-layout unshifted character is `c`", which is exactly the mapping
  upstream's US-layout LUT encodes. Named keys are enum variants, so the
  encoder's `switch` becomes an exhaustive `match`.
- **The adapter is the one home for modes.** `types::InputModes` is a snapshot
  built by `AdaptDispatch::input_modes()`; `TerminalInput` holds no mode state.
  The kitty stack, the alternate-buffer flag and every `DECSET` mode stay where
  `DECRQM` reads them.
- **`KeyEvent` carries the raw virtual key and scan code for win32-input-mode.**
  That mode exists to forward a Win32 record to ConPTY, so the host that has
  one supplies it; the values are plain integers, and every other path ignores
  them.

## Rationale

- **One home per flag.** A second copy of "is DECCKM set" would desynchronise
  the first time a new mode is added, and the failure would be silent input
  corruption.
- **Testable without a keyboard.** An event is data, so the expected sequences
  can be asserted directly, including cases the encoder used to derive from
  timestamps.
- **The `Key` enum makes the tables checkable.** Upstream stores kitty codes in
  `uint8_t` LUTs truncated to the PUA offset; an exhaustive `match` fails to
  compile when a key is added.
- **The gaps are narrow and named.** Layout-dependent base keys degrade to the
  US table, and a deliberately held Ctrl+AltGr is reported as AltGr because
  there is no timing state to tell them apart.

## Consequences

Positive:

- Key translation runs on any platform, in tests, with no window or keyboard.
- The kitty and win32 encodings are exercised in CI.

Negative:

- A platform that provides neither `base_codepoint` nor a `Key::Char` loses the
  kitty "base layout key" parameter and the Ctrl-fallback for dead keys.
- win32-input-mode is only byte-identical when the host fills the raw fields.

## Alternatives considered

| Alternative | Why not |
|-------------|---------|
| Keep `INPUT_RECORD` and `cfg(windows)` | Puts platform vocabulary in emulation and leaves other platforms with no input at all |
| Let `TerminalInput` own the modes, as upstream | Two homes for DECSET state; a missed mirror silently corrupts input |
| Port `ToUnicodeEx` via a layout crate | A second keyboard-layout engine to keep in sync with the OS, for one optional parameter |
| Infer repeat and AltGr as upstream does | Needs timestamps and synthetic-event knowledge the platform can just report |
