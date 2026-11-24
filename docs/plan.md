# Windows Terminal C++ → Cross-Platform Rust: Porting Plan

**Reference project:** Microsoft's Windows Terminal, a gitignored local checkout
at `.design/refs/terminal/`. It is the oracle: where this code and upstream
disagree, upstream wins (CONTRIBUTING.md §Porting).
**Goal:** Port the core terminal emulator to a cross-platform Rust codebase targeting Windows, Linux (Wayland/X11), and macOS.

## Status (2026-11-22)

Verified by running the gate, not by reading the previous status:

| Crate | Lines | Tests | State |
|---|---|---|---|
| `utils` | ~6.6k | 213 + 1 ignored | Complete for Phase 1; `PointSpan` joins the geometry primitives |
| `types` | ~6.0k | 121 | Complete for Phase 1; owns `TextAttribute`, the VT vocabulary, `VTID`, `VTParameters` (with sub-parameters), `LineRendition`, the charset tables, `SgrStack`, the colour table, `ColorFix` (Oklab nudging), XParseColor, the X11 colour-name tables, `InputModes`, the console `InputRecord` vocabulary and `FilterStringForPaste` |
| `buffer` | ~7.3k | 223 | Phase 2 — scrollback, `logical_lines` and `resize` (reflow), OSC 8 hyperlink registry, per-row image bands (ADR 005), rectangle scrolling, line rendition, the read-side `TextBufferCellIterator`, the selection copy path (`GetTextRects`/`GetTextSpans`, plain/HTML/RTF), document rows for copies that cross history (ADR 020), and `find_text`, the `search.cpp` logical-line search that reports every match in document coordinates |
| `vt` | ~1.9k | 47 | Phase 3 — the full DFA ported from `stateMachine.cpp`: parameters, sub-parameters, intermediates, OSC/DCS/APC strings and VT52; drives an `IStateMachineEngine`. No fuzz target; the conformance corpus lives beside the `term` loop |
| `adapter` | ~9.7k | 190 | Phase 4 state and interfaces; print, SGR, cursor, margins, insert/delete, scrolling, paging, erase, rectangular, mode (incl. DECCOLM/DECANM), report, presentation state, line rendition, DECRQSS, DRCS, hyperlink/OSC, colour table (incl. DECAC), macro, window, sixel, kitty, sound, save-restore, tab handlers and the ConEmu/iTerm2/FinalTerm/VS Code/WT/urxvt action strings ported |
| `term` | ~5.6k | 199 | The parser-to-dispatch loop, ported from `OutputStateMachineEngine.cpp`: C0, ESC, CSI, OSC, DCS, APC and VT52 mapping; byte-to-UTF-8 decoding; a `Terminal` façade that applies DECANM, C1, presentation-state and DECRQSS work to the adapter; `TerminalInput`'s key and mouse encoders from `terminalInput.cpp` and `mouseInput.cpp` (regular forms, kitty protocol, win32-input-mode, focus, the default/UTF-8/SGR mouse encodings, alternate scroll); and `Terminal.cpp`'s focus dedup, viewport clamp, AltGr aliasing and mouse-tracking report. 113 of the tests are the conformance corpus, six of them UTF-8 |
| `renderer` | ~6.5k | 132 + 1 ignored | Phase 5 — font, cell-metric, cluster, cursor, CSS-length, composition, frame-info and render-settings vocabulary; the `RenderData`, `RenderEngine` and `RenderEngineBase` surfaces; the `renderer.cpp` grid-line, hover and composition paint paths; a host-driven `Renderer::paint_frame` whose `resize` moves the pixel target and the cell viewport together and whose `set_default_colors` feeds the pane's scheme to the engines; the atlas model, `AtlasEngine`, `IBackend`, a software backend and a `fontdue` rasteriser, with the payload carrying the engine's own settings and every row cleared at the start of a frame (ADRs 043, 044); and the `FontCache.h` collection with its SFNT face reader (ADR 008), whose family-and-italic choice the shell now uses (ADR 029). The WinUI shell's painter draws the same payload with Direct2D on the pane's own device: solid fills for the grid, DirectWrite glyph runs for the text (ADR 063) |
| `host` | ~9.6k | 258 | Phase 6's portable modules are done: `CommandHistory`, `InputBuffer`, `CookedReadData` (with alias expansion wired in), `InputStateMachineEngine`, the portable `ConsoleInformation` and `ScreenInformation` (ADR 012), `Settings`, the registry schema that layers a key over them (ADR 037), `AliasTable`, `InputKeyInfo`, `Scrolling`, `SelectionState`/`Selection`/`SelectionInput` (ADR 014), `cmdline` and `utils`. `directio` waits for the server's API surface; `handle` and `tracing` have no counterpart here (a lock with no ctrl-event queue, and ETW) |
| `server` | ~1.0k | 17 + 1 ignored | Phase 7 — `ConsoleShimPolicy` (ADR 015), the `DeviceComm` trait, and the pty layer: ConPTY and `openpty` behind two `#[allow(unsafe_code)]` modules (ADR 016), with a headless `OpenConsole.exe` host that forwards image sequences (ADR 055). The driver protocol is deliberately not ported |
| `settings` | ~3.0k | 86 | Phase 10 (ADRs 022–032, 034, 035, 047): the TOML file's path and schema version, the named-profile schema, keybindings with their own chord syntax and action names (including the scrolling six), colour schemes applied to the adapter's table, window themes the tab strip paints, the font size, the normalization that makes a partial file usable, typed warnings for what it had to decide, a load report that says why a file was not used, a writer, a stamp the application polls to reload a save, the default file a first start is given, the migration from the earlier JSON file, and the table that names a scheme's colours |
| `audio` | ~0.4k | 7 | Phase 11 — sound without a device (ADR 033): the DECPS arithmetic from `MidiAudio.cpp` (note to frequency, the wave rate DirectSound was given, the General MIDI 2 gain law), a note-to-tone map, and the console host's bell tone. The Windows sink is in the app |
| `winter` | ~8.3k | 214 + 1 ignored | Phase 8's shell (ADRs 018, 019, 020), Phase 9's tabs (ADR 021) and panes, and the settings consumers: `session`, `shell_integration`, `term_connection`, `term_control`, `term_events`, `term_settings`, `grid`, `input`, `terminal_render`, `view`, `blink`, `tabs`, `layout`, `app`, `terminal_widget`, `config`, `window` and `icon` (egui on winit/wgpu; ADR 038), plus `winui` on Windows (WinUI 3, ADRs 040–053, 064–066), whose pane fits the window's cells and its DPI, starts from the settings file's profile, font size and scheme, runs its own tab strip in the title bar whose tabs each own a terminal, carry their program's own icon and whose selected tab takes the terminal's own fill and meets the pane, offers one profile per installed WSL distribution beside the new-tab button, with each row's program icon (ADRs 065, 066), matches the file's chords before the terminal sees them (copy, paste, tabs, scrolling; copy declines with no selection so Ctrl+C still interrupts), reads the wheel from a low-level hook and forwards it to a mouse-tracking program unless Shift is held, reloads a saved settings file within a second, opens a settings dialog and a find bar, offers a profile menu beside the new-tab button, paints a retransmitted image once per batch (ADR 057), keeps a held key down through Win32 input records (ADR 056), and selects with the pointer. Mouse selection and forwarding, copy/paste across history, scrollback, drag-to-scroll selection, OSC titles, bell attention, cursor shapes, text decorations, reverse video, intense brightening, faint text, italics from the host's monospace face (ADR 029), tabs, splittable panes whose dividers drag, a shell chosen per tab from the profile menu, a command palette over the profiles and the window's actions, an in-terminal find bar, a scrollbar with prompt marks, a PowerShell hook that reports each command's exit code so a failed one shows red (ADRs 059, 060), desktop notifications
for a program's OSC 777, a settings window that reports the file, lists what it adjusted, edits every setting it holds — profiles, keybindings, schemes, themes, font size and defaults — and previews the document a save writes, a bell that rings the console host's tone and flashes the pane, and DECPS notes played on a thread of their own. The terminal itself is a `Control` over a `Connection` trait, with the viewport and the selection in the widget (ADR 036), and a shell that exits closes its tab |

`just gate` is green: `cargo fmt --all --check`, `clippy -D warnings`, `cargo-deny`,
1724 tests, doc tests. Phases 1–7 are complete. Phase 8 has the last of
`Terminal.cpp`'s input surface and a runnable egui shell with mouse selection,
copy/paste and scrollback; its keyboard, mouse and kitty rows are retired into
`term` (ADR 017), and native window backends are deferred (ADR 018); ADR 040
takes the Windows shell to WinUI 3, and the first WinUI shell (ADRs 041, 042)
runs a shell and paints it through `renderer`, with egui kept for Linux and
macOS until their own shells land; the pane now fits the panel's cells and its
composition scale (ADR 043), starts from the settings file's profile, font
size and scheme (ADR 044), runs a tab strip of tabs that each own a terminal,
matches the file's chords before the terminal sees them (ADR 046), and reaches
scrollback by chord and by a low-level wheel hook that a tracking program
receives unless Shift is held (ADRs 047, 048), reloads a saved settings
file within a second (ADR 025), opens a settings dialog (ADR 049) and a find bar
(ADR 050), and makes the tab strip the title bar (ADR 051); a profile menu beside the new-tab button lists the profiles and the window commands (ADR 052), and Ctrl+C copies a selection while still interrupting when there is none (ADR 053).
Phase 9 has tabs, panes
and the shell's own modules (ADR 038); Phase 10 has the settings
file, its profiles, its keybindings, its colour schemes, the window themes the
tab strip paints, the default file a
first start is written, the migration from the earlier JSON file and the table
that names a scheme's colours; Phase
11 has the bell and DECPS as tones and a flashing
pane, with a sound sink on Windows only (ADR 033). Phase 12 is an ongoing
test sweep, Phase 13 is not started, and Phase 14 runs the gate on three
platforms and publishes a versioned archive per platform for a `v*` tag.

A status that the code has outgrown is a bug in this document. When a milestone
changes, change its row in the same commit.

## Pre-Phase-5 cleanup

Phase 5 puts a renderer on top of the adapter. Nothing stands between here and
it; the rest of this section is the record of what the cleanup passes settled.

**Ported families.**

1. **Several `ITermDispatch` families have no handler.** Ported so far: printing
   (`print`, `print_string`, via `_WriteToBuffer` with delayed EOL wrap, insert
   mode and margin-clipped wrapping), graphics rendition (`set_graphics_rendition`
   from `adaptDispatchGraphics.cpp`: 16/256/RGB colours, extended underline
   styles, faint/blink/intense/inverse/italic/crossed-out/invisible/overline,
   and XTPUSHSGR/XTPOPSGR through `types::SgrStack`), DECSCA protection, cursor
   positioning (CUU, CUD, CUF, CUB, CNL, CPL, CHA, VPA, HPR, VPR, CUP/HVP) and
   the scrolling margins (DECSTBM, DECSLRM behind DECLRMM), the line controls
   (CR, LF/IND/NEL with LNM, RI, DECBI, DECFI), and erase (ED, EL, ECH, DECSED,
   DECSEL, with `_GetEraseAttributes` and DECECM), modes (DECSET, DECRST,
   DECRQM through a decoded ModeParams table, with DECCOLM resizing the page),
   device reports (DSR/CPR/DECXCPR/DA1/DA2/DA3/VT52/DECREQTPARM), cursor
   save/restore (DECSC/DECRC), tab stops, charset designation and shifts,
   rectangular operations (DECCARA, DECRARA with DECSACE extent, DECCRA,
   DECFRA, DECERA, DECSERA, DECRQCRA), hyperlinks and OSC strings (OSC 8
   into the buffer's hyperlink registry, OSC 0/2 title, OSC 7 CWD, OSC 52
   clipboard), macros (DECDMAC text/hex with repeats, DECINVM, DECMSR,
   DECCKSR), window manipulation (CSI 1/2/3/7/8/14/16/18 t, with the
   reports answered from the visible page), the colour table (OSC
   4/10/11/104/110/111 and the DECRQTSR/DECRSTS colour-table report), and
   sixel (DCS `q`, decoded into row bands; see the sixel entry below).
   The ConEmu/iTerm2/FinalTerm/VS Code/WT/urxvt action strings were the last
   family: the buffer marks, taskbar progress, working directory, completions,
   missing-command search and notifications they carry are queued for the
   host. Every `ITermDispatch` method now has a handler, so none falls through
   to the trait default that `AdaptDispatch::unhandled` records; the ratchet
   test in `adapt_dispatch.rs` calls the whole surface and keeps it that way.

   The parser now reaches this list: `term::OutputStateMachineEngine`
   implements `IStateMachineEngine` and maps the C0/ESC/CSI/OSC/DCS/VT52
   families onto `ITermDispatch`. A family with no adapter handler is still
   reported by `unhandled`, not silently dropped.

**Settled.**

- **The parser-to-dispatch loop exists in `term`.** The crate sits to the
  right of `adapter` in the ring and owns both grammars. `LineRendition` was
  the fabricated screen/buffer enum; it is now upstream's buffer enum
  (single/double width, double-height top/bottom), and the module that
  mis-used it is gone.

- **`term::Terminal` is the application's handle.** It owns the parser, the
  engine and an `AdaptDispatch`, feeds bytes through `process_bytes`, and
  forwards the host queues (`take_responses`, clipboard, macro input, bells,
  sounds, window requests) and the title/CWD. `PageManager` now keeps its page
  size, so `Terminal::resize` and DECCOLM size every existing page and every
  page created afterwards.

- **DECRSPS is applied after its DCS ends.** A DCS handler cannot borrow the
  adapter, so the handler parses the report into a shared slot and `Terminal`
  applies it after each byte. DECRQPSR reports the cursor information and the
  tabulation stops; DECRSPS restores both.

- **The stray `adapter/src/main.rs` binary is gone.** The crate is a library
  only again.

- **Images have a home (ADR 005).** `ImageSlice` owns the pixel buffer and the
  cell size, tracks its covered columns, and supports growing, erasing,
  copying and insert-shift; `Row` carries an optional band. Writing a cell or
  clearing a range erases the pixels under it, `fill_rect` erases the block,
  `copy_row` and the vertical scroll move the band, horizontal scroll leaves it
  alone (as upstream does), reflow keeps the first row's band on the first
  destination row, and DECCRA copies bands across pages from a snapshot. The
  DRCS decoder is the remaining step and now has
  `ImageSlice::mutable_pixels` to write through.

- **The colour table and its reports are ported.** `types::ColorTable` is the
  real VT table: the Campbell palette for 0–15, the 6×6×6 cube for 16–231, 24
  greys for 232–255, six reserved slots and four aliases, with the unset slots
  left empty. OSC 4/104 program and reset the indexed entries, OSC 10/11 move
  the foreground/background aliases, OSC 12/17 reach the cursor and selection
  slots, and queries answer in xterm's 16-bit `rgb:` form. DECRQTSR only ever
  supports the colour-table format upstream, so that is the whole of
  `request_terminal_state_report`/`restore_terminal_state` here: the report is
  `2$s` plus `N;model;x;y;z` entries, the restore handler feeds them back, and
  a round-trip test checks the table. The HLS and percentage conversions are
  ported from `Utils::ColorToHLS`/`ColorFromHLS` and friends. `COLORREF`
  packing (`0x00BBGGRR`) is the adapter's wire format for the `u32` colour
  parameter, and RIS restores the defaults.

