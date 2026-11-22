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
