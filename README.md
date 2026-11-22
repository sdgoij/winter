# winterm

A cross-platform terminal emulator in Rust, ported module by module from
[Microsoft's Windows Terminal](https://github.com/microsoft/terminal).

The goal is behavioural parity with the Windows Terminal adapter — the same
escape sequences, the same DEC mode handling, the same rendering decisions — on
Windows, Linux, and macOS, with the parser and text buffer written once and
shared by all three.

## Status

Early but coherent. The terminal core — parsing, the buffer, dispatch, the
renderer, and the input encoders — is written and tested; the emulator is not
runnable yet because the host and server layers are still landing.

| Crate | What it is | State |
|---|---|---|
| [`crates/utils`](crates/utils) | Geometry, colour, containers, locks | Complete |
| [`crates/types`](crates/types) | Terminal vocabulary: colours, attributes, enums, input records | Complete |
| [`crates/buffer`](crates/buffer) | `OutputCell`, `Row`, `TextBuffer`, marks, search, image bands | Complete for Phase 2 |
| [`crates/vt`](crates/vt) | DFA parser for VT escape sequences | Complete for Phase 3 |
| [`crates/adapter`](crates/adapter) | Dispatch state, page manager, sixel, macros | Complete for Phase 4 |
| [`crates/term`](crates/term) | Parser-to-dispatch loop, `Terminal` façade, key and mouse encoders | Complete for Phase 5 input |
| [`crates/renderer`](crates/renderer) | Fonts, atlas, software backend, painted frames | GPU backend deferred (ADR 007) |
| [`crates/host`](crates/host) | Command history, input buffer, cooked read with aliases, input engine, console, screen, settings, key info, scrolling, selection input, console utils | Phase 6 under way |

1301 tests pass, clippy is clean at `-D warnings`, and `cargo-deny` passes. The
fourteen-phase plan, the milestone table, and the next module are in
[`.agents/notes/plan.md`](.agents/notes/plan.md).

## Building and testing

The toolchain is pinned in `rust-toolchain.toml` (Rust 1.98.0 and edition 2024).

```sh
just gate        # formatting, lints, licences, tests, doc tests
just fast        # the inner loop: formatting, lints, tests
just test-crate vt
just test-long   # release mode, ignored cases included (font discovery)
just hooks       # enable the pre-commit hook, once per clone
```

`just` is the entry point; `.github/workflows/ci.yml` runs the same recipes on
Linux, Windows, and macOS. `cargo-deny` is required for `just deny`:

```sh
cargo install cargo-deny --locked
```

## Layout

```
crates/utils      geometry, colour, small vector, locks, hash — the only crate with unsafe
crates/types      colours, character attributes, cursor type, viewport, input records
crates/buffer     cells, rows, the scrollback buffer, marks, search, images
crates/vt         the escape-sequence state machine and its dispatch actions
crates/adapter    applies parsed sequences to buffers: dispatch, pages, sixel, macros
crates/term       wires the parser to the adapter; the `Terminal` handle and input encoding
crates/renderer   fonts, cell metrics, atlas, and the software paint path
crates/host       the console side: history, input buffer, cooked read, input parser, selection
```

The dependency ring starts at `utils → types → {buffer, vt} → adapter → term`.
`renderer` sits on `utils`, `types` and `buffer` so the adapter may call it later;
`host` sits on `types`, `vt` and `adapter`, consuming the parser and the console
interfaces. The ring is enforced by the manifests and described in
[ADR 001](.agents/notes/adr/001-workspace-layering.md). Phases 7–14 add `server`,
`input`, `settings`, and the application crate on top of it.

## Contributing

Read [`AGENTS.md`](AGENTS.md) first: it is the working agreement, and it is short.
Procedures (porting a C++ module, testing, writing a commit message) are skills
under [`.agents/skills/`](.agents/skills). Decisions that constrain future code
are ADRs under [`.agents/notes/adr/`](.agents/notes/adr/README.md). The C++ source
is expected at `.design/refs/terminal` (gitignored); the port is written against
it, not from memory.

## Licence

Dual-licensed under either [Apache-2.0](https://www.apache.org/licenses/LICENSE-2.0)
or [MIT](https://opensource.org/licenses/MIT), at your option — see
[LICENSE.md](LICENSE.md).
