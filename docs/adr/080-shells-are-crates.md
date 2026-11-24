# ADR 080 — Each shell is its own crate

- Status: Accepted
- Date: 2026-11-24
- Related: `crates/winter`, `crates/gtk4`, `crates/cocoa`, `crates/winui`,
  `crates/app`, `Cargo.toml`; ADRs 038, 040, 072, 075, 079

## Context

ADR 079 left one crate holding the shell-independent core and all three
shells, selected by `cfg` in `main`. Three consequences came with it: every
build compiled the manifest of three toolkits (GTK, AppKit, Win32) even though
one is used; a change to the GTK shell and a change to the Windows shell sat in
the same crate and the same test binary; and `crates/winter` was both the
library and the application, so the name could not be given to anything else.

A Cargo package cannot solve this by depending on itself: a binary that names
a shell which depends on the crate's own library is a cyclic package dependency
and Cargo refuses it (measured: `error: cyclic package dependency`). The
library and the binary have to be different packages.

## Decision

1. **`crates/winter` is the shell-independent core.** It keeps the pty session,
   the grid, the input rules, the control and its connection, the settings
   draft, the fonts list and the drawn icon. It names no toolkit. Items the
   shells need are `pub`: the crate is the shells' API, which is what makes it
   the place a future embedding API would live.
2. **One crate per shell.** `crates/gtk4` (`winter-gtk4`), `crates/cocoa`
   (`winter-cocoa`) and `crates/winui` (`winter-winui`) each own their
   platform's toolkit dependencies, UI code and tests, and depend on `winter`
   for everything else.
3. **A shell crate is empty where it does not apply.** Each crate root is
   `#![cfg(<its platform>)]` and its toolkit dependencies sit under the same
   `cfg`, so the three can be built unconditionally on every host and
   `cargo clippy --workspace` still checks the one that matters there.
4. **`crates/app` is the binary.** `main` selects the shell by `cfg`; the
   package is named `app` and the binary keeps the name `winter`, so every
   artefact, script and installer path is unchanged. The Windows icon resource
   moves with the executable into `crates/app/build.rs`.
5. **The package names are `winter-<shell>`, not the toolkit's name.** A crate
   called `gtk4` would collide with the gtk-rs binding it depends on; the
   directory names stay as the platform names.
6. **The tooling names the binary's package.** `just package`, `just dist`,
   `just linux` and the CI cross-check build `-p app`; the `write-icon` example
   stays in `winter` because the drawing does.

## Rationale

- **A build names one toolkit.** Linux no longer compiles the AppKit or Win32
  manifests, and `cargo check -p winter-gtk4` is a shell-only check.
- **The dependency arrow runs one way.** `winter` knows nothing of a shell; the
  shells know nothing of each other; `app` knows all three and nothing else.
- **The names say what they are.** `crates/winter` is the library the shells
  are built on, and `crates/app` is the thing a user runs.

## Consequences

Positive:

- The lock, the manifest graph and the test binaries are per shell, and the
  Linux test run no longer carries Windows-only or AppKit-only code.
- `winter`'s public surface is now the contract each shell consumes; a change
  to it is reviewable as an API change.
- The `unsafe` exemptions are per crate: `winter::sound`, `winui`'s four
  modules, `cocoa`'s AppKit/Metal boundary and `gtk4::gl`.

Negative:

- **Movement without behaviour change is a large diff**, and every path in the
  ADRs, the skills and the scope pages describes where the code used to live.
  The current pages say where it lives now; the ADRs keep their history.
- **`crates/winter` is wider than it should be.** Making the shells' whole
  surface `pub` exposes internals that a stricter API would hide; the boundary
  is now worth tightening deliberately rather than by accident.
- **A shell's tests can use the core's test doubles.** `term_connection::tests`
  is no longer `cfg(test)`-gated, because a different crate's tests need it.

## Alternatives considered

| Alternative | Why not |
|---|---|
| Keep one crate and a `cfg` per platform | Cargo rejects the cycle a binary crate that depends on its own shells creates |
| Move the shared modules too, into a fifth crate | They are the core by definition; a second move would rename the same code twice |
| Name the crates after their toolkits (`gtk4`, `cocoa`) | `gtk4` collides with the binding the shell depends on |
| Leave the binary in `crates/winter` behind a feature | Still a package cycle, and the feature would have to pick a shell |

## Amendment

The drawing is a crate of its own: `crates/icon` (`winter-icon`) holds the
geometry with no dependencies, because the binary's build script needs the
same pixels and a build script cannot reach into another package's `src/`
without a `#[path]` that only some tooling resolves. `winter` re-exports
`SIZE` and `rgba` from it, the Windows resource asks it for `ico`, and the
packager's PNG comes from the same function.