- **Window manipulation is ported.** CSI 1 t and CSI 2 t record a visibility
  request, CSI 7 t a redraw request, and CSI 8 t a size request; all three are
  host actions, so they wait in `take_window_visibility`,
  `take_redraw_request` and `take_window_resize` instead of calling a window
  API that does not exist. The size reports (CSI 14/16/18 t) answer immediately
  from the visible page and the sixel cell size, so applications that probe the
  cell grid get an answer now. Unlike CSI 8 t, DECCOLM resizes the buffer
  directly: its column count is terminal state, while the window size belongs
  to the host.

- **Macros, sixel and DRCS are ported.** `MacroBuffer` now
  carries upstream's definition parser (text and hex-pair encodings,
  `!count;…;` repeats, the 64 macro ids and 256 KiB shared budget), its
  checksum and its reset semantics. DECDMAC returns the handler, DECINVM
  queues the sequence, and DECMSR/DECCKSR answer from the buffer. Upstream
  injects an invoked macro into its own state machine; with no parser here the
  bytes go to the application through `take_macro_input`, so the re-entrancy
  guard is the application's job.

  `define_sixel_image` now returns the DCS handler. `SixelParser` is the real
  decoder: the `#` colour, `!` repeat, `$` carriage return, `-` next line, `+`
  home and level-3 raster-attribute commands; the VT340 palette for 0–15 and
  the xterm cube and greys for 16–255; colour-number mapping with the VT240
  nearest-colour fallback; the aspect ratio from the DCS parameter and raster
  attributes; six-pixel columns honoured through the aspect ratio; and an
  opaque or transparent background. At ESC the decoded image is painted into
  the starting row's band and the rows below it, growing each band to whole
  cells. The cursor is hidden for the duration and restored afterwards.

  Two upstream deviations, both cosmetic to an application:

  - The image is decoded whole and painted at ESC. Upstream flushes it
    periodically so palette animation is visible mid-image; no host here can
    display it before the DCS ends.
  - With display mode reset, upstream pans and clips at the page margins.
    This scrolls the page to make room and clamps at the bottom, which is the
    same visible result for images that fit on one screen.

  DRCS is ported: `FontBuffer` parses the DECDLD data string, works out the
  cell size, packs and centres the bit patterns and fills the unused
  characters with an error glyph, and `AdaptDispatch::download_drcs`
  registers the designation on the terminating ESC. The renderer is still
  required before a soft font can be seen.

