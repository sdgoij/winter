# ADR 062 — The Linux shell stays egui; the GTK shell is parked

- Status: Superseded by ADR 074 (the GTK shell ships)
- Date: 2026-11-23
- Related: ADR 061 (the decision this amends), ADR 018 (the egui shell),
  `docs/linux-shell-scope.md`, `crates/winter/src/linux/`

## Context

ADR 061 made GTK 4 the host of a native Linux shell, built beside the egui shell
behind a `gtk` feature, with parity with the WinUI checklist as the bar. That
shell reached a live terminal: a window whose title bar is a tab strip this code
paints, one pty per tab, the shared `input` rules, wheel scrollback and a
profile menu. Milestones 1–4 are done and half of 5.

The remaining work is the expensive half: the find bar, the settings window,
pointer selection and the clipboard, the scrollbar with prompt marks, IME, and
the accessibility tree that ADR 061 names as the reason for a native shell in the
first place. Each is a second implementation of something the egui shell already
does, in a build no CI job compiles — the gate stays dependency-free, so nothing
catches a GTK regression until someone runs it by hand.

The project's dependency runs the other way: the emulation crates are the
product, and a second shell on a third platform buys less than a first native
shell on macOS, which has none.

## Decision

- **No further work on the GTK shell.** It is parked at its current state:
  working, opt-in, and not a supported shell.
- **Linux ships the egui shell**, which already covers what the project has:
  tabs, panes, scrollback, selection, copy/paste, the settings window, the find
  bar, notifications and the palette.
- **The code stays.** `crates/winter/src/linux/` and the `gtk` feature remain: a
  working reference for the Wayland traps the notes record (a widget paintable
  pictures the last painted frame, `GtkWindowHandle` claims double presses, a
  GTK call can re-enter a borrowed shell), and the feature flag already keeps it
  out of every artefact.
- **Nothing defaults to `gtk`.** The feature stays off; the gate and the release
  archives never ask for it.
- **The next native shell is macOS**, scoped the way this one was: a note and an
  ADR before code.

## Rationale

A shell is a maintenance surface, not a checkbox. Two Linux shells means two
places for every chrome behaviour, and the one that ships would be the one CI
does not test. A parked shell costs only the feature flag it sits behind; a
half-built second shell that someone mistakes for the product costs the trust
that makes the rest of the port worth using.

## Consequences

- `linux-shell-scope.md` keeps its measurements and its traps and gains a parked
  status, so a future attempt starts from those notes rather than a blank page.
- The `gtk` build is not in the gate and does not need to be, because it is not
  shipped. It should not gain dependencies or CI time while parked.
- `just linux` stays as the way to run the parked shell on a machine that can
  build it.
- The macOS shell inherits the lesson: scope it before building it, and make the
  gate cover whatever it ships.

## Alternatives considered

- **Finish the GTK shell.** The remaining items are most of its value —
  selection, clipboard, settings, find, IME, accessibility — and none of them is
  covered by CI.
- **Delete the GTK code.** Discards a working reference for the Wayland traps,
  and the feature flag already keeps it out of the build.
- **Switch Linux to GTK and drop egui there.** Ships an incomplete shell to every
  Linux user and leaves macOS with an untested one.
- **Build macOS while Linux is still in flight.** Two shells in progress is the
  cost this decision avoids.
