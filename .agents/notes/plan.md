# Windows Terminal C++ → Cross-Platform Rust: Porting Plan

**Reference project:** Microsoft's Windows Terminal (`refs/terminal/`)
**Goal:** Port the core terminal emulator to a cross-platform Rust codebase targeting Windows, Linux (Wayland/X11), and macOS.

## Status (2026-11-22)

Verified by running the gate, not by reading the previous status:

| Crate | Lines | Tests | State |
|---|---|---|---|
| `utils` | ~4.6k | 207 + 1 ignored | Complete for Phase 1; `PointSpan` joins the geometry primitives |
| `types` | ~3.7k | 110 | Complete for Phase 1; owns `TextAttribute`, the VT vocabulary, `VTID`, `VTParameters` (with sub-parameters), `LineRendition`, the charset tables, `SgrStack`, the colour table, `ColorFix` (Oklab nudging), XParseColor, the X11 colour-name tables, `InputModes`, the console `InputRecord` vocabulary and `FilterStringForPaste` |
| `buffer` | ~6.8k | 201 | Phase 2 — scrollback, `logical_lines` and `resize` (reflow), OSC 8 hyperlink registry, per-row image bands (ADR 005), rectangle scrolling, line rendition, the read-side `TextBufferCellIterator` |
| `vt` | ~2.5k | 46 | Phase 3 — the full DFA ported from `stateMachine.cpp`: parameters, sub-parameters, intermediates, OSC/DCS strings and VT52; drives an `IStateMachineEngine`. No fuzz target; the conformance corpus lives beside the `term` loop |
| `adapter` | ~11.6k | 175 | Phase 4 state and interfaces; print, SGR, cursor, margins, insert/delete, scrolling, paging, erase, rectangular, mode (incl. DECCOLM/DECANM), report, presentation state, line rendition, DECRQSS, DRCS, hyperlink/OSC, colour table (incl. DECAC), macro, window, sixel, kitty, sound, save-restore, tab handlers and the ConEmu/iTerm2/FinalTerm/VS Code/WT/urxvt action strings ported |
| `term` | ~4.1k | 181 | The parser-to-dispatch loop, ported from `OutputStateMachineEngine.cpp`: C0, ESC, CSI, OSC, DCS and VT52 mapping; byte-to-UTF-8 decoding; a `Terminal` façade that applies DECANM, C1, presentation-state and DECRQSS work to the adapter; and `TerminalInput`'s key and mouse encoders from `terminalInput.cpp` and `mouseInput.cpp` (regular forms, kitty protocol, win32-input-mode, focus, the default/UTF-8/SGR mouse encodings, alternate scroll). 105 of the tests are the conformance corpus |
| `renderer` | ~12.4k | 121 + 1 ignored | Phase 5 — font, cell-metric, cluster, cursor, CSS-length, composition, frame-info and render-settings vocabulary; the `RenderData`, `RenderEngine` and `RenderEngineBase` surfaces; the `renderer.cpp` grid-line, hover and composition paint paths; a host-driven `Renderer::paint_frame`; the atlas model, `AtlasEngine`, `IBackend`, a software backend and a `fontdue` rasteriser; and the `FontCache.h` collection with its SFNT face reader (ADR 008). A GPU backend is not written |
| `host` | ~6.3k | 182 | Phase 6 under way: `CommandHistory`, `InputBuffer`, `CookedReadData` (with alias expansion wired in), `InputStateMachineEngine`, the portable `ConsoleInformation` and `ScreenInformation` (ADR 012), `Settings`, `AliasTable`, `InputKeyInfo`, and `Scrolling` |

`just gate` is green: `cargo fmt --all --check`, `clippy -D warnings`, `cargo-deny`,
1235 tests, doc tests. Phases 1–5 are complete. Phase 6 is under way and Phases
7–14 are not started.

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

All library crates live under `crates/`. The main terminal application is `crates/winterm`.