- **Hyperlinks and OSC 0/2, 7 and 52 are ported.** OSC 8 allocates a numeric id
  (`TextBuffer::GetHyperlinkId` plus `AddHyperlinkToMap`, which every upstream
  caller does together) and puts it on the page attributes, so every cell
  written until `end_hyperlink` carries it; the buffer resolves it back to the
  URI. Repeated custom ids for the same URI reuse the id; the `(custom id, uri)`
  pair replaces upstream's `custom id + hash(uri)` key with the same effect.
  Titles over 1000 UTF-16 units are ignored, as xterm does.

  OSC 7 records the URI as given: upstream converts it with a Win32 API
  (`PathCreateFromUrlW`) before calling the host, and that conversion is the
  host layer's job here. OSC 52 stores the content for the host to drain with
  `take_clipboard_content`; it is not gated in the adapter, matching upstream
  (the DA1 feature 52 bit and the app's setting are the gate).

- **Rectangular operations are ported.** `_CalculateRectArea` is shared by all
  seven handlers, so origin-mode offsets, zero-as-extent and clamping are
  defined once. DECCARA/DECRARA apply AND/XOR masks (DECSACE picks rectangle or
  stream extent), DECCRA snapshots the source before writing so overlapping
  moves and cross-page copies are safe, and DECRQCRA computes the DEC checksum
  only when `OptionalFeature::ChecksumReport` is enabled (upstream's
  `Feature_VtChecksumReport` is always on, the optional bit is what gates the
  report). DECCRA's double-width clipping is not replicated: rows are uniform
  width here and cells carry no DBCS lead/trail marking.

- **`buffer` has `resize`/`reflow`.** `TextBuffer::logical_lines` joins rows the
  wrap flag says belong together (across the scrollback boundary) and
  `TextBuffer::resize` rewraps them, re-derives the wrap flags, moves overflow
  into the scrollback, and remaps the cursor by its offset in the line.
  `DECCOLM` now calls it behind the `ALLOW_DECCOLM` (XTERM mode 40) gate. The
  upstream `ut_textbuffer` reflow cases for SBCS text are ported; the DBCS and
  line-rendition cases are not, because this buffer stores one cell per column
  and never sets a line rendition. A cursor whose line is pushed entirely into
  the scrollback is clamped to the top visible row rather than stopping the
  copy early as upstream's circular buffer does.

**Character sets translate.** `types::charsets` now carries `char` tables rather
than bytes, so DEC Special Graphics holds the drawing code points (`U+2500`,
`U+2502`, …) instead of Latin-1 substitutes. `TerminalOutput` re-applies the
locking shifts when a set is designated, `SI`/`SO` read the table they select,
`translate_key` maps GL and GR by range rather than by one flag, and a single
shift (`SS2`/`SS3`) applies to one character and is then cleared.

- The upstream source was not present when the cleanup started; it now is, at
  `.design/refs/terminal`, and the handler port is running against it (ADR 004).
- `AdaptDispatch` is declared in `lib.rs`, compiles, and implements the cursor
  and margin families. Its unused cursor-save slots and tab-stop scaffolding were
  removed rather than annotated; they come back with the handlers that read them.
- `TextAttribute` moved from `buffer` to `types`; `buffer` re-exports it, so the
  old path still resolves (ADR 002).
- `adapter::dispatch_types` moved to `types::dispatch_types` — one definition of
  `VTParameter`, `VTParameters`, `VTSubParameters`, `VTID` and the dispatch enums.
- The 94/96-character translation tables moved from `adapter::charsets` to
  `types::charsets`. They are data, and the renderer will need the same tables.
  `vt::charsets` is a different thing — the G0-G3 manager the parser owns — and
  stays where it is.
- `PageManager` holds a real `buffer::TextBuffer`; page 2 is created with
  `new_alternate`, and two tests assert it. The three-field stub is gone.

Note on the ring: `adapter` depends on `buffer`, not on `vt`. The parser-to-
dispatch loop (`StateMachine` → `IStateMachineEngine` → `ITermDispatch`) belongs
to `term`, the crate that owns both.

## Monorepo Workspace Layout

All library crates live under `crates/`. The main terminal application is `crates/winter`.

```
winter/
├── .github/
│   └── workflows/          (CI/CD for all test types)
├── benches/                (criterion benchmarks)
├── Cargo.lock
├── Cargo.toml              (workspace root, [workspace] members = ["crates/*", "tests", "benches"])
├── crates/
│   ├── adapter/            (terminal-adapter protocol, dispatch)
│   ├── audio/              (beep, visual bell)
│   ├── buffer/             (text buffer, rows, cells, scrollback, reflow)
│   ├── host/               (console host state machine)
│   ├── input/              (keyboard, mouse, IME processing)
│   ├── renderer/           (render engine, glyph atlas, text shaping)
│   ├── server/            (PTY abstraction, handle table)
│   ├── settings/          (config model, profiles, themes, keybindings)
│   ├── term/              (parser-to-dispatch loop, output/input engines)
│   ├── types/              (color, viewport, character attributes, cursors)
│   ├── ui/                 (window metrics, event loop, platform backends)
│   ├── utils/              (utility types: geometry, primitives, small_vec, enumset, hash, etc.)
│   ├── vt/                 (VT escape sequence parser, state machine)
│   └── winter/            (main terminal app — GUI, tabs, panes, command palette)
├── examples/               (demo apps)
├── fuzz/                   (honggfuzz / afl.rs fuzzers)
└── tests/                  (integration, conformance, snapshot, e2e tests)
```

---

## Prerequisites

### Rust Toolchain

- **Toolchain:** 1.98.0, pinned in `rust-toolchain.toml` with `rustfmt` and `clippy`. A change that needs a newer compiler bumps the pin in the same commit.
- **Edition:** 2024, `resolver = "3"`.
- **Formatting:** `max_width = 120`, from `rustfmt.toml`.
- **Lints:** `unsafe_code = "deny"` workspace-wide (`utils` opts out at its root), and clippy runs with `-D warnings`.

### Build & Test Commands

The `Justfile` is the entry point; `.github/workflows/ci.yml` runs the same
recipes and is the truth when the two disagree.

```sh
just gate        # fmt, clippy, deny, test, doc — run before claiming green
just fast        # fmt, clippy, test — the inner loop
just test-crate vt
just hooks       # enable the pre-commit hook once per clone
```

`cargo-deny` is not part of Cargo: `cargo install cargo-deny --locked`.
Benchmarks and fuzz targets arrive with the phases that need them; the recipes
then live in the `Justfile` beside the others.

---

## Architectural Overview

The Windows Terminal codebase maps to these Rust module groups:

| C++ Module | Rust Module | Responsibility |
|---|---|---|
| `src/buffer/` | `winter/crates/buffer` | Text buffer, rows, cells, scrollback, cursors |
| `src/terminal/parser/` | `winter/crates/vt` | VT escape sequence state machine |
| `src/terminal/adapter/` | `winter/crates/adapter` | VT input/output dispatch, terminal-adapter protocol |
| `src/terminal/input/` | `winter/crates/input` | Mouse input, keyboard input processing |
| `src/host/` | `winter/crates/host` | Console host state, screen buffers, I/O |
| `src/server/` | `winter/crates/server` | ConDrv device communication, process list, wait queues |
| `src/renderer/` | `winter/crates/renderer` | Render engine abstraction, glyph atlas |
| `src/interactivity/` | `winter/crates/ui` | Window metrics, interactivity (platform-dependent) |
| `src/audio/` | `winter/crates/audio` | Beep/midi audio (optional) |
| `src/types/` | `winter/crates/types` | Color, viewport, accessibility types |
| `src/cascadia/` | `winter/crates/winter` | High-level terminal app (platform-dependent) |
| `src/til/` + `src/inc/til/` | `winter/crates/utils` | Utility library (reusable in Rust) |

---

## Testing Strategy

**Testing is a first-class deliverable, not an afterthought.** Every module shipped must include:

1. **Unit tests** — `#[cfg(test)]` module inside each source file (mirrors C++ `ut_*` test projects)
2. **Property-based tests** — for state machines, parsing, reflow logic (via `proptest`)
3. **Snapshot tests** — for rendering output comparison against reference images (via `insta`)
4. **Integration tests** — standalone `tests/` crate that exercises multi-module workflows
5. **Conformance tests** — VT sequence input/output against xterm reference output
6. **Fuzzing** — `afl.rs` / `honggfuzz` for VT parser, Sixel parser, and cell iterator
7. **Benchmarks** — `criterion` for throughput (cells/sec), reflow, and rendering

### Test Infrastructure

- [ ] `winter/Cargo.toml` — workspace root with `[workspace] members = ["crates/*", "tests", "benches"]` and test dependencies (`proptest`, `criterion`, `insta`, `honggfuzz`)
- [ ] `winter/tests/test_helpers.rs` — shared test fixtures, mock backends, VT sequence builders
- [ ] `winter/tests/mock_window.rs` — Mock `WindowBackend` for renderer/UI tests
- [ ] `winter/tests/mock_pty.rs` — Mock PTY process for server/host tests
- [ ] `winter/tests/utf8_corpus.rs` — Embedded UTF-8 text corpora for reflow/iterator tests
- [ ] `winter/tests/vt_reference/` — Reference VT sequences from xterm, Kitty, iTerm2
- [ ] `winter/tests/render_snapshots/` — Baseline render output images per backend

> **Note:** All paths below use `winter/` as workspace root. Library crates are at `winter/crates/<name>/src/`, unit tests are `winter/crates/<name>/src/unit_tests/`, shared tests at `winter/tests/`, benchmarks at `winter/benches/`, and fuzzers at `winter/fuzz/`. The main app is `winter/crates/winter/`.

---

## Phase 1: Foundation — Utility Library & Types

**Estimated effort:** 2.5 weeks (0.5 wks for tests)

### Geometry & Primitive Types

- [x] `winter/crates/utils/src/point.rs` — `til::point` with checked arithmetic, comparison, operators
  - [x] **Tests:** 15 unit tests — equality, comparison, addition/subtraction overflow, multiplication overflow, division by zero, `operator bool()` for negative coords, Win32/WinRT conversion roundtrips, cross-platform `RawWindowHandle` conversion
  - [x] **Property tests:** `(a + b) - b == a` when no overflow, `point + 0 == point`, `point.x > 0 && point.y > 0` iff `bool(point)`
- [x] `winter/crates/utils/src/size.rs` — `til::size` with checked arithmetic, scale, area
  - [x] **Tests:** 11 unit tests — creation with zero/negative dimensions, overflow arithmetic, `scale()` with float, `divide_ceil()` edge cases, area overflow for large sizes
  - [x] **Property tests:** `(a.scale(s)).scale(1.0/s) == a` (within tolerance, bounded input), `size.width * size.height == size.area()` (within overflow bounds)
- [x] `winter/crates/utils/src/rect.rs` — `til::rect` (exclusive) with union/intersection/difference iterators
  - [x] **Tests:** 13 unit tests — creation, empty rect, contains check, union of overlapping/disjoint rects, intersection with full overlap/partial overlap/no overlap, difference producing up to 4 rects, iterator bounds correctness, `to_origin()`, `to_inclusive_rect()`, Win32 `RECT` conversion roundtrips
  - [x] **Property tests:** `rect | other` always contains both; `rect & other` is always subset of both; `rect - (rect & other)` produces rects that union back to the difference
  - [x] **Snapshot tests:** render difference region for large rect with known input (rect.rs snapshot_tests)
- [x] `winter/crates/utils/src/inclusive_rect.rs` — `til::inclusive_rect` (inclusive coordinates)
  - [x] **Tests:** 6 unit tests — creation, overlap, contains, Win32 `SMALL_RECT` conversion
- [x] `winter/crates/utils/src/color.rs` — `til::color` RGBA
  - [x] **Tests:** 10 unit tests — RGB construction, ABGR conversion, COLORREF, alpha blending, from_hex, to_css_string, equality
  - [x] **Property tests:** `color.to_hex().to_rgba() == color`, alpha blend is commutative
- [x] `winter/crates/utils/src/small_vec.rs` — `til::small_vector<T, N>` stack-optimized vector
  - [x] **Tests:** 10 unit tests — push/pop/insert/remove, capacity growth, iterator correctness, zero-initialize, drop order, `From<Vec<T>>` conversion
  - [x] **Property tests:** `small_vec.as_slice().to_vec() == small_vec.into_iter().collect::<Vec<_>>()`, push_pop roundtrip
- [x] `winter/crates/utils/src/enum_set.rs` — `til::enumset<Enum>` bitmask
  - [x] **Tests:** 11 unit tests — insert/remove/contains, union/intersection/difference/symmetric difference, is_subset/is_superset, iteration order, bitflag serialization
  - [x] **Property tests:** `(a | b) & a == a`, `(a & b) == (b & a)`, `a | Empty == a`
- [x] `winter/crates/utils/src/hash.rs` — `til::hasher` (wyhash-based)
  - [x] **Tests:** 7 unit tests — hash consistency (same input → same hash), hash of empty input, hash of various string/byte/struct inputs, incremental write bounds check, std hash divergence
  - [x] **Property tests:** identical values produce identical hashes; `std::hash::<T>(v) != til::hash(v)` for at least some types
  - [ ] **Property tests:** `different_data → different_hash` (ignored: extremely unlikely but possible 64-bit collision)
- [x] `winter/crates/utils/src/mutex.rs` — Cross-platform mutex
  - [x] **Tests:** 5 unit tests — lock/unlock, re-entrant, poisoned state, drop restores lock
- [x] `winter/crates/utils/src/atomic.rs` — Atomic wait/notify
  - [x] **Tests:** 2 unit tests — wait/notify roundtrip, timeout, spurious wakeup handling
- [x] `winter/crates/utils/src/ticket_lock.rs` — Ticket lock + recursive ticket lock
  - [x] **Tests:** 3 unit tests — mutual exclusion, fairness (FIFO order), recursive lock same-thread, suspend/resume
  - [ ] **Property tests:** between two threads, no two threads ever hold lock simultaneously
- [x] `winter/crates/utils/src/throttled.rs` — Throttled/debounced function calls
  - [x] **Tests:** 2 unit tests — single call, rapid-fire calls (only last executes), delay elapse → final call, reset
- [x] `winter/crates/utils/src/io.rs` — Cross-platform file I/O
  - [x] **Tests:** 3 unit tests — read/write file, path resolution, temp file cleanup, non-existent file handling
- [x] `winter/crates/utils/src/unicode.rs` — Unicode helpers
  - [x] **Tests:** 8 unit tests — is_alphanumeric, is_whitespace, is_grapheme_boundary, is_cjk, surrogate handling
  - [ ] **Property tests:** `is_grapheme_boundary(a, b)` iff `grapheme_break(a, b)` from Unicode spec
- [x] `winter/crates/utils/src/string.rs` — String utilities
  - [x] **Tests:** 11 unit tests — trim, split, replace_all, truncate_ellipsis, case conversion, starts_with, ends_with, equals insensitive
- [x] `winter/crates/utils/src/replace.rs` — String replacement
  - [x] **Tests:** 3 unit tests — simple replace, replace_all, replace at index, overlapping patterns
- [x] `winter/crates/utils/src/math.rs` — Checked arithmetic
  - [x] **Tests:** 10 unit tests — checked_add/sub/mul/div, saturating operations, rounding, clamping
  - [ ] **Property tests:** `checked_add(a, b).is_none()` iff `a + b` would overflow
- [x] `winter/crates/utils/src/colorbrewer.rs` — Color palettes
  - [x] **Tests:** 4 unit tests — palette generation (pastel1, dark2), color access, named palette availability
- [x] `winter/crates/utils/src/rle.rs` — Run-length encoding
  - [x] **Tests:** 5 unit tests — encode/decode roundtrip, empty input, all-same input, alternating input, `at()` bounds
  - [ ] **Property tests:** `decode(encode(data)) == data`
- [x] `winter/crates/utils/src/flat_set.rs` — Compact set
  - [x] **Tests:** 2 unit tests — insert/remove/contains, iteration, deduplication
- [x] `winter/crates/utils/src/generational.rs` — Generational indices
  - [x] **Tests:** 5 unit tests — create/access/validate, invalidation on update, epoch overflow wrap-around
  - [ ] **Property tests:** valid index stays valid until explicitly invalidated
- [x] `winter/crates/utils/src/spsc.rs` — Single-producer single-consumer queue
  - [x] **Tests:** 2 unit tests — push/pop, full/empty states, capacity, multi-item throughput
  - [ ] **Property tests:** FIFO order preserved
- [x] `winter/crates/utils/src/latch.rs` — One-shot synchronization
  - [x] **Tests:** 4 unit tests — count_down/wait, multiple waiters, double count_down, thread-safety
- [x] `winter/crates/utils/src/bit.rs` — Bit manipulation
  - [x] **Tests:** 9 unit tests — popcount, count_zeros, leading/trailing zeros, bit reversals, byte swaps
  - [x] **Property tests:** `popcount(x) == count_ones(x)` matches stdlib; `reverse_bits(reverse_bits(x)) == x`
- [x] `winter/crates/utils/src/bytes.rs` — Byte slice helpers
  - [x] **Tests:** 9 unit tests — read_u16/u32/u64 LE/BE, write, find_pattern, contains_sequence
  - [x] **Property tests:** `write/read` roundtrip for all valid byte sequences (bytes.rs proptests)

### Core Color & Character Types

- [x] `winter/crates/types/src/text_color.rs` — `TextColor` enum
  - [x] **Tests:** all variants, `From<COLORREF>`, display formatting, equality
- [x] `winter/crates/types/src/cursor_type.rs` — `CursorType`
  - [x] **Tests:** all variants, serialization roundtrip
- [x] `winter/crates/types/src/character_attrs.rs` — `CharacterAttributes` bitflags
  - [x] **Tests:** flag manipulation, all/none, intersection, string formatting
  - [x] **Property tests:** `a | b` contains both flags; `a & a == a` (idempotent) (character_attrs.rs proptests)
- [x] `winter/crates/types/src/underline_style.rs` — `UnderlineStyle`
- [x] `winter/crates/types/src/mark_kind.rs` — `MarkKind`
- [x] `winter/crates/types/src/viewport.rs` — Screen dimension type
  - [x] **Tests:** creation, clamp, contains point, area, intersection, union,
    the inclusive/exclusive bounds, bounds walking and reading-order comparison
- [x] `winter/crates/types/src/line_rendition.rs` — `LineRendition` conversion functions
  - [x] **Tests:** `ScreenToBufferLine`, `BufferToScreenLine` and the inclusive
    point form for single- and double-width renditions
- [x] `winter/crates/types/src/codepoint_width.rs` — Unicode width detection
  - [x] **Tests:** ASCII, CJK, emoji, combining marks, surrogate pairs
  - [x] **Property tests:** width matches `unicode_width::UnicodeWidthChar::width()` for all test cases (codepoint_width.rs proptests)
- [x] `winter/crates/types/src/theme_utils.rs` — Light/dark detection
- [x] `winter/crates/types/src/color_table.rs` — 16-color palette
  - [x] **Tests:** default palette, custom palette, color index lookup, reset

### Text Attribute System

- [x] `winter/crates/buffer/src/text_attribute.rs` — `TextAttribute`
  - [x] **Tests:** default creation, legacy WORD conversion, RGB construction, attribute getters/setters, `HasIdenticalVisualRepresentationForBlankSpace` edge cases, `IsAnyGridLineEnabled`, `HasAnyVisualAttributes`
  - [x] **Property tests:** setting then unsetting an attribute returns to default; `operator==` is reflexive, symmetric, transitive; `memcmp` equality matches field-by-field equality (text_attribute.rs proptests)
- [x] `winter/crates/buffer/src/row_attributes.rs` — Per-row attributes
  - [x] **Tests:** dirty flag
- [x] `winter/crates/buffer/src/dbcs_attribute.rs` — `DbcsAttribute`
- [x] `winter/crates/buffer/src/text_attribute_behavior.rs` — `TextAttributeBehavior` enum

---

## Phase 2: Text Buffer — Core Data Structure

**Estimated effort:** 4 weeks (1 wk for tests)
**Tests:** 180 passing (166 unit + 1 proptest per proptest crate)

### Output Cell Types

- [x] `crates/buffer/src/output_cell.rs` — `OutputCell`
  - [x] **Tests:** default creation, construction from string/attributes, `Chars()`, `TextAttr()`, behavior variants (Stored/Current/StoredOnly), deep copy, move semantics
  - [x] **Property tests:** `OutputCell::new(s).Chars() == s`; creating with Stored behavior preserves attributes across modifications
- [ ] `crates/buffer/src/output_cell_view.rs` — `OutputCellView`
  - [ ] **Tests:** borrowing from cell, read-only access
- [x] `crates/buffer/src/output_cell_iterator.rs` — `OutputCellIterator`
  - [x] **Tests:** advance, advance_past, distance, bounds checking, empty range
  - [x] **Property tests:** iterating forward then backward returns to start; `distance(begin, end) == number_of_steps`
- [x] `crates/buffer/src/output_cell_rect.rs` — `OutputCellRect`
  - [x] **Tests:** creation, iteration, write/read, bounds checking
- [x] `crates/buffer/src/image_slice.rs` — `ImageSlice`
  - [x] **Tests:** create, dimensions, validity check

### Row Implementation

- [x] `crates/buffer/src/row.rs` — `Row`
  - [x] **Tests:** construction with width, `WriteCells()`, `CopyTextFrom()`, `ClearCell()`, `SetAttrToEnd()`, `ReplaceCharacters()`, `ReplaceText()`, `GetText()`, `GetAttrByColumn()`, `ContainsText()`, `GetLastNonSpaceColumn()`, `GetReadableColumnCount()`, `MeasureLeft/Right()`, wrap forcing, double-byte padding, `LineRendition` flag, `ScrollbarData` getter/setter, `StartPrompt()`/`EndOutput()`
  - [x] **Property tests:** writing N cells then calling `ContainsText()` returns true; `ClearCell()` on all cells then `ContainsText()` returns false; `GetText()` returns string with correct display-column length
  - [ ] **Snapshot tests:** render row with mixed widths (ASCII + CJK + emoji)
- [x] `crates/buffer/src/cell_width.rs` — `CharToColumnMapper`
  - [x] **Tests:** char offset → column mapping for ASCII, DBCS, CJK, emoji
  - [x] **Property tests:** `get_column_at(char_offset_0) == 0`; `get_column_at(end) == column_count`
- [x] `crates/buffer/src/line_rendition_ext.rs` — Dimension conversion helpers

### TextBuffer Core

- [x] `crates/buffer/src/text_buffer.rs` — `TextBuffer` (circular scrollback)
  - [x] **Tests:** create, resize, resize_with_reflow, resize_traditional, `GetRowByOffset()`, `GetCellDataAt()`, `GetTextDataAt()`, `Write()`, `WriteLine()`, `FillRect()`, `ScrollRows()`, `CopyRow()`, cursor navigation, word boundary detection, `GetWordStart/End`, `GetGlyphStart/End`, `MoveToNextGlyph`, `ScreenToBufferPosition`, `BufferToScreenPosition`, `GetTextRects()`, `GetTextSpans()`, `GetPlainText(CopyRequest)`, `GenHTML()`, `GenRTF()`, `_AppendRTFText()`, hyperlink map, `SearchText()`, `GetMarkExtents()`, `ClearMarksInRange()`, mutation ID tracking
  - [x] **Property tests:** writing then reading same cells returns original data; `GetPlainText()` for empty buffer is `""`; reflow preserves total character count
  - [ ] **Snapshot tests:** write known text pattern, compare rendered output
  - Not ported: `GetWithControlSequences()`
- [ ] `crates/buffer/src/text_buffer_cell_iterator.rs` — Cell iterator
  - [ ] **Tests:** advance to column, line iteration, bounds, empty buffer
- [x] `crates/buffer/src/text_buffer_text_iterator.rs` — Text iterator
  - [x] **Tests:** advance, line collection, bounds, multi-line text
- [x] `crates/buffer/src/cursor.rs` — `Cursor`
  - [x] **Tests:** position, visibility, blink state, type, clamp to buffer bounds
  - [x] **Property tests:** `clamp(cursor.pos, buffer.size())` always within buffer
- [x] `crates/buffer/src/search.rs` — Text search (simple regex with `*` and `?`)
  - [x] **Tests:** literal search, regex search, case-sensitive/insensitive, multi-line, boundary cases
  - [ ] **Property tests:** searching for text that was just written finds it; searching for empty string finds all positions
  - [ ] **Fuzz tests:** random search patterns on random text corpora
- [x] `crates/buffer/src/marks.rs` — Scroll marks, shell integration marks
  - [x] **Tests:** `StartPrompt()`, `StartCommand()`, `StartOutput()`, `EndCurrentCommand()`, `GetMarkExtents()`, `ClearMarksInRange()`, `ClearAllMarks()`, `GetMarkExtents()` for prompt-only/command-only/output-only, error category marks
  - [x] **Property tests:** marks are correctly bounded; clearing range removes only marks in range
- [ ] `crates/buffer/src/reflow.rs` — Line reflow algorithm
  - [ ] **Tests:** narrow→wide, wide→narrow, all-single-width, all-double-width, mixed-width, emoji with grapheme clusters, CJK text, DBCS text, wrap forced, image rows
  - [ ] **Property tests:** reflow(narrow) → reflow(wide) → reflow(original) produces same layout as original; total character count preserved after reflow; no line exceeds new width
  - [ ] **Snapshot tests:** reflow known paragraph at widths 80→40→20→80
- [ ] `crates/buffer/src/scrollback.rs` — Scrollback management
  - [ ] **Tests:** circular buffer wrap-around, commit watermark, decommit, row eviction, maximum height, sliding window
  - [ ] **Property tests:** writing N rows where N > capacity results in exactly capacity rows; oldest rows are evicted first
- [x] `crates/buffer/src/hyperlinks.rs` — Hyperlink map
  - [x] **Tests:** add/get/remove/clear, generational index invalidation, custom ID mapping
  - [x] **Property tests:** URI lookup is bidirectional for same ID; removing all hyperlinks empties map
- [x] ~~`crates/buffer/src/selection.rs` — Selection~~
  - Removed: ADR 014 moves the selection state and its operations to the host.
  - [x] The rectangle-to-text read it promised landed as
    `TextBuffer::get_text_rects`, `get_text_spans` and `get_plain_text`,
    with the `Row` glyph and readable-content rules they need.
- [ ] `crates/buffer/src/clipboard.rs` — Clipboard transport (the Win32 handle, `CF_HTML` registration); the HTML/RTF text itself is `TextBuffer::gen_html`/`gen_rtf`

### Row Memory Layout

- [ ] Arena-style allocation via `bumpalo` for contiguous row memory
  - [ ] **Tests:** allocation/deallocation, memory alignment, overflow detection, reuse after decommit
- [x] `VecDeque<Row>` with sliding window (implemented via `std::collections::VecDeque`)
  - [x] **Tests:** push_front/push_back, pop_front, capacity growth, iteration order
- [ ] `Vec<u16>` per row for char-to-column offset table
  - [ ] **Tests:** push/pop, DBCS leading/trailing byte handling

---

## Phase 3: VT State Machine — Escape Sequence Parser

**Estimated effort:** 4 weeks (1 wk for tests)
**Tests:** 46 passing

### Core State Machine

- [x] `crates/vt/src/state_machine.rs` — `StateMachine` DFA (20 states)
  - [x] Parameters (32 max, clamped at 65535), sub-parameters (`:`-delimited,
    6 max per parameter), intermediates, OSC command and string, the DCS
    data-string hand-off, VT52 mode and C1 introducers
  - [x] CAN/SUB interrupt from anywhere; DEL is ignored inside sequences
  - [x] **Tests:** each state family, empty/private/intermediate CSI,
    sub-parameters, OSC BEL and ST, a DCS handler and an unsupported DCS,
    VT52 raw parameters, SOS/PM/APC, and a 20 000-byte no-panic run
  - [x] **Divergence:** a disabled C1 byte is dispatched as data rather than
    dropped. Upstream sees decoded `wchar_t`s; this parser sees UTF-8 bytes,
    and continuation bytes of many characters fall in `0x80..=0x9F`.
  - [ ] **Fuzz target:** `honggfuzz`/`afl.rs` remains to be wired
- [x] `crates/vt/src/i_state_machine_engine.rs` — `IStateMachineEngine`
  - [x] Byte-oriented callbacks; `IS_INPUT` selects the input grammar
  - [x] `ActionPassThroughString` and injection tracking omitted: they exist
    for ConPTY, and a terminal has nowhere to forward a sequence to
- [x] `crates/types/src/dispatch_types.rs` — `VTID` (intermediates plus the
  final byte, with `subsequence`) and `VTParameters` (flat sub-parameter
  buffer with one range per parameter)
- [x] `crates/vt/src/state.rs` — the 20 `VTStates` from upstream
- [x] `crates/vt/src/ascii.rs` — ASCII constants + helper functions
- [x] `crates/vt/src/base64.rs` — Base64 encode/decode with proper padding
  - [x] **Tests:** standard/base64url, padding, roundtrip, invalid input
  - [x] **Property tests:** `decode(encode(s)) == s` for byte lengths 0-256
- [x] `crates/vt/src/charsets.rs` — VT character sets (G0-G3 management)
  - [x] **Tests:** G0/G1/G2/G3 assignment, default charset, alternate charset
    activation, SI/SO switching
- [x] `crates/term/src/output_engine.rs` — `OutputStateMachineEngine`
  - [x] C0 execute, print (through one UTF-8 decoder), ESC, VT52, CSI,
    OSC and DCS mapping, ported from `OutputStateMachineEngine.cpp`
  - [x] **Tests:** C0, cursor, SGR, charset designation, save/restore,
    OSC title, hyperlink, clipboard, REP, unknown sequences, an incomplete
    UTF-8 prefix, and a sixel DCS end-to-end into the buffer
  - [x] Colour specs: `rgb:`/`#` forms and the X11 name and variant
    tables, all in `types::color_table`
- [x] `crates/term/src/input/terminal_input.rs` — `TerminalInput`, the key path
  - [x] Ported from `terminalInput.cpp`: the regular encoder (cursor, function,
    keypad, editing keys, VT52 forms), the kitty keyboard protocol flags, the
    shifted and base key parameters, win32-input-mode and focus reporting.
    Repeat, release and AltGr come from the platform (ADR 009)
  - [x] **Tests:** 30 cases covering every sequence family above
- [x] `crates/term/src/input/mouse_input.rs` — `TerminalInput`, the mouse path
  - [x] Ported from `mouseInput.cpp`: the default, UTF-8 and SGR encodings,
    wheel accumulation, hover and drag filtering, alternate scroll. UTF-8
    coordinates are written as UTF-8 characters, not UTF-16 units
  - [x] **Tests:** 16 cases covering every encoding and filter
- [x] `crates/host/src/input_state_machine_engine.rs` — `InputStateMachineEngine`
  - [x] Ported from `src/terminal/parser/InputStateMachineEngine.cpp`: C0 and
    ESC+key records, CSI arrows/Home/End/F1-F4, generic `~` keys, SS3, shift-tab,
    focus, SGR mouse (buttons, drag, wheel, double click), win32-input-mode
    records, the DA1 answer and the captured cursor position report, with the
    VT-input passthrough rules.
  - [x] **Tests:** every sequence family, modifier wrapping, Ctrl+C's control-key
    path, UTF-8 text, mouse state across press/release/drag/wheel/double click,
    DA1 once, the cursor report, and VT-input passthrough
  - [x] Key synthesis uses the fixed US-layout table and scan code 0 (ADR 011)
  - [ ] **Tests:** mouse tracking (1000/1001/1002/1003/1006), bracketed paste,
    kitty keyboard (KKP), Win32 input mode (9001), focus (1004), cursor
    events (1005/1015/1036)
- [ ] `crates/vt/src/tracing.rs` — VT sequence tracing

### VT Conformance Test Sequences (Deliverable)

The corpus is implemented in `crates/term/tests/conformance/` and wired through
`[[test]]` in `crates/term/Cargo.toml`; `CONTRIBUTING.md` and the `winter-testing`
skill are the live description. The paths below are the original sketch of the
sequence inventory, kept as a coverage list rather than as file locations.

- [ ] `winter/tests/vt/csi_sgr.rs` — SGR: all standard escape codes (1/2/3/4/5/7/8/22/23/24/25/27/28/29/30-37/38;2;R;G;B/38;5;n/39/40-47/48;2;R;G;B/48;5;n/90-97/100-107)
- [ ] `winter/tests/vt/csi_cursor.rs` — CUP, CUD, CUF, CUB, CNL, CPL, CHA, CHT, HVP, DCH, ICH
- [ ] `winter/tests/vt/csi_scroll.rs` — SU, SD, DL, IL
- [ ] `winter/tests/vt/csi_erase.rs` — ED (J), EL (K),ECH
- [ ] `winter/tests/vt/osc.rs` — OSC 0/1/2/4/10-17/52/8/9001 (all parameter variations)
- [ ] `winter/tests/vt/dcs.rs` — DCS $ q (Sixel): valid/invalid images, resize, crop
- [ ] `winter/tests/vt/private_modes.rs` — DECSET/DECRST: 1/6/7/20/47/1000/1002/1003/1004/1006/1047/1048/1049
- [ ] `winter/tests/vt/mouse.rs` — Mouse tracking modes 1000/1002/1006 with all button states
- [ ] `winter/tests/vt/kitty.rs` — Kitty keyboard protocol: KKP push/pop, query
- [ ] `winter/tests/vt/bracketed_paste.rs` — Bracketed paste mode activation/deactivation
- [ ] `winter/tests/vt/win32_input_mode.rs` — Win32 Input Mode (CSI <) encoding
- [ ] `winter/tests/vt/escape.rs` — ESC $, ESC (, ESC ), ESC B, ESC D, ESC H, ESC M, ESC Z, ESC 7, ESC 8, ESC =, ESC >, ESC #3-8, ESC c
- [ ] `winter/tests/vt/vt52.rs` — VT52 mode: A-J, F, Y, Z, <, =, >
- [ ] `winter/tests/vt/sixel.rs` — Sixel: valid/invalid images, color palette, resize, interlace
- [ ] `winter/tests/vt/hyperlink.rs` — OSC 8 hyperlinks: start/end/metadata/uri parsing

### VT I/O Layer

- [ ] `crates/vt/src/vt_io.rs` — `VtIo` dual-buffer VT output
  - [ ] **Tests:** initialization, cork/un cork, flush, async write, writer creation/validation
- [ ] `crates/vt/src/writer.rs` — `Writer` struct
  - [ ] **Tests:** cursor backup/restore, write UTF-8/UTF-16/UCS2, CUP, DECTCEM, SGR1006, DECAWM, ASB, DSR, window visibility, title, attributes, screen info
- [ ] `crates/vt/src/input_thread.rs` — Async input thread

### New Modules (Phase 3)

- [x] `crates/vt/src/state.rs` — `VtState` enum (19 states)
- [x] `crates/vt/src/lib.rs` — Module declarations and re-exports
- [x] `crates/vt/Cargo.toml` — Package config with `types` and `utils` deps

---

## Phase 4: Terminal Adapter — Terminal ↔ Host Protocol

**Estimated effort:** 2.5 weeks (0.5 wks for tests)
**Status:** ✅ Complete (2026-06-03)

### Adapter Layer

- [x] `winter/crates/adapter/src/dispatch_types.rs` — Command types
  - VTInt, VTParameter, VTParameters, VTSubParameters, VTID types
  - Enums: EraseType, ChangeExtent, TaskbarState, GraphicsOptions (SGR), CursorStyle, ModeParams, ModeResponses, StatusType, LineFeedType, SixelBackground, DrcsCellMatrix, DrcsFontSet, DrcsFontUsage, CharsetSize, MacroDeleteControl, MacroEncoding, ReportFormat, PresentationReportFormat, ColorItem, ColorModel
  - Constants: DECCOLM_SET_COLUMNS, DECCOLM_RESET_COLUMNS, SINGLE_UNDERLINE, UNDERLINE_NONE, OVERLINE_NONE
  - 46 unit tests passing
- [x] `winter/crates/adapter/src/interact_dispatch.rs` — `IInteractDispatch`
  - Input state machine callback interface
  - InputRecord, InputEventType, ControlKeyState types
- [x] `winter/crates/adapter/src/term_dispatch.rs` — `ITermDispatch`
  - Complete trait with 80+ methods for all VT output sequence handlers
  - Print, cursor movement (CUU/CUD/CUF/CUB/CNL/CPL/HPA/CHA/VPA/HPR/VPR/CUP), save/restore (DECSC/DECRC)
  - Insert/delete (ICH/DCH/SU/SD/NP/PP/PPA/PPR/PPB/IL/DL/IC/DC)
  - Modes (SM/R/DESET/DECSET with 30+ mode flags)
  - Erase (ED/EL/ECH/DECSED/DECSEL), rectangular ops (DECCARA/DECRARA/DECCRA/DECFRA/DECERA/DECSERA)
  - SGR, line rendition, character protection, push/pop graphics
  - Device status (DSR/DA1/DA2/DA3/VT52), character sets (DOCS/SCS/LS0-3/LS1R-3/SS2/SS3/C1 controls)
  - Resets (DECSTR/RIS), cursor style, clipboard, window manipulation
  - Hyperlinks, terminal-specific actions (ConEmu/iTerm2/FinalTerm/VSCode/WT)
  - Sixel, DRCS, user preference charset, macros, terminal state, presentation state, sounds
  - OptionalFeatures enum
- [x] `winter/crates/adapter/src/adapt_dispatch.rs` — Main adapter logic
  - AdaptDispatch struct with mode flags, cursor state, margins, SGR stack
  - CursorState, ChangeOps structs for save/restore and attribute changes
  - ModeFlags bitflags (IRM, DECOM, DECAWM, LNM, BRACKETED_PASTE, DECCKM, DECCOLM, DECSCNM, DECECM)
  - Methods: is_insert_mode(), is_origin_mode(), is_auto_wrap(), is_lnm_mode(), is_bracketed_paste()
  - Margin getters/setters, cursor style management, soft/hard reset
  - 7 unit tests passing
- [x] `winter/crates/adapter/src/font_buffer.rs` — DECDLD soft fonts
  - `SetEraseControl`, `SetAttributes`, `SetStartChar` validate the DECDLD
    parameters; the matrix-size and 80/132-column rules decide the cell
    size (VT2xx/VT320/VT420/VT340/VT382 detection)
  - `AddSixelData`/`FinalizeSixelData` parse the sixel payload, pack and
    centre the bit patterns, and fill unused characters with the error
    glyph
  - 7 unit tests, including upstream's cell-size table and a bit-pattern
    generation case
- [x] `winter/crates/adapter/src/macro_buffer.rs` — VT macro support
  - MacroBuffer for storing/retrieving terminal macros (IDs 0-9)
  - MacroEntry with encoding support (Text/HexPair)
  - CRUD operations: define_macro(), invoke_macro(), delete_macro(), delete_all_macros()
  - defined_count() and total_size() query methods
  - 7 unit tests passing
- [x] `winter/crates/adapter/src/page_manager.rs` — Buffer switching
  - Page, TextBuffer, PageManager for main/alternate screen buffer management
  - Support for up to 6 pages with lazy initialization
  - Methods: get(), active_page(), visible_page(), move_to(), move_relative(), make_active_page_visible()
  - activate_alternate()/deactivate_alternate() for screen buffer switching
  - 7 unit tests including: main→alternate, alternate→main, relative moves, invalid page validation
- [x] `winter/crates/adapter/src/sixel_parser.rs` — Sixel decoding
  - SixelParser state machine (Normal/Attributes/Color/Repeat) and command
    dispatch for `#`, `!`, `$`, `-`, `+`, `"` and ESC
  - VT340 palette, xterm cube and greys, colour mapping, VT240 fallback
  - Aspect-ratio-aware sixel columns, opaque/transparent background filling
  - DecodedImage handed to the adapter, which paints row bands
  - 9 decoder tests plus an adapter-level paint test
- [x] `winter/crates/adapter/src/terminal_output.rs` — Character set translation
  - TerminalOutput for GL/GR character set translation management
  - Methods: designate_94_charset(), designate_96_charset(), translate_key(), locking_shift()
  - User preference charset (UPSS), DRCS designation, GR translation enable
  - Soft/hard reset support, state save/restore
  - 5 unit tests passing
- [x] `winter/crates/adapter/src/charsets.rs` — Character sets
  - Translation tables: ASCII, Latin-1/2, Greek, Hebrew, Cyrillic, Turkish
  - DEC Special Character Set and Graphics
  - 94/96 character set variants (Latin1Based94/96, Latin2Based96, etc.)
  - CHARSET_SIZE_94, CHARSET_SIZE_96, DRCS_BASE_CHAR constants
  - 3 unit tests passing

### Dispatch Operation Tests

> **Note:** Inline unit tests included in each module (46 total). External integration test files planned for Phase 12.

- [x] Cursor operations — covered in adapt_dispatch.rs tests (cursor style, margins)
- [x] Page management — covered in page_manager.rs tests (buffer switching, state restoration)
- [x] Macro operations — covered in macro_buffer.rs tests (define/invoke/delete)
- [x] Font operations — DECDLD parsing and soft-font generation in
  font_buffer.rs (cell-size detection, bit patterns, error glyphs)
- [x] Sixel parsing — covered in sixel_parser.rs tests (commands, palette,
  aspect ratio, background) and adapt_dispatch.rs (payload paints a row band)
- [x] Charset translation — covered in terminal_output.rs and charsets.rs tests

### Implementation Notes

- Ported from Microsoft's Windows Terminal C++ `terminal/adapter` module
- 10 source files, 3,545 lines of code (including tests and documentation)
- Zero compiler warnings, 46/46 unit tests passing
- Type-safe Rust idioms while maintaining C++ API compatibility
- VTID redesigned from bit-packing to array-based storage for reliability
- GraphicsOptions enum fixed duplicate values (aligned with Rust repr(u8) limits)
- ModeParams enum fixed duplicate discriminants (Vt200MouseMode moved to 9000)

---

## Phase 5: Renderer — Cross-Platform Drawing

**Estimated effort:** 6 weeks (1.5 wks for tests)

### Renderer Abstraction

- [x] `winter/crates/renderer/src/render_engine.rs` — `IRenderEngine` trait
  - [x] **Tests:** all virtual methods called correctly, trait object safety
- [x] `winter/crates/renderer/src/render_data.rs` — `IRenderData`
- [x] `winter/crates/renderer/src/render_settings.rs` — `RenderSettings`
- [x] `winter/crates/renderer/src/render_frame_info.rs` — `RenderFrameInfo`, `GridLines`, `GridLineSet`
- [x] `winter/crates/renderer/src/render_engine_base.rs` — `RenderEngineBase`
- [x] `winter/crates/renderer/src/cluster.rs` — `Cluster` (per-glyph shaping result)
- [x] `winter/crates/renderer/src/font_info_base.rs` — `FontInfoBase`
- [x] `winter/crates/renderer/src/font_info.rs` — `FontInfo`
- [x] `winter/crates/renderer/src/font_info_desired.rs` — `FontInfoDesired`
- [x] `winter/crates/renderer/src/font_cache.rs` — the font collection from
  `FontCache.h`: an ordered directory scan, nearby `.ttf` precedence, an SFNT
  `name`/`post`/`OS/2`/`head` reader and a process-wide cache (ADR 008)
  - [x] **Tests:** family, style, weight, italic and fixed-pitch reads, `ttcf`
    faces, truncation and out-of-range offsets, nearby precedence, style
    resolution, default-monospace choice, first-build caching, arbitrary bytes
    never panic
  - [x] **Host check:** `a_host_monospace_face_loads_with_fontdue` is ignored by
    default and opens a real fixed-pitch face when run with `--ignored`
- `winter/crates/renderer/src/font_resource.rs` — **not ported**: upstream's
  `FontResource.cpp` builds a GDI `.FNT` in memory for DRCS soft fonts and
  registers it with `AddFontMemResourceEx`. The DRCS glyphs are ported in
  `adapter` (DECDLD); the GDI registration has no cross-platform equivalent and
  no renderer consumer.
- [x] `winter/crates/renderer/src/css_length.rs` — CSS length parsing
  - [x] **Tests:** pixel, point, percentage, `ch` and bare-number parsing, validation
- [x] `winter/crates/renderer/src/cursor_options.rs` — `CursorOptions`
- [x] `winter/crates/renderer/src/composition.rs` — IME composition state

### Glyph Atlas Engine

- [ ] `winter/crates/renderer/src/atlas/engine.rs` — `AtlasEngine`
  - [ ] **Tests:** font update, text shaping, glyph mapping, atlas creation, dirty region computation, scroll handling
- [ ] `winter/crates/renderer/src/atlas/backend.rs` — `IBackend` trait
- [ ] `winter/crates/renderer/src/atlas/backend_wgpu.rs` — `WgpuBackend`
  - [ ] **Tests:** adapter creation, swapchain creation, texture upload, render pass, present
  - [ ] **Snapshot tests:** render known text to PNG, compare against baseline
- [ ] `winter/crates/renderer/src/atlas/backend_skia.rs` — `SkiaBackend`
  - [ ] **Tests:** surface creation, font rendering, texture upload
  - [ ] **Snapshot tests:** render known text to PNG, compare against baseline
- [ ] `winter/crates/renderer/src/atlas/text_analysis.rs` — Text shaping
  - [ ] **Tests:** grapheme clustering, word boundary, script detection, bidirectional text
  - [ ] **Property tests:** grapheme cluster boundaries match Unicode TR29; word boundaries match Unicode TR29
- [ ] `winter/crates/renderer/src/atlas/builtin_glyphs.rs` — Builtin glyphs
- [ ] `winter/crates/renderer/src/atlas/text_shaping.rs` — Harfbuzz shaping
  - [ ] **Tests:** basic shaping, ligatures, mark attachment, variation fonts, directionality, script handling, feature tags
  - [ ] **Property tests:** shaping "ab" produces same glyphs as shaping "a" then "b" (no inter-glyph features); shaping RTL text mirrors glyph order
- [ ] `winter/crates/renderer/src/atlas/shader_common.wgsl` — WGSL shaders
  - [ ] **Tests:** shader compilation (per-platform)
- [ ] `winter/crates/renderer/src/atlas/shader_ps.wgsl` — Pixel shader
- [ ] `winter/crates/renderer/src/atlas/shader_vs.wgsl` — Vertex shader
- [ ] `winter/crates/renderer/src/atlas/rect_packer.rs` — 2D bin-packing
  - [ ] **Tests:** insert rects, find placement, packing efficiency, empty atlas, full atlas
  - [ ] **Property tests:** all packed rects are non-overlapping; packed area ≤ total requested area + margin

### Rendering Pipeline

- [ ] `winter/crates/renderer/src/render_loop.rs` — Frame render loop
  - [ ] **Tests:** enable/disable, redraw trigger, teardown, background change detection, frame change detection
- [ ] `winter/crates/renderer/src/scrolling.rs` — Scroll animation
  - [ ] **Tests:** scroll delta, animation frames, target reached detection
- [ ] `winter/crates/renderer/src/dirty_region.rs` — Dirty area tracking
  - [ ] **Tests:** invalidate/merge/reset, empty invalidation, full invalidation, multiple regions
  - [ ] **Property tests:** union of dirty regions is always subset of total viewport
- [ ] `winter/crates/renderer/src/paint_background.rs` — Background rendering
  - [ ] **Snapshot tests:** solid color, gradient, pattern background → compare against baseline
- [ ] `winter/crates/renderer/src/paint_buffer.rs` — Visible row rendering
  - [ ] **Snapshot tests:** render ASCII text, CJK text, emoji, mixed-width text → compare against baseline per backend
- [ ] `winter/crates/renderer/src/paint_selection.rs` — Selection rendering
  - [ ] **Snapshot tests:** block selection, line selection, word selection → compare against baseline
- [ ] `winter/crates/renderer/src/paint_cursor.rs` — Cursor rendering
  - [ ] **Snapshot tests:** all cursor types (bar, underscore, box, full box), all sizes, blink on/off → compare against baseline
- [ ] `winter/crates/renderer/src/paint_title.rs` — Title bar rendering
  - [ ] **Snapshot tests:** short title, long title (truncated), Unicode title → compare against baseline
- [ ] `winter/crates/renderer/src/paint_image.rs` — Sixel/image rendering
  - [ ] **Tests:** image upload, crop, resize, interlace
  - [ ] **Snapshot tests:** render known sixel image → compare against baseline
- [ ] `winter/crates/renderer/src/paint_grid_lines.rs` — Grid line rendering
  - [ ] **Snapshot tests:** all gridline combinations → compare against baseline
- [ ] `winter/crates/renderer/src/paint_highlight.rs` — Search highlight rendering

### Font & Shaping

- [ ] `winter/crates/renderer/src/font_loading.rs` — `fontdb` integration
  [ ] **Tests:** font enumeration, face loading, collection matching
  - [ ] **Property tests:** all system fonts can be loaded; font families match system font list
- [ ] `winter/crates/renderer/src/glyph_extraction.rs` — Glyph extraction
  [ ] **Tests:** glyph lookup, advance width, bounding box, color bitmap glyphs
- [ ] `winter/crates/renderer/src/text_shaping.rs` — High-level shaping API
  [ ] **Tests:** single char, multi-char, mixed scripts, mixed widths
- [ ] `winter/crates/renderer/src/font_metrics.rs` — Font metrics
  [ ] **Tests:** ascent/descent/cap height/x-height/line gap, cell size calculation

---

## Phase 6: Console Host — Backend Process

**Estimated effort:** 3.5 weeks (0.5 wks for tests)

Order: `history`, `input_buffer`, `read_data_cooked` (alias expansion now wired
through `alias`), `input_state_machine_engine`, `console_information`,
`screen_info`, `settings`, `alias`, `input_key_info`, `scrolling`, `selection`,
`selection_input` and `utils` are done. `directio` is the `ApiRoutines` surface
that needs the server and its codepage/`CHAR_INFO` translation, so it waits for
Phase 7 with the read-data wait path; `registry`, `handle` and `tracing` follow.
Each module lands with its tests and its declaration in `lib.rs`, as the
porting skill requires.

### Core Console State

- [x] `winter/crates/host/src/console_information.rs` — `ConsoleInformation`
  - [x] Ported from `consoleInformation.cpp` and the class in `server.h`: the
    recursive console lock, the input buffer, the per-application histories,
    the flags, the titles and their prefix, bracketed paste, the pending
    clipboard data (text, HTML and RTF), and the pending-cooked-read marker.
  - [x] **Tests:** title composition, original and link titles, bracketed paste,
    clipboard consumption in all three formats, the popup marker, recursive
    locking, the input buffer, keyed histories, and the flag bits
  - Not ported: the screen buffers, VT I/O, MIDI, code pages, `AllocateConsole`
    and `AsCharInfo`; they need `screenInfo.cpp`, the server layer and the UI
- [x] `winter/crates/host/src/settings.rs` — `Settings`
  - [x] Ported from `settings.cpp` and `settings.hpp`: the constructor and
    desktop defaults, validation, the command-line size override, and the
    console values the host reads (insert mode, history size and duplicate
    suppression, sizes, fill attributes, font request, cursor, paste and
    selection switches, terminal scrolling).
  - [x] **Tests:** constructor and desktop defaults, validation of sizes,
    wrapping and alpha, fill-attribute masking, the command-line paths, and a
    full accessor round-trip
  - Not ported: `RenderSettings` and the colour table (the renderer owns them),
    `STARTUPINFO` and `CONSOLE_STATE_INFO`, the registry serialization, and the
    UIA/MSAA/glyph switches
- [x] `winter/crates/host/src/screen_info.rs` — `ScreenBuffer`
  - [x] Ported from `screenInfo.cpp` as the portable record (ADR 012): the
    text buffer, the viewport with clamping and origin moves, the virtual
    bottom, the virtual viewport and VT page area, the cursor (position,
    visibility, size, type, double-width, make-visible, snap-on-input), the
    attributes, output mode, scroll scale, terminal-scrolling mode, and the
    ConPTY cursor flag.
  - [x] **Tests:** viewport and buffer sizes, virtual viewport and page area,
    viewport clamping, origin validation, terminal-scrolling clamping, cursor
    visibility scrolling, position validation, clipping, cursor information,
    attribute defaults, clearing, snap-on-input, output mode, and the ConPTY
    flag
  - Not ported: the object list, VT state machine, fonts, scroll bars, window
    metrics and resize, `AsCharInfo`, word boundary, and the read-side
    iterators
- [x] `winter/crates/host/src/input_buffer.rs` — `InputBuffer`
  - [x] **Tests:** default console modes, single and batch writes, coalescing
    of a repeated key, a mouse move and a window resize, the batch boundary
    rule, text and NUL writes, prepend, peek versus read, and both flush paths
- [x] `winter/crates/host/src/read_data_cooked.rs` — `COOKED_READ_DATA`
  - [x] Ported from `readDataCooked.cpp`: the edit line (grapheme and
    overwrite-aware insert, backspace, Ctrl+Backspace, Home/End with Ctrl to
    erase, Delete, Escape, Insert), the word-wise cursor movement, history
    recall (Up/Down, PgUp/PgDn, Right-at-end, F3, F5, F8), history expansion
    through `alias.cpp` when the read has an alias source, and the F2/F4/F7/F9
    popups' state and input. The pager, the popups' drawing and the
    `IWaitRoutine::Notify` path are not in this module; see its doc comment.
  - [x] **Tests:** edit, overwrite and insert modes, grapheme backspace,
    combining marks, word movement, erase-to-edge, history recall, prefix
    search, command completion, every popup, the wakeup mask, CR/CRLF, echo
    off, and the read loop
- [x] `winter/crates/host/src/history.rs` — `CommandHistory` and
  `CommandHistoryList`, ported from `history.cpp`
  - [x] **Tests:** add, empty, ring drop, duplicate suppression, retrieval both
    ways with clamping, realloc, prefix and exact search, and the
    per-application list
  - [ ] **Property tests:** cycling through history returns to the most recent
    command
- [ ] `winter/crates/host/src/handle.rs` — Handle management
  - Not ported: `handle.cpp` is `LockConsole` and `UnlockConsole`, which take
    the console lock and, on the outermost unlock, drain the ctrl-event queue.
    `ConsoleInformation` owns the lock (`lock`/`unlock`/`recursion_depth`), so
    the locking half is already there, and this port has no ctrl-event queue to
    drain: the input records come from the pty. A wrapper whose extra work is a
    call to nothing would be a stub, so the file has no counterpart until a
    ctrl-event source exists.
- [x] ~~`winter/crates/host/src/output_stream.rs` — Output stream~~
  - Not ported: `outputStream.cpp` implements `ITerminalApi`, a seam this port
    replaced with adapter-owned state and host-drained queues. ADR 013 records
    the decision and where its content lives.
- [x] `winter/crates/host/src/scrolling.rs` — Scroll logic
  - [x] Ported from `scrolling.cpp` and `scrolling.hpp`: scroll mode
    (`s_DoScroll`/`s_ClearScroll`), the wheel arithmetic (accumulated deltas,
    direction reset, Shift half-screen, both axes, buffer clamping), and
    `s_HandleKeyScrollingEvent` with Escape/Enter/Ctrl+C, the arrows, the page
    keys, horizontal Home/End, Ctrl+Home/Ctrl+End and the beep. The `SB_*`
    commands move the viewport, which is what the window's scroll bar does.
  - [x] **Tests:** metrics off, one notch, partial accumulation, direction
    change, Shift, buffer ends, horizontal wheel, leaving scroll mode, every
    key family, the editor hand-off, modifiers, and the beep
  - Not ported: `s_UpdateSystemMetrics` (the window supplies the metrics),
    `s_ScrollIfNecessary` and its point test (window and selection), and the
    window title/selection side effects of scroll mode
- [x] `winter/crates/host/src/selection.rs` — Selection state
  - [x] Ported from `selection.cpp` and `selectionState.cpp`: the flags (mouse,
    keyboard mark, line/block, the inverted bit), the anchor, the rectangle,
    the exclusive end, the saved cursor, the line-versus-block predicate, and
    `ShouldAllowMouseDragSelection`; plus `InitializeMarkSelection`,
    `InitializeMouseSelection`, `CheckAndSetAlternateSelection`,
    `AdjustSelection`, `ExtendSelection`, `ClearSelection`, `SelectNewRegion`,
    `SelectAll` (whose input-line half takes the no-input-line path, so
    "all" ends at the saved cursor or the cursor), `GetSelectionSpans`, and
    the clipboard copy: plain text plus the optional `GenHTML`/`GenRTF`
    forms, stored on the console and then cleared, as `Clipboard::Copy`
    does.
  - [x] **Tests:** mark and mouse start, the one-cell rectangle, the Alt flip,
    extending with clamping, adjust stopping the drag, cursor save and restore,
    both clear paths, the no-op clear, replacing a selection through
    `SelectNewRegion`, select-all in line and block form, its saved-cursor end,
    the restored viewport origin, line and block copy text, the spans of both
    forms, the formatted copy's output, and the clipboard handoff
  - Not ported: painting and the accessibility notifications (renderer and
    window), and colour selection
- [x] `winter/crates/host/src/selection_input.rs` — Selection input
  - [x] Ported from `selectionInput.cpp`: `HandleKeySelectionEvent` (Escape,
    Enter and Ctrl+C/Ctrl+Insert copy, mark-mode navigation, mouse-selection
    cancellation by a non-system key), `HandleKeyboardLineSelectionEvent` for
    Shift and Ctrl+Shift, `_HandleMarkModeSelectionNav`, `WordByWordSelection`,
    `s_IsValidKeyboardLineSelection`, `GetValidAreaBoundaries` and
    `s_IsWithinBoundaries`. Cell walks come from the new `Viewport`
    `IncrementInBounds`/`DecrementInBounds`/`CompareInBounds` family.
  - [x] **Tests:** Escape, the three copy forms, an unmodified `C`, mouse
    cancellation and system-key survival, every arrow/Home/End/Page move and
    its edge, Shift extending and resetting the anchor, a started line
    selection, both word-by-word walks (ported from `SelectionTests.cpp`),
    wide-character stepping, and the boundary test
  - Not ported: `_HandleColorSelection` (needs `ColorSelection` and the buffer
    search; only reachable with `enable_color_selection` on, which is off by
    default), and the paint/accessibility side effects. The edit line's screen
    bounds need the cooked read's pager, so an input-line-aware selection takes
    upstream's "no input line" path.
- [x] `winter/crates/host/src/cmdline.rs` — Word delimiters
  - [x] Ported from `cmdline.cpp`: `IsWordDelim` for a character and for a
    cell's text, and `DelimiterClass`. The configurable list is
    `registry::DEFAULT_WORD_DELIMITERS`, the same list the registry key
    replaces; the cooked read shares this one home.
  - [x] **Tests:** every default delimiter and space, multi-character and empty
    text, and the three classes
- [x] `winter/crates/host/src/input_key_info.rs` — Keyboard state
  - [x] Ported from `inputKeyInfo.cpp` and the class in `input.h`: the virtual
    key, the control-key state, and the modifier predicates (`IsCtrlOnly`,
    `IsShiftOnly`, `IsShiftAndCtrlOnly`, `IsAltOnly`, `HasNoModifiers`).
  - [x] **Tests:** every modifier combination, both Ctrl keys, and locks not
    counting as modifiers
  - Not ported: `input.cpp`'s `GetControlKeyState`, `ClearKeyInfo` and the
    menu/focus/ctrl handlers, which need window state. `IsSystemKey` is here as
    a free predicate because selection asks it.
- [ ] `winter/crates/host/src/directio.rs` — Direct I/O
  - Deferred: `directio.cpp` implements the `ApiRoutines` entry points, which
    need the server, the codepage/`CHAR_INFO` translation and the VT writer —
    the seam ADR 013 declined to invent. It has no portable half to land ahead
    of them, so the module waits for the server's API surface.
- [x] `winter/crates/host/src/registry.rs` — Registry access
  - Ported from `registry.cpp` and `RegistrySerialization.cpp` as a schema, not
    a reader (ADR 037): the value names, the type each is coerced as (DWORD,
    WORD, boolean, byte, coordinate, string), the values handled by hand —
    colour entries, code page, the word-delimiter quirk, a window position
    turning auto-placement off, `AllowAltF4Close` above one — behind a `Values`
    source with an in-memory `Map`. `Settings::apply_registry` layers what a key
    held over the defaults, and `cmdline` re-exports the delimiter list from
    here. The Win32 reads are the server's.
  - **Tests:** every mapped value and its coercion, an empty key leaving every
    value absent, a value of the wrong type ignored, the low-bits and
    coordinate rules, a cursor type outside the six shapes, `AllowAltF4Close`,
    the three word-delimiter cases, the colour entries as COLORREFs, the
    layering over the defaults, and a position turning auto-placement off
  - Not ported: the answerback message, `UseDx`, `CopyColor`,
    `TextMeasurement`, `MSAADelay`, `UIADelay` and `EnableBuiltinGlyphs`, which
    have no field here; and the code page's East Asian rule, which needs the
    host's OEM code page.
- [x] `winter/crates/host/src/utils.rs` — Console utilities
  - [x] Ported from `utils.cpp`: `CalcWindowSizeX`/`CalcWindowSizeY`,
    `CalcCursorYOffsetInPixels`, `ConvertStringToDec`, `s_CompareCoords` and
    `s_GetOppositeCorner`. Selection's inline corner and comparison code now
    calls these.
  - [x] **Tests:** inclusive window sizes, the cursor offset, decimal parsing
    (including the `WORD` wrap), all eight relative positions of the
    comparison, and every rectangle corner
  - Not ported: `_LoadString` and `s_LoadStringEx`, which read Win32 string
    resources through the window layer.
- [ ] `winter/crates/host/src/tracing.rs` — Event tracing
  - Not ported: `tracing.cpp` is ETW — `TraceLoggingWrite` against a provider
    manifest, plus a debug-flag word and a variadic string tracer. There is no
    ETW outside Windows, no manifest in this port, and no consumer for the
    events: what the user can act on is the settings window and the window's
    own notifications. ADR 013 records why the host has no seam for it.
- [x] `winter/crates/host/src/alias.rs` — Command aliases
  - [x] Ported from `alias.cpp` and `alias.h`: the per-application table with
    case-insensitive application and alias names, add/get/remove/list, the
    cmd.exe clear, and `s_MatchAndCopyAlias`'s expansion (`$1`-`$9`, `$*`,
    `$l`/`$g`/`$b`/`$t`, unknown escapes, the trailing line break and the line
    count). The `ConsoleAlias` A/W API conversions are not part of it.
  - [x] **Tests:** parameters, case folding, `$*` spacing, every escape, an
    empty parameter, removal by empty target, an empty source, leading-space
    and tab argument splitting, and clearing one application

