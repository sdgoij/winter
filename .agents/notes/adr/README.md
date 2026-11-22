# Architecture decision records

Each record is one decision that the code cannot explain by itself. The format is
fixed:

```markdown
# ADR NNN — Title

- Status: Proposed | Accepted | Superseded by ADR NNN
- Date: YYYY-MM-DD
- Related: files, ADRs

## Context
## Decision
## Rationale
## Consequences
## Alternatives considered
```

Rules:

- An ADR is written when a decision constrains code that does not exist yet. If
  a test already enforces the decision, the test is the record and no ADR is
  needed.
- A decision is changed by superseding its ADR, never by editing it in place.
  Edit for typos and links only.
- "Why not X" belongs in the ADR. A future contributor who proposes X should
  find the answer without asking.

| ADR | Decision | Status |
|-----|----------|--------|
| [001](001-workspace-layering.md) | The crate dependency ring | Accepted |
| [002](002-one-type-universe.md) | One home per type; `adapter` consumes `buffer` and `vt` | Accepted |
| [003](003-porting-method.md) | Port method: upstream as oracle, one module per C++ file, no stubs | Accepted |
| [004](004-incremental-dispatch.md) | Port the dispatcher family by family, with explicit `unhandled` defaults | Accepted |
| [005](005-images-in-the-buffer.md) | Images are buffer content: per-row bands, carried by cell operations | Accepted |
| [006](006-render-loop-model.md) | The renderer is host-driven; no thread, timers are data | Accepted |
| [007](007-gpu-backend-and-surface.md) | The GPU backend is created by the UI layer with a surface | Accepted |
| [008](008-font-discovery.md) | Fonts are found by scanning host-supplied directories, read with a small SFNT reader | Accepted |
| [009](009-platform-neutral-input.md) | Input translation takes portable events and a mode snapshot | Accepted |
| [010](010-input-parser-in-the-host.md) | The input parser is ported with the host layer | Accepted |
| [011](011-input-keyboard-layout.md) | The input engine synthesizes keys from a fixed US layout | Accepted |
| [012](012-screen-information-scope.md) | `SCREEN_INFORMATION` is a buffer, viewport and console modes | Accepted |
| [013](013-no-terminal-api-seam.md) | There is no `ITerminalApi` seam; the host drains queues | Accepted |
| [014](014-selection-is-host-state.md) | Selection is host state; the buffer keeps no selection | Accepted |
| [015](015-server-portable-scope.md) | The server layer keeps the console API's portable halves | Accepted |
| [016](016-pty-ffi.md) | The pty layer is hand-written FFI, confined to two modules | Accepted |
| [017](017-input-encoders-stay-in-term.md) | The input encoders stay in `term`; there is no `input` crate | Accepted |
| [018](018-egui-first-shell.md) | egui is the first shell; native backends come later | Accepted |
| [019](019-terminal-selection-is-app-state.md) | The terminal's viewport and selection are application state | Accepted |
| [020](020-selection-adds-document-rows.md) | Selection addresses document rows (history first) | Accepted |
| [021](021-tabs-own-terminals.md) | A tab owns one terminal; the window owns the chords | Accepted |
| [022](022-settings-json-profiles.md) | Settings are a JSON file of named profiles | Superseded in part by 026 (format and file name) |
| [023](023-keybindings-are-settings.md) | Keybindings are settings data: typed chords, flat action names, merged over the defaults | Accepted |
| [024](024-colour-schemes.md) | Colour schemes are settings; `settings` may depend on `types` for the colour vocabulary | Accepted |
| [025](025-settings-are-reloaded-by-polling.md) | A saved settings file is reloaded by polling its stamp; a bad save keeps the session | Accepted |
| [026](026-settings-are-toml.md) | Settings are TOML with a schema `version`; a newer version is refused | Accepted |
| [027](027-normalization-warns.md) | Normalization returns typed warnings; the window lists what it adjusted | Accepted |
| [028](028-settings-window-edits.md) | The window edits font size and the default profile and scheme, saving atomically | Amended by 030 and 031 |
| [029](029-host-font-family.md) | The shell renders with the host's monospace family; italic is a real face when installed | Accepted |
| [030](030-settings-window-edits-profiles.md) | The window edits the profile list; schemes and chords stay file-only | Amended by 031 (keybindings) |
| [031](031-settings-window-edits-keybindings.md) | The window edits keybindings; the list shown is the effective one; an invalid chord blocks Save | Amended by 032 (schemes) |
| [032](032-settings-window-edits-schemes.md) | The window edits schemes as text fields with swatches; an empty field means the built-in colour | Accepted |
| [033](033-audio-tone-model.md) | Audio is a portable tone model in `audio`; the Windows `Beep` sink lives in the app | Accepted |
| [034](034-settings-file-and-migration.md) | A fresh install is given the default file; an earlier `settings.json` is migrated once and kept | Accepted |
| [035](035-themes.md) | A theme is the four fills the tab strip paints; the rest of upstream's `Theme` is WinUI-only | Accepted |
| [036](036-control-connection-events.md) | The connection is a trait, the control owns the terminal, the widget owns the viewport; an exit closes the tab | Accepted |
