# ADR 008 — Fonts are found by scanning directories

- Status: Accepted
- Date: 2026-11-22
- Related: `crates/renderer/src/font_cache.rs`, `atlas/font.rs`,
  `src/renderer/base/FontCache.h`; ADR 001, ADR 006, ADR 007

## Context

Upstream asks DirectWrite for the system font collection and caches it in
`FontCache.h`. It also lets `.ttf` files in the executable's directory take
precedence: they are added to a font-set builder first, and the builder ignores
any family already present, so a shipped font shadows the installed one.

None of that is portable. DirectWrite exists only on Windows, and the collection
is the only way upstream learns a family name, a style, and whether a face is
fixed-pitch. A cross-platform renderer needs the same three facts and a way to
open the file.

The available alternatives are a font-discovery crate (`fontdb`, `font-kit`)
or a font database service (`fontconfig` on Unix). Both bring configuration and
hinting policy that a terminal does not use, and `fontdb` parses with
`ttf-parser`, the unmaintained crate that already forces one `RUSTSEC` ignore in
`deny.toml`. Neither can be tested without the host's fonts.

## Decision

A font collection is an ordered list of directories supplied by the caller,
scanned by the renderer, with a small reader for the four SFNT tables a face
choice needs.

- **The host supplies directories.** The renderer never names a platform path
  and contains no `cfg(windows)`/`cfg(unix)`, so locating fonts belongs to the
  same layer as the window and the pty (`FontCache.h`'s job stays with the
  cache; the OS knowledge moves to the host).
- **Nearby fonts win.** A `.ttf` beside the executable shadows an installed
  face of the same family and style, as upstream's font-set builder does.
  System directories are scanned recursively; the nearby directory is not.
- **The renderer reads `name`, `post`, `OS/2` and `head` only.** Family and
  subfamily come from `name`; `post.isFixedPitch` says whether a face is
  monospace; `OS/2` supplies weight and italic; `head.macStyle` is the fallback
  for italic. Glyph data is not parsed here and stays with the rasteriser.
- **`ttcf` collections are supported.** Each face is enumerated with its
  collection index, which is exactly what `fontdue`'s `collection_index`
  expects.
- **A family's styles are separate entries.** Upstream sees one family with a
  style collection; a directory scan sees `Regular`, `Bold` and `Italic` files.
  The dedupe key is therefore family *and* style, so a nearby `Regular` shadows
  the installed `Regular` without hiding the installed `Bold`.
- **No font-discovery dependency.** `cargo-deny` stays as it is and the parser
  is exercised by synthetic fonts built in tests.

## Rationale

- **Testable without the host.** Every table read is a function of bytes, so a
  synthetic face proves the behaviour; no test needs an installed font.
- **Bounded and total.** Every offset and length is checked; malformed input
  yields "no face", never a panic. A font file is attacker-controlled input in
  the same sense a pty stream is.
- **The renderer stays platform-independent.** ADR 006 already keeps platform
  code out of this crate; a hard-coded font directory would put it back.
- **No policy we do not honour.** `fontconfig` matching, font substitution and
  hinting settings are not behaviour this terminal implements, so depending on
  them would buy a failure mode, not a feature.

## Consequences

Positive:

- The collection builds from any directory list, including a temporary one in a
  test or a bundled font beside the binary.
- `fontdue` opens a discovered face directly; no adapter layer between them.
- The only dependency cost is the existing `fontdue`.

Negative:

- No `fontconfig` aliases: a request for a family that is not installed falls
  back to the preference list in `font_cache`, not to the system's substitution
  rules.
- Only the four tables are read. A `.otc` face whose family name lives in an
  unusual table is skipped rather than guessed at.
- The host must supply the directories before a frame can draw text; until the
  host layer exists, callers build a collection themselves.

## Alternatives considered

| Alternative | Why not |
|-------------|---------|
| Use `fontdb` | Parses with unmaintained `ttf-parser`; its discovery features are not used by the renderer |
| Use `fontconfig` on Unix and DirectWrite on Windows | Two implementations, two behaviours, one of them untestable here |
| Keep DirectWrite behind `cfg(windows)` | Puts platform code in the renderer and leaves Unix with no font path at all |
| Require the settings to name a font file | A terminal whose default font must be configured before its first frame is not a usable default |
| Bundle a font in the crate | Adds megabytes to every build and dilutes the port; nearby fonts already cover the case |