### Screen Buffer Management

- [ ] `winter/tests/host/scrollbar_state.rs` — Scrollbar computation for various buffer/window sizes
- [ ] `winter/tests/host/buffer_switching.rs` — Alternate/main buffer switching
- [ ] `winter/tests/host/font_updates.rs` — Font changes propagate correctly

---

## Phase 7: Server Layer — PTY Communication

**Estimated effort:** 2.5 weeks (0.5 wks for tests)

The server's Windows driver plumbing is not ported (ADR 015): conhost's
`ConDrvDeviceComm`, `Entrypoints`, `WinNTControl` and the API dispatcher tables
serve `condrv.sys`, and a portable terminal has no driver to talk to. The
portable policy modules land as they get consumers.

The pty layer is the port's own (ADR 016): hand-written FFI behind two
already-named, `#[allow(unsafe_code)]` modules, with a blocking `Read`/`Write`
API. Non-blocking I/O and signal forwarding belong to the input layer.

### Portable Server Modules

- [x] `winter/crates/server/src/console_shim_policy.rs` — `ConsoleShimPolicy`
  - [x] Ported from `ConsoleShimPolicy.cpp`: `cmd.exe`, `powershell.exe` and
    `pwsh.exe`, ASCII case-insensitive, over the client's file name.
  - [x] **Tests:** every case variant, both PowerShells, unrelated names, and
    the full-path boundary
  - Not ported: reading the image name from a process handle; the platform
    layer supplies the name.
