# winterm

A cross-platform terminal emulator in Rust, ported module by module from
[Microsoft's Windows Terminal](https://github.com/microsoft/terminal).

The goal is behavioural parity with the Windows Terminal adapter — the same
escape sequences, the same DEC mode handling, the same rendering decisions — on
Windows, Linux, and macOS, with the parser and text buffer written once and
shared by all three.

## Status

Early but coherent. The terminal core — parsing, the buffer, dispatch, the
renderer, the input encoders, the console host, a working pty layer — is
written and tested, and `cargo run -p winterm` opens a shell in an egui window
on Windows, Linux and macOS (ADR 018). That shell is a first shell, not the
final GUI: native backends come after the behaviour is right.

| Crate | What it is | State |
|---|---|---|
| [`crates/utils`](crates/utils) | Geometry, colour, containers, locks | Complete |
| [`crates/types`](crates/types) | Terminal vocabulary: colours, attributes, enums, input records | Complete |
| [`crates/buffer`](crates/buffer) | `OutputCell`, `Row`, `TextBuffer`, marks, search, image bands | Complete for Phase 2 |
| [`crates/vt`](crates/vt) | DFA parser for VT escape sequences | Complete for Phase 3 |
| [`crates/adapter`](crates/adapter) | Dispatch state, page manager, sixel, macros | Complete for Phase 4 |
| [`crates/term`](crates/term) | Parser-to-dispatch loop, `Terminal` façade, key and mouse encoders | Complete for Phase 5 input; Phase 8 adds focus, clamp and AltGr rules |
| [`crates/renderer`](crates/renderer) | Fonts, atlas, software backend, painted frames; its font collection supplies the shell's faces | GPU backend deferred (ADR 007) |
| [`crates/host`](crates/host) | Command history, input buffer, cooked read with aliases, input engine, console, screen, settings, key info, scrolling, selection input, console utils | Portable modules complete (Phase 6) |
| [`crates/server`](crates/server) | Console API surface: shim policy, and the ConPTY/`openpty` pty layer | Complete for Phase 7 (ADRs 015, 016) |
| [`crates/settings`](crates/settings) | The TOML settings file: named profiles, keybindings, colour schemes, normalization, the platform path | Phase 10 (ADRs 022–026) |
| [`crates/winterm`](crates/winterm) | The application: tabs of splittable panes, pty sessions with their pumps, document snapshots, scroll/selection views, egui shell | Phase 9 in progress (ADRs 018–022) |

1450 tests pass, clippy is clean at `-D warnings`, and `cargo-deny` passes. The
fourteen-phase plan, the milestone table, and the next module are in
[`.agents/notes/plan.md`](.agents/notes/plan.md).

## Configuration

`crates/settings` reads `%APPDATA%\winterm\settings.toml` on Windows and
`$XDG_CONFIG_HOME/winterm/settings.toml` (else
`$HOME/.config/winterm/settings.toml`) elsewhere. A missing or invalid file
starts the platform shell with the default chords; the application never writes
the file.

```toml
version = 1

default_profile = "default"
default_scheme = "muted"

[[profiles]]
name = "default"
command = "pwsh.exe"
args = ["-NoLogo"]

[[keybindings]]
keys = "ctrl+alt+t"
action = "newTab"

[[keybindings]]
keys = "ctrl+shift+c"
action = "unbound"

[[schemes]]
name = "muted"
foreground = "#C8C8C8"
background = "#101014"
selection_background = "#264F78"
red = "#D2504A"

[font]
size = 16
```

`version` is the schema version. A file that names a newer version than the
build understands is refused, and the defaults load instead (ADR 026).

A chord is `[ctrl+][alt+][shift+]<key>` in any modifier order, case-insensitive.
Keys are `0-9`, `a-z`, a single punctuation character, `tab`, `enter`, `esc`,
`space`, `backspace`, `insert`, `delete`, `home`, `end`, `pgup`, `pgdn`, the
arrows and `f1`–`f24`. Actions are `newTab`, `closeTab`, `nextTab`, `prevTab`,
`splitHorizontal`, `splitVertical`, `focusLeft`/`Right`/`Up`/`Down`,
`resizeLeft`/`Right`/`Up`/`Down`, `copy`, `paste`, `openSettings` and `unbound`.

A binding replaces a default with the same chord and adds a new chord to the
defaults; `unbound` removes one, so the key reaches the shell. An empty list
means the defaults. A chord or action the build does not know rejects the file,
and the defaults load instead (ADR 023).

A scheme names any of `foreground`, `background`, `cursor_color`,
`selection_background`, `black` … `white` and `bright_black` … `bright_white`. A
colour is written in the grammar Windows Terminal accepts — `#RGB`, `#RRGGBB`,
`rgb:r/g/b` — and the colours it leaves out keep the built-in Campbell palette.
A scheme is chosen by `default_scheme`; a name that matches nothing paints the
built-in palette. A bad colour rejects the file (ADR 024).

`font.size` sets the grid's point size (4–100), written as an integer or a
float; a size outside that range is clamped. The face is not configurable yet:
the shell uses one embedded monospace family.

The settings window (Ctrl+, or the gear beside the tabs) shows the file that was
read, whether it parsed, and what is in force, and lists what normalization had
to adjust — an unknown `default_profile`, a blank command, a clamped size
(ADR 027). It can change the font size, the default profile and the default
scheme: **Save** writes the file and reloads it, and **Revert** takes the file
as it is. A saved file is also picked up
within a second when another program writes it: chords, font size and colours
apply at once, and a changed profile applies to the next tab. A save that does
not parse leaves the running session alone and is reported in the window
(ADRs 025, 028).

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
crates/server     the console API surface; the pty layer (ConPTY and `openpty`)
```

The dependency ring starts at `utils → types → {buffer, vt} → adapter → term`.
`renderer` sits on `utils`, `types` and `buffer` so the adapter may call it later;
`host` sits on `types`, `vt` and `adapter`, consuming the parser and the console
interfaces; `server` sits on `host` and exposes the console's API surface;
`settings` needs no workspace crate; `winterm` is the application on top and the
only crate that sees everything (ADR 018). The ring is enforced by the manifests
and described in [ADR 001](.agents/notes/adr/001-workspace-layering.md).

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
