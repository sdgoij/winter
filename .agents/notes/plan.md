# Windows Terminal C++ → Cross-Platform Rust: Porting Plan

**Reference project:** Microsoft's Windows Terminal (`refs/terminal/`)
**Goal:** Port the core terminal emulator to a cross-platform Rust codebase targeting Windows, Linux (Wayland/X11), and macOS.

## Status (2026-11-22)

Verified by running the gate, not by reading the previous status:

| Crate | Lines | Tests | State |
|---|---|---|---|
| `utils` | ~4.5k | 203 + 1 ignored | Complete for Phase 1 |
| `types` | ~2.0k | 80 | Complete for Phase 1; owns `TextAttribute`, the VT vocabulary, the charset tables and `SgrStack` |
| `buffer` | ~5.6k | 192 | Phase 2 — scrollback stored, `logical_lines` and `resize` (reflow) landed; OSC 8 hyperlink registry |
| `vt` | ~2.3k | 57 | Phase 3 partial — no conformance corpus, no fuzz target |
| `adapter` | ~8.4k | 121 | Phase 4 state and interfaces; print, SGR, cursor, margin, line, erase, rectangular, mode (incl. DECCOLM), report, hyperlink/OSC, save-restore and tab handlers ported |

`just gate` is green: `cargo fmt --all --check`, `clippy -D warnings`, `cargo-deny`,
653 tests, doc tests. Phases 5–14 are not started.

A status that the code has outgrown is a bug in this document. When a milestone
changes, change its row in the same commit.

## Pre-Phase-5 cleanup

Phase 5 puts a renderer on top of the adapter. Two items still stand between here
and it; the rest of this section is the record of what the first cleanup pass
settled.

**Open.**

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
   save/restore (DECSC/DECRC), tab stops, charset designation and shifts, and
   rectangular operations (DECCARA, DECRARA with DECSACE extent, DECCRA,
   DECFRA, DECERA, DECSERA, DECRQCRA), and hyperlinks and OSC strings (OSC 8
   into the buffer's hyperlink registry, OSC 0/2 title, OSC 7 CWD, OSC 52
   clipboard).
   Still ported nowhere: colour-table OSC strings, sixel/DCS, DRCS, macros,
   presentation state, and window manipulation.
   Everything else falls through to the trait default, which
   `AdaptDispatch::unhandled` records; the ratchet test in `adapt_dispatch.rs`
   names the ported set and must grow with each family.

2. **`adapter/src/main.rs` is `println!("Hello, world!")`.** It makes `adapter`
   a binary as well as a library for no reason. Delete it, or replace it with a
   real conformance runner.

**Settled.**

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
dispatch loop (`StateMachine` → `DispatchAction` → `ITermDispatch`) belongs to
whatever owns both, which is the application crate, not the adapter.

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
│   ├── server/             (PTY abstraction, handle table)
│   ├── settings/           (config model, profiles, themes, keybindings)
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
  - [x] **Tests:** creation, clamp, contains point, area, intersection, union
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
- [x] `crates/buffer/src/selection.rs` — Selection
  - [x] **Tests:** point_span creation, iteration over rows, text extraction (plain, HTML, RTF), block selection, trimmed trailing whitespace, clipboard copy request
  - [x] **Property tests:** selecting and copying then pasting returns identical text; block selection text is rectangular
  - [ ] **Snapshot tests:** render selection rects over known buffer
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
**Tests:** 57 passing

### Core State Machine

- [x] `crates/vt/src/state_machine.rs` — `StateMachine` DFA (19 states)
  - [x] **Tests:** all state transitions (Ground, Escape, CsiParam, CsiIntermediate, OscStr, DcsParam, etc.), `process_char()`, `reset_state()`, `state()`, `params()`, `intermediate()`, `string()`, parser mode switching
  - [x] **Property tests:** feeding empty string produces no actions; feeding identical string twice produces identical actions; reset clears all state
  - [ ] **Fuzz tests:** `honggfuzz` on random byte sequences → valid or graceful recovery
- [ ] `crates/vt/src/state_machine_engine.rs` — `IStateMachineEngine` trait
  - [ ] **Tests:** trait object construction, all virtual methods called correctly
