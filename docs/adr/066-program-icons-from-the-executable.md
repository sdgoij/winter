# ADR 066 — A profile's icon is taken from its program's executable

- Status: Accepted
- Date: 2026-11-24
- Related: `crates/winter/src/winui/profile_icon.rs`,
  `crates/winter/src/winui/png.rs`, `crates/winter/src/winui/app.rs`, ADR 064,
  ADR 058

## Context

Windows Terminal shows a shell's own icon on its tab and beside its name in the
profile menu. The strip is hand-built (ADR 064), so the icon has to be set on a
control the shell creates; `windows-reactor-pre` 0.108 exposes no `IconSource` on
a `Border` or a `Button`'s content, and its `Icon` type is only accepted by the
controls that take one (`TitleBar`, `NavigationViewItem`, `AppBarButton`).

What the binding does have is `ImageSource`, with four ways to name an image: a
URI, a file, encoded bytes, or an SVG document. A program's icon exists only
inside the executable's resource section, so it has to become bytes.

## Decision

- `winui::profile_icon` finds a profile's program on `PATH` (or takes the path a
  profile wrote), asks `PrivateExtractIconsW` for its 32-pixel icon, reads the
  colour bitmap through `GetDIBits` and hands the `RGBA` grid to `winui::png`.
- `winui::png` writes a PNG with stored deflate blocks and unfiltered rows. It
  is the smallest writer that produces a file every Windows release decodes, it
  has no dependency, and its two checksums are checked against the vectors their
  specifications print.
- The icon bytes are cached per program for the life of the process. `view` runs
  every frame; extraction costs a resource lookup and a device-context round
  trip, which is not something to repeat per tab per frame.
- A tab keeps the `program` it started, and the strip puts a 16-DIP `Image`
  before its title. The drop-down's profile rows put the same image before the
  name.
- A program with no icon, no executable on this machine, or an extraction that
  fails, is a tab with a title and no image. The row is not hidden and the
  terminal still starts.

## Rationale

The alternative to all of this is a drawn glyph or nothing. A glyph would be
honest but wrong: Windows Terminal shows the real icon, users recognise `cmd` by
it, and a WSL distribution's icon is the only cue its menu row has.

PNG over a raw bitmap because XAML's decoder is WIC, and WIC's BMP path treats a
32-bit alpha channel inconsistently depending on the header version. PNG's
alpha is unambiguous. Stored deflate because a 32×32 icon is four kilobytes and
compressing it would mean a compression dependency for no measurable gain.

## Consequences

- `winui::profile_icon` is a ninth permitted `unsafe` module: `PrivateExtractIconsW`,
  `GetIconInfo`, `GetObjectW`, `CreateCompatibleDC`, `GetDIBits`, `DeleteDC`,
  `DeleteObject` and `DestroyIcon`, each with its own `SAFETY:` note.
- An icon from before the alpha channel existed comes back fully transparent and
  is treated as opaque, so its corners are square rather than cut out. Reading
  the icon's mask is the fix if one of those shells shows up.
- The icon is extracted at 32 pixels because a tab's slot is 16 DIP. A 300%
  display stretches it slightly; asking for 256 would be a megabyte through the
  writer for a 16-DIP slot.
- The icon does not follow a scheme change, a theme change or an OSC 11 reply:
  it is the program's, not the palette's.
- A WSL distribution's row shows `wsl.exe`'s icon, not the distribution's own
  logo from its Appx manifest. Upstream reads that manifest; doing the same is a
  change of its own.
- The extraction is confirmed by a test that runs `cmd.exe`'s icon through the
  writer and reads the dimensions back. What it cannot confirm is that the
  pixels look like the icon, which is why the manual check is still the shell.

## Alternatives considered

- **An SVG glyph per shell family.** No `unsafe`, no writer, no caching — and a
  menu of invented shapes rather than the icons the user already knows.
- **`SHGetFileInfo` with `SHGFI_ICON`.** The same `HICON` and the same bitmap
  reading, plus the shell's icon cache; it also returns a 32-pixel image and does
  not let the size be chosen, so it buys nothing.
- **A `.png` per program written beside the settings file.** Icons would then be
  stale when a shell is updated, and a machine's file set would grow for
  something a cache in memory already solves.
- **A compression dependency for the PNG.** Stored blocks are spec-compliant
  deflate, the images are tiny, and the workspace has no PNG writer to reuse.
