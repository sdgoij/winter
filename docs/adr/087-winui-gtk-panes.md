# 087 — WinUI and GTK panes: one surface, a viewport per pane

Status: Accepted.

## Context

ADR 085 gave the macOS shell a tab as `winter::layout`'s tree plus a pane per
leaf, each with its own terminal, viewport, find bar, renderer and content
backend. The WinUI and GTK shells still held one terminal per tab and one
renderer for the window, so a split had nothing to address.

Both shells already own exactly one swap chain (`SwapChainPanel` on Windows,
`GtkGLArea` on Linux). The `GtkGLArea` is an overlay above the base drawing
area, and the WinUI swap chain is a grid child under the XAML chrome.

## Decision

- A tab is `winter::layout`'s tree plus a `Vec<Pane>`, as on macOS. A pane owns
  its terminal, viewport, find bar, title, program, renderer and grid. Pane
  identities are unique across tabs.
- **WinUI.** The single `Surface` composits one frame. `begin_composite` clears
  it once, every pane's Direct2D backend draws its grid at a device-pixel
  offset, and the software path blits each pane's pixels into the same frame.
  Scrollbars, dividers and the focused pane's border are XAML elements placed by
  the layout fractions. The `RefCell<Option<Surface>>` borrow is held only for
  `begin_composite`, `end_composite` and a software blit; a pane's backend
  borrows the same cell while it paints.
- **GTK.** The single `GtkGLArea` spans the whole pane area. Each pane's `Gl`
  backend draws into a viewport at its grid rectangle, scissored so a frame
  cannot leak into its neighbour; the first pane clears the framebuffer. A
  drawing area above the GL area paints the dividers, one scrollbar per pane,
  the bells and the focused pane's border, so the chrome stays visible over the
  content.
- The split, focus and resize actions in the settings file are performed by both
  shells. A new pane is spawned at the size its half will fill, so its shell lays
  out its first prompt once.
- The pointer, wheel, selection, scrollbar, find bar and input method address the
  pane under the pointer, or the focused pane for keys and find. The focused
  pane drives the tab's title and icon and the strip's colours.
- `WINTER_SPLIT=1` splits the first tab for a headless live check, matching the
  macOS shell.

## Consequences

- One renderer and one content backend per pane, as on macOS. A window with
  several panes pays a renderer, its pipelines and an atlas per pane; sharing
  them across panes is still the obvious later change.
- WinUI presents one frame per tick for every visible pane, so a split does not
  add a second `Present`. GTK draws every pane in the GL area's one `render`
  signal, with a viewport and scissor switch per pane.
- The chrome above the GTK content is a second drawing area, not the base
  widget: the GL area is opaque and full-size now, and a border painted below
  it would be invisible.
- Panes in inactive tabs keep their renderers and are resized with the window,
  so switching tabs does not rebuild an atlas.
- Mouse selection and forwarding stay inside the pane the press started in; a
  divider, a scrollbar and the tab strip take their presses first.

## Alternatives considered

| Alternative | Why not |
|---|---|
| One `GtkGLArea` per pane in a fixed grid | Each area gets its own context and renderer; input routing and layout would be duplicated, and the shared surface is already one drawable |
| Composite every pane into one CPU bitmap | Adds a full-window copy per frame and loses the GPU's direct presentation |
| Panes only in the software path | The busiest tabs, which are the split ones, would be the ones off the GPU |
