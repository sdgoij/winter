# ADR 040 — The Windows shell is WinUI 3 through windows-reactor

- Status: Accepted; amended by ADR 041 (surface) and ADR 042 (binding)
- Date: 2026-11-22
- Related: `crates/winterm/src/{app,terminal_widget,config,window,icon}.rs`,
  `crates/renderer`; ADRs 006, 007, 009, 018, 038, 039

## Context

ADR 039 proposed making the existing egui window look native: a drawn caption,
the toolkit's dark theme, and one `win32` module for what the toolkit could not
express. It was rejected before acceptance, and the objection is the right one.
A caption and a theme are not a native UI: egui's widgets are drawn imitations,
and the requirement is the controls themselves.

Upstream's shell is XAML throughout: `App.xaml`, `TerminalPage.xaml`,
`TabRowControl.xaml`, `TabHeaderControl.xaml`, `CommandPalette.xaml`,
`TitlebarControl.xaml`, `MinMaxCloseControl.xaml`, `Toast.xaml`, and
`TerminalSettingsEditor/`. The controls are WinUI 3 (`Microsoft.UI.Xaml`).

Rust has a first-party route to the same stack. `windows-reactor` is published
from `microsoft/windows-rs` by Kenny Kerr, the windows-rs lead, as "Build native
WinUI 3 applications with Rust components, state, and typed messages". It is
75,570 lines of Rust, edition 2024, version 0.100.0, and marked preview. It uses
an installed Windows App Runtime, or stages one with `windows-reactor-setup`;
this machine already has the runtime installed (1.1 through 1.8 and 2.x).

A spike settled the open question. The readme's Counter sample, pinned to 0.100
and with one line changed for the older API, built and ran:

- `cargo build` succeeded, and the probe binary linked in 0.87 s after the
  dependency tree was built.
- `App::run_component::<Counter>` started a real window: `MainWindowHandle` was
  non-zero and `MainWindowTitle` was `"Counter"`.
- The window carried the Windows 11 caption and two Fluent 2 buttons: native
  controls, not drawn imitations.
- API note: 0.100 takes the root `View` as `view`'s return value and exposes
  `ViewContext::window_title`; the readme's `window_frame` is 0.101/master. The
  pin decides which API a commit uses.

Nothing from the probe is committed.

The public surface maps onto the upstream files closely: `TabView` and
`TabViewItem` with reorder and close events, `TitleBar` with left and right
headers, `NavigationView` and `NavigationViewItem` for the settings editor,
`AutoSuggestBox`, `TextBox`, `NumberBox`, `ComboBox`, `CheckBox`,
`ToggleSwitch`, `ColorPicker`, `Expander`, `InfoBar`, `ContentDialog`,
`MenuBar`, `CommandBar`, `Flyout`, `Tooltip` and `SwapChainPanel`. Window
appearance is `WindowVisuals` with `theme`, `backdrop` (Mica, MicaAlt, Acrylic),
`icon`, `constraints` and `initial_placement`. Input is `KeyEventInfo`,
`CharacterEventInfo` and `PointerEventInfo`; the lifecycle is
`Component::{create, update, view}` with `spawn_background` and `set_timeout`.

## Decision

1. **The Windows shell is WinUI 3 through `windows-reactor`.** egui is not
   underneath it, and no hand-written Win32 host is written.
2. **The dependency is Windows-only and confined.** `windows-reactor` goes in
   `[target.'cfg(windows)'.dependencies]` and is named only inside
   `crates/winterm/src/winui/`. Nothing below the shell sees it.
3. **One module per upstream XAML file**, as the port does everywhere else:
   `terminal_page`, `tab_row`, `titlebar`, `palette`, `settings_editor` and
   `app`, each naming its XAML source.
4. **The terminal content is painted by `renderer`, not by WinUI text.** A
   `RenderData` implementation exposes the terminal's buffer, viewport, font,
   cursor and selection, and `Renderer::paint_frame` runs the existing paint
   path. The first surface is an `Image` whose `SoftwareBitmapSource` is filled
   from `SoftwareBackend`'s ARGB buffer; the durable surface is a
   `SwapChainPanel` with a D3D11 backend, which is the row ADR 007 deferred.
