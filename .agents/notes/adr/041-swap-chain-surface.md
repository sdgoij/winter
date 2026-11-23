# ADR 041 — The terminal surface is a SwapChainPanel

- Status: Accepted
- Date: 2026-11-22
- Related: ADR 040, ADR 007, ADR 016; `crates/winter/src/terminal_render.rs`,
  `crates/renderer/src/atlas/shared.rs`, `crates/winter/src/winui/surface.rs`

## Context

ADR 040 chose `windows-reactor` for the Windows shell and named an `Image`
whose `SoftwareBitmapSource` is filled from `SoftwareBackend` as the first
surface. That surface is not reachable through the crate:

- `ElementRef<Image>::request_set_native_source` casts the object to WinUI 3's
  `Microsoft.UI.Xaml.Media.ImageSource`.
- WinUI 3 lives in the Windows App SDK metadata, not the Windows SDK. The
  `windows` crate projects `Windows.UI.Xaml` (UWP); searching 0.62 for
  `SoftwareBitmapSource` finds nothing.
- The only other image path reactor exposes is encoded bytes
  (`Image::source_data(EncodedImage)`), which means encoding and decoding a
  bitmap every frame.

`SwapChainPanel` is reachable. `ElementRef<SwapChainPanel>::request_set_swap_chain`
does the `ISwapChainPanelNative` cast inside reactor, so the shell only has to
produce an `IDXGISwapChain` as an `IUnknown`.

The toolkit-free half is already in place: `TerminalData` paints a `Control`
through `Renderer::paint_frame`, and `SharedBackend` exposes the software
backend's pixels afterwards. This record decides only how those pixels reach
the window.

## Decision

- **The pane's surface is a `SwapChainPanel`.**
- The shell creates a D3D11 device and a composition swap chain, hands the swap
  chain to reactor, and presents the software backend's pixels through a
  Direct2D device context: a bitmap made from `SharedBackend`'s buffer, drawn
  onto the back buffer.
- **The interop is one module.** `crates/winter/src/winui/surface.rs` carries a
  module-level `#[allow(unsafe_code)]` and a `SAFETY:` note per call, the shape
  ADR 016 gave the pty backends. No other module gains `unsafe`.
- **The text path stays `renderer`'s.** The bitmap is the software backend's
  output; Direct2D only blits it.
- ADR 040's other decisions stand.

## Rationale

- It is the durable path ADR 007 named, and it needs no third-party WinUI
  projection beside reactor.
- It avoids a per-frame encode and decode, so the shell can paint at the
  display's rate.
- The alternative that keeps `Image` would require a WinUI 3 projection crate
  for one type, or a throwaway encoder.

## Consequences

Positive:

- The Windows shell presents the same pixels the toolkit-free tests already
  assert.
- The software backend remains the tested path; the GPU backend is still a
  separate, later decision.

Negative:

- `crates/winter` gains the `windows` crate (Win32 Direct3D 11, DXGI and
  Direct2D features) and `windows-core` for the interface conversion;
  `windows-reactor` stays at 0.100.
- The gate cannot prove that a swap chain presents; the commit that lands it
  carries a live check, as the pty and theme work did.
- A resize recreates the swap-chain buffers and the Direct2D target.
- One new `unsafe` module is a review event.

## Alternatives considered

| Alternative | Why not |
|---|---|
| `Image` + `SoftwareBitmapSource` | Unreachable: WinUI 3 types are outside the `windows` crate |
| `Image` + an encoded BMP each frame | Encode and decode per frame; a prototype, not a terminal |
| `naui-winui3` for the image source | A second WinUI projection for one type |
| Hand-written Win32 host and GDI | Not WinUI; reimplements the chrome ADR 040 keeps |
