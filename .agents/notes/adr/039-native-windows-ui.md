# ADR 039 — Native UI starts with the Windows window, not a new toolkit

- Status: Proposed
- Date: 2026-11-22
- Related: `crates/winterm/src/{app,terminal_widget,config,window,icon}.rs`,
  `crates/winterm/Cargo.toml`; ADRs 007, 009, 016, 018, 038

## Context

ADR 018 chose egui on winit and wgpu as the first shell and deferred the four
hand-written window backends. The shell now runs a real pty, and ADR 038 split
it into the modules that name a toolkit. The next request is native UI, starting
with Windows.

Two facts decide the shape of the work:

1. **The window is already native.** eframe creates a `winit` window, and on
   Windows that is a real `HWND` with a real window class, a real message loop,
   DPI handling, IME, clipboard and `FlashWindowEx` attention. egui draws
   *inside* it. "Native UI" therefore cannot mean "get a native window"; it
   means "make the window and its chrome look and behave like Windows".
2. **The toolkit already reaches most of the chrome.** `eframe::Frame`
   implements `HasWindowHandle`, so the shell can obtain its `HWND`.
   `egui::ViewportCommand::SetTheme(SystemTheme::Dark)` makes egui-winit call
   `winit::Window::set_theme(Some(Dark))`, which is `SetWindowTheme` plus the
   undocumented `SetWindowCompositionAttribute` — the dark title bar, on
   Windows 10 1809 and later. `StartDrag` and `BeginResize` are egui commands
   that reach `winit::Window::drag_window` and `drag_resize_window`, so a
   caption we draw can still drag and resize the window. `with_icon` sets the
   taskbar icon from `icon::data()`. `RequestUserAttention` is `FlashWindowEx`.

Upstream is the other half of the answer. Windows Terminal does not use the
OS caption either: it extends its content into the title bar and puts the tab
strip there (`ExtendsContentIntoTitleBar`). A caption we draw is not a
departure from the oracle; it is the oracle's own model.

What egui cannot express is a small, bounded set: Windows 11 rounded corners and
border once the standard frame is gone, the native system menu, and later the
placement of an IME candidate window. Those are `DwmSetWindowAttribute` and
`TrackPopupMenu` calls, and nothing more.

## Decision

1. **"Native" means native appearance, not a native toolkit.** The shell keeps
   one widget set (egui) and gains a Windows-looking window. A second, WinUI
   shell is not built.
2. **The window draws its own caption.** `ViewportBuilder::with_decorations(false)`
   plus a caption module: the tab strip moves into the caption, as upstream's
   does; dragging and edge resizing are `ViewportCommand::StartDrag` and
   `ViewportCommand::BeginResize`, so no `WM_NCCALCSIZE` is hand-rolled.
3. **Native appearance is asked for through the toolkit first.** The dark frame
   is `ViewportCommand::SetTheme(SystemTheme::Dark)`, the icon is `with_icon`,
   the bell is `RequestUserAttention`, the last tab is `Close`. These add no
   `unsafe` and no dependency.
4. **What the toolkit cannot express goes in one module.**
   `crates/winterm/src/win32.rs` is `#[cfg(windows)]`, carries a module-level
   `#[allow(unsafe_code)]`, and every call carries a `SAFETY:` note — the shape
   ADR 016 gave the pty backends. It holds the DWM corner and border
   attributes and the system menu. No other module gains `unsafe`.
5. **No WinUI, no XAML, no `windows` crate, no MSIX.** The application must
   keep building and running from `cargo build` on a machine with no Windows
   App SDK installed.
6. **Input translation gets a toolkit-free vocabulary before a second host
   exists.** ADR 009 already says input translation takes portable events and a
   mode snapshot; the key map currently takes `egui::Event`. A neutral
   `winterm::input` vocabulary, with `terminal_widget` translating egui into
   it, is the piece a later host reuses instead of re-deriving.