```
winterm/
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
│   └── winterm/            (main terminal app — GUI, tabs, panes, command palette)
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
| `src/buffer/` | `winterm/crates/buffer` | Text buffer, rows, cells, scrollback, cursors |
| `src/terminal/parser/` | `winterm/crates/vt` | VT escape sequence state machine |
| `src/terminal/adapter/` | `winterm/crates/adapter` | VT input/output dispatch, terminal-adapter protocol |
| `src/terminal/input/` | `winterm/crates/input` | Mouse input, keyboard input processing |
| `src/host/` | `winterm/crates/host` | Console host state, screen buffers, I/O |
| `src/server/` | `winterm/crates/server` | ConDrv device communication, process list, wait queues |
| `src/renderer/` | `winterm/crates/renderer` | Render engine abstraction, glyph atlas |
| `src/interactivity/` | `winterm/crates/ui` | Window metrics, interactivity (platform-dependent) |
| `src/audio/` | `winterm/crates/audio` | Beep/midi audio (optional) |
| `src/types/` | `winterm/crates/types` | Color, viewport, accessibility types |
| `src/cascadia/` | `winterm/crates/winterm` | High-level terminal app (platform-dependent) |
| `src/til/` + `src/inc/til/` | `winterm/crates/utils` | Utility library (reusable in Rust) |

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

- [ ] `winterm/Cargo.toml` — workspace root with `[workspace] members = ["crates/*", "tests", "benches"]` and test dependencies (`proptest`, `criterion`, `insta`, `honggfuzz`)
- [ ] `winterm/tests/test_helpers.rs` — shared test fixtures, mock backends, VT sequence builders
- [ ] `winterm/tests/mock_window.rs` — Mock `WindowBackend` for renderer/UI tests
- [ ] `winterm/tests/mock_pty.rs` — Mock PTY process for server/host tests
- [ ] `winterm/tests/utf8_corpus.rs` — Embedded UTF-8 text corpora for reflow/iterator tests
- [ ] `winterm/tests/vt_reference/` — Reference VT sequences from xterm, Kitty, iTerm2
- [ ] `winterm/tests/render_snapshots/` — Baseline render output images per backend

> **Note:** All paths below use `winterm/` as workspace root. Library crates are at `winterm/crates/<name>/src/`, unit tests are `winterm/crates/<name>/src/unit_tests/`, shared tests at `winterm/tests/`, benchmarks at `winterm/benches/`, and fuzzers at `winterm/fuzz/`. The main app is `winterm/crates/winterm/`.

---

## Phase 1: Foundation — Utility Library & Types

**Estimated effort:** 2.5 weeks (0.5 wks for tests)

### Geometry & Primitive Types

- [x] `winterm/crates/utils/src/point.rs` — `til::point` with checked arithmetic, comparison, operators
  - [x] **Tests:** 15 unit tests — equality, comparison, addition/subtraction overflow, multiplication overflow, division by zero, `operator bool()` for negative coords, Win32/WinRT conversion roundtrips, cross-platform `RawWindowHandle` conversion
  - [x] **Property tests:** `(a + b) - b == a` when no overflow, `point + 0 == point`, `point.x > 0 && point.y > 0` iff `bool(point)`
- [x] `winterm/crates/utils/src/size.rs` — `til::size` with checked arithmetic, scale, area
  - [x] **Tests:** 11 unit tests — creation with zero/negative dimensions, overflow arithmetic, `scale()` with float, `divide_ceil()` edge cases, area overflow for large sizes
  - [x] **Property tests:** `(a.scale(s)).scale(1.0/s) == a` (within tolerance, bounded input), `size.width * size.height == size.area()` (within overflow bounds)
- [x] `winterm/crates/utils/src/rect.rs` — `til::rect` (exclusive) with union/intersection/difference iterators
  - [x] **Tests:** 13 unit tests — creation, empty rect, contains check, union of overlapping/disjoint rects, intersection with full overlap/partial overlap/no overlap, difference producing up to 4 rects, iterator bounds correctness, `to_origin()`, `to_inclusive_rect()`, Win32 `RECT` conversion roundtrips
  - [x] **Property tests:** `rect | other` always contains both; `rect & other` is always subset of both; `rect - (rect & other)` produces rects that union back to the difference
  - [x] **Snapshot tests:** render difference region for large rect with known input (rect.rs snapshot_tests)
- [x] `winterm/crates/utils/src/inclusive_rect.rs` — `til::inclusive_rect` (inclusive coordinates)
  - [x] **Tests:** 6 unit tests — creation, overlap, contains, Win32 `SMALL_RECT` conversion
- [x] `winterm/crates/utils/src/color.rs` — `til::color` RGBA
  - [x] **Tests:** 10 unit tests — RGB construction, ABGR conversion, COLORREF, alpha blending, from_hex, to_css_string, equality
  - [x] **Property tests:** `color.to_hex().to_rgba() == color`, alpha blend is commutative
- [x] `winterm/crates/utils/src/small_vec.rs` — `til::small_vector<T, N>` stack-optimized vector
  - [x] **Tests:** 10 unit tests — push/pop/insert/remove, capacity growth, iterator correctness, zero-initialize, drop order, `From<Vec<T>>` conversion
  - [x] **Property tests:** `small_vec.as_slice().to_vec() == small_vec.into_iter().collect::<Vec<_>>()`, push_pop roundtrip
- [x] `winterm/crates/utils/src/enum_set.rs` — `til::enumset<Enum>` bitmask
  - [x] **Tests:** 11 unit tests — insert/remove/contains, union/intersection/difference/symmetric difference, is_subset/is_superset, iteration order, bitflag serialization
  - [x] **Property tests:** `(a | b) & a == a`, `(a & b) == (b & a)`, `a | Empty == a`
- [x] `winterm/crates/utils/src/hash.rs` — `til::hasher` (wyhash-based)
  - [x] **Tests:** 7 unit tests — hash consistency (same input → same hash), hash of empty input, hash of various string/byte/struct inputs, incremental write bounds check, std hash divergence
  - [x] **Property tests:** identical values produce identical hashes; `std::hash::<T>(v) != til::hash(v)` for at least some types
  - [ ] **Property tests:** `different_data → different_hash` (ignored: extremely unlikely but possible 64-bit collision)
- [x] `winterm/crates/utils/src/mutex.rs` — Cross-platform mutex
  - [x] **Tests:** 5 unit tests — lock/unlock, re-entrant, poisoned state, drop restores lock
- [x] `winterm/crates/utils/src/atomic.rs` — Atomic wait/notify
  - [x] **Tests:** 2 unit tests — wait/notify roundtrip, timeout, spurious wakeup handling
- [x] `winterm/crates/utils/src/ticket_lock.rs` — Ticket lock + recursive ticket lock
  - [x] **Tests:** 3 unit tests — mutual exclusion, fairness (FIFO order), recursive lock same-thread, suspend/resume
  - [ ] **Property tests:** between two threads, no two threads ever hold lock simultaneously
- [x] `winterm/crates/utils/src/throttled.rs` — Throttled/debounced function calls
  - [x] **Tests:** 2 unit tests — single call, rapid-fire calls (only last executes), delay elapse → final call, reset
- [x] `winterm/crates/utils/src/io.rs` — Cross-platform file I/O
  - [x] **Tests:** 3 unit tests — read/write file, path resolution, temp file cleanup, non-existent file handling
- [x] `winterm/crates/utils/src/unicode.rs` — Unicode helpers
  - [x] **Tests:** 8 unit tests — is_alphanumeric, is_whitespace, is_grapheme_boundary, is_cjk, surrogate handling
  - [ ] **Property tests:** `is_grapheme_boundary(a, b)` iff `grapheme_break(a, b)` from Unicode spec
- [x] `winterm/crates/utils/src/string.rs` — String utilities
  - [x] **Tests:** 11 unit tests — trim, split, replace_all, truncate_ellipsis, case conversion, starts_with, ends_with, equals insensitive
- [x] `winterm/crates/utils/src/replace.rs` — String replacement
  - [x] **Tests:** 3 unit tests — simple replace, replace_all, replace at index, overlapping patterns
- [x] `winterm/crates/utils/src/math.rs` — Checked arithmetic
  - [x] **Tests:** 10 unit tests — checked_add/sub/mul/div, saturating operations, rounding, clamping
  - [ ] **Property tests:** `checked_add(a, b).is_none()` iff `a + b` would overflow
- [x] `winterm/crates/utils/src/colorbrewer.rs` — Color palettes
  - [x] **Tests:** 4 unit tests — palette generation (pastel1, dark2), color access, named palette availability
- [x] `winterm/crates/utils/src/rle.rs` — Run-length encoding
  - [x] **Tests:** 5 unit tests — encode/decode roundtrip, empty input, all-same input, alternating input, `at()` bounds
  - [ ] **Property tests:** `decode(encode(data)) == data`
- [x] `winterm/crates/utils/src/flat_set.rs` — Compact set
  - [x] **Tests:** 2 unit tests — insert/remove/contains, iteration, deduplication
- [x] `winterm/crates/utils/src/generational.rs` — Generational indices
  - [x] **Tests:** 5 unit tests — create/access/validate, invalidation on update, epoch overflow wrap-around
  - [ ] **Property tests:** valid index stays valid until explicitly invalidated
- [x] `winterm/crates/utils/src/spsc.rs` — Single-producer single-consumer queue
  - [x] **Tests:** 2 unit tests — push/pop, full/empty states, capacity, multi-item throughput
  - [ ] **Property tests:** FIFO order preserved
- [x] `winterm/crates/utils/src/latch.rs` — One-shot synchronization
  - [x] **Tests:** 4 unit tests — count_down/wait, multiple waiters, double count_down, thread-safety
- [x] `winterm/crates/utils/src/bit.rs` — Bit manipulation
  - [x] **Tests:** 9 unit tests — popcount, count_zeros, leading/trailing zeros, bit reversals, byte swaps
  - [x] **Property tests:** `popcount(x) == count_ones(x)` matches stdlib; `reverse_bits(reverse_bits(x)) == x`
- [x] `winterm/crates/utils/src/bytes.rs` — Byte slice helpers
  - [x] **Tests:** 9 unit tests — read_u16/u32/u64 LE/BE, write, find_pattern, contains_sequence
  - [x] **Property tests:** `write/read` roundtrip for all valid byte sequences (bytes.rs proptests)

### Core Color & Character Types

- [x] `winterm/crates/types/src/text_color.rs` — `TextColor` enum
  - [x] **Tests:** all variants, `From<COLORREF>`, display formatting, equality
- [x] `winterm/crates/types/src/cursor_type.rs` — `CursorType`
  - [x] **Tests:** all variants, serialization roundtrip
- [x] `winterm/crates/types/src/character_attrs.rs` — `CharacterAttributes` bitflags
  - [x] **Tests:** flag manipulation, all/none, intersection, string formatting
  - [x] **Property tests:** `a | b` contains both flags; `a & a == a` (idempotent) (character_attrs.rs proptests)
- [x] `winterm/crates/types/src/underline_style.rs` — `UnderlineStyle`
- [x] `winterm/crates/types/src/mark_kind.rs` — `MarkKind`
- [x] `winterm/crates/types/src/viewport.rs` — Screen dimension type
  - [x] **Tests:** creation, clamp, contains point, area, intersection, union,
    the inclusive/exclusive bounds, bounds walking and reading-order comparison
- [x] `winterm/crates/types/src/line_rendition.rs` — `LineRendition` conversion functions
  - [x] **Tests:** `ScreenToBufferLine`, `BufferToScreenLine` for all rendition types, roundtrip
  - [x] **Property tests:** `BufferToScreenLine(ScreenToBufferLine(x)) == x` (line_rendition.rs proptests)
- [x] `winterm/crates/types/src/codepoint_width.rs` — Unicode width detection
  - [x] **Tests:** ASCII, CJK, emoji, combining marks, surrogate pairs
  - [x] **Property tests:** width matches `unicode_width::UnicodeWidthChar::width()` for all test cases (codepoint_width.rs proptests)
- [x] `winterm/crates/types/src/theme_utils.rs` — Light/dark detection
- [x] `winterm/crates/types/src/color_table.rs` — 16-color palette
  - [x] **Tests:** default palette, custom palette, color index lookup, reset

### Text Attribute System

- [x] `winterm/crates/buffer/src/text_attribute.rs` — `TextAttribute`
  - [x] **Tests:** default creation, legacy WORD conversion, RGB construction, attribute getters/setters, `HasIdenticalVisualRepresentationForBlankSpace` edge cases, `IsAnyGridLineEnabled`, `HasAnyVisualAttributes`
  - [x] **Property tests:** setting then unsetting an attribute returns to default; `operator==` is reflexive, symmetric, transitive; `memcmp` equality matches field-by-field equality (text_attribute.rs proptests)
- [x] `winterm/crates/buffer/src/row_attributes.rs` — Per-row attributes
  - [x] **Tests:** dirty flag
- [x] `winterm/crates/buffer/src/dbcs_attribute.rs` — `DbcsAttribute`
- [x] `winterm/crates/buffer/src/text_attribute_behavior.rs` — `TextAttributeBehavior` enum

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
  - [x] **Tests:** create, resize, resize_with_reflow, resize_traditional, `GetRowByOffset()`, `GetCellDataAt()`, `GetTextDataAt()`, `Write()`, `WriteLine()`, `FillRect()`, `ScrollRows()`, `CopyRow()`, cursor navigation, word boundary detection, `GetWordStart/End`, `GetGlyphStart/End`, `MoveToNextGlyph`, `ScreenToBufferPosition`, `BufferToScreenPosition`, `GetPlainText()`, `GetWithControlSequences()`, `GenHTML()`, `GenRTF()`, `GetTextRects()`, `GetTextSpans()`, hyperlink map, `SearchText()`, `GetMarkExtents()`, `ClearMarksInRange()`, mutation ID tracking
  - [x] **Property tests:** writing then reading same cells returns original data; `GetPlainText()` for empty buffer is `""`; reflow preserves total character count
  - [ ] **Snapshot tests:** write known text pattern, compare rendered output
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
    The buffer gains a rectangle-to-text read when the copy path needs it.
- [ ] `crates/buffer/src/clipboard.rs` — Clipboard formatting

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
`[[test]]` in `crates/term/Cargo.toml`; `AGENTS.md` and the `winterm-testing`
skill are the live description. The paths below are the original sketch of the
sequence inventory, kept as a coverage list rather than as file locations.

- [ ] `winterm/tests/vt/csi_sgr.rs` — SGR: all standard escape codes (1/2/3/4/5/7/8/22/23/24/25/27/28/29/30-37/38;2;R;G;B/38;5;n/39/40-47/48;2;R;G;B/48;5;n/90-97/100-107)
- [ ] `winterm/tests/vt/csi_cursor.rs` — CUP, CUD, CUF, CUB, CNL, CPL, CHA, CHT, HVP, DCH, ICH
- [ ] `winterm/tests/vt/csi_scroll.rs` — SU, SD, DL, IL
- [ ] `winterm/tests/vt/csi_erase.rs` — ED (J), EL (K),ECH
- [ ] `winterm/tests/vt/osc.rs` — OSC 0/1/2/4/10-17/52/8/9001 (all parameter variations)
- [ ] `winterm/tests/vt/dcs.rs` — DCS $ q (Sixel): valid/invalid images, resize, crop
- [ ] `winterm/tests/vt/private_modes.rs` — DECSET/DECRST: 1/6/7/20/47/1000/1002/1003/1004/1006/1047/1048/1049
- [ ] `winterm/tests/vt/mouse.rs` — Mouse tracking modes 1000/1002/1006 with all button states
- [ ] `winterm/tests/vt/kitty.rs` — Kitty keyboard protocol: KKP push/pop, query
- [ ] `winterm/tests/vt/bracketed_paste.rs` — Bracketed paste mode activation/deactivation
- [ ] `winterm/tests/vt/win32_input_mode.rs` — Win32 Input Mode (CSI <) encoding
- [ ] `winterm/tests/vt/escape.rs` — ESC $, ESC (, ESC ), ESC B, ESC D, ESC H, ESC M, ESC Z, ESC 7, ESC 8, ESC =, ESC >, ESC #3-8, ESC c
- [ ] `winterm/tests/vt/vt52.rs` — VT52 mode: A-J, F, Y, Z, <, =, >
- [ ] `winterm/tests/vt/sixel.rs` — Sixel: valid/invalid images, color palette, resize, interlace
- [ ] `winterm/tests/vt/hyperlink.rs` — OSC 8 hyperlinks: start/end/metadata/uri parsing

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

- [x] `winterm/crates/adapter/src/dispatch_types.rs` — Command types
  - VTInt, VTParameter, VTParameters, VTSubParameters, VTID types
  - Enums: EraseType, ChangeExtent, TaskbarState, GraphicsOptions (SGR), CursorStyle, ModeParams, ModeResponses, StatusType, LineFeedType, SixelBackground, DrcsCellMatrix, DrcsFontSet, DrcsFontUsage, CharsetSize, MacroDeleteControl, MacroEncoding, ReportFormat, PresentationReportFormat, ColorItem, ColorModel
  - Constants: DECCOLM_SET_COLUMNS, DECCOLM_RESET_COLUMNS, SINGLE_UNDERLINE, UNDERLINE_NONE, OVERLINE_NONE
  - 46 unit tests passing
- [x] `winterm/crates/adapter/src/interact_dispatch.rs` — `IInteractDispatch`
  - Input state machine callback interface
  - InputRecord, InputEventType, ControlKeyState types
- [x] `winterm/crates/adapter/src/term_dispatch.rs` — `ITermDispatch`
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
- [x] `winterm/crates/adapter/src/adapt_dispatch.rs` — Main adapter logic
  - AdaptDispatch struct with mode flags, cursor state, margins, SGR stack
  - CursorState, ChangeOps structs for save/restore and attribute changes
  - ModeFlags bitflags (IRM, DECOM, DECAWM, LNM, BRACKETED_PASTE, DECCKM, DECCOLM, DECSCNM, DECECM)
  - Methods: is_insert_mode(), is_origin_mode(), is_auto_wrap(), is_lnm_mode(), is_bracketed_paste()
  - Margin getters/setters, cursor style management, soft/hard reset
  - 7 unit tests passing
- [x] `winterm/crates/adapter/src/font_buffer.rs` — DECDLD soft fonts
  - `SetEraseControl`, `SetAttributes`, `SetStartChar` validate the DECDLD
    parameters; the matrix-size and 80/132-column rules decide the cell
    size (VT2xx/VT320/VT420/VT340/VT382 detection)
  - `AddSixelData`/`FinalizeSixelData` parse the sixel payload, pack and
    centre the bit patterns, and fill unused characters with the error
    glyph
  - 7 unit tests, including upstream's cell-size table and a bit-pattern
    generation case
- [x] `winterm/crates/adapter/src/macro_buffer.rs` — VT macro support
  - MacroBuffer for storing/retrieving terminal macros (IDs 0-9)
  - MacroEntry with encoding support (Text/HexPair)
  - CRUD operations: define_macro(), invoke_macro(), delete_macro(), delete_all_macros()
  - defined_count() and total_size() query methods
  - 7 unit tests passing
- [x] `winterm/crates/adapter/src/page_manager.rs` — Buffer switching
  - Page, TextBuffer, PageManager for main/alternate screen buffer management
  - Support for up to 6 pages with lazy initialization
  - Methods: get(), active_page(), visible_page(), move_to(), move_relative(), make_active_page_visible()
  - activate_alternate()/deactivate_alternate() for screen buffer switching
  - 7 unit tests including: main→alternate, alternate→main, relative moves, invalid page validation
- [x] `winterm/crates/adapter/src/sixel_parser.rs` — Sixel decoding
  - SixelParser state machine (Normal/Attributes/Color/Repeat) and command
    dispatch for `#`, `!`, `$`, `-`, `+`, `"` and ESC
  - VT340 palette, xterm cube and greys, colour mapping, VT240 fallback
  - Aspect-ratio-aware sixel columns, opaque/transparent background filling
  - DecodedImage handed to the adapter, which paints row bands
  - 9 decoder tests plus an adapter-level paint test
