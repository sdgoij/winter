---
name: winterm-porting
description: Port one module from Microsoft's Windows Terminal C++ source into a Rust crate in this workspace. Use when adding or finishing a ported module (buffer, vt, adapter, renderer, host), translating C++ types and error handling, or deciding where a shared type belongs.
---

# Porting a module

The method is in `AGENTS.md` §Porting and ADR 003. This is the procedure.

## 1. Locate the upstream file

The Rust module doc comment names its origin, e.g. `Ported from TextBuffer.hpp in
the Windows Terminal C++ codebase`. If a module has no such line, find the file
in `microsoft/terminal` under `src/terminal/` and add it.

Read the **header** first and write down the public surface: types, methods,
constants, and the meaning of every defaulted parameter. Read the `.cpp` second.
The header is the contract; the implementation has accidents in it that are not
worth reproducing unless a test depends on them.

## 2. Choose the home

| What it is | Crate | Test |
|---|---|---|
| Geometry, colour, container, lock | `utils` | unit, in module |
| Vocabulary with no storage (colours, attributes, enums, parameters) | `types` | unit, in module |
| Stored cells, rows, buffer, selection | `buffer` | unit + proptest |
| Parser state, dispatch actions | `vt` | unit + conformance sequence |
| Applies parsed state to buffers | `adapter` | unit + `ITermDispatch` contract |

If the module needs a type that lives in a crate the ring forbids (ADR 001), the
type is in the wrong place — move it (ADR 002), never duplicate it. Duplicating
is how `VTParameter` ended up with three definitions.

## 3. Translate the shapes

| C++ | Rust here |
|---|---|
| `std::optional<T>` | `Option<T>`; a defaulted parameter is `None`, not `0` |
| `gsl::span<T>` / `T* , size` | `&[T]` / `&mut [T]` |
| `std::string` holding terminal data | `&str` only after UTF-8 decode; otherwise `&[u8]` |
| `THROW_HR`, `HRESULT`, `RETURN_IF_FAILED` | `Result<T, E>` with a typed error; never a panic |
| `wil::unique_any`, `com_ptr` | `Option<Box<T>>`, or an owned struct |
| bit-packed `DWORD` flags | `bitflags!` (already the workspace dependency) |
| `til::CoordType`, `VTInt` | the crate's `VTInt` alias, never `usize` in an operand |
| `_countof(x)` | `x.len()` |
| `static_cast<char>(c)` on a byte | keep `u8`; convert to `char` only for output |

Terminal data is **bytes**. `char` is a rendering concept. A conversion that
loses bytes is a bug unless the loss is the specified behaviour (UTF-8
replacement), and then it happens in exactly one place.

## 4. Port the tests with the code

Upstream's `ut_adapter` / `ut_parser` cases for that module become Rust tests in
the owning module, named after the sequence they exercise:

```rust
#[test]
fn csi_question_1049_h_enters_alternate_screen() { /* ... */ }
```

A handler with no case naming its sequence is not ported. A test that would pass
against the unported stub is not a test; prove it fails before claiming it.

## 5. Wire it in

Add `pub mod <name>;` to `lib.rs` **only when it compiles and its tests pass**.
A module that is written but not declared is invisible to `cargo check`, drifts,
and stops compiling — which is the current state of
`adapter/src/adapt_dispatch.rs`. If a module cannot be finished, delete it rather
than leave it beside the working ones.

## 6. Run the gate

`just gate`. If a ported signature trips a lint (long argument lists are common),
`#[allow(clippy::...)]` with the reason "signature mirrors upstream `X`" is the
correct answer; restructuring the signature is not.

## Traps seen in this port

- **A defaulted VT parameter is not zero.** `VTParameter::new()` is "unset";
  `value_or(default)` is the only way to read it.
- **`DispatchAction` is not a command.** The parser reports what it saw; it does
  not touch a buffer. Only `adapter` does.
- **DCS/OSC payloads arrive in pieces.** Do not assume one callback receives the
  whole string.
- **Sixel and DRCS carry binary data.** `String` will corrupt it.
- **The alternate-screen switch is a page swap, not a clear.** `PageManager` owns
  both pages; erasing the main page is a different operation.
- **Windows cursor/mode state is not in the buffer.** It belongs to the adapter's
  state struct, which is why `AdaptDispatch` exists separately from `TextBuffer`.
