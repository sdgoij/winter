# ADR 061 — The Linux shell is GTK 4 for the host, our renderer for the chrome

- Status: Amended by ADR 062 (the GTK shell is parked)
- Date: 2026-11-23
- Related: `crates/winter/src/linux/`, `crates/renderer`, `.agents/notes/linux-shell-scope.md`, ADRs 018, 038, 040, 041, 043, 051

## Context

The egui shell paints everything itself on Linux and macOS. It works, but it
cannot give a Linux user what their desktop expects: native menus and dialogs,
the desktop's own theme and font, IME preedit, or an accessibility tree. That
is the limit that motivated a native shell for Windows (ADR 040).

A native Linux shell has no single answer, because Linux has two native
toolkits. GTK is native on GNOME and, through Breeze-GTK, close on Plasma —
measured on this repository's Fedora 44 guest, where KDE already writes
`gtk-icon-theme-name=breeze`, `gtk-cursor-theme-name=breeze_cursors`,
`gtk-decoration-layout` and `Noto Sans` into `~/.config/gtk-4.0/settings.ini`
and installs `/usr/share/themes/Breeze/gtk-4.0`. Qt is exact on Plasma and
foreign on GNOME, where no maintained platform theme exists.

Browsers resolve the same problem by splitting the UI into layers and giving
each layer to whoever does it best: the content is theirs everywhere; the
chrome next to the content (tabs, toolbar, omnibox) is drawn with their own
toolkit — Chromium's Views, Firefox's XUL; and the host integration (window
frame, caption buttons, native menus, file dialogs, theme colours, IME,
accessibility, scale) is the platform's. Firefox is a GTK program on Linux;
Chrome reads GTK's theme and uses GTK dialogs. Windows Terminal already does
the same thing here: XAML chrome, its own renderer for the grid, Win32 for the
frame and input (ADRs 040, 041, 056).

A terminal has exactly those layers: the grid, the tab strip and the scrollbar
are chrome we already draw on Windows; the window, its menus, its dialogs and
its text input are host work that a toolkit does better than we ever will.

## Decision

- **The host is GTK 4**, with libadwaita where it saves writing a widget: the
  window, `GtkPopoverMenu` menus, `AdwPreferencesWindow` for settings,
  `GtkSearchBar` for find, `GtkFileChooserNative` if a dialog is ever needed,
  and GTK's IME and accessibility plumbing.
- **The tab strip, the profile menu's contents, the scrollbar and the grid are
  our pixels**, painted by `renderer` into the window's drawing area, exactly
  as the WinUI shell blits them into a swap chain (ADR 041). The tab strip is
  the visible chrome, so it is the part a Plasma user judges; it must not look
  like a GNOME app.
- **The custom grid implements `GtkAccessibleText`** (GTK 4.14+, and the guest
  has 4.22), which exists for precisely this case: a widget that draws its own
  text still reports text, caret and selection to Orca.
- **The shell lives in `crates/winter/src/linux/`**, chosen in `main.rs` the
  way `winui` is (ADR 040), and it shares `input`, `view`, `tabs`,
  `term_control`, `session` and `shell_integration` unchanged.
- **The toolkit is an optional dependency behind a `gtk` feature** while the
  two shells coexist, so `just gate` stays runnable on a host without the
  development packages. The feature becomes the Linux default when the shell
  reaches parity with the WinUI checklist, and egui remains selectable until
  then.
- **No new `unsafe`**: gtk-rs is safe to call, so the Linux shell adds nothing
  to the workspace's exemption list.

## Consequences

- A Plasma user gets Breeze menus, Breeze dialogs, Breeze cursor theme,
  Noto Sans text and our tab strip — the split a browser makes, not a GNOME
  impression.
- Menus, dialogs and IME stop being things we implement. The list of features
  that cannot be hand-rolled shrinks to accessibility, which GTK provides the
  interface for.
- Two shells on Linux during the transition: `main.rs` selects, and the GUI
  crates stay out of the default build so the gate needs no `gtk4-devel`.
- CI gains one job that builds the `gtk` feature on a Linux host with the
  development packages installed; the gate itself stays dependency-free.
- Qt stays a possible second Linux shell rather than a rejected option: the
  shared layers are what a shell plugs into, so a Plasma-native chrome could be
  added later without touching the terminal.
- No server-side decorations: the title bar is the tab strip, the Linux spelling
  of ADR 051.

## Alternatives considered

- **Qt 6 via `cxx-qt`.** Exact Breeze chrome on Plasma, which GTK approximates.
  Rejected as the first shell because it puts a C++ bridge in the hottest UI
  path, builds need Qt + CMake + moc on every host, and Qt has no equivalent of
  `GtkAccessibleText` for a self-drawn text widget.
- **Keep egui and theme it.** Cheapest by far, and the reason we are here: no
  native menus, dialogs, IME or accessibility, which is the whole point.
- **Write our own widget toolkit** (the kitty/Alacritty route). Honest for a
  terminal, but it buys identity-neutrality at the cost of the two things we
  cannot fake: IME and accessibility. Browsers, which can afford anything,
  still delegated the host layer.
- **`libadwaita` for the chrome as well**, including an `AdwTabBar`. This is
  the "GTK app on Plasma" impression we are avoiding; the tab strip is the one
  widget worth owning.
- **libcosmic / iced / Slint.** Rust-native and pleasant, but each is its own
  identity: no KDE, no GNOME, and the accessibility story is thinner.