- [x] `winterm/crates/adapter/src/terminal_output.rs` — Character set translation
  - TerminalOutput for GL/GR character set translation management
  - Methods: designate_94_charset(), designate_96_charset(), translate_key(), locking_shift()
  - User preference charset (UPSS), DRCS designation, GR translation enable
  - Soft/hard reset support, state save/restore
  - 5 unit tests passing
- [x] `winterm/crates/adapter/src/charsets.rs` — Character sets
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

- [x] `winterm/crates/renderer/src/render_engine.rs` — `IRenderEngine` trait
  - [x] **Tests:** all virtual methods called correctly, trait object safety
- [x] `winterm/crates/renderer/src/render_data.rs` — `IRenderData`
- [x] `winterm/crates/renderer/src/render_settings.rs` — `RenderSettings`
- [x] `winterm/crates/renderer/src/render_frame_info.rs` — `RenderFrameInfo`, `GridLines`, `GridLineSet`
- [x] `winterm/crates/renderer/src/render_engine_base.rs` — `RenderEngineBase`
- [x] `winterm/crates/renderer/src/cluster.rs` — `Cluster` (per-glyph shaping result)
- [x] `winterm/crates/renderer/src/font_info_base.rs` — `FontInfoBase`
- [x] `winterm/crates/renderer/src/font_info.rs` — `FontInfo`
- [x] `winterm/crates/renderer/src/font_info_desired.rs` — `FontInfoDesired`
- [x] `winterm/crates/renderer/src/font_cache.rs` — the font collection from
  `FontCache.h`: an ordered directory scan, nearby `.ttf` precedence, an SFNT
  `name`/`post`/`OS/2`/`head` reader and a process-wide cache (ADR 008)
  - [x] **Tests:** family, style, weight, italic and fixed-pitch reads, `ttcf`
    faces, truncation and out-of-range offsets, nearby precedence, style
    resolution, default-monospace choice, first-build caching, arbitrary bytes
    never panic
  - [x] **Host check:** `a_host_monospace_face_loads_with_fontdue` is ignored by
    default and opens a real fixed-pitch face when run with `--ignored`
- `winterm/crates/renderer/src/font_resource.rs` — **not ported**: upstream's
  `FontResource.cpp` builds a GDI `.FNT` in memory for DRCS soft fonts and
  registers it with `AddFontMemResourceEx`. The DRCS glyphs are ported in
  `adapter` (DECDLD); the GDI registration has no cross-platform equivalent and
  no renderer consumer.
- [x] `winterm/crates/renderer/src/css_length.rs` — CSS length parsing
  - [x] **Tests:** pixel, point, percentage, `ch` and bare-number parsing, validation