5. **Input gets a toolkit-free vocabulary first (ADR 009).** `KeyEventInfo`,
   `CharacterEventInfo` and `PointerEventInfo` are translated into it exactly as
   egui's events are today. `Control::send_key` and `Control::send_mouse` are
   the consumer either way.
6. **egui remains the shell on Linux and macOS** until their native shells
   land. `main` selects the shell at compile time. `session`, `term_control`,
   `grid`, `view`, `tabs`, `layout`, `blink`, `term_events` and `term_settings`
   name no toolkit and are shared.
7. **The settings drafts leave `config.rs`.** `Draft`, `ProfileDraft`,
   `BindingDraft`, `SchemeDraft` and `ThemeDraft` become a toolkit-free module,
   so both editors edit the same model.
8. **No `unsafe` in the shell.** The COM and XAML plumbing belongs to
   `windows-reactor`.

## Rationale

- **It is the oracle's own stack.** The module map is the XAML file list, so
  upstream's behaviour and control semantics transfer rather than being
  approximated.
- **It is first-party.** The workspace's "no dependency without a reason" rule
  is met by the reason being the entire UI layer; the alternative is writing
  WinUI by hand.
- **Native controls bring what egui cannot**: Fluent styling, UIA
  accessibility, IME composition windows, drag and drop, and the platform's
  animations, without this repository implementing them.
- **The seam already exists.** ADRs 018 and 038 promised that the shell could be
  replaced without touching the emulation core; this is that replacement, and
  the 1,576 tests stay where they are.
- **The renderer finally has a consumer.** Painting through `RenderData` is the
  path ADRs 006 and 007 designed, and the software backend is already tested.

## Consequences

Positive:

- Windows gets the same toolkit as Windows Terminal, from safe Rust.
- The emulation core, the pty and the console host are unchanged.
- The buffer snapshot and the settings model are shared with the egui shell.

Negative:

- **The Windows App Runtime becomes a prerequisite.** It is installed here;
  packaging must either require it or stage it with `windows-reactor-setup`
  (Phase 13).
- **The crate is a preview.** Pin `0.100`; moving to the readme's `0.101` API is
  a deliberate change, not an incidental one.
- **Two shells exist until Linux and macOS get theirs.** The tab row and the
  settings editor are written twice; the shared drafts and input vocabulary
  keep that duplication to drawing.
- **The first content path copies a frame.** Software rendering into a bitmap is
  correct but not fast; the `SwapChainPanel` and D3D11 backend is the
  performance milestone and adds a confined `unsafe` module.
- **The gate still runs on three platforms.** The WinUI shell is
  `cfg(windows)`; Linux and macOS compile the egui shell, and CI must keep
  proving that.

## Alternatives considered

| Alternative | Why not |
|---|---|
| Native chrome on the egui window (ADR 039) | Rejected: drawn controls are not native, which was the requirement |
| Hand-written Win32 common controls | Native, but renders like Windows 10 rather than WinUI, and reimplements the tab row, palette and settings editor |
| WinUI 3 through the `windows` crate by hand | That is what windows-reactor is, with Microsoft maintaining the projection and the reconciler |
| `naui`, one API over WinUI 3, AppKit and GTK4 | Attractive for a single cross-platform shell, but young (0.18) and third-party; the port is taking one platform at a time |
| Tauri or another WebView shell | Web content is not native controls |
| Keep egui permanently | The stated target is native UI, and egui's ceiling is why this record exists |

## Slices

| # | Slice | Touches |
|---|---|---|
| 1 | Toolkit-free input vocabulary and settings drafts | `winterm` only, tested |
| 2 | `winui` skeleton: window, `TitleBar`, `TerminalPage`, one tab, an `Image` fed by the software backend, keys to `Control` | `winui` |
| 3 | `RenderData` over `Control`, painting through `Renderer::paint_frame` | `winterm`, `renderer` consumer |
| 4 | `TabView` row with close and reorder; command palette | `winui` |
| 5 | Settings editor on `NavigationView` and the shared drafts | `winui` |
| 6 | `SwapChainPanel` and a D3D11 backend (ADR 007) | `renderer`, `winui`, one `unsafe` module |
