# Winter

A cross-platform terminal emulator in Rust, ported module by module from
[Microsoft's Windows Terminal](https://github.com/microsoft/terminal).

The goal is behavioural parity with the Windows Terminal adapter — the same
escape sequences, the same DEC mode handling, the same rendering decisions — on
Windows, Linux, and macOS, with the parser and text buffer written once and
shared by all three.

## Status

Early but coherent. The terminal core — parsing, the buffer, dispatch, the
renderer, the input encoders, the console host, a working pty layer — is
written and tested. On Windows the shell is native WinUI 3, with tabs in the
title bar, chords, pointer selection, scrollback, the wheel, live settings
reload, a settings dialog and a find bar (ADRs 040–051, 025); Linux and macOS
run the egui shell (ADR 018). The native shells for those two platforms come
after the behaviour is right.

| Crate | What it is | State |
|---|---|---|
| [`crates/utils`](crates/utils) | Geometry, colour, containers, locks | Complete |
| [`crates/types`](crates/types) | Terminal vocabulary: colours, attributes, enums, input records | Complete |
| [`crates/buffer`](crates/buffer) | `OutputCell`, `Row`, `TextBuffer`, marks, search, image bands | Complete for Phase 2 |
| [`crates/vt`](crates/vt) | DFA parser for VT escape sequences | Complete for Phase 3 |
| [`crates/adapter`](crates/adapter) | Dispatch state, page manager, sixel and kitty graphics, macros | Complete for Phase 4 |
| [`crates/term`](crates/term) | Parser-to-dispatch loop, `Terminal` façade, key and mouse encoders | Complete for Phase 5 input; Phase 8 adds focus, clamp and AltGr rules |
| [`crates/renderer`](crates/renderer) | Fonts, atlas, software backend, painted frames; its font collection supplies the shell's faces | The WinUI shell paints on the GPU (ADR 063); the software backend is the tested path |
| [`crates/host`](crates/host) | Command history, input buffer, cooked read with aliases, input engine, console, screen, settings, the registry schema, key info, scrolling, selection input, console utils | Portable modules complete (Phase 6, ADR 037) |
| [`crates/server`](crates/server) | Console API surface: shim policy, and the ConPTY/`openpty` pty layer, including a headless OpenConsole host | Complete for Phase 7 (ADRs 015, 016, 055) |
| [`crates/settings`](crates/settings) | The TOML settings file: named profiles, keybindings, colour schemes, window themes, normalization, the default file and the migration from the earlier JSON one | Phase 10 (ADRs 022–026, 034, 035) |
| [`crates/audio`](crates/audio) | The bell and the DECPS tones: the upstream frequency, wave-rate and gain arithmetic, with no device | Phase 11 (ADR 033) |
| [`crates/winter`](crates/winter) | The application: a `Control` over a `Connection` trait, `app.rs` for the window and its tabs, `terminal_widget.rs` for the pane, `config.rs` for the settings window, a drawn icon, pty sessions, document snapshots and scroll/selection views; `winui/` is the WinUI 3 shell on Windows | Phase 9 complete (ADRs 018–022, 036, 038); WinUI 3 on Windows (ADRs 040–053, 064–068), including our own tab strip in the title bar, its tabs equal-width and ellipsising so they never push the controls under the caption buttons, the selected tab in the terminal's colour and painting down to the pane, a profile menu beside a split new-tab/▾ control, one row per installed WSL distribution and each program's own icon on its row and its tab, a pane whose text is inset from its edges and wraps before the scrollbar, Ctrl+C copy that still interrupts when nothing is selected, a saved settings file picked up within a second (ADR 025), a settings dialog and a find bar |

1731 tests pass, clippy is clean at `-D warnings`, and `cargo-deny` passes. The
fourteen-phase plan, the milestone table, and the next module are in
[`docs/plan.md`](docs/plan.md).

## Configuration

`crates/settings` reads `%APPDATA%\winter\settings.toml` on Windows and
`$XDG_CONFIG_HOME/winter/settings.toml` (else
`$HOME/.config/winter/settings.toml`) elsewhere. A missing file is written
with the defaults on first start, so there is something to edit; an invalid one
starts PowerShell on Windows (the user's `SHELL` elsewhere) from the default
profiles and the default chords. A settings file left by the pre-rename build
is read from the old `winterm` directory once and written to the new one, and
an installation from the earlier JSON schema keeps its profiles:
`settings.json` beside the new file is read once and written out as TOML
(ADR 034).

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
`resizeLeft`/`Right`/`Up`/`Down`, `copy`, `paste`, `openSettings`,
`commandPalette`, `find` and `unbound`.

A binding replaces a default with the same chord and adds a new chord to the
defaults; `unbound` removes one, so the key reaches the shell. An empty list
means the defaults. A chord or action the build does not know rejects the file,
and the defaults load instead (ADR 023).

The plus button starts any profile by name; Ctrl+Shift+P opens the command
palette, which lists a tab per profile followed by the window's actions with the
chord each is bound to (type to filter, arrows move, Enter runs, Escape closes).
On Windows the drop-down also offers one profile per installed WSL
distribution, generated from `wsl.exe -l -q` at load: the list belongs to the
machine, so it is never written to the settings file, and a distribution the
file already names is left to the file (ADR 065). Each profile's row and each
tab carry the icon of the program that profile runs, taken from the executable;
a profile whose program has none wears the shell's own icon, so no row or tab is
left blank. A hairline separates the profiles from `Settings` and `About`, which
carry a gear and a question mark, and the About dialog links this repository
(ADRs 066, 070).
Ctrl+Shift+F opens the find bar: every match in the history and on screen is
highlighted, Enter and Shift+Enter step through them, and Escape closes it. On
Windows the bar is an overlay in the pane (ADR 050). A
scrollbar on the right edge shows where the view sits in the history; its thumb
drags, the track jumps, and the prompts a shell reports with OSC 133 are ticks,
red when the command failed, that jump to the prompt when clicked. A program's
OSC 777 becomes a desktop toast. A shell that exits closes its tab, and the
last tab closes the window (ADR 036).

The bell rings the console host's tone — 800 Hz for 200 ms — and flashes the
pane. A second BEL while the flash runs is ignored, so a program that repeats it
gets one flash and one beep rather than a strobe. DECPS (`CSI … ,~`) plays its
notes as tones on a thread of their own. The tone comes from the Windows `Beep`
API today; the other platforms flash the pane and stay silent (ADR 033).

A scheme names any of `foreground`, `background`, `cursor_color`,
`selection_background`, `black` … `white` and `bright_black` … `bright_white`. A
colour is written in the grammar Windows Terminal accepts — `#RGB`, `#RRGGBB`,
`rgb:r/g/b` — and the colours it leaves out keep the built-in Campbell palette.
A scheme is chosen by `default_scheme`; a name that matches nothing paints the
built-in palette. A bad colour rejects the file (ADR 024).

`theme` names a window theme, and a theme names the fills the tab strip paints:
`tab_row.background`, `tab_row.unfocused_background`, `tab.background` and
`tab.unfocused_background`. A fill that is left out keeps the shell's colour,
an unfocused fill falls back to the focused one, and the tab label picks black
or white by the fill's luminance. An unknown name keeps the shell's colours
(ADR 035).

`font.size` sets the grid's point size (4–100), written as an integer or a
float; a size outside that range is clamped. The face is not configurable yet:
the shell uses one embedded monospace family.

The settings window (Ctrl+, or the gear beside the tabs) shows the file that was
read, whether it parsed, and what is in force, and lists what normalization had
to adjust — an unknown `default_profile`, a blank command, a clamped size
(ADR 027). It edits everything the file holds: the font size, the default
profile, scheme and theme, the profile list, the keybinding list (a chord that
does not parse blocks **Save**), the schemes (a text field and a swatch per
colour; an empty field keeps the built-in colour) and the themes (a name and
four fills). A **Preview** section shows the exact
document a save would write; a value that does not parse leaves it empty and
the editor names the row that blocks the save. The list scrolls, while **Save**,
**Revert** and **Close** are the dialog's own buttons, in its command area, so
neither is ever below the fold (ADR 071). **Save** writes the file and
reloads it, and **Revert** takes the file as it is. A saved file is also picked up
within a second when another program writes it: chords, font size and colours
apply at once, and a changed profile applies to the next tab. A save that does
not parse leaves the running session alone and is reported in the window
(ADRs 025, 028, 030–032). On Windows the same editor opens as a modal dialog
from Ctrl+, (ADR 049).

## Building and testing

The toolchain is pinned in `rust-toolchain.toml` (Rust 1.98.0 and edition 2024).

```sh
just gate        # formatting, lints, licences, tests, doc tests
just fast        # the inner loop: formatting, lints, tests
just test-crate vt
just test-long   # release mode, ignored cases included (font discovery)
just hooks       # enable the pre-commit hook, once per clone
```

On Windows, image protocols (sixel, kitty) need a console host that forwards
them. `just console-host` copies the one Windows Terminal ships beside the
development binaries; `just dist` assembles a runnable release folder with that
host beside `winter.exe`, so an installed copy needs neither Windows Terminal
nor a cached host (ADR 055).

`just package` builds the release binary and wraps that folder in one archive
named after the version and the target:
`dist/winter-<version>-<target>.zip` on Windows, `.tar.gz` elsewhere. Pushing a
tag named `v<version>` — or `v<version>-<prerelease>`, which publishes a
prerelease — runs the gate on all four hosts, then each host packages the binary
it tested and the archives are published with a `SHA256SUMS`
(`.github/workflows/ci.yml`). A prerelease tag labels the archives with the
release's own version (`winter-0.1.0-preview.1-…`), so a preview and the final
release never share a file name.

`just` is the entry point; `.github/workflows/ci.yml` runs the same recipes on
x86-64 and arm64 Linux, Windows, and macOS, plus `just test-long` as a `Full
tests` job — the `#[ignore]`d font cases and the release profile, which the gate
leaves out. `cargo-deny` is required for `just deny`:

```sh
cargo install cargo-deny --locked
```

Linux runs the egui shell. A GTK 4 shell reached a live terminal and is parked
(ADR 062). It stays behind an opt-in feature, so the default build needs no GTK
headers, and running it needs a display:

```sh
sudo dnf install gtk4-devel     # or: sudo apt install libgtk-4-dev
cargo run -p winter --features gtk
```

`WINTER_SCREENSHOT=<path>` makes that shell write its toplevel to a PNG and
quit, which is how it is photographed on a machine with no display server.

`just linux` runs the same build. On Windows the GTK headers live in WSL, so
the first argument names the distro — `just linux FedoraLinux-44`, and
anything after it goes to cargo. There is no default, because a distro's name
is a fact about a machine; `WSL_DISTRO` sets the same thing from the
environment. The guest keeps its artefacts in its own home rather than in the
repository's `target/`.

Wayland gives a client the compositor's keymap and nothing else, so the
compositor is where a layout lives. WSLg always composes `us` and cannot be
told otherwise, so a custom Windows layout never reaches it — every GUI app in
WSL types QWERTY, `konsole` included. The fix belongs on the Windows side,
where a scancode remapper makes that `us` keymap produce the right letters.

## Layout

```
crates/utils      geometry, colour, small vector, locks, hash — the only crate with unsafe
crates/types      colours, character attributes, cursor type, viewport, input records
crates/buffer     cells, rows, the scrollback buffer, marks, search, images
crates/vt         the escape-sequence state machine and its dispatch actions
crates/adapter    applies parsed sequences to buffers: dispatch, pages, sixel, kitty graphics, macros
crates/term       wires the parser to the adapter; the `Terminal` handle and input encoding
crates/renderer   fonts, cell metrics, atlas, and the software paint path
crates/host       the console side: history, input buffer, cooked read, input parser, selection
crates/server     the console API surface; the pty layer (ConPTY, `openpty`, a packed OpenConsole host)
```

The dependency ring starts at `utils → types → {buffer, vt} → adapter → term`.
`renderer` sits on `utils`, `types` and `buffer` so the adapter may call it later;
`host` sits on `types`, `vt` and `adapter`, consuming the parser and the console
interfaces; `server` sits on `host` and exposes the console's API surface;
`settings` needs no workspace crate; `winter` is the application on top and the
only crate that sees everything (ADR 018). The ring is enforced by the manifests
and described in [ADR 001](docs/adr/001-workspace-layering.md).

## Contributing

Read [`CONTRIBUTING.md`](CONTRIBUTING.md) first: it is the working agreement.
Procedures (porting a C++ module, testing, changing the WinUI shell, writing a
commit message) are Markdown files under [`.agents/skills/`](.agents/skills), one
directory each. Decisions that constrain future code are ADRs under
[`docs/adr/`](docs/adr/README.md). The C++ source is expected at
`.design/refs/terminal` (gitignored); the port is written against it, not from
memory.

## Licence

Dual-licensed under either [Apache-2.0](https://www.apache.org/licenses/LICENSE-2.0)
or [MIT](https://opensource.org/licenses/MIT), at your option — see
[LICENSE.md](LICENSE.md).
