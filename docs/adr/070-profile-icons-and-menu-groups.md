# ADR 070 — A profile without an icon wears the shell's, and the menu is grouped

- Status: Accepted
- Date: 2026-11-24
- Related: `crates/winter/src/winui/profile_icon.rs`,
  `crates/winter/src/winui/app.rs`, `crates/winter/src/winui/settings.rs`,
  ADR 066, ADR 052

## Context

ADR 066 gave every profile row and every tab the icon of the program it runs.
A profile whose `command` is not a Windows executable with an icon — `bash`,
a script, a command that resolves to nothing on this machine — got no image at
all, so its row was a bare name among rows with icons, and its tab was a title
among tabs with icons.

The menu also had no grouping: the profiles and the two window commands were one
undifferentiated list, and `Settings` and `About` had no glyph of their own even
though Windows Terminal's flyout gives every row an icon. The About dialog said
nothing about where the source lives.

Separately, the settings dialog's content was a `ScrollViewer` with a fixed
480-DIP height. A `ContentDialog` sizes its content to what the window can give
it; when that is less than 480, the content runs under the dialog's own Close
button and the Apply row is painted over.

## Decision

- `profile_icon::png_for` falls back to the icon this executable carries (its
  own resource, read once) when the profile's program has none. The rule is in
  `lookup`, which takes the fallback as an argument so a test can supply its
  own — the test binary is not the executable the icon is embedded in.
- The drop-down's commands take the symbol font's own glyphs: `U+E713` (gear)
  for `Settings` and `U+E897` (question mark) for `About`, each in a square the
  size of an icon so every label starts at the same x.
- One hairline separates the last profile from `Settings`. It is inserted once,
  before the first non-profile row, and not at all when there are no profiles.
- The About dialog links to the repository with a `HyperlinkButton`, which
  navigates the URI itself. The shell does not call `ShellExecute` for it.
- The settings dialog's `ScrollViewer` gets `max_height` (420 DIP) instead of a
  fixed `height`. The content is then never forced taller than the dialog can
  show, and it still scrolls when the lists grow.

## Rationale

A missing image is a visible hole in a list of images. Falling back to the
shell's own icon keeps every row and tab uniform, and it is the honest answer:
the row starts *this* application's shell, not a program of its own.

`max_height` instead of `height` is the difference between "at most this tall"
and "exactly this tall". The dialog owns the outer budget, so the inner scroller
must be allowed to be shorter than its cap.

## Consequences

- A profile with no extractable icon looks like a Winter tab rather than a
  blank one. It also means `png_for` never returns `None` in a running shell,
  which is what the strip and the menu now assume.
- The fallback reads `current_exe`, so a build whose executable carries no icon
  (a test binary, an unusual packaging) gets no fallback — the tab is simply
  without an image, as before.
- The menu's two commands are glyphs, not images, so they cannot be replaced by
  a program's icon without changing `menu_row`'s leading argument.
- The separator is a `Border` with a 1-DIP height rather than a `MenuFlyout`
  separator, because the binding has no `MenuFlyout` (ADR 052).
- The About link opens the user's browser. It is one click and it navigates
  away; there is no confirmation.

## Alternatives considered

- **Leave the row blank.** It is the one row that looks broken rather than
  plain.
- **Draw a generic terminal glyph for every profile without an icon.** A
  second invented icon next to real ones is worse than the shell's own.
- **Give the settings dialog a taller fixed height.** The window decides how
  much room a dialog gets; the fixed number is what caused the overlap.
- **Move Apply and Revert into the dialog's primary and secondary buttons.**
  They are draft edits with a preview, and the dialog's own buttons report a
  result, not an edit; the shell would lose the "this save is blocked" state.
