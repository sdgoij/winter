# ADR 049 — The WinUI settings window is a modal dialog over the shared draft

- Status: Accepted
- Date: 2026-11-22
- Related: `crates/winui/src/settings.rs`, `crates/winter/src/config.rs`,
  `crates/settings/src/parser.rs`; ADR 025, ADR 028, ADR 030–032

## Context

The egui shell has edited the settings file since ADR 028, growing to cover
profiles, keybindings, schemes and themes (ADRs 030–032). The WinUI shell read
the file at startup (ADR 044) and reloaded a saved one (ADR 025), but
`openSettings` fell through its `perform` match: Ctrl+, did nothing on Windows.

`config::Draft` is the toolkit-free model the egui window already uses. It owns
normalization, validation, the document a save writes and the preview. A second
window could reuse it, or grow a second model that would drift from the first.

## Decision

- The WinUI settings window is a `ContentDialog` attached to the window's root
  view. `openSettings` (Ctrl+, by default) opens it; its Close button dismisses
  it, and closing discards unsaved edits.
- Both shells use `config::Draft`. Save writes through `settings::save`; Revert
  and the live poll read through `settings::load_with_report`. The document is
  therefore the same one the egui window writes.
- An edit is a `SettingsMessage` applied to the draft. Save and Revert are
  commands for the shell, because they rebuild the face, re-apply the scheme
  and resolve the next tab's profile.
- A dirty draft is not replaced by the poll: the dialog shows a conflict and
  Save overwrites the file while Revert loads it.
- The low-level wheel hook is gated while the dialog is open. The dialog's own
  `ScrollViewer` gets the wheel and the terminal behind it stays put.

## Rationale

One model with two renderers is the project's one-home rule applied to the
editor. A `ContentDialog` is the native modal: it gives keyboard and pointer
isolation from the terminal without a second window, component or shutdown
path. Save and Revert as in-content buttons, rather than the dialog's primary
button, keep the dialog open when a save fails so the error stays visible.

## Consequences

- The dialog is modal; the terminal cannot be typed into while it is open.
- The WinUI editor has no separate swatch control. A colour field is followed
  by a coloured glyph, which is enough to see a typo but not a picker.
- The dialog is limited by the window's height, so its scroller has a fixed
  480-DIP height; the editor grows by scrolling.
- The `openSettings` chord cannot close the dialog because the modal consumes
  the key; the Close button is the exit.

## Alternatives considered

- **A second WinUI `Window`.** More lifecycle, focus hand-off and shutdown
  handling for an editor that should block the window anyway.
- **A WinUI-only draft.** Duplicates normalization, validation and the preview,
  and two models would disagree the first time one grows a field.
- **Save as the dialog's primary button.** A failed save closes the dialog
  before the error can be read; the in-content button keeps it open.