7. **A full Win32 host and a `renderer`-driven GPU content pass are not decided
   here.** They get their own ADR when the chrome work is done, and only if the
   evidence calls for them.

The slices, in order:

| Slice | Content | New `unsafe` | New dependency |
|---|---|---|---|
| 0 | Dark native frame; icon and attention already work | no | no |
| 1 | Drawn caption: tab strip in the caption, drag and edge resize | no | no |
| 2 | `win32.rs`: Win11 corners and border, native system menu | yes, one module | `raw-window-handle`, DWM feature |
| 3 | `winterm::input`: neutral events, egui translates into them | no | no |

## Rationale

- **The cheapest change with the largest perceived effect, and it is
  reversible.** Slices 0 and 1 touch the shell only; `session`, `grid`, `term`
  and the 1,576 passing tests do not move.
- **It matches the oracle.** Upstream draws its caption and puts tabs in it. The
  port inherits that decision rather than inventing a different one.
- **`unsafe` stays a review event.** Slice 2 is the only new `unsafe`, it is
  confined to one `cfg(windows)` module with per-call `SAFETY:` notes, and it is
  additive: deleting the module restores the standard frame.
- **Cross-platform is unaffected by construction.** `win32.rs` is compiled only
  on Windows; `window.rs` keeps its viewport arms everywhere else, and Linux and
  macOS keep `decorations(true)` until their own chrome lands.
- **No runtime dependency is imposed on users.** WinUI 3 would make the Windows
  App SDK a prerequisite; that is a packaging decision the project has not
  made (Phase 13).

## Consequences

Positive:

- One code path for all three desktop targets, with Windows appearance on
  Windows.
- The tab strip gets the place upstream gives it, without a XAML dependency.
- The `renderer` crate is not disturbed: egui's painter remains the frame path
  until a separate ADR says otherwise.

Negative:

- With `decorations(false)` the OS no longer supplies resize borders. The shell
  owns hit-testing, and a mistake there is a usability bug no unit test catches.
  Mitigation: the edge bands are pure geometry in the caption module and are
  unit-tested; the window itself is live-checked.
- The gate cannot assert that a title bar is dark or that corners are round.
  Those commits carry a live check with a screenshot, as the pty and theme work
  did; the tests cover only what is data.
- Slice 2 adds `raw-window-handle = "0.6"` as a direct dependency because
  eframe does not re-export the `HasWindowHandle` trait, and adds
  `Win32_Graphics_Dwm` (plus `Win32_UI_WindowsAndMessaging` for the system
  menu) to the existing `windows-sys` feature list.
- Windows 10 before 1809 has no dark title bar; the toolkit falls back to the
  light frame, and the caption's own colours still follow the scheme.

## Alternatives considered

| Alternative | Why not |
|---|---|
| WinUI 3 / XAML through the `windows` crate | Needs the Windows App SDK and MSIX, a hand-written COM surface, and locks the shell to Windows; months of work and a user-visible runtime prerequisite |
| A hand-written Win32 host now | Replaces all five shell modules and the 788-line settings editor; raw Win32 controls with a v6 manifest render like Windows 10, not Windows 11, so the effort does not buy the look that motivated it |
| Only set the dark theme and stop | A one-line win, but the tab strip stays below the caption; it is slice 0, not the destination |
| `decorations(false)` with a hand-rolled `WM_NCCALCSIZE` host | Re-implements what `StartDrag`/`BeginResize` already reach through winit, and adds a message-proc `unsafe` surface for no new behaviour |
| Switch to iced, Slint or Dioxus | Another full rewrite with another look; the native-backend goal is unchanged and the churn is not paid for |
| Drive `renderer` into a wgpu content pass now | ADR 007 defers the GPU backend until a window exists, and egui's painter is not the bottleneck; the software path is the tested one. Revisit after the chrome work, if at all |
| Ship a `.ico` and a manifest instead of drawing chrome | The icon is computed and tested (ADR 038); a resource pipeline is Phase 13 |