- [ ] `crates/vt/src/output_engine.rs` — `OutputStateMachineEngine`
  - [ ] **Tests:** `ActionExecute()` for all execute chars (BEL, BS, HT, LF, CR, FF, ESC), `ActionPrint()`, `ActionPrintString()`, `ActionEscDispatch()` for all escape codes (ESC $, ESC (, ESC ), ESC B, ESC D, ESC H, ESC M, ESC Z, ESC 7, ESC 8, ESC =, ESC >, ESC #3-8, ESC c), `ActionCsiDispatch()` for all CSI commands (SGR, CUP, CUD, CUF, CUB, CNL, CPL, CHA, CHT, ED, EL, IL, DL, DCH, ICH, SU, SD, NP, PP, HVP, TBC, SM, RM, DECSET, DECRST, DSR, DECSTBM, DECSLRM, DECSCUSR, DECSTR), `ActionOscDispatch()` for all OSC codes, `ActionDcsDispatch()` for DCS commands including Sixel, `ActionSs3Dispatch()`
  - [ ] **Property tests:** every CSI action resets parser to Ground state; OSC action with valid parameter calls handler exactly once
- [ ] `crates/vt/src/input_engine.rs` — `InputStateMachineEngine`
  - [ ] **Tests:** mouse tracking (1000/1001/1002/1003/1006), bracketed paste, kitty keyboard (KKP), Win32 input mode (9001), focus tracking (1004), cursor events (1005/1015/1036)
  - [ ] **Property tests:** enabling mouse mode via DECSET 1000 then pressing mouse key produces correct CSI u sequence
- [x] `crates/vt/src/dispatch_types.rs` — `VtId`, `VtParameters`, `DeviceAttributesResponse`
  - [x] **Tests:** VtId construction/comparison, parameter parsing, device attribute bitflags
- [x] `crates/vt/src/ascii.rs` — ASCII constants + helper functions (`is_printable`, `is_control`, `is_digit`, `is_intermediate`, `is_csi_dispatch`)
- [x] `crates/vt/src/base64.rs` — Base64 encode/decode with proper padding handling
  - [x] **Tests:** standard/base64url, padding, roundtrip, invalid input handling
  - [x] **Property tests:** `decode(encode(s)) == s` for byte lengths 0-256
- [x] `crates/vt/src/charsets.rs` — VT character sets (G0-G3 management)
  - [x] **Tests:** G0/G1/G2/G3 set assignment, default charset, alternate charset activation, SI/SO switching
- [ ] `crates/vt/src/tracing.rs` — VT sequence tracing
- [x] `crates/vt/src/parameters.rs` — VT parameter parsing
  - [x] **Tests:** empty params, single param, multi-param, semicolon-separated, subparameters, overflow, missing params, negative values
  - [x] **Property tests:** parsing then serializing parameter list preserves all values

### VT Conformance Test Sequences (Deliverable)

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
- [x] `winterm/crates/adapter/src/font_buffer.rs` — Font tracking
  - FontInfo struct with face_name, size, weight, italic, charset, pitch_and_family
  - FontBuffer with FIFO replacement (max 16 fonts), duplicate detection
  - FontError enum (IndexOutOfBounds, NotFound, Empty)
  - 6 unit tests passing
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
- [x] `winterm/crates/adapter/src/sixel_parser.rs` — Sixel parsing
  - SixelParser with state machine (Normal/Attributes/Color/Repeat states)
  - Color table management (256 colors), image buffer, dimensions tracking
  - Static methods: cell_size_for_level(), max_colors_for_level()
  - Command/parameter parsing, soft reset, display mode control
  - 8 unit tests passing
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
- [x] Font operations — covered in font_buffer.rs tests (add/duplicate/clear)
- [x] Sixel parsing — covered in sixel_parser.rs tests (state transitions, parameters)
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

- [ ] `winterm/crates/renderer/src/render_engine.rs` — `IRenderEngine` trait
  - [ ] **Tests:** all virtual methods called correctly, trait object safety
- [ ] `winterm/crates/renderer/src/render_data.rs` — `IRenderData`
- [ ] `winterm/crates/renderer/src/render_settings.rs` — `RenderSettings`
- [ ] `winterm/crates/renderer/src/render_frame_info.rs` — `RenderFrameInfo`
- [ ] `winterm/crates/renderer/src/render_engine_base.rs` — `RenderEngineBase`
- [ ] `winterm/crates/renderer/src/cluster.rs` — `Cluster` (per-glyph shaping result)
- [ ] `winterm/crates/renderer/src/font_info.rs` — `FontInfo`
- [ ] `winterm/crates/renderer/src/font_info_desired.rs` — `FontInfoDesired`
- [ ] `winterm/crates/renderer/src/font_cache.rs` — LRU font cache
  - [ ] **Tests:** add/get/evict, LRU eviction order, capacity enforcement, hit rate
  - [ ] **Property tests:** with infinite capacity, cache never evicts; with capacity N, cache holds at most N entries
- [ ] `winterm/crates/renderer/src/font_resource.rs` — Font resource loading
- [ ] `winterm/crates/renderer/src/css_length.rs` — CSS length parsing
  - [ ] **Tests:** pixel, percentage, em, rem parsing, validation
- [ ] `winterm/crates/renderer/src/cursor_options.rs` — `CursorOptions`
- [ ] `winterm/crates/renderer/src/composition.rs` — IME composition state

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

### Core Console State

- [ ] `winterm/crates/host/src/console_info.rs` — `ConsoleInformation`
  - [ ] **Tests:** allocate/free console, set title, lock/unlock, VT mode toggle, cooked read, clipboard, bracketed paste
  - [ ] **Property tests:** locking prevents concurrent access; releasing cooked read restores input buffer
- [ ] `winterm/crates/host/src/settings.rs` — `Settings`
- [ ] `winterm/crates/host/src/screen_info.rs` — `ScreenBuffer`
  - [ ] **Tests:** creation/destruction, resize (traditional + reflow), buffer switching, viewport management, scroll bar state, cursor visibility, font updates
  - [ ] **Property tests:** resizing buffer preserves content within bounds; switching to alternate buffer then back restores main buffer state
- [ ] `winterm/crates/host/src/input_buffer.rs` — `InputBuffer`
  - [ ] **Tests:** cooked/raw/vt mode switching, read events, popup, line editing
- [ ] `winterm/crates/host/src/cooked_read.rs` — `CookedReadData`
  - [ ] **Tests:** line edit (insert/delete/overwrite), arrow key navigation, history recall
- [ ] `winterm/crates/host/src/scroll_history.rs` — `CommandHistory`
  - [ ] **Tests:** add/get/clear, cycle through history, search history, duplicate suppression
  - [ ] **Property tests:** cycling through history returns to most recent command
- [ ] `winterm/crates/host/src/handle.rs` — Handle management
- [ ] `winterm/crates/host/src/output_stream.rs` — Output stream
- [ ] `winterm/crates/host/src/scrolling.rs` — Scroll logic
- [ ] `winterm/crates/host/src/selection.rs` — Selection state
- [ ] `winterm/crates/host/src/input_key_info.rs` — Keyboard state
- [ ] `winterm/crates/host/src/direct_io.rs` — Direct I/O
- [ ] `winterm/crates/host/src/registry.rs` — Registry access
- [ ] `winterm/crates/host/src/utils.rs` — Console utilities
- [ ] `winterm/crates/host/src/tracing.rs` — Event tracing
- [ ] `winterm/crates/host/src/alias.rs` — Command aliases

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

- [ ] `winterm/tests/conformance/csi/sgr.rs` — All SGR codes
- [ ] `winterm/tests/conformance/csi/cursor.rs` — All cursor movement commands
- [ ] `winterm/tests/conformance/csi/scroll.rs` — All scroll commands
- [ ] `winterm/tests/conformance/csi/erase.rs` — All erase commands
- [ ] `winterm/tests/conformance/csi/insert_delete.rs` — IL/DL/ICH/DCH
- [ ] `winterm/tests/conformance/osc.rs` — All OSC sequences
- [ ] `winterm/tests/conformance/dcs/sixel.rs` — All Sixel sequences
- [ ] `winterm/tests/conformance/private_modes.rs` — All DECSET/DECRST
- [ ] `winterm/tests/conformance/mouse.rs` — All mouse tracking modes
- [ ] `winterm/tests/conformance/kitty_keyboard.rs` — All KKP sequences
- [ ] `winterm/tests/conformance/bracketed_paste.rs` — Bracketed paste
- [ ] `winterm/tests/conformance/win32_input_mode.rs` — Win32 Input Mode
- [ ] `winterm/tests/conformance/escape.rs` — All escape sequences
- [ ] `winterm/tests/conformance/vt52.rs` — All VT52 commands
- [ ] `winterm/tests/conformance/hyperlink.rs` — OSC 8 hyperlinks
- [ ] `winterm/tests/conformance/generate.rs` — Fuzz-generated sequences against reference

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