- [x] `winterm/crates/renderer/src/cursor_options.rs` — `CursorOptions`
- [x] `winterm/crates/renderer/src/composition.rs` — IME composition state

### Glyph Atlas Engine

- [ ] `winterm/crates/renderer/src/atlas/engine.rs` — `AtlasEngine`
  - [ ] **Tests:** font update, text shaping, glyph mapping, atlas creation, dirty region computation, scroll handling
- [ ] `winterm/crates/renderer/src/atlas/backend.rs` — `IBackend` trait
- [ ] `winterm/crates/renderer/src/atlas/backend_wgpu.rs` — `WgpuBackend`
  - [ ] **Tests:** adapter creation, swapchain creation, texture upload, render pass, present
  - [ ] **Snapshot tests:** render known text to PNG, compare against baseline
- [ ] `winterm/crates/renderer/src/atlas/backend_skia.rs` — `SkiaBackend`
  - [ ] **Tests:** surface creation, font rendering, texture upload
  - [ ] **Snapshot tests:** render known text to PNG, compare against baseline
- [ ] `winterm/crates/renderer/src/atlas/text_analysis.rs` — Text shaping
  - [ ] **Tests:** grapheme clustering, word boundary, script detection, bidirectional text
  - [ ] **Property tests:** grapheme cluster boundaries match Unicode TR29; word boundaries match Unicode TR29
- [ ] `winterm/crates/renderer/src/atlas/builtin_glyphs.rs` — Builtin glyphs
- [ ] `winterm/crates/renderer/src/atlas/text_shaping.rs` — Harfbuzz shaping
  - [ ] **Tests:** basic shaping, ligatures, mark attachment, variation fonts, directionality, script handling, feature tags
  - [ ] **Property tests:** shaping "ab" produces same glyphs as shaping "a" then "b" (no inter-glyph features); shaping RTL text mirrors glyph order
- [ ] `winterm/crates/renderer/src/atlas/shader_common.wgsl` — WGSL shaders
  - [ ] **Tests:** shader compilation (per-platform)
- [ ] `winterm/crates/renderer/src/atlas/shader_ps.wgsl` — Pixel shader
- [ ] `winterm/crates/renderer/src/atlas/shader_vs.wgsl` — Vertex shader
- [ ] `winterm/crates/renderer/src/atlas/rect_packer.rs` — 2D bin-packing
  - [ ] **Tests:** insert rects, find placement, packing efficiency, empty atlas, full atlas
  - [ ] **Property tests:** all packed rects are non-overlapping; packed area ≤ total requested area + margin

### Rendering Pipeline

- [ ] `winterm/crates/renderer/src/render_loop.rs` — Frame render loop
  - [ ] **Tests:** enable/disable, redraw trigger, teardown, background change detection, frame change detection
- [ ] `winterm/crates/renderer/src/scrolling.rs` — Scroll animation
  - [ ] **Tests:** scroll delta, animation frames, target reached detection
- [ ] `winterm/crates/renderer/src/dirty_region.rs` — Dirty area tracking
  - [ ] **Tests:** invalidate/merge/reset, empty invalidation, full invalidation, multiple regions
  - [ ] **Property tests:** union of dirty regions is always subset of total viewport
- [ ] `winterm/crates/renderer/src/paint_background.rs` — Background rendering
  - [ ] **Snapshot tests:** solid color, gradient, pattern background → compare against baseline
- [ ] `winterm/crates/renderer/src/paint_buffer.rs` — Visible row rendering
  - [ ] **Snapshot tests:** render ASCII text, CJK text, emoji, mixed-width text → compare against baseline per backend
- [ ] `winterm/crates/renderer/src/paint_selection.rs` — Selection rendering
  - [ ] **Snapshot tests:** block selection, line selection, word selection → compare against baseline
- [ ] `winterm/crates/renderer/src/paint_cursor.rs` — Cursor rendering
  - [ ] **Snapshot tests:** all cursor types (bar, underscore, box, full box), all sizes, blink on/off → compare against baseline
- [ ] `winterm/crates/renderer/src/paint_title.rs` — Title bar rendering
  - [ ] **Snapshot tests:** short title, long title (truncated), Unicode title → compare against baseline
- [ ] `winterm/crates/renderer/src/paint_image.rs` — Sixel/image rendering
  - [ ] **Tests:** image upload, crop, resize, interlace
  - [ ] **Snapshot tests:** render known sixel image → compare against baseline
- [ ] `winterm/crates/renderer/src/paint_grid_lines.rs` — Grid line rendering
  - [ ] **Snapshot tests:** all gridline combinations → compare against baseline
- [ ] `winterm/crates/renderer/src/paint_highlight.rs` — Search highlight rendering

### Font & Shaping

- [ ] `winterm/crates/renderer/src/font_loading.rs` — `fontdb` integration
  [ ] **Tests:** font enumeration, face loading, collection matching
  - [ ] **Property tests:** all system fonts can be loaded; font families match system font list
- [ ] `winterm/crates/renderer/src/glyph_extraction.rs` — Glyph extraction
  [ ] **Tests:** glyph lookup, advance width, bounding box, color bitmap glyphs
- [ ] `winterm/crates/renderer/src/text_shaping.rs` — High-level shaping API
  [ ] **Tests:** single char, multi-char, mixed scripts, mixed widths
- [ ] `winterm/crates/renderer/src/font_metrics.rs` — Font metrics
  [ ] **Tests:** ascent/descent/cap height/x-height/line gap, cell size calculation

---

## Phase 6: Console Host — Backend Process

**Estimated effort:** 3.5 weeks (0.5 wks for tests)

Order: `history`, `input_buffer`, `read_data_cooked` (alias expansion now wired
through `alias`), `input_state_machine_engine`, `console_information`,
`screen_info`, `settings`, `alias`, `input_key_info`, `scrolling`, `selection`
and `selection_input` are done. `directio` is next. Each module lands with its
tests and its declaration in `lib.rs`, as the porting skill requires.

### Core Console State

- [x] `winterm/crates/host/src/console_information.rs` — `ConsoleInformation`
  - [x] Ported from `consoleInformation.cpp` and the class in `server.h`: the
    recursive console lock, the input buffer, the per-application histories,
    the flags, the titles and their prefix, bracketed paste, the pending
    clipboard text, and the pending-cooked-read marker.
  - [x] **Tests:** title composition, original and link titles, bracketed paste,
    clipboard consumption, the popup marker, recursive locking, the input
    buffer, keyed histories, and the flag bits
  - Not ported: the screen buffers, VT I/O, MIDI, code pages, `AllocateConsole`
    and `AsCharInfo`; they need `screenInfo.cpp`, the server layer and the UI
- [x] `winterm/crates/host/src/settings.rs` — `Settings`
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
- [x] `winterm/crates/host/src/screen_info.rs` — `ScreenBuffer`
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
- [x] `winterm/crates/host/src/input_buffer.rs` — `InputBuffer`
  - [x] **Tests:** default console modes, single and batch writes, coalescing
    of a repeated key, a mouse move and a window resize, the batch boundary
    rule, text and NUL writes, prepend, peek versus read, and both flush paths
- [x] `winterm/crates/host/src/read_data_cooked.rs` — `COOKED_READ_DATA`
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
- [x] `winterm/crates/host/src/history.rs` — `CommandHistory` and
  `CommandHistoryList`, ported from `history.cpp`
  - [x] **Tests:** add, empty, ring drop, duplicate suppression, retrieval both
    ways with clamping, realloc, prefix and exact search, and the
    per-application list
  - [ ] **Property tests:** cycling through history returns to the most recent
    command
- [ ] `winterm/crates/host/src/handle.rs` — Handle management
- [x] ~~`winterm/crates/host/src/output_stream.rs` — Output stream~~
  - Not ported: `outputStream.cpp` implements `ITerminalApi`, a seam this port
    replaced with adapter-owned state and host-drained queues. ADR 013 records
    the decision and where its content lives.