- [ ] Handle tables, wait queues, process lists and the API sorter's size
  validation — when a consumer exists.
- [x] ~~`api_dispatchers.rs`, `api_message.rs`, `api_message_state.rs`,
  `api_sorter.rs`, `device_handle.rs`, `io_dispatchers.rs`, `object_handle.rs`,
  `object_header.rs`, `process_handle.rs`, `process_list.rs`,
  `wait_block.rs`, `wait_queue.rs`, `win_nt_control.rs`~~
  - Not ported: the console-driver protocol and NT object metadata (ADR 015).

### Cross-Platform PTY Abstraction

- [x] `winter/crates/server/src/pty.rs` — the façade: `PtySize`, `PtyCommand`,
  and the platform `Pty` re-export
  - [x] **Tests:** command construction, the default size
- [x] `winter/crates/server/src/conpty.rs` — ConPTY
  - [x] Created with `CreatePseudoConsole`, `ResizePseudoConsole` and
    `ClosePseudoConsole`; the client starts with
    `PROC_THREAD_ATTRIBUTE_PSEUDOCONSOLE` and `STARTF_USESTDHANDLES` with null
    handles, so it inherits nothing and uses the pseudoconsole's console.
  - [x] **Tests:** command-line quoting, output, input, exit status, resize
- [x] `winter/crates/server/src/posix_pty.rs` — `openpty`
  - [x] `posix_openpt`/`grantpt`/`unlockpt`, the slave opened for the child's
    standard streams, `setsid` + `TIOCSCTTY` in `pre_exec`, `TIOCSWINSZ`.
  - [x] **Tests:** output, the window size as `stty` sees it, input, exit
    status, resize
