# ADR 075 — The Linux shell is GTK 4, built by the gate

- Status: Accepted
- Date: 2026-11-24
- Related: ADRs 061 (the toolkit decision), 062 (the parking decision this
  supersedes), 018 (egui), 033 (Linux has no sound sink), 072/073 (the macOS
  shell that set the precedent), `docs/linux-shell-scope.md`,
  `crates/gtk4/src/`

## Context

ADR 062 parked the GTK shell at a live terminal because the expensive half —
selection, clipboard, the scrollbar with prompt marks, the find bar, the
settings window, IME and accessibility — was unbuilt, and because no CI job
compiled it. It named macOS as the next native shell instead.

macOS now has an AppKit shell (ADRs 072, 073), and Linux is the only platform
whose window still comes from egui. The parked shell has since grown the rest of
the WinUI checklist: pointer selection with drag scrolling, copy and paste,
mouse forwarding, the scrollbar and its prompt marks, the find bar, a native
settings window, settings reload, IME preedit with a candidate anchor, a bell
flash, OSC 777 notifications, tab drag-reorder, and the visible screen published
to the accessibility tree. What is left is named below.

The objection ADR 062 raised — a shell the gate does not compile — is answered
by making the gate compile it rather than by not shipping it.

## Decision

- **GTK 4 is the Linux shell.** `gtk` is a default feature of `winter`, so
  `cargo build`, the gate and the release archive build it on Linux while
  Windows keeps WinUI and macOS keeps AppKit.
- **The egui shell stays as the fallback.** `--no-default-features` builds it on
  Linux and needs no GTK headers; the gate checks that build so it cannot rot.
- **The toolkit is GTK 4.10 or newer.** The accessible-property API the shell
  publishes the screen with arrived in 4.10.
- **CI installs the headers.** The gate, the full-test job and the packaging job
  install `libgtk-4-dev` on their Linux runners, so the shell is compiled and
  tested on both Linux architectures and shipped by the host that tested it.
- **The shell declares no new `unsafe`.** gtk-rs is safe, so the workspace's
  exemption list (ADR 016, 033, 041, 047, 058, 066, 072) is unchanged.

## Named gaps

These are deliberate and visible, not placeholders:

- **Primary selection is not wired.** GDK exposes `PRIMARY` as a clipboard only
  from 4.16, above the GTK floor the Ubuntu runners carry; middle-click pastes
  `CLIPBOARD` for now. Lifting the floor to 4.16 is the follow-up.
- **Accessibility publishes one value.** The visible screen is the pane's
  accessible description, which a screen reader can read; there is no
  cell-addressable `GtkAccessibleText` tree, because that needs a custom widget
  class. macOS has the same shape of gap (ADR 072).
- **A tab has no program icon.** Launch Services gives macOS one (ADR 072) and
  the executable gives WinUI one (ADR 066); the GTK strip draws titles only.
- **Linux is silent.** ADR 033 stands: the bell flashes the pane and DECPS tones
  have no Linux sink.
- **Panes** are the macOS shell's since ADR 085 and absent here; the
  GTK window still holds one pane per tab, which is a shared Phase 9 row rather
  than a Linux one. The command palette was a gap here when this shell shipped
  and is now this shell's: `crates/winter/src/palette.rs` holds the rows, filter
  and selection, and the GTK card draws them. macOS draws the same rows in
  `crates/cocoa`; WinUI still declines the action.

## Rationale

- **The chrome matches the other hosts.** The tab strip, scrollbar and grid are
  `renderer`'s pixels, exactly as on Windows and macOS, so a theme reaches them
  the same way; GTK supplies the window, menus, dialogs and text input.
- **A Linux user gets what the desktop expects.** Native menus, a native search
  entry, IME with the desktop's candidate window, the desktop's keybindings for
  copy and paste, and an accessibility value.
- **The maintenance objection is met where it was raised.** The cost of a second
  shell was being invisible to CI; the gate now builds, lints and tests it on
  both Linux architectures, and packages it on the host that tested it.
- **One shell per host is the model already accepted.** `main.rs` selects a
  shell at compile time, and `session`, `term_control`, `grid`, `view`, `tabs`,
  `layout`, `blink`, `find`, `scrollbar` and `input` name no toolkit.

## Consequences

- Building Winter from source on Linux needs GTK 4.10 development headers
  (`libgtk-4-dev` on Debian/Ubuntu, `gtk4-devel` on Fedora). The README and
  `CONTRIBUTING.md` say so; the release archives are built in CI where the
  headers are installed.
- The gate on Linux becomes a real test of the shell: a GTK regression fails CI
  instead of waiting for a manual run.
- egui remains compiled and tested on Linux with `--no-default-features`, so the
  fallback keeps working and the shared modules keep their second consumer.
- The Wayland traps recorded in `docs/linux-shell-scope.md` stay accurate: the
  widget paintable pictures the last painted frame, `GtkWindowHandle` claims
  double presses, and a GTK call can re-enter a borrowed shell.

## Alternatives considered

- **Stay parked (ADR 062).** Leaves Linux as the only platform without a native
  shell, after macOS proved the pattern and the CI objection has a fix.
- **Delete the GTK code.** Discards a working shell and the Wayland knowledge.
- **Keep GTK opt-in forever.** Ships egui on Linux while the tested shell is the
  one nobody runs, and leaves the GTK code to drift unwatched.
- **Raise the floor to GTK 4.16 now.** Buys `PRIMARY` and loses the Ubuntu 24.04
  runner without a PPA; the clipboard fallback is the smaller cost.