- [x] `winterm/crates/host/src/scrolling.rs` — Scroll logic
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
- [x] `winterm/crates/host/src/selection.rs` — Selection state
  - [x] Ported from `selection.cpp` and `selectionState.cpp`: the flags (mouse,
    keyboard mark, line/block, the inverted bit), the anchor, the rectangle,
    the exclusive end, the saved cursor, the line-versus-block predicate, and
    `ShouldAllowMouseDragSelection`; plus `InitializeMarkSelection`,
    `InitializeMouseSelection`, `CheckAndSetAlternateSelection`,
    `AdjustSelection`, `ExtendSelection` and `ClearSelection`.
  - [x] **Tests:** mark and mouse start, the one-cell rectangle, the Alt flip,
    extending with clamping, adjust stopping the drag, cursor save and restore,
    both clear paths, and the no-op clear
  - Not ported: painting and the accessibility notifications (renderer and
    window), the clipboard write and `SelectAll` (they need the cooked read's
    line and the buffer's rectangle read), and colour selection
- [x] `winterm/crates/host/src/selection_input.rs` — Selection input
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
- [x] `winterm/crates/host/src/cmdline.rs` — Word delimiters
  - [x] Ported from `cmdline.cpp`: `IsWordDelim` for a character and for a
    cell's text, and `DelimiterClass`. The configurable list follows
    `registry.cpp`'s defaults until the registry is ported; the cooked read now
    shares this one home.
  - [x] **Tests:** every default delimiter and space, multi-character and empty
    text, and the three classes
- [x] `winterm/crates/host/src/input_key_info.rs` — Keyboard state
  - [x] Ported from `inputKeyInfo.cpp` and the class in `input.h`: the virtual
    key, the control-key state, and the modifier predicates (`IsCtrlOnly`,
    `IsShiftOnly`, `IsShiftAndCtrlOnly`, `IsAltOnly`, `HasNoModifiers`).
  - [x] **Tests:** every modifier combination, both Ctrl keys, and locks not
    counting as modifiers
  - Not ported: `input.cpp`'s `GetControlKeyState`, `ClearKeyInfo` and the
    menu/focus/ctrl handlers, which need window state. `IsSystemKey` is here as
    a free predicate because selection asks it.
- [ ] `winterm/crates/host/src/directio.rs` — Direct I/O
- [ ] `winterm/crates/host/src/registry.rs` — Registry access
- [ ] `winterm/crates/host/src/utils.rs` — Console utilities
- [ ] `winterm/crates/host/src/tracing.rs` — Event tracing
- [x] `winterm/crates/host/src/alias.rs` — Command aliases
  - [x] Ported from `alias.cpp` and `alias.h`: the per-application table with
    case-insensitive application and alias names, add/get/remove/list, the
    cmd.exe clear, and `s_MatchAndCopyAlias`'s expansion (`$1`-`$9`, `$*`,
    `$l`/`$g`/`$b`/`$t`, unknown escapes, the trailing line break and the line
    count). The `ConsoleAlias` A/W API conversions are not part of it.
  - [x] **Tests:** parameters, case folding, `$*` spacing, every escape, an
    empty parameter, removal by empty target, an empty source, leading-space
    and tab argument splitting, and clearing one application

### Screen Buffer Management

- [ ] `winterm/tests/host/scrollbar_state.rs` — Scrollbar computation for various buffer/window sizes
- [ ] `winterm/tests/host/buffer_switching.rs` — Alternate/main buffer switching
- [ ] `winterm/tests/host/font_updates.rs` — Font changes propagate correctly

---

## Phase 7: Server Layer — PTY Communication

**Estimated effort:** 2.5 weeks (0.5 wks for tests)

### Server Components

- [ ] `winterm/crates/server/src/api_dispatchers.rs` — API dispatch table
- [ ] `winterm/crates/server/src/api_message.rs` — API message format
- [ ] `winterm/crates/server/src/api_message_state.rs` — Message state
- [ ] `winterm/crates/server/src/api_sorter.rs` — API sorting
- [ ] `winterm/crates/server/src/device_handle.rs` — Device handle
- [ ] `winterm/crates/server/src/io_dispatchers.rs` — I/O dispatch
- [ ] `winterm/crates/server/src/object_handle.rs` — Handle table
- [ ] `winterm/crates/server/src/object_header.rs` — Object header
- [ ] `winterm/crates/server/src/process_handle.rs` — Process handle tracking
- [ ] `winterm/crates/server/src/process_list.rs` — Process list
- [ ] `winterm/crates/server/src/wait_block.rs` — Wait block
- [ ] `winterm/crates/server/src/wait_queue.rs` — Wait queue
- [ ] `winterm/crates/server/src/win_nt_control.rs` — NT control

### Cross-Platform PTY Abstraction

- [ ] `winterm/crates/server/src/pty.rs` — PTY abstraction
  - [ ] **Tests:** create, write, read, close, signal handling
- [ ] `winterm/crates/server/src/conpty.rs` — Windows ConPTY
  - [ ] **Tests:** pseudconsole create, process attach, IO, resize, close
- [ ] `winterm/crates/server/src/posix_pty.rs` — POSIX PTY
  - [ ] **Tests:** open pty, fork, master/slave IO, signal passthrough
  - [ ] **Property tests:** writing N bytes to master results in N bytes readable on slave
- [ ] `winterm/crates/server/src/device_comm.rs` — Device communication trait
  - [ ] **Tests:** abstract write/read/close, all backends implement correctly

---

## Phase 8: Input Processing & Interactivity

**Estimated effort:** 2.5 weeks (0.5 wks for tests)

### Input Processing

- [ ] `winterm/crates/input/src/keyboard.rs` — Keyboard events
  - [ ] **Tests:** key down/up, modifier detection (Ctrl/Alt/Shift), VT key encoding, dead key handling, repeat detection, key code mapping
  - [ ] **Property tests:** key_down → key_up sequence produces correct VT input; modifiers combine correctly
- [ ] `winterm/crates/input/src/mouse.rs` — Mouse events
  - [ ] **Tests:** button down/up, scroll (vertical/horizontal), motion, focus, click detection, wheel delta normalization
  - [ ] **Property tests:** mouse move within viewport produces coordinate within bounds; scroll delta clamps to reasonable values
- [ ] `winterm/crates/input/src/kitty.rs` — Kitty keyboard
  - [ ] **Tests:** enable/disable, key encoding, event report, past event report, normal/dispatch/report modes
- [ ] `winterm/crates/input/src/ime.rs` — IME composition
  - ] **Tests:** composition start/end, preedit text, commit, cancellation
- [ ] `winterm/crates/input/src/input_thread.rs` — Input thread
  - [ ] **Tests:** event loop, shutdown, event delivery order
- [ ] `winterm/crates/input/src/pty_signal.rs` — PTY signals

### Platform Backends

- [ ] `winterm/crates/ui/src/platform/win32_window.rs` — Win32 backend
  - [ ] **Tests:** window create/destroy/resize, cursor, clipboard, DPI, transparency, focus
- [ ] `winterm/crates/ui/src/platform/x11_window.rs` — X11 backend
  - [ ] **Tests:** window create/destroy/resize, cursor, clipboard, DPI, focus
- [ ] `winterm/crates/ui/src/platform/wayland_window.rs` — Wayland backend
  - [ ] **Tests:** window creation, surface presentation, input events
- [ ] `winterm/crates/ui/src/platform/cocoa_window.rs` — macOS backend
  - [ ] **Tests:** window create/destroy/resize, cursor, clipboard, DPI, focus, transparency

---

## Phase 9: Terminal App — UI Layer

**Estimated effort:** 5 weeks (1 wk for tests)

### Application Core

- [ ] `winterm/crates/winterm/src/app.rs` — App lifecycle
- [ ] `winterm/crates/winterm/src/config.rs` — Config loading/saving
- [ ] `winterm/crates/winterm/src/settings.rs` — Settings model
- [ ] `winterm/crates/winterm/src/theme.rs` — Theme system
- [ ] `winterm/crates/winterm/src/window.rs` — Window management
- [ ] `winterm/crates/winterm/src/icon.rs` — Embedded resources

### Tab & Pane Management

- [ ] `winterm/crates/winterm/src/tab_bar.rs` — Tab row
  - [ ] **Tests:** add/remove tabs, tab ordering, tab close animation, drag reorder
- [ ] `winterm/crates/winterm/src/tab.rs` — Tab state
  - [ ] **Tests:** title updates, close, focus, resize
- [ ] `winterm/crates/winterm/src/pane.rs` — Pane management
  - [ ] **Tests:** split H/V, resize, focus, close, nested splits
  - [ ] **Property tests:** splitting then unsplitting returns to original single pane; resizing to zero shows adjacent pane
- [ ] `winterm/crates/winterm/src/terminal_widget.rs` — egui terminal viewport
- [ ] `winterm/crates/winterm/src/terminal_renderer.rs` — wgpu render pass

### Command Palette & Settings

- [ ] `winterm/crates/winterm/src/command_palette.rs` — Command palette
  - [ ] **Tests:** open/close, filtering, selection, action execution, keyboard navigation
- [ ] `winterm/crates/winterm/src/settings_editor.rs` — Settings editor
  - [ ] **Tests:** JSON preview, property editing, validation errors, save/load
- [ ] `winterm/crates/winterm/src/search.rs` — In-terminal search overlay
  - [ ] **Tests:** open/close, input, results navigation, replace, highlight
- [ ] `winterm/crates/winterm/src/scrollbar.rs` — Scrollbar with marks
  - [ ] **Tests:** drag, click, scroll mark indicators, mark categories

### Keybindings & Actions

- [ ] `winterm/crates/winterm/src/keybindings.rs` — Key chord matching
  - [ ] **Tests:** exact match, partial match, no match, modifier variations
  - [ ] **Property tests:** matching is case-sensitive for letters; modifiers are order-independent; exact key + modifier match has higher priority than modifier-only match
