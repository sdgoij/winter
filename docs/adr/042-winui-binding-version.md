# ADR 042 — The WinUI binding is the 0.108 pre-release snapshot

- Status: Accepted
- Date: 2026-11-22
- Related: ADR 040, ADR 041; `crates/winter/Cargo.toml`,
  `crates/winter/src/winui/app.rs`

## Context

ADR 040 pinned `windows-reactor` 0.100.0. That release has no keyboard or
character events, no timer on the component context and no way to close the
window: `on_preview_key_down`, `on_character_received`, `RoutedCallback`,
`InputModifiers`, `set_timeout` and `close_window` are all absent. A terminal
that cannot be typed into is not a shell.

The Microsoft mainline snapshot published as `windows-reactor-pre` 0.108.0 has
them. It is the same source tree (`microsoft/windows-rs` at `cd03f93`),
republished so that a dependent project can use it before the release exists.
Its API matches the 0.101 readme the shell was designed against.

The shell has been run live on 0.108: PowerShell painted through `renderer`
into a `SwapChainPanel`, and `echo hello` was typed and executed.

## Decision

- Depend on `windows-reactor = { package = "windows-reactor-pre", version = "0.108" }`.
- Pin 0.108, and move to the first-party release that carries this API when it
  lands.
- `windows-core` stays at 0.100, which is the projection the snapshot uses and
  the type the swap-chain handoff needs.

## Rationale

- 0.100 cannot express the input a terminal needs; the snapshot can.
- The snapshot is Microsoft's own source; only the publication is third-party.
- Following the documented 0.101 API keeps the shell's shape stable when the
  first-party release arrives.

## Consequences

- A pre-release dependency, republished outside Microsoft's account, joins the
  tree with `windows-window-pre` 0.104.
- The binding is confined to `crates/winter/src/winui/`; a change of version is
  a change of that directory.
- `cargo-deny` licences and sources apply to the new crates like any other.

## Alternatives considered

| Alternative | Why not |
|---|---|
| Stay on 0.100 and hook Win32 keyboard messages | A second, hand-written input path for the whole window |
| Wait for a first-party release | Blocks the shell on another project's schedule |
| `naui` | Another toolkit and another projection beside reactor |