- [x] `winter/crates/server/src/device_comm.rs` — `DeviceComm` trait
  - [x] **Tests:** `Pty` implements it; the pty tests exercise it
  - These are not upstream files; see ADR 016.

---

## Phase 8: Input Processing & Interactivity

**Estimated effort:** 2.5 weeks (0.5 wks for tests)

The encoders landed with `term` in Phase 4, not in a separate crate: a crate is
created when its first real content exists (ADR 001), and `terminalInput.cpp`
and `mouseInput.cpp` had a consumer there. The plan's `crates/input` rows are
retired (ADR 017); what remains in this phase is the platform-specific window
layer and the app's interactivity layer.

### Input Processing

- [x] ~~`winter/crates/input/src/keyboard.rs`~~ — `terminalInput.cpp`
  - The encoder is `crates/term/src/input/terminal_input.rs`: key down/up,
    modifier and lock detection, the regular encodings, win32-input-mode and
    focus reporting, with the upstream cases ported beside it.
- [x] ~~`winter/crates/input/src/mouse.rs`~~ — `mouseInput.cpp`
  - The encoder is `crates/term/src/input/mouse_input.rs`: tracking modes,
    clicks, motion, wheel accumulation and normalization, the UTF-8 and SGR
    encodings, and alternate scroll.
- [x] ~~`winter/crates/input/src/kitty.rs`~~ — the kitty keyboard protocol
  - Part of `terminal_input.rs` (ADR 009): enabling, disabling, disambiguate,
    report-event-types, report-alternate-keys and report-all-keys.