- [ ] `winterm/crates/winterm/src/actions.rs` — Action definitions
- [ ] `winterm/crates/winterm/src/action_registry.rs` — Action registry
  - [ ] **Tests:** register/unregister/lookup, dispatch, parameter passing
- [ ] `winterm/crates/winterm/src/notifications.rs` — Desktop notifications
- [ ] `winterm/crates/winterm/src/remoting.rs` — WebSocket terminal

### Terminal Control

- [ ] `winterm/crates/winterm/src/term_control.rs` — High-level terminal control
- [ ] `winterm/crates/winterm/src/term_connection.rs` — Terminal-backend connection
- [ ] `winterm/crates/winterm/src/term_settings.rs` — Per-terminal settings
- [ ] `winterm/crates/winterm/src/term_events.rs` — Event callbacks

---

## Phase 10: Settings, Serialization & Configuration

**Estimated effort:** 1.5 weeks (0.5 wks for tests)

### Settings Model

- [ ] `winterm/crates/settings/src/schema.rs` — JSON schema
- [ ] `winterm/crates/settings/src/parser.rs` — JSON/YAML parser
  - [ ] **Tests:** parse valid settings, parse invalid JSON, parse missing required fields, parse unknown fields, YAML parse
  - [ ] **Property tests:** `parse(serialize(settings)) == settings` for all valid settings; parse errors include field name
- [ ] `winterm/crates/settings/src/profiles.rs` — Profile list
  - [ ] **Tests:** create profile, add/remove, modify, defaults, validation
- [ ] `winterm/crates/settings/src/palette.rs` — Color palettes
  - [ ] **Tests:** load palette, validate colors, apply palette
- [ ] `winterm/crates/settings/src/fonts.rs` — Font config
  - [ ] **Tests:** face validation, size parsing, weight parsing, feature parsing
- [ ] `winterm/crates/settings/src/keybindings.rs` — Keybinding definitions
- [ ] `winterm/crates/settings/src/actions.rs` — Action definitions
- [ ] `winterm/crates/settings/src/themes.rs` — Theme definitions
- [ ] `winterm/crates/settings/src/sgr_aliases.rs` — SGR alias table
- [ ] `winterm/crates/settings/src/defaults.rs` — Defaults

### Migration & Compatibility

- [ ] `winterm/crates/settings/src/migration.rs` — Version migration
  - [ ] **Tests:** migrate from N-1 to N, migrate from old schema, reverse migration
  - [ ] **Property tests:** migrating through all versions produces valid final settings; migrating twice is idempotent
- [ ] `winterm/crates/settings/src/validation.rs` — Settings validation
- [ ] `winterm/crates/settings/src/watch.rs` — Settings file watching
  - [ ] **Tests:** file change detection, hot-reload, error handling

---

## Phase 11: Audio & Miscellaneous

**Estimated effort:** 1 week

- [ ] `winterm/crates/audio/src/beep.rs` — Terminal beep
  - [ ] **Tests:** generate beep, duration, frequency
- [ ] `winterm/crates/audio/src/visual_bell.rs` — Visual bell animation
  - [ ] **Tests:** trigger, animation frames, completion
- [ ] `winterm/crates/audio/src/midi.rs` — MIDI audio

---

## Phase 12: Comprehensive Testing & Validation

**Estimated effort:** 4 weeks (significantly expanded)

### A. Unit Test Suite (per-crate)

- [ ] `winterm/crates/utils/src/unit_tests/point_tests.rs` — 50+ unit tests
- [ ] `winterm/crates/utils/src/unit_tests/size_tests.rs` — 40+ unit tests
- [ ] `winterm/crates/utils/src/unit_tests/rect_tests.rs` — 60+ unit tests
- [ ] `winterm/crates/utils/src/unit_tests/color_tests.rs` — 30+ unit tests
- [ ] `winterm/crates/utils/src/unit_tests/small_vec_tests.rs` — 30+ unit tests
- [ ] `winterm/crates/utils/src/unit_tests/enum_set_tests.rs` — 30+ unit tests
- [ ] `winterm/crates/utils/src/unit_tests/hash_tests.rs` — 20+ unit tests
- [ ] `winterm/crates/utils/src/unit_tests/ticket_lock_tests.rs` — 15+ unit tests
- [ ] `winterm/crates/utils/src/unit_tests/atomic_tests.rs` — 15+ unit tests
- [ ] `winterm/crates/utils/src/unit_tests/throttled_tests.rs` — 10+ unit tests
- [ ] `winterm/crates/utils/src/unit_tests/unicode_tests.rs` — 40+ unit tests
- [ ] `winterm/crates/utils/src/unit_tests/math_tests.rs` — 20+ unit tests
- [ ] `winterm/crates/utils/src/unit_tests/spsc_tests.rs` — 15+ unit tests
- [ ] `winterm/crates/types/src/text_color_tests.rs` — 20+ unit tests
- [ ] `winterm/crates/types/src/character_attrs_tests.rs` — 20+ unit tests
- [ ] `winterm/crates/types/src/viewport_tests.rs` — 20+ unit tests
- [ ] `winterm/crates/buffer/src/output_cell_tests.rs` — 30+ unit tests
- [ ] `winterm/crates/buffer/src/row_tests.rs` — 50+ unit tests
- [ ] `winterm/crates/buffer/src/text_buffer_tests.rs` — 80+ unit tests
- [ ] `winterm/crates/buffer/src/cursor_tests.rs` — 20+ unit tests
- [ ] `winterm/crates/buffer/src/search_tests.rs` — 30+ unit tests
- [ ] `winterm/crates/buffer/src/marks_tests.rs` — 25+ unit tests
- [ ] `winterm/crates/buffer/src/reflow_tests.rs` — 40+ unit tests
- [ ] `winterm/crates/buffer/src/hyperlinks_tests.rs` — 15+ unit tests
- [ ] `winterm/crates/buffer/src/selection_tests.rs` — 35+ unit tests
- [ ] `winterm/crates/vt/src/state_machine_tests.rs` — 60+ unit tests
- [ ] `winterm/crates/vt/src/output_engine_tests.rs` — 80+ unit tests
- [ ] `winterm/crates/vt/src/input_engine_tests.rs` — 50+ unit tests
- [ ] `winterm/crates/vt/src/parameter_tests.rs` — 30+ unit tests
- [ ] `winterm/crates/vt/src/base64_tests.rs` — 15+ unit tests
- [ ] `winterm/crates/adapter/src/page_manager_tests.rs` — 20+ unit tests
- [ ] `winterm/crates/adapter/src/sixel_parser_tests.rs` — 30+ unit tests
- [ ] `winterm/crates/renderer/src/font_cache_tests.rs` — 15+ unit tests
- [ ] `winterm/crates/renderer/src/rect_packer_tests.rs` — 15+ unit tests
- [ ] `winterm/crates/renderer/src/text_analysis_tests.rs` — 40+ unit tests
- [ ] `winterm/crates/host/src/console_info_tests.rs` — 30+ unit tests
- [ ] `winterm/crates/host/src/screen_info_tests.rs` — 40+ unit tests
- [ ] `winterm/crates/host/src/scroll_history_tests.rs` — 15+ unit tests
- [ ] `winterm/crates/host/src/cooked_read_tests.rs` — 20+ unit tests
- [ ] `winterm/crates/server/src/wait_queue_tests.rs` — 20+ unit tests
- [ ] `winterm/crates/server/src/process_list_tests.rs` — 15+ unit tests
- [ ] `winterm/crates/input/src/keyboard_tests.rs` — 30+ unit tests
- [ ] `winterm/crates/input/src/mouse_tests.rs` — 25+ unit tests
- [ ] `winterm/crates/input/src/kitty_tests.rs` — 20+ unit tests
- [ ] `winterm/crates/winterm/src/keybindings_tests.rs` — 30+ unit tests
- [ ] `winterm/crates/winterm/src/actions_tests.rs` — 15+ unit tests
- [ ] `winterm/crates/winterm/src/pane_tests.rs` — 20+ unit tests
- [ ] `winterm/crates/settings/src/parser_tests.rs` — 30+ unit tests
- [ ] `winterm/crates/settings/src/profiles_tests.rs` — 20+ unit tests
- [ ] `winterm/crates/settings/src/migration_tests.rs` — 15+ unit tests
- [ ] `winterm/crates/audio/src/visual_bell_tests.rs` — 10+ unit tests

### B. Property-Based Tests (proptest)

