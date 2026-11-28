# 085 — macOS panes: one renderer and layer per pane

Status: Accepted.

## Context

ADR 079 deleted the egui shell, and with it the only copy of the pane
tree; every native shell has drawn one pane per tab since. ADR 072's
shell holds one renderer and one `TerminalData` for the active tab, and its
Metal backend presents through a single `CAMetalLayer`, which is one drawable.

A split needs two grids of different sizes in one window at the same time.
Upstream has the same shape: every `TermControl` owns its render engine, its
viewport and its selection, and a pane is the unit a tab splits.

`winter::layout` was restored as part of this change: a binary split tree over
fractional rectangles, with split, remove-with-collapse, resize, divider paths
and the focus walk. It names no toolkit.

## Decision

- A tab is a `winter::layout` tree plus a `Vec<Pane>`. A leaf names a
  `PaneId`; a pane owns its `Control`, `ViewState`, `FindBar`, renderer, content
  backend and frame. Pane identities are unique across tabs, so the controller
  can keep a layer per pane for the life of the window.
- Each pane renders its own grid at its own size. The shell maps the layout
  fractions onto the view's pane area, below the strip, and gives each pane the
  cells its rectangle holds (left inset and its own scrollbar excluded).
- **Metal.** The controller creates one `CAMetalLayer` per visible pane, below
  the find and bell overlays and above the view's drawing. Layers are created
  lazily, hidden when their tab is inactive, and dropped with the pane. A pane's
  backend is rebuilt at a new scale or font size; the sync re-attaches it to the
  same layer.
- **Software.** Each pane keeps its own `Frame`; `chrome` blits it at the pane's
  grid rectangle and paints the pane fills, the dividers, the focused pane's
  border and one scrollbar per pane. A screenshot forces a readback of every
  visible pane, exactly as the single-pane capture did.
- A new pane is spawned at the size its half will fill, so its shell lays its
  first prompt out for the width it keeps.
- The split, focus and resize actions in the settings file are performed by this
  shell. The menu adds Split Right (⌘D), Split Down (⇧⌘D) and Close Pane (⇧⌘W);
  ⌘⌥arrows move focus and ⌘⌃arrows resize. A press within three points of a
  divider drags it, clamped by the tree's ratios.
- The wheel, selection, scrollbar, find bar and input method address the pane
  under the pointer, or the focused pane for keys and composition. The focused
  pane drives the tab's title and icon, the strip's colours and the window's
  appearance.

## Consequences

- A pane costs a renderer, a device, its pipelines and an atlas. A split pays
  that construction on the interaction, and a full-screen program in each pane
  costs the memory. Sharing one device, pipeline set and atlas across panes is
  the obvious next step; the renderer does not expose that seam yet.
- Layers for panes in inactive tabs stay allocated but hidden, so switching tabs
  does not rebuild an atlas. A closed pane's layer is dropped on the next sync.
- The find bar belongs to the focused pane and sits over its area; the
  controller moves it when focus moves. Per-pane bells are drawn by the focused
  pane's layer, so only the focused pane's flash is visible on the GPU path.
- A split resizes the original pane in place, so its shell redraws; zsh may
  print its end-of-line mark once. The new pane avoids that by starting at its
  final grid.
- Mouse selection and forwarding stay inside the pane the press started in;
  a divider, a scrollbar and the tab strip take their presses first.
- Panes remain absent from the WinUI and GTK shells; both adopt the same
  `winter::layout` tree in ADR 086, with one viewport per pane in their one
  surface.

## Alternatives considered

| Alternative | Why not |
|---|---|
| One renderer and one target, compositing panes into a single frame | Needs a blit of every pane every frame; loses the per-pane drawable and its copy-free presentation |
| Software panes only, keeping Metal for single-pane tabs | The split tabs, where the terminal is busiest, would be the ones off the GPU |
| Share a device, pipelines and atlas between pane renderers now | The renderer constructs them per `Renderer`; splitting that seam is a larger change than the first cut of panes |
| Panes in every shell in one change | WinUI and GTK need their own surfaces and input routing; the tree is shared and each shell adopts it |
