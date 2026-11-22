# ADR 058 — The icon is drawn once and embedded at build time

- Status: Accepted
- Date: 2026-11-22
- Related: `crates/winterm/src/icon_draw.rs`, `crates/winterm/src/icon.rs`, `crates/winterm/build.rs`, `crates/winterm/src/winui/icon.rs`, ADR 038, ADR 055

## Context

The egui shell draws its window icon instead of shipping a file: a dark rounded
square with a prompt chevron and an underscore, 64×64 (ADR 038). The Windows
build carried no icon resource at all, so Explorer and the taskbar showed the
generic executable icon. Embedding a resource is not enough for the WinUI
shell: its window keeps a framework default, so the taskbar button ignored the
executable's icon.

Two things had to meet: the drawing must stay the single source, and the
Windows build must carry it both as a file resource and as the window icon.

## Decision

- `icon_draw.rs` holds the geometry and two encoders: `rgba(size)` for a window
  toolkit, and `ico(sizes)` for the Windows resource. It has no crate imports,
  so `build.rs` can include it with `#[path]` and compile the same pixels the
  library uses.
- `icon.rs` wraps `rgba(SIZE)` in `eframe`'s `IconData`; the egui shell is
  unchanged.
- `build.rs` writes an ICO with 16, 32, 48, 64 and 256 pixel images into
  `OUT_DIR` and embeds it through the `embed-resource` build dependency. Each
  image is an uncompressed 32-bit DIB, which every Windows version reads.
- `winui::icon` loads the embedded group (`LoadImageW`) and hands it to the
  window with `WM_SETICON` for both the big and small slots when
  `run_window` reports the handle. This is the module's one `unsafe` site, and
  it carries a `SAFETY:` note.
- `just dist` packages the same executable, so the release folder's icon is
  the drawing too.

## Consequences

- The taskbar, Alt-Tab and explorer entries show the drawn prompt; a
  prebuilt `.ico` cannot drift from the egui window icon because there is no
  prebuilt file.
- `embed-resource` is a build-time dependency, not a linked one. It is needed
  only when the target is Windows.
- A test drives the ICO header and entries and the pixel drawing, so a broken
  encoder fails `just test` rather than only showing up in Explorer.
- The `unsafe` budget gains `winui::icon` beside `surface` and `wheel`
  (ADR 016's pattern: a named module with a per-call `SAFETY:` note).

## Alternatives considered

- **Commit a `.ico` file.** It would be the only binary asset in the tree, and
  the egui icon would drift from it.
- **`AppWindow.SetIcon`.** It needs a file path, and the bound `windows` crate
  carries no `Microsoft.UI.Windowing` namespace; writing an icon file beside
  the executable just to read it back is worse than the resource.
- **Embed the resource without `WM_SETICON`.** Verified live: WinUI 3's
  taskbar button kept its default icon, so the resource alone is not enough.
- **Load the icon from a font glyph or XAML.** The window icon is a Win32
  concept; XAML has no window-icon property.
