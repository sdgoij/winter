# ADR 035 — A theme paints the chrome the shell owns

- Status: Accepted
- Date: 2026-11-22
- Related: `crates/settings/src/themes.rs`, `crates/winterm/src/ui.rs`;
  ADRs 022, 024, 032, 034

## Context

Upstream's `Theme.idl` describes a named set of window colours: a requested
light or dark mode, Mica, a rainbow frame, a frame colour and an unfocused
frame, the tab row's fills, the tabs' fills, a close-button visibility, an icon
style, and a theme for the settings page. It is a WinUI description of a window
this shell does not have: egui draws a tab strip, the panes, and windows.

ADR 022's rule is that a schema holds what is consumed. Porting the whole
`Theme` would add a dozen fields that nothing reads.

## Decision

- **A theme is four fills.** `tab_row.background`,
  `tab_row.unfocused_background`, `tab.background` and
  `tab.unfocused_background`, each optional, in `themes.rs`. That is the chrome
  egui draws.
- **`theme` names one**, and an unknown name leaves the shell's own colours
  (a `Warning`, never a fallback to another theme — the same rule schemes
  follow, ADR 024).
- **A missing unfocused fill falls back to the focused one**, so a one-colour
  theme does not change when the window loses focus.
- **The shell paints it.** `tab_bar` fills the tab row's panel with the row
  colour for the window's focus state, and each tab with its own, picking a
  black or white label by luminance because a theme names backgrounds only.
- **The settings window edits it**: a theme selector, and a name and four
  colour fields per theme, with the same empty-means-shell and
  bad-text-blocks-Save rules as a scheme (ADR 032).

Not ported, because nothing here draws them: `requestedTheme`, `useMica`,
`rainbowFrame`, `window.frame`/`unfocusedFrame`, `settings.requestedTheme`,
`tab.showCloseButton` and `tab.iconStyle`.

## Rationale

- **A field the window does not paint is a field that drifts.** The four fills
  are exactly what the tab strip sets from them; a test drives each through the
  file and the draft.
- **Contrast is the shell's job.** Upstream's `ThemeColor` can be an accent or
  the terminal background; the label colour is computed there too. Naming only
  fills keeps the file simple, and the luminance cut-off is the one place to
  change if it reads badly.
- **The fallback is the file's convenience, not a guess.** "Use the same colour
  unfocused" is what a one-colour theme means, so it needs no warning.

## Consequences

- A theme can recolour the tab strip and nothing else. The window frame, Mica,
  the settings page and the close button keep egui's own styling until a native
  window backend exists (ADR 018).
- The theme is read every frame, so a reload repaints the strip without a
  message.
- Adding a fill later is a field in `themes.rs`, a helper, and a row in the
  editor; the test that drives every fill through the draft keeps the three in
  step.

## Alternatives considered

| Alternative | Why not |
|---|---|
| Port the whole `Theme` schema | Most of it names WinUI concepts with no drawing code here |
| Recolour `egui::Visuals` globally | It would repaint menus, windows and the settings page with the terminal's chrome colours |
| Let the theme name a foreground as well | Two more fields per fill, and the label is not a terminal colour a user thinks in |
| Store the theme on a profile, as upstream does | This shell has one window; a per-tab theme is a decision for when tabs differ in more than their shell |