- [x] `Terminal.cpp`'s input surface — `crates/term/src/terminal.rs`
  - `FocusChanged` dedups (a repeated state is not an event), `SendMouseEvent`
    clamps the position into the viewport (GH#6401), and `_altGrAliasing` is a
    `Terminal` setting that discards a translated character for Ctrl+Alt when
    aliasing is off (GH#5525, GH#6211).
- [ ] Platform event decoding — building `KeyEvent` and `MouseEvent` from the
  window messages, with the keyboard layout the platform owns (ADR 011).
  Lands with the window backends below.
- [ ] IME composition — the platform control hands composed text to
  `Terminal::handle_key` as a character event; there is no portable module.
- [ ] ~~`winter/crates/input/src/input_thread.rs`~~, ~~`pty_signal.rs`~~
  - An input pump reads the pty and feeds `Terminal`, and resize reaches the
    child through `Pty::resize`; both are the application's composition of
    `server` and `term`, so they land with the app crate (Phase 9, ADR 017).

### Interactivity

`ControlInteractivity.cpp` is `src/cascadia` and maps to the application crate
(Phase 9). Its portable state — the multi-click counter, its timer and the
last-click position — goes with it, not into `term`.

### Platform Backends

The first shell is egui on winit and wgpu (ADR 018), in
`crates/winter/src/app.rs`, `terminal_widget.rs`, `config.rs`, `window.rs`
and `icon.rs` (ADR 038). The rows below are retired: they assume four
hand-written backends, and ADR 018 defers them until the application works
everywhere. A native backend replaces those five without touching `session.rs` or
`grid.rs`. ADR 040 replaces the Windows shell with WinUI 3; `src/winui/app.rs`
and `surface.rs` are the first one — a window, one pane, the pty, keys and the
renderer's pixels on a `SwapChainPanel` (ADRs 041, 042). Tabs, the palette and
the settings editor are still to come, and egui keeps Linux and macOS until
their native shells land.

- [x] ~~`winter/crates/ui/src/platform/win32_window.rs`~~ — Win32 backend
  - Retired: winit supplies the window, event loop, DPI and clipboard (ADR 018).
- [x] ~~`winter/crates/ui/src/platform/x11_window.rs`~~ — X11 backend
  - Retired for the same reason.
- [x] ~~`winter/crates/ui/src/platform/wayland_window.rs`~~ — Wayland backend
  - Retired for the same reason.
- [x] ~~`winter/crates/ui/src/platform/cocoa_window.rs`~~ — macOS backend
  - Retired for the same reason.

### Application Shell

- [x] `winter/crates/winter/src/session.rs` — the pty and its pump
  - **Tests:** chunk order and end of stream, an error after data, the repaint
    wake for a chunk and for the end, and a real `cmd`/`sh` session through
    the pty.
- [x] `winter/crates/winter/src/grid.rs` — a buffer snapshot for a painter
  - Reads document rows (`history` first, then the screen) and reports the
    document row shown at the top, so the painter can map a selection.
  - **Tests:** row runs, the cursor, a hidden cursor, and the history revealed
    by scrolling with its top row.
- [x] `winter/crates/winter/src/view.rs` — scroll and selection (ADR 019)
  - Character, word, line and block selection in document cells; the
    multi-click counter; the scroll offset into history. A selection survives
    scrolling and can cross from history into the screen (ADR 020).
  - **Tests:** scroll clamping and selection keeping, click cycling, drag and
    backwards drag spans, block spans, word and line expansion, and the copied
    text of a line, a block and a history selection.
- [x] `winter/crates/winter/src/blink.rs` — the cursor's blink state
  - **Tests:** alternation at the period, steady when the cursor is not
    blinking or the window is unfocused, and the delay to the next toggle.
- [x] ~~`winter/crates/winter/src/ui.rs`~~ — the egui shell
  - Split into `app.rs`, `terminal_widget.rs`, `config.rs`, `window.rs` and
    `icon.rs` when it reached 4,000 lines (ADR 038).
  - Mouse: a tracking application owns clicks, drags and the wheel; Shift is
    the selection override. Without tracking, drag selects, double-click
    selects a word, triple-click selects a row and Alt makes a block
    selection. Ctrl+Shift+C/V copy and paste; Ctrl+C/X/V reach the shell
    because eframe's clipboard shortcuts are off.
  - The window follows the terminal: an OSC 0/2 title sets the OS title, a bell
    asks for attention, and the cursor is drawn in the shape the terminal
    asked for (block, outline, bar, underscore, double underscore) with an
    optional blink.
  - Text honours invisible, faint (blended towards the background), underline
    (single and double), strikeout and overline. Curly, dotted and dashed
    underlines are drawn straight: the line's shape is a font detail. Pane
    borders show which pane has focus.
  - **Tests:** the key map, modifier sides and the AltGr chord, palette
    resolution, the pty cell clamp, the screen-to-document mapping, the Shift
    override and the button map.
  - The pump wakes the window with `request_repaint` after each chunk and at
    end of stream, so the frame loop does not poll.

---

## Phase 9: Terminal App — UI Layer

**Estimated effort:** 5 weeks (1 wk for tests)

Phase 9 is the shell's own modules (ADR 038): `crates/winter/src/tabs.rs` is
the tab list (add, close, select, wrap), `layout.rs` is the pane tree,
`terminal_widget.rs` paints a pane and translates its input, `config.rs` edits
the settings, and `app.rs` draws the strip, splits the panes and owns the
window-level chords:
Ctrl+Shift+T/W (tab), Ctrl+Shift+D/E (split), Ctrl+Tab (switch), Alt+Arrow
(focus), Alt+Shift+Arrow (resize), Ctrl+Shift+C/V (clipboard), Ctrl+Shift+P
(the command palette) and Ctrl+Shift+F (the find bar). The plus menu starts any
profile by name. The rest of `src/cascadia` remains.

### Application Core

- [x] `winter/crates/winter/src/app.rs` — App lifecycle
  - Landed as the window itself, from `App.cpp` and `AppHost.cpp` (ADR 038):
    `App` and its tabs, the tab strip, the command palette, the chord map, the
    font registration and `run`. It drains what every terminal reports once a
    frame and turns an event into attention, a toast or a closed tab.
- [x] `winter/crates/winter/src/config.rs` — Config loading/saving
  - The settings window and its draft, from `TerminalSettingsEditor/` and the
    way `CascadiaSettings` is loaded and saved: the per-list drafts, the form,
    the preview, atomic save and the poll that reloads a saved file. The file
    itself is `crates/settings` (ADRs 025–032, 034).
- [x] ~~`winter/crates/winter/src/settings.rs`~~ — Settings model
  - Not ported: the model is `crates/settings` (the file, profiles, chords,
    schemes, themes) and the per-terminal slice is `term_settings.rs` (ADR 036).
    A second model in the shell would be a copy.
- [x] ~~`winter/crates/winter/src/theme.rs`~~ — Theme system
  - Not ported: themes are `crates/settings/src/themes.rs`, applied by the tab
    strip (ADR 035).
- [x] `winter/crates/winter/src/window.rs` — Window management
  - The three viewport commands the shell asks for — a title, an attention
    request and a close — named so the toolkit's vocabulary stays in one place
    (ADR 038).
- [x] `winter/crates/winter/src/icon.rs` — Embedded resources
  - The window icon is drawn rather than shipped: a dark rounded square with a
    prompt chevron and an underscore, at the 64×64 a window manager wants. There
    is no resource pipeline here and a binary asset would be unreadable
    (ADR 038). **Tests:** the size, the rounded corners, the painted middle, the
    background, and that the prompt covers pixels in its colour.

### Tab & Pane Management

- [x] ~~`winter/crates/winter/src/tab_bar.rs`~~ — Tab row
  - Drawn in `app.rs`: one selectable title per tab, a close button and a plus
    menu that starts any profile by name. **Tests:** the chord map
    (Ctrl+Shift+T/W, Ctrl+Tab) and the profile a name selects, not the pixels.
- [x] ~~`winter/crates/winter/src/tab.rs`~~ — Tab state
  - Landed as `tabs.rs` (the list) plus `Tab` in `app.rs` (one session, its
    terminal and its view). **Tests:** add, close, select and wrap, including
    the active index after a neighbour closes.
- [x] ~~`winter/crates/winter/src/pane.rs`~~ — Pane management
  - Landed as `layout.rs` (the binary split tree with fractional rectangles)
    plus `Pane` in `terminal_widget.rs` and `Tab` in `app.rs`. **Tests:** split, remove with collapse,
    nearest-boundary resize, ratio clamping and the focus walk in a 2x2 grid.
- [x] `winter/crates/winter/src/terminal_widget.rs` — egui terminal viewport
  - `TermControl.cpp` as `Pane`: painting a `grid` snapshot, turning egui events
    into `term` keys and mouse records, the find bar, the scrollbar and the
    bell's flash, over the `Control` that owns the terminal (ADR 036). The
    viewport and the selection stay here, as ADR 019 has them (ADR 038).
- [x] ~~`winter/crates/winter/src/terminal_renderer.rs`~~ — wgpu render pass
  - Not ported: egui's painter *is* the render pass, on top of wgpu through
    eframe, and `renderer`'s software backend is the path that is tested. A
    second pass would duplicate it (ADRs 007, 018, 038).

### Command Palette & Settings

- [x] ~~`winter/crates/winter/src/command_palette.rs`~~ — Command palette
  - Landed in `app.rs` as `Palette` and `palette_window`: Ctrl+Shift+P (the
    `commandPalette` action) opens a filter field over a list holding a tab per
    profile and then the window's actions, each with the chord the file binds to
    it. The filter matches a label in any case, the arrows wrap the selection,
    Enter runs it and Escape closes it. **Tests:** the chord, the entries, the
    filter, the open/close reset, and that a text field or the palette keeps
    keys out of the shell, not the pixels.
- [x] ~~`winter/crates/winter/src/settings_editor.rs`~~ — Settings editor
  - Landed in `config.rs`: the settings window edits everything the schema
    holds — font size, the default profile and scheme, the profile list, the
    keybinding list and the color schemes (a text field and a swatch per
    colour; an empty field means the built-in colour). **Save** writes the
    file through `settings::save` (a temporary file renamed over it) and
    reloads it; a dirty draft blocks the reload poll and reports the conflict
    (ADRs 028, 030–032). **Tests:** the draft's diff and document, the profile,
    chord and scheme round-trips, and the invalid chord and colour blocks, in
    `config.rs`; directory creation and overwrite in `parser.rs`.
  - [x] A preview of the document as it will be written.
    Landed as the **Preview** collapsible in `config.rs`: `Draft::preview` renders
    the same document **Save** writes through `settings::to_toml`, and a value
    that does not parse leaves it empty with a line naming the row that blocks
    the save. The window itself now scrolls, because the editor had grown past
    a screen's height and put **Save** below the bottom edge. **Tests:** the
    preview is the document a save writes, and a broken colour leaves it empty.
- [x] ~~`winter/crates/winter/src/search.rs`~~ — In-terminal search overlay
  - Landed as the find bar in `terminal_widget.rs`, on Ctrl+Shift+F (the `find` action) over
    `TextBuffer::find_text`: every match in the document, history included, is
    highlighted and the current one more strongly; Enter and Shift+Enter wrap
    through them and scroll each into the middle of the view, and Escape closes
    the bar. The query never reaches the shell. There is nothing to replace: a
    terminal's text belongs to its program.
  - Matching is the buffer's logical-line search, ported from `search.cpp` and
    the UText adapter: a row contributes its readable cells and a line break
    only when it was not wrapped, so a match can run through an autowrap but
    never through a real line break. A whitespace-only needle finds nothing, and
    case is ignored. A match is a half-open span in document coordinates, so a
    wrapped match paints on every row it covers. **Tests:** the upstream
    `Unicode` sample, an autowrap match, a line break that stops one, wide
    characters, overlap, case, history, the scroll, the row spans, and the bar's
    state, not the pixels. The ICU regular-expression mode is not ported.
- [x] ~~`winter/crates/winter/src/scrollbar.rs`~~ — Scrollbar with marks
  - Landed as `scrollbar.rs` (toolkit-free geometry and drag state) and the
    pane's right-edge track in `terminal_widget.rs`: the thumb shows where the view sits, a
    press on the track centres the clicked row, a press on a prompt tick brings
    that prompt into view, the thumb drags, and every shell-integration prompt
    is a tick, red when its command failed. The snapshot carries the marks
    (`grid::MarkTick`, paired with the command's exit code by row). **Tests:**
    the thumb/scroll mapping, grab and drag, tick hit-testing, prompt and error
    ticks, and the track's place on the pane, not the pixels.
  - The adapter's main pages now start with `DEFAULT_SCROLLBACK_HEIGHT` rows of
    history; without them the window had no scrollback to show.

### Keybindings & Actions

- [ ] `winter/crates/winter/src/keybindings.rs` — Key chord matching
  - [ ] **Tests:** exact match, partial match, no match, modifier variations
  - [ ] **Property tests:** matching is case-sensitive for letters; modifiers are order-independent; exact key + modifier match has higher priority than modifier-only match
- [ ] `winter/crates/winter/src/actions.rs` — Action definitions
- [ ] `winter/crates/winter/src/action_registry.rs` — Action registry
  - [ ] **Tests:** register/unregister/lookup, dispatch, parameter passing
- [x] ~~`winter/crates/winter/src/notifications.rs`~~ — Desktop notifications
  - Landed as `notifications.rs` over `notify-rust`: a pane enables the
    adapter's `DesktopNotification` feature, `pump_output` drains OSC 777 into
    the window's queue, and `App::logic` shows each as a toast. A blank title
    falls back to the application name, and a platform with no notification
    service drops the call. On Windows an unpackaged binary borrows PowerShell's
    application id, which is where the toast's attribution comes from.
    **Tests:** the message normalisation, not the OS call.
- [ ] `winter/crates/winter/src/remoting.rs` — WebSocket terminal

### Terminal Control

- [x] `winter/crates/winter/src/term_control.rs` — High-level terminal control
  - Landed as `Control`, upstream's `ControlCore`: the terminal, its connection,
    the bell, the title and the exit, with no egui type in a signature. `Pane`
    is the widget over it and keeps the viewport and the selection (ADR 019).
    **Tests:** a pump feeds the parser and the snapshot shows it, a bell is one
    event per flash and a new one after it, a notification carries its title
    and body, an exit is reported once, typed text and a paste reach the
    connection, a resize reaches both the terminal and the connection, the
    title is kept until the program sets another, a response goes back to the
    program, and the cell count is clamped (ADR 036).
- [x] `winter/crates/winter/src/term_connection.rs` — Terminal-backend connection
  - Upstream's `ITerminalConnection` as a trait: drain, write, resize, `exited`
    and close. `session::Session` is the pty implementation; the control takes
    a boxed trait object, so a test drives a terminal from a byte buffer
    (ADR 036). `exited` is new because a ConPTY keeps its output pipe open
    after the client exits, which is why `server::Pty` gained `try_wait`.
- [x] `winter/crates/winter/src/term_settings.rs` — Per-terminal settings
  - `TerminalSettings` resolves the command, the tab's title and the scheme when
    a tab opens, as upstream's `TerminalSettings` is resolved from a profile.
    The command-and-title rules moved here from the shell (ADR 036). **Tests:**
    the default profile's command and title, a named profile, a profile title
    overriding the program name, the scheme, and a program with no stem.
- [x] `winter/crates/winter/src/term_events.rs` — Event callbacks
  - `TerminalEvent::{Bell, Notification, Exited}`, drained once a frame.
    Upstream raises the title too; this shell reads it every frame instead, so
    there is no event for it (ADR 036). An exit closes the tab and the last tab
    closes the window, which is upstream's `closeOnExit`; the bell asks for
    attention and rings the console host's tone.

---

## Phase 10: Settings, Serialization & Configuration

**Estimated effort:** 1.5 weeks (0.5 wks for tests)

Started: `crates/settings` owns the file, its path and the normalization that
makes a partial file usable (ADR 022). The schema is a named-profile list and
`default_profile`, and the application starts the default profile's command. The
window's chords are settings too (ADR 023): `crates/settings/src/keybindings.rs`
parses upstream's chord syntax into a typed `Chord`, `actions.rs` names the
commands, and the user's list is merged over `DEFAULT_KEYBINDINGS`. Colour
colour
schemes (ADR 024) live in `palette.rs`: a `Scheme` reads specs with the ported
`ColorFromXParseColorSpec` and writes the adapter's `ColorTable` when a pane is
created. The settings window (Ctrl+, or the gear in the tab strip) shows the
path, the parse error if there is one, the profile, the chord count, the scheme
and the font size, which is what makes a malformed file visible. A saved file
is polled once a second and reloaded (ADR 025); editing the file happens in an
editor, and migration follows.

### Settings Model

- [x] `winter/crates/settings/src/schema.rs` — TOML schema
  - The root lives in `lib.rs`: `version`, `profiles`, `default_profile`,
    `keybindings`, `schemes`, `default_scheme` and `font`, with
    `Profile { name, command, args, title }` in `profiles.rs`. **Tests:** the
    defaults, an empty document, unknown fields and the version gate.
- [x] `winter/crates/settings/src/parser.rs` — TOML parser
  - TOML only; YAML is not read. **Tests:** a valid file, a missing file, a
    broken file, a newer version, and the platform path.
  - Also `load`, `load_with_report`, `to_toml` and `settings_path`; the
    application never writes the file.
- [x] `winter/crates/settings/src/profiles.rs` — Profile list
  - One list with a named default. **Tests:** the default profile names the
    platform shell; an unknown or absent default falls back to the first
    profile; a blank command is filled in.
- [x] `winter/crates/settings/src/palette.rs` — Color palettes
  - `Color` reads the ported xparse grammar (`#RGB` … `#RRGGBB`, `rgb:r/g/b`);
    `Scheme` names the twenty colours upstream's scheme names and `apply`
    writes them to the table, including the reserved default slots.
    **Tests:** spec grammar, canonical hex output, only-named colours written,
    alias moves, both boundary entries in ANSI order (ADR 024).
- [x] `winter/crates/settings/src/fonts.rs` — Font config
  - The object holds `size`, clamped to 4–100 points; the egui shell measures
    its cell from it. **Tests:** the default, a file's size, the clamp and a
    non-numeric size rejecting the file.
  - [ ] Face, weight and feature parsing when a font loader exists; egui's
    stack is one embedded monospace family today (ADR 022's consumer rule).
- [x] `winter/crates/settings/src/keybindings.rs` — Keybinding definitions
  - `Chord` parses `[ctrl+][alt+][shift+]<key>` as upstream's
    `KeyChordSerialization.cpp` does; `Key` is the vocabulary an egui event
    reports; `resolve` merges the user's list over `DEFAULT_KEYBINDINGS`, where
    the same chord replaces a default and `unbound` removes one. **Tests:**
    order- and case-insensitivity, a second key rejected, exact modifiers,
    round-trip of every default, replace/add/unbind/duplicate rules.
- [x] `winter/crates/settings/src/actions.rs` — Action definitions
  - Flat command names (`newTab`, `splitHorizontal`, `focusLeft`, …,
    `unbound`); the application maps them to window actions.
    **Tests:** the schema-driven cases in `lib.rs` and `app.rs`.
- [x] `winter/crates/settings/src/themes.rs` — Theme definitions
  - Upstream's `Theme` narrowed to the four fills this shell draws — the tab
    row and a tab, focused and not — because the rest of `Theme.idl` (Mica, a
    rainbow frame, the frame and settings-page themes, the close button and the
    icon style) names WinUI with no counterpart here (ADR 035). A one-colour
    theme uses it unfocused too; `theme` names one, and an unknown name keeps
    the shell's colours with a warning. The tab strip paints it and the window
    edits it. **Tests:** the file's nesting, a missing section, one colour
    serving both states, the unfocused colour when named, a broken colour
    rejected, a nameless theme dropped and an unknown one warned, the draft
    round trip and its bad-colour message, and the window's rows and editor.
- [x] `winter/crates/settings/src/sgr_aliases.rs` — SGR alias table
  - `SchemeColor` names a scheme's twenty colours, the palette slot each fills
    and the two that move a `ColorAlias`; `Scheme`'s getter and setter, `apply`
    and the editor's field list all read it, so the four copies of the list
    ADR 032 left are one table (ADR 034). **Tests:** unique names and slots,
    the sixteen ANSI colours in slot order, the two aliased colours and no
    others, a round trip through the table, the file's own names reading back,
    and the table matching exactly what a full scheme serialises.
- [x] `winter/crates/settings/src/defaults.rs` — Defaults
  - The base layer in one place: `Settings::default` is it, and it is the
    document `settings::ensure_file` writes on a first start, so the window's
    path always exists (ADR 034). **Tests:** the written defaults read back
    with no warnings, the default profile exists, the file is written once and
    never over an existing one, and an earlier `settings.json` is not shadowed
    by it.

### Migration & Compatibility

- [x] `winter/crates/settings/src/migration.rs` — Version migration
  - Version 0 is ADR 022's `settings.json`: a profile list and a default, nothing
    else. When `settings.toml` is absent it is read, normalized like any other
    document, written out as TOML so the migration happens once, and left in
    place (ADR 034). There is no reverse migration: the old file is never
    deleted, a document without a version already reads as the current one,
    and deleting the TOML falls back to the JSON again. **Tests:** the fields
    the old file held, the settings it never held, an empty profile list, a
    blank command and an unknown default with their warnings, a TOML file
    winning over the old one, a broken old file reported and not overwritten,
    the earlier path, and a property test that whatever the old file says, the
    migration is a fixed point.
- [x] `winter/crates/settings/src/validation.rs` — Settings validation
  - `Warning` names every guess normalization makes: a nameless profile or
    command, an unknown `default_profile` or `default_scheme`, dropped or
    repeated scheme names, repeated profile names, a clamped font size, a
    chord bound twice and an `unbound` that matched nothing. `parse_with_warnings`
    returns them, `Loaded` carries them, and the window lists them (ADR 027).
    **Tests:** a clean document warns about nothing; one odd file produces each
    warning; each rule's warning fails when it is removed.
- [x] `winter/crates/settings/src/watch.rs` — Settings file watching
  - The watcher is a poll: `Stamp { modified, len }`, compared once a second;
    a change reloads chords, font, scheme and the next tab's profile, and a
    save that does not parse leaves the running session alone (ADR 025).
    **Tests:** a missing file, a changed file, a deleted file.

---

## Phase 11: Audio & Miscellaneous

**Estimated effort:** 1 week

- [x] `winter/crates/audio/src/midi.rs` — MIDI audio
  - The DECPS arithmetic from `MidiAudio.cpp`: a note to its frequency, the
    rate DirectSound was asked for, the General MIDI 2 gain law, and a note to
    a tone. There is no device here; the host plays the tones (ADR 033).
    **Tests:** A4 at 440 Hz, C5, the octave, the wave rate, the gain law at
    full, half and zero velocity, a silent note that keeps its duration, and a
    sequence that keeps its order.
- [x] `winter/crates/audio/src/beep.rs` — Terminal beep
  - BEL is the console host's `Beep(800, 200)` (`ConsoleWindow.cpp`), described
    rather than played. **Tests:** the tone's frequency and duration.
- [x] `winter/crates/winter/src/visual_bell.rs` — Visual bell animation
  - Landed in the app, not the crate: it is a flash, not a sound. `VisualBell`
    is the light `BellLightOn` switches on: it fades to nothing over
    `TerminalWarningBellInterval`, ignores a bell that arrives while it runs,
    and says when the next frame is due (ADR 006). The pane paints it over the
    text as the last thing it draws. **Tests:** the fade, the ignored second
    bell, the repaint schedule, and a bell after the end.
- [x] The sink — `winter/crates/winter/src/sound.rs`
  - A thread plays a whole DECPS sequence with `Beep`, note after note, as
    `MidiAudio::PlayNote` blocks; a failed call still waits out the note so the
    rhythm survives. Windows only: other targets drop the tone (ADR 033). The
    bell is one tone per flash, not per BEL, so a program that repeats it gets
    one beep.

---

## Phase 12: Comprehensive Testing & Validation

**Estimated effort:** an audit, not a build — see below.

This section was written as a checklist of filenames and counts ("50+ unit tests
for `Point`", one test file per crate). The suite that exists is inline in the
module that owns each rule, 1,678 tests across twelve crates, and the status
table above is its breakdown. What a coverage review needs is not a count but an
answer to "what failure would this layer catch that the one below cannot" — the
question the `winter-testing` skill asks. The audit kept the invariants the
corpus could not enumerate and retired the rows that would have added no layer.

### Added: the invariants the corpus could not enumerate

- [x] `crates/term/tests/conformance/generate.rs` — the parser never panics.
  Two properties over generated input: any bytes at all leave the screen a
  rectangle with the cursor inside it and the same bytes twice give the same
  screen; and a sequence cut off by the end of the input does not swallow what
  follows. This is the iron rule the corpus cannot enumerate: truncated UTF-8,
  absurd parameter counts, a DCS string that never ends.
- [x] `crates/adapter/src/sixel_parser.rs` — any sixel stream decodes without a
  panic, and an image that comes out is a rectangle (`width <= stride`, whole
  rows of pixels).
- [x] `crates/utils/src/rle.rs` — any value sequence round-trips, the run count
  is the number of transitions, `at` agrees with the sequence, and `compact`
  leaves an encoded sequence alone.
- [x] `crates/utils/src/generational.rs` — a token is valid exactly while no
  write follows it, and the value it guards is the last one written.

### Fidelity: oracle cases not yet ported

The port is method-complete — `AdaptDispatch` implements every virtual method in
upstream's `ITermDispatch.hpp`, and no handler in it falls back to the trait's
`unhandled` default (ADR 004) — but fidelity is measured by the oracle's own
cases, and these have no Rust counterpart yet. The counts are the vendored files
under `.design/refs/terminal/`.

| Upstream file | Lines | Ours today |
|---|---|---|
| `terminal/adapter/ut_adapter/adapterTest.cpp` | 4039 | spread over the adapter's unit modules |
| `terminal/adapter/ut_adapter/inputTest.cpp` | 866 | 3 conformance cases plus the `TerminalInput` unit tests |
| `buffer/out/ut_textbuffer/ReflowTests.cpp` | 726 | unit and property tests |
| `terminal/adapter/ut_adapter/MouseInputTest.cpp` | 621 | 7 conformance cases |
| `terminal/adapter/ut_adapter/kittyKeyboardProtocol.cpp` | 373 | 5 conformance cases |
| `buffer/out/ut_textbuffer/TextAttributeTests.cpp`, `TextColorTests.cpp` | 327 + 201 | 10 colour and attribute tests |

Two surfaces are absent rather than untested:

- `TextBuffer::GetWithControlSequences`, which serialises a row back to text and
  SGR. No shell consumes it yet.
- Painting a DRCS glyph. `adapter::font_buffer` validates DECDLD, parses the
  sixel payload, and packs and centres the glyphs; the paint path belongs to
  `renderer`, which does not read them.

Take the list smallest-first: a pure function's cases cost an afternoon and find
real bugs, while `adapterTest.cpp` is a family at a time, each family its own
commit. When a case is ported, delete its row.

`UTextAdapterTests.cpp` is not on the list: `find_text_matches_the_upstream_unicode_sample`
already ports its only case. The audit that wrote this table missed it because
the Rust case is named after what it checks, not after the C++ class — and the
pass that went looking for it found a real bug underneath, in
`Row::replace_characters`.

### Retired rows

**A. Per-crate `src/*_tests.rs` files.** Every crate's tests are inline in the
module that owns the rule, as the porting skill requires; moving them into
`src/unit_tests/` would restructure the suite without adding an assertion, and
the counts in this section were targets rather than cases. Three rows name
crates that do not exist: `crates/input` (the encoders live in `term`, ADR 017),
`crates/audio/src/visual_bell.rs` (the flash is `winter::visual_bell`), and
`server`'s `wait_queue` and `process_list` (the driver protocol is deliberately
not ported, ADR 015).

**B. The rest of the property tests.** `utils`, `types` and `buffer` already
carry proptest modules — point, size, rect, color, enum_set, hash, small_vec,
bytes and bit; character_attrs, codepoint_width and text_attribute; cell_width,
cursor, hyperlinks, marks, the two cell iterators, row and text_buffer — plus
`settings`' migration. What was genuinely missing is added above. The remaining
row, `spsc`, has a thread schedule as its property, which its own tests cover.

**C and E. PNG snapshots (`insta`).** Not ported: a golden image pins the
toolkit's rasteriser rather than this code, and re-baselining one is the
"golden re-baseline" the testing skill forbids. The renderer's grid paint is
tested by its own snapshot module; the shell paints through egui.

**D. The conformance corpus** is done, with `generate.rs` added above as its
last row.

**F. Criterion benchmarks.** Not ported: `criterion` is not a dependency, there
is no performance budget to regress against, and a benchmark outside the gate
does not catch a failure. `just test-long` already covers the host-font cases.

**G. Fuzzing (honggfuzz / afl.rs).** Not ported: both need a nightly toolchain
and a separate target, and the invariant they would chase — "the parser never
panics" — is the proptest in `generate.rs`, which runs in the gate on all three
platforms instead of only where the fuzzer is installed.

**H. Platform window, clipboard and DPI tests.** Not ported: they would test
winit, arboard and the window manager, not this code. The pty layer has its own
tests (`session`, and `server`'s two backends), and the clipboard paths are
exercised through `host`'s selection tests.

**I. End-to-end application tests.** Not ported: they need a window and a
desktop, which the gate does not have. The evidence is the live check each
user-visible change carries — a probe that launches the app, drives it and reads
the screen — recorded in the commit.

**J. Cross-platform matrix.** Redundant: every portable crate's tests already
run on ubuntu (x86-64 and arm64), windows and macos in the same gate
(`.github/workflows/ci.yml`), so "the same sequences give the same buffer on
every platform" is the gate.

---

## Phase 13: Polish & Distribution

**Estimated effort:** 2 weeks

- [ ] `winter/crates/winter/src/args.rs` — CLI argument parsing with `clap`
  - [ ] **Tests:** all arguments, help text, error messages, long/short forms, combined flags
- [ ] `winter/crates/winter/src/logging.rs` — Structured logging with `tracing`
- [ ] `winter/crates/winter/src/singleton.rs` — Single-instance enforcement
- [ ] `winter/crates/winter/src/upgrade.rs` — Auto-update mechanism
- [ ] `winter/crates/winter/src/windows` — Windows installer (MSI/NSIS)
- [ ] `winter/crates/winter/src/linux` — Linux packaging (AppImage, Flatpak, deb)
- [ ] `winter/crates/winter/src/macos` — macOS packaging (dmg, notarization)
- [ ] `winter/crates/winter/src/docs` — Documentation and user guide

---

## Phase 14: CI/CD & Test Automation

**Estimated effort:** 1 week

- [ ] `.github/workflows/unit-tests.yml` — Unit tests on Linux/macOS/Windows (all crates)
- [ ] `.github/workflows/integration-tests.yml` — Integration tests (PTY, VT input)
- [ ] `.github/workflows/conformance-tests.yml` — VT conformance suite
- [ ] `.github/workflows/snapshot-tests.yml` — Snapshot comparison with tolerance
- [ ] `.github/workflows/benchmarks.yml` — Benchmark regression detection
- [ ] `.github/workflows/fuzzing.yml` — Nightly fuzzing (honggfuzz)
- [ ] `.github/workflows/xplat-matrix.yml` — Cross-platform matrix test
- [ ] `.github/workflows/e2e-tests.yml` — End-to-end application tests
- [ ] `winter/tests/CI-README.md` — How to run each test suite locally

---

## Dependency Map (Rust Ecosystem)

| Purpose | Crate(s) |
|---------|----------|
| Serialization | `serde` + `toml` |
| Async runtime | `tokio` |
| Cross-platform window | `winit` |
| Immediate-mode GUI | `egui` + `egui-winit` + `eframe` |
| GPU rendering | `wgpu` + `wgpu-types` |
| Font loading | `fontdb` + `skrifa` + `swash` |
| Text shaping | `text-shaping` (harfbuzz) |
| Unicode | `unicode-general-category` + `unicode-ccc` + `unicode-bidi` |
| Hashing | `ahash` |
| Memory pool | `bumpalo` |
| Small string | `compact_str` |
| Color | `rgb` |
| Image loading | `image` (Sixel support) |
| File watching | `notify` |
| Clipboard | `arboard` |
| Logging | `tracing` + `tracing-subscriber` |
| Config | `config` (layered) |
| CLI args | `clap` |
| **Testing: property-based** | `proptest` |
| **Testing: snapshots** | `insta` |
| **Testing: benchmarks** | `criterion` |
| **Testing: fuzzing** | `honggfuzz` + `afl.rs` |
| **Testing: mock** | `mockall` |
| **Testing: assertion** | `pretty_assertions` |
| JSON schema | `schemars` |
| Base64 | `base64` |

---

## Milestone Summary

The table at the top of this document is the live status; this is the same
state at a glance, with the owning crate's measured test count.

| Phase | State | Where it stands |
|-------|-------|-----------------|
| **1. Foundation** | ✅ DONE | `utils` (213 + 1 ignored), `types` (121), `buffer` (223) |
| **2. Text Buffer** | ✅ DONE | Scrollback, reflow, search, marks and image bands (ADR 005) |
| **3. VT Parser** | ✅ DONE | The full DFA, plus 107 conformance cases in `term` |
| **4. Adapter** | ✅ DONE | 190 tests; the `ITermDispatch` families, paging, sixel, macros (ADR 004) |
| **5. Renderer** | ✅ DONE | 132 + 1 ignored; the software backend paints, and the WinUI shell paints on the GPU (ADR 063) |
| **6. Console Host** | ⚠️ PORTABLE | 258 tests; `directio` waits for the server's API surface (ADR 013) |
| **7. Server** | ✅ DONE | 17 + 1 ignored tests; shim policy, both pty backends, a headless OpenConsole host, and `just dist` packaging it beside the binary (ADRs 015, 016, 055) |
| **8. Input/UI** | ✅ DONE | Chords, encoders and focus in `term` (ADR 017); the egui shell (ADR 018) and WinUI 3 on Windows (ADRs 040–064) |
| **9. Terminal App** | ✅ DONE | Tabs, panes, palette, find bar, scrollbar, notifications (ADRs 019–021, 036, 038) |
| **10. Settings** | ✅ DONE | 83 tests; TOML, profiles, chords, schemes, themes, editor (ADRs 022–032, 034, 035) |
| **11. Audio** | ✅ DONE | 7 tests; bell and DECPS tones (ADR 033) |
| **12. Testing** | ⏳ SWEEP | The corpus, property and long-running cases grow with each port |
| **13. Polish** | ⏳ NOT STARTED | CLI arguments, logging, packaging |
| **14. CI/CD** | ✅ DONE | One workflow (`.github/workflows/ci.yml`): the gate and a `Full tests` job (`just test-long`, the `#[ignore]`d cases in release mode) on x86-64 and arm64 Linux, Windows and an Intel macOS host (`macos-15-intel`); a `v*` tag packages a versioned archive per host and publishes it (`just package`) |
| **15. Native Linux shell** | 💤 PARKED | ADR 062 parks the GTK 4 shell: Linux ships the egui shell. `crates/winter/src/linux/` runs a window whose title bar is a tab strip it paints, one pty per tab, the shared input rules and wheel scrollback, all behind an opt-in `gtk` feature; the find bar, the settings window and the rest of the chrome were never built. Its measurements and Wayland traps stay in [`linux-shell-scope.md`](linux-shell-scope.md) |
| **Post-1.0 Chaos layer** | 💤 PARKED | Optional "Shitty Winter" edition: Slag-driven `bugs` instead of extensions, behind a narrow mischief surface. See below |

## Post-1.0 milestone: the chaos layer

**State:** parked. Nothing in this section blocks the 1.0 release.

**The idea.** After Winter is a dependable terminal on Windows, Linux and macOS,
ship an optional "Shitty Winter" edition: a complete chaos machine driven by
[Slag](https://github.com/sdgoij/slag). Instead of extensions, a user installs
**bugs** — small scripts that perturb input, timing, rendering, the cursor, the
scrollback, colours or the bell in the most profound ways the hook surface
allows. The joke is deliberate; the point is a safe sandbox for the terminal's
own behaviour.

**The boundary.** Winter's core stays deterministic, testable and free of the
chaos layer. `winter` exposes a narrow mischief surface — hooks around input,
timing, rendering, cursor, scrolling, colours and the bell — and a separate
`winter-chaos` crate (or a feature-gated module) consumes it. The dependency
ring still points one way: core never depends on chaos. An ADR records the
surface before the first hook lands.

**A bug is an artifact.** It is a Slag script plus a manifest that declares the
target, intensity, seed and cleanup. A run is reproducible from its seed, and
the application writes a replay log, so a chaotic session is art that can be
inspected, not just breakage.

**Install surface.** `winter bugs install <name>`, `winter bugs list`,
`winter bugs enable|disable <name>`. Bugs live in the user's config directory,
are enabled per profile, and none are enabled by default.

**Safety rails.** Per-profile opt-in, a one-key kill switch, no access to pty
data, credentials or the filesystem beyond the bug's own directory, panic-free
by construction, and a replay log. A bug can make the terminal unpleasant; it
must not make it unsafe.

**Prerequisites.** Winter 1.0 has shipped on all supported hosts, the shell and
input surfaces have settled, and the mischief ADR is accepted. Until then this
stays a design note, not a work item.

## Test coverage

Coverage is a review property, not a percentage (see `CONTRIBUTING.md` §Test layers).
The live per-crate counts are in the Status table at the top. A target that no
case enforces is not a target.

## High-Risk Areas & Mitigation

| Risk | Impact | Mitigation |
|------|--------|------------|
| **Text reflow algorithm** | Core functionality | Property tests: character preservation, width constraints; snapshot tests at several widths |
| **Glyph atlas management** | Performance + correctness | Property tests: non-overlapping packed rects; snapshot tests per backend |
| **VT escape sequence conformance** | Compatibility | Cases from ECMA-48, DEC, xterm and upstream's `ut_*`; `generate.rs` feeds arbitrary bytes and asserts survival |
| **IME/Composition input** | Non-Latin input quality | Windows IME through the host; egui's text input on Linux and macOS |
| **Clipboard formatting** | User expectations | Round-trip tests for plain, HTML and RTF through the buffer's copy path |
| **DPI awareness** | Multi-monitor quality | The WinUI pane fits its composition scale (ADR 043); egui's scale factor covers the other shells |
| **Sixel/image rendering** | Advanced features | Conformance cases with known sixel images; band layout in the buffer (ADR 005) |
| **Cross-platform windowing** | Platform parity | The two shells share `input`, `view`, `tabs` and `term_control`; CI runs the gate on every supported host |

## Historical proposal

The original plan named a `WindowBackend` trait and a `crates/ui` with Win32,
X11, Wayland and Cocoa backends. ADR 018 replaced it with egui on winit and
wgpu for Linux and macOS, and ADR 040 replaced the Windows shell with WinUI 3.
No `crates/ui` exists; the two shells share the portable crates through
`input`, `view`, `tabs` and `term_control`. The proposal is recorded in the
ADRs, not here.
