# winterm

A cross-platform terminal emulator in Rust, ported module by module from
[Microsoft's Windows Terminal](https://github.com/microsoft/terminal).

The goal is behavioural parity with the Windows Terminal adapter — the same
escape sequences, the same DEC mode handling, the same rendering decisions — on
Windows, Linux, and macOS, with the parser and text buffer written once and
shared by all three.

## Status

Early. The foundation is complete and tested; the emulator is not runnable yet.

| Crate | What it is | State |
|---|---|---|
| [`crates/utils`](crates/utils) | Geometry, colour, containers, locks | Complete |
| [`crates/types`](crates/types) | Terminal vocabulary: colours, attributes, enums | Complete |
| [`crates/buffer`](crates/buffer) | `OutputCell`, `Row`, `TextBuffer`, selection, search | No reflow yet |
| [`crates/vt`](crates/vt) | DFA parser for VT escape sequences | Parser complete, corpus pending |
| [`crates/adapter`](crates/adapter) | Dispatch state, page manager, sixel, macros | Print, SGR, cursor, margin, line, erase and mode handlers ported |

609 tests pass, clippy is clean at `-D warnings`, and `cargo-deny` passes. The
fourteen-phase plan, the milestone table, and the cleanup list that stands
between here and the renderer are in
[`.agents/notes/plan.md`](.agents/notes/plan.md).

## Building and testing

The toolchain is pinned in `rust-toolchain.toml` (Rust 1.98.0 and edition 2024).

```sh
just gate        # formatting, lints, licences, tests, doc tests
just fast        # the inner loop: formatting, lints, tests
just test-crate vt
just hooks       # enable the pre-commit hook, once per clone
```

`just` is the entry point; `.github/workflows/ci.yml` runs the same recipes on
Linux, Windows, and macOS. `cargo-deny` is required for `just deny`:

```sh
cargo install cargo-deny --locked
```

## Layout

```
crates/utils     geometry, colour, small vector, locks, hash — the only crate with unsafe
crates/types     colours, character attributes, cursor type, viewport
crates/buffer    cells, rows, the scrollback buffer, selection, marks, search
crates/vt        the escape-sequence state machine and its dispatch actions
crates/adapter   applies parsed sequences to buffers: dispatch, pages, sixel, macros
```

The dependency ring is `utils → types → {buffer, vt} → adapter`, enforced by the
manifests and described in
[ADR 001](.agents/notes/adr/001-workspace-layering.md). Phases 5–14 add
`renderer`, `host`, `server`, `input`, `settings`, and the application crate on
top of it.

## Contributing

Read [`AGENTS.md`](AGENTS.md) first: it is the working agreement, and it is short.
Procedures (porting a C++ module, writing a commit message) are skills under
[`.agents/skills/`](.agents/skills). Decisions that constrain future code are
ADRs under [`.agents/notes/adr/`](.agents/notes/adr/README.md). The C++ source is
expected at `.design/refs/terminal` (gitignored); the port is written against it,
not from memory.

## Licence

Dual-licensed under either [Apache-2.0](https://www.apache.org/licenses/LICENSE-2.0)
or [MIT](https://opensource.org/licenses/MIT), at your option — see
[LICENSE.md](LICENSE.md).