- [ ] `winterm/tests/properties/point_props.rs` — Arithmetic invariants, comparison properties
- [ ] `winterm/tests/properties/size_props.rs` — Scale, area, overflow properties
- [ ] `winterm/tests/properties/rect_props.rs` — Union/intersection/difference invariants
- [ ] `winterm/tests/properties/color_props.rs` — Alpha blend commutativity, hex roundtrip
- [ ] `winterm/tests/properties/enum_set_props.rs` — Bitflag algebra properties
- [ ] `winterm/tests/properties/rle_props.rs` — Encode/decode roundtrip
- [ ] `winterm/tests/properties/small_vec_props.rs` — Iterator equivalence, push/pop order
- [ ] `winterm/tests/properties/spsc_props.rs` — FIFO ordering
- [ ] `winterm/tests/properties/generational_props.rs` — Index validity
- [ ] `winterm/tests/properties/reflow_props.rs` — Character preservation, width constraints
- [ ] `winterm/tests/properties/iterator_props.rs` — Forward/backward symmetry, distance
- [ ] `winterm/tests/properties/vt_params_props.rs` — Parameter serialization roundtrip
- [ ] `winterm/tests/properties/vt_state_machine_props.rs` — Stateless transitions, recovery from corruption
- [ ] `winterm/tests/properties/unicode_props.rs` — Grapheme boundary spec compliance
- [ ] `winterm/tests/properties/text_attribute_props.rs` — Equality reflexivity/symmetry/transitivity

### C. Integration Tests

- [ ] `winterm/tests/integration/vt_output.rs` — Full VT output pipeline: write sequences → verify buffer state
- [ ] `winterm/tests/integration/vt_input.rs` — Full VT input pipeline: generate sequences → verify decoded events
- [ ] `winterm/tests/integration/buffer_operations.rs` — Write text → reflow → scroll → select → copy → verify
- [ ] `winterm/tests/integration/pty_process.rs` — Start PTY process → write input → read output → verify
- [ ] `winterm/tests/integration/console_host.rs` — Allocate console → write → read → resize → verify
- [ ] `winterm/tests/integration/terminal_app.rs` — Create app → open tab → write → render → verify UI elements
- [ ] `winterm/tests/integration/tab_pane.rs` — Create tab → split → resize → focus → close → verify
- [ ] `winterm/tests/integration/settings.rs` — Write settings file → load → validate → migrate → verify
- [ ] `winterm/tests/integration/keybindings.rs` — Configure bindings → simulate input → verify action dispatch
- [ ] `winterm/tests/integration/clipboard.rs` — Copy text → paste → verify roundtrip
- [ ] `winterm/tests/integration/theme.rs` — Switch light/dark → verify colors update
- [ ] `winterm/tests/integration/font_rendering.rs` — Change font → verify rendering matches
- [ ] `winterm/tests/integration/search.rs` — Search text → verify highlights → verify selection
- [ ] `winterm/tests/integration/scrollback.rs` — Write 1000 lines → scroll up → verify visible content → scroll down → verify
- [ ] `winterm/tests/integration/alternate_buffer.rs` — Switch to alt buffer → write → switch back → verify main buffer unchanged

### D. VT Conformance Suite (against xterm/kitty/iterm2)

The corpus drives the public `term::Terminal` API and reads the screen, so it
lives in the crate that owns the parser-to-dispatch loop:
`crates/term/tests/conformance/`, with one entry point declared in
`crates/term/Cargo.toml`.

- [x] `crates/term/tests/conformance/csi/sgr.rs` — SGR colours, the attribute
  pairs, the `:`-form underline styles and full reset
- [x] `crates/term/tests/conformance/csi/cursor.rs` — CUP/HVP, CUU/CUD,
  CUF/CUB, CHA/VPA, CNL/CPL, HPR/VPR, DECSC/DECRC, CSI s/u and 8-bit C1
- [x] `crates/term/tests/conformance/csi/scroll.rs` — SU/SD, LF and CR+LF at the
  bottom margin, DECSTBM and RI
- [x] `crates/term/tests/conformance/csi/erase.rs` — EL 0/1/2, ED 0/1/2 and ECH
- [x] `crates/term/tests/conformance/csi/insert_delete.rs` — IL/DL/ICH/DCH
- [x] `crates/term/tests/conformance/osc.rs` — window title (0/2), working
  directory (7), clipboard (52), palette set/reset (4/104)
- [x] `crates/term/tests/conformance/dcs/sixel.rs` — an image's colour, its
  starting row, cell coverage, the repeat count, unterminated input and the
  erase-on-write rule (ADR 005)
- [x] `crates/term/tests/conformance/private_modes.rs` — DECTCEM, DECAWM,
  the alternate buffer and DECRQM (set, reset and unsupported)
- [x] `crates/term/tests/conformance/mouse.rs` — modes 1000, 1002, 1006 and
  1007, alternate scroll, and the X11 modifier bits
- [x] `crates/term/tests/conformance/kitty_keyboard.rs` — the query, set, push
  and pop forms of the kitty flags and their effect on encoding
- [x] `crates/term/tests/conformance/bracketed_paste.rs` — the wrapper, the
  paste filter (control codes, line endings, a pasted end marker) and the
  mode report
- [x] `crates/term/tests/conformance/win32_input_mode.rs` — mode 9001 records
  and the kitty protocol's precedence
- [x] `crates/term/tests/conformance/escape.rs` — DECSC/DECRC, IND/NEL/RI,
  line rendition, tab stops and character-set designation
- [x] `crates/term/tests/conformance/vt52.rs` — DECANM, the cursor moves,
  `ESC Y` addressing, erase, reverse index, identify, the graphics set and
  the return to ANSI
- [x] `crates/term/tests/conformance/hyperlink.rs` — OSC 8 hyperlinks
- [ ] `crates/term/tests/conformance/generate.rs` — Fuzz-generated sequences against reference

### E. Snapshot Tests (insta)

- [ ] `winterm/tests/snapshots/ascii_text.png` — Render "Hello World 123!" at various sizes
- [ ] `winterm/tests/snapshots/cjk_text.png` — Render CJK characters
- [ ] `winterm/tests/snapshots/emoji_text.png` — Render emoji (ZWJ sequences, skin tones)
- [ ] `winterm/tests/snapshots/mixed_width_text.png` — Mixed ASCII/CJK/emoji
- [ ] `winterm/tests/snapshots/large_buffer.png` — Render 1000 lines scrollback
- [ ] `winterm/tests/snapshots/color_palette.png` — Render 16-color + 256-color + RGB
- [ ] `winterm/tests/snapshots/sgr_styles.png` — Render all SGR styles
- [ ] `winterm/tests/snapshots/selection_rendering.png` — Render selections
- [ ] `winterm/tests/snapshots/cursor_rendering.png` — Render all cursor types
- [ ] `winterm/tests/snapshots/reflow_ascii.png` — Reflow ASCII paragraph
- [ ] `winterm/tests/snapshots/reflow_cjk.png` — Reflow CJK paragraph
- [ ] `winterm/tests/snapshots/reflow_emoji.png` — Reflow emoji paragraph
- [ ] `winterm/tests/snapshots/sixel_image.png` — Render known Sixel image
- [ ] `winterm/tests/snapshots/dark_theme.png` — Render with dark theme
- [ ] `winterm/tests/snapshots/light_theme.png` — Render with light theme

### F. Performance Benchmarks (criterion)

- [ ] `winterm/benches/buffer_write.rs` — Write throughput (cells/sec)
- [ ] `winterm/benches/buffer_scroll.rs` — Scroll throughput
- [ ] `winterm/benches/reflow.rs` — Reflow performance at various buffer sizes
- [ ] `winterm/benches/vt_parse.rs` — VT sequence parsing throughput
- [ ] `winterm/benches/vt_dispatch.rs` — VT sequence dispatch latency
- [ ] `winterm/benches/iterator.rs` — Cell/text iteration performance
- [ ] `winterm/benches/search.rs` — Text search performance
- [ ] `winterm/benches/hyperlink_map.rs` — Hyperlink lookup/add/remove
- [ ] `winterm/benches/row_write.rs` — Per-row cell write performance
- [ ] `winterm/benches/small_vec.rs` — small_vector vs Vec comparison
- [ ] `winterm/benches/ticket_lock.rs` — Ticket lock vs parking_lot comparison
- [ ] `winterm/benches/rect_ops.rs` — Rectangle union/intersection/difference

### G. Fuzzing

- [ ] `winterm/fuzz/vt_parser.rs` — Fuzz VT escape sequence parser (honggfuzz)
- [ ] `winterm/fuzz/sixel_parser.rs` — Fuzz Sixel image parser (honggfuzz)
- [ ] `winterm/fuzz/cell_iterator.rs` — Fuzz cell iterator bounds (afl.rs)
- [ ] `winterm/fuzz/reflow.rs` — Fuzz reflow with random text (honggfuzz)
- [ ] `winterm/fuzz/parameter_parsing.rs` — Fuzz VT parameter parsing (afl.rs)
- [ ] `winterm/fuzz/unicode_width.rs` — Fuzz Unicode width detection (honggfuzz)

### H. Platform-Specific Tests

