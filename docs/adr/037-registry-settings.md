# ADR 037 — The registry's settings are a schema; the reads are the server's

- Status: Accepted
- Date: 2026-11-22
- Related: `crates/host/src/{registry,settings,cmdline}.rs`; ADRs 013, 016

## Context

`registry.cpp` reads `HKCU\Console` (and the subkey named after the title)
into the host's `Settings`, and `RegistrySerialization.cpp` holds the table
that maps each value name to a field and a type. The reads are `RegOpenKeyEx`/
`RegQueryValueEx`; everything else — which keys exist, how a DWORD, WORD, byte
or coordinate is coerced, which values are handled by hand — is data and
arithmetic.

This crate has no Win32 registry calls and no server seam to hang them on
(ADR 013). A module that opened a key would drag the whole console API into a
crate that is otherwise portable and testable.

## Decision

- **The schema is portable and lives in `host::registry`.** A [`Values`] trait
  supplies what a key holds, `Map` is an in-memory key, and `load` returns a
  `ConsoleSettings` whose every field is an `Option`.
- **Absent means unchanged.** `Settings::apply_registry` writes only the values
  the key held, which is upstream's rule: the registry is a layer over the
  constructor's defaults, never a replacement.
- **The types are honoured.** A value of the wrong type is ignored, as upstream
  ignores a `REG_SZ` where it asked for a `REG_DWORD`. WORD and byte values keep
  their low bits, a coordinate is the low word for x and the high word for y,
  and a cursor type outside the six shapes is ignored.
- **The word-delimiter quirk is kept.** The key was `REG_DWORD` when it should
  have been `REG_SZ`, and the empty buffer meant "no delimiters"; a DWORD means
  none, a string means its characters, and a missing key means the defaults. The
  defaults now live here and `cmdline` re-exports them, so the list has one home.
- **Colours stay `COLORREF`s until they are applied.** `ConsoleSettings::apply_colours`
  writes the sixteen entries and the three reserved ones onto a `ColorTable`,
  which is where the adapter reads them.
- **A named window position turns automatic placement off**, whatever the
  coordinate says.
- **The reads are the server's.** A host backend calls `load` with a `Values`
  backed by the registry; nothing else changes.

Not ported, because this port's `Settings` has no field for them: the
answerback message, `UseDx`, `CopyColor`, `TextMeasurement`, `MSAADelay`,
`UIADelay` and `EnableBuiltinGlyphs`. The code page's East Asian rule needs the
host's OEM code page, which a portable console does not have; a key that names a
code page is taken as written.

## Rationale

- **The tests are the port.** Every coercion, every quirk and the layering rule
  are asserted against a `Map`; a pty, a window and a Windows machine are not
  needed to know that a WORD keeps its low bits.
- **One home for the delimiters.** `cmdline` and the registry had the same
  twelve characters twice; now the list is `registry::DEFAULT_WORD_DELIMITERS`
  and `cmdline` re-exports it.
- **A registry read writes nothing else.** Keeping the Win32 calls out means the
  loader cannot close a key it did not open or act on a key it failed to read.

## Consequences

- `host` can load a console's settings from a map today, and from the registry
  when the server layer adds a `Values` implementation.
- `Settings` gained no public setters: the loader writes the fields directly,
  because a setter per field would be a second, unused surface.
- `handle.cpp` and `tracing.cpp` remain unported — see the plan rows: one is two
  wrappers over a lock with no ctrl-event queue behind it, and the other is
  ETW.

## Alternatives considered

| Alternative | Why not |
|---|---|
| `RegQueryValueExW` in `host` | It would put Win32 in the portable crate and need a key handle to own |
| A `Settings` setter per mapped field | Twenty public setters that only the loader calls |
| Ignoring the word-delimiter quirk | It changes what a long-standing key means; upstream keeps it on purpose |
| Filling absent values with zero | It would erase the constructor's defaults, which is what `Option` avoids |