- [ ] `winterm/tests/platform/win32/window.rs` — Win32 window create/destroy/resize
- [ ] `winterm/tests/platform/win32/clipboard.rs` — Windows clipboard
- [ ] `winterm/tests/platform/win32/dpi.rs` — DPI awareness (scaling)
- [ ] `winterm/tests/platform/win32/conpty.rs` — ConPTY process management
- [ ] `winterm/tests/platform/wayland/window.rs` — Wayland surface/presentation
- [ ] `winterm/tests/platform/wayland/input.rs` — Wayland input events
- [ ] `winterm/tests/platform/x11/window.rs` — X11 window management
- [ ] `winterm/tests/platform/x11/clipboard.rs` — X11 clipboard
- [ ] `winterm/tests/platform/macos/window.rs` — Cocoa window management
- [ ] `winterm/tests/platform/macos/clipboard.rs` — macOS clipboard
- [ ] `winterm/tests/platform/macos/dpi.rs` — HiDPI rendering

### I. End-to-End Application Tests

- [ ] `winterm/tests/e2e/startup.rs` — Launch app → verify window → load settings
- [ ] `winterm/tests/e2e/new_tab.rs` — New tab → verify creation → close
- [ ] `winterm/tests/e2e/split_pane.rs` — Split → verify two panes → merge
- [ ] `winterm/tests/e2e/command_palette.rs` — Open palette → search → execute command
- [ ] `winterm/tests/e2e/settings_change.rs` — Modify settings → hot-reload → verify
- [ ] `winterm/tests/e2e/theme_switch.rs` — Switch theme → verify colors update
- [ ] `winterm/tests/e2e/long_running.rs` — Run for 1 hour → check memory leaks
- [ ] `winterm/tests/e2e/crash_recovery.rs` — Kill subprocess → verify console host survives

### J. Cross-Platform Matrix Tests

- [ ] `winterm/tests/xplat/ansi_output.rs` — Same VT sequences → verify same buffer state on all platforms
- [ ] `winterm/tests/xplat/keyboard_input.rs` — Same key events → verify same decoded events
- [ ] `winterm/tests/xplat/clipboard.rs` — Copy/paste text → verify roundtrip
- [ ] `winterm/tests/xplat/dpi.rs` — Same content at 100%/125%/150%/200% DPI → verify scaling
- [ ] `winterm/tests/xplat/window_resize.rs` — Same resize sequence → verify same buffer state

---

## Phase 13: Polish & Distribution

**Estimated effort:** 2 weeks

- [ ] `winterm/crates/winterm/src/args.rs` — CLI argument parsing with `clap`
  - [ ] **Tests:** all arguments, help text, error messages, long/short forms, combined flags
- [ ] `winterm/crates/winterm/src/logging.rs` — Structured logging with `tracing`
- [ ] `winterm/crates/winterm/src/singleton.rs` — Single-instance enforcement
- [ ] `winterm/crates/winterm/src/upgrade.rs` — Auto-update mechanism
- [ ] `winterm/crates/winterm/src/windows` — Windows installer (MSI/NSIS)
- [ ] `winterm/crates/winterm/src/linux` — Linux packaging (AppImage, Flatpak, deb)
- [ ] `winterm/crates/winterm/src/macos` — macOS packaging (dmg, notarization)
- [ ] `winterm/crates/winterm/src/docs` — Documentation and user guide

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
- [ ] `winterm/tests/CI-README.md` — How to run each test suite locally

---

## Dependency Map (Rust Ecosystem)

| Purpose | Crate(s) |
|---------|----------|
| Serialization | `serde` + `serde_json` + `serde_yaml` |
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

| Phase | Effort | Deliverable |
|-------|--------|-------------|
| **1. Foundation** | ✅ DONE | `winterm/crates/utils` (25 modules) + `winterm/crates/types` (10 modules) + `winterm/crates/buffer` (4 modules) — **275 tests passing, 1 ignored** (hash collision proptest) + 6 proptests + 3 snapshot tests added |
| **2. Text Buffer** | 4 wks | `winterm/crates/buffer` + **500+ unit tests** + property tests |
| **3. VT Parser** | 4 wks | `winterm/crates/vt` + **300+ unit tests** + conformance suite + fuzzing |
| **4. Adapter** | ⚠️ PARTIAL | `winterm/crates/adapter` — state, dispatch traits, page manager on the real buffer, sixel, macros. `AdaptDispatch` compiles and is tested; the `ITermDispatch` handlers are pending |
| **5. Renderer** | 6 wks | `winterm/crates/renderer` + **150+ unit tests** + 15 snapshot tests |
| **6. Console Host** | 3.5 wks | `winterm/crates/host` + **200+ unit tests** |
| **7. Server** | 2.5 wks | `winterm/crates/server` + **100+ unit tests** |
| **8. Input/UI** | 2.5 wks | `winterm/crates/input` + **100+ unit tests** + 4 platform backends |
| **9. Terminal App** | 5 wks | `winterm/crates/winterm` + **200+ unit tests** + integration tests |
| **10. Settings** | 1.5 wks | `winterm/crates/settings` + **100+ unit tests** |
| **11. Audio** | 1 wk | `winterm/crates/audio` + **10+ unit tests** |
| **12. Testing** | **4 wks** | **3500+ unit tests + 15+ properties + 40+ integration tests + 30 conformance tests + 15 snapshots + 12 benchmarks + 6 fuzz targets + 20 platform tests + 8 e2e tests + 5 xplat tests** |
| **13. Polish** | 2 wks | CLI, logging, packaging, docs |
| **14. CI/CD** | 1 wk | **8 GitHub Actions workflows** |
| **Total** | **38 wks** | Fully tested cross-platform terminal emulator |

---
## Test Coverage Targets

Completed for Phase 1:

| Module | Target Coverage | Critical Tests |
|--------|----------------|----------------|
| `utils/` | ✅ ≥95% | 199 tests — all edge cases, overflow, bounds, proptests |
| `types/` | ✅ ≥90% | All enum variants, conversions tested |

Pending for future phases:

| Module | Target Coverage | Critical Tests |
|--------|----------------|----------------|
| `buffer/` | ≥90% | Reflow, iteration, scrollback |
| `vt/` | ≥90% | All VT sequences, state transitions |
| `adapter/` | ⚠️ **Interfaces only** — 46 tests cover the modules that exist; dispatch is unimplemented |
| `renderer/` | ≥80% | Per-backend rendering, atlas |
| `host/` | ≥85% | Console API, buffer management |
| `server/` | ≥80% | PTY IO, handle management |
| `input/` | ≥85% | All input encodings |
| `app/` | ≥80% | Keybindings, tabs, panes |
| `settings/` | ≥85% | Parse/serialize/migrate |

---

## High-Risk Areas & Mitigation

| Risk | Impact | Mitigation |
|------|--------|------------|
| **Text reflow algorithm** | Core functionality | Property tests: char preservation, width constraints; snapshot tests at various widths; fuzz test with random text |
| **Glyph atlas management** | Performance + correctness | Benchmarks: atlas creation/update; snapshot tests per backend; property tests: non-overlapping packed rects |
| **VT escape sequence conformance** | Compatibility | 30+ conformance tests against xterm/kitty/iterm2 reference; fuzz all 19 state machines |
| **IME/Composition input** | Non-Latin input quality | Platform-specific IME tests for Win32/X11/Wayland/Cocoa |
| **Clipboard formatting** | User expectations | Integration tests: copy → paste roundtrip for plain/HTML/RTF; cross-platform clipboard tests |
| **DPI awareness** | Multi-monitor quality | Platform DPI tests at 100/125/150/200%; snapshot tests at each DPI level |
| **Sixel/image rendering** | Advanced features | Conformance tests with known sixel images; snapshot comparison |
| **Cross-platform windowing** | Platform parity | 4 platform backends each with identical test matrix; xplat matrix tests |

---

## Cross-Platform Abstraction Strategy

### Window Backend Trait

```rust
pub trait WindowBackend {
    fn create(size: Size, title: &str) -> Self;
    fn resize(&mut self, size: Size);
    fn position(&self) -> Point;
    fn dpi(&self) -> f32;
    fn set_cursor_visible(&mut self, visible: bool);
    fn set_cursor_style(&mut self, style: CursorStyle);
    fn clipboard_text(&self) -> Option<String>;
    fn set_clipboard_text(&mut self, text: &str);
    fn display_rect(&self) -> Rect;
    fn is_focused(&self) -> bool;
    fn set_transparent(&mut self, transparent: bool);
}
```

### Platform Backend Modules

- `winterm/crates/ui/src/platform/win32_window.rs` — Win32 API + Direct2D
- `winterm/crates/ui/src/platform/x11_window.rs` — Xlib/XCB + XRender
- `winterm/crates/ui/src/platform/wayland_window.rs` — Wayland client protocol
- `winterm/crates/ui/src/platform/cocoa_window.rs` — Cocoa + Metal

---

*Last updated: 2026-06-03 — Phase 1 complete: 275 tests passing, 1 ignored; all 39 modules implemented, 6 proptests added, 3 snapshot tests added, 9 additional proptests remaining as optional*
