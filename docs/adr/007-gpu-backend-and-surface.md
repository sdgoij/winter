# ADR 007 — The GPU backend waits for the window surface

- Status: Accepted
- Date: 2026-11-22
- Related: `crates/renderer/src/atlas/software.rs`, `atlas/font.rs`,
  `IBackend`; ADR 006

## Context

The atlas backend trait is `IBackend::render(&mut RenderingPayload)`. The
software backend is complete and tested. The next backend on the plan is `wgpu`.

A GPU backend has a dependency the software backend does not: it presents to a
surface. Creating a `wgpu::Surface` requires a window handle (`raw-window-handle`),
and configuring a swap chain requires the window size and format. The window is
the UI layer's job (Phase 9), which does not exist yet.

A headless `wgpu` backend that renders into an offscreen texture is possible, but
its tests depend on a GPU or software adapter being present. The project forbids
flaky and environment-dependent tests, and the CI matrix does not guarantee an
adapter on every runner.

## Decision

The GPU backend is created by the UI layer, with a surface, once that layer
exists. Until then:

- `IBackend` stays target-agnostic. It receives a payload and nothing else.
- The concrete backend owns its render target: a `wgpu::Surface` when there is a
  window, or an offscreen texture for a tool.
- The software backend is the default and the only one built today.
- No `wgpu` dependency is added until there is a crate that constructs a device,
  a surface and a swap chain.

## Rationale

- **No dead weight.** A large dependency with no consumer lengthens every build
  and every `cargo-deny` run for no behaviour.
- **No flaky tests.** A test that needs a GPU adapter is not a test the gate can
  trust; a test that skips without one proves nothing.
- **The phase order is real.** The renderer draws into a backend; the window owns
  the backend. Reversing that would put platform surface types into `renderer`.
- **The software path is already visible.** Text, sixel, DRCS and the cursor all
  rasterise through `SoftwareBackend`, so the emulator is not blocked on a GPU.

## Consequences

Positive:

- The atlas is exercised end to end by the software backend today.
- The GPU backend can be added without changing `IBackend` or the engine.

Negative:

- Desktop performance waits for Phase 9; the software rasteriser is the frame
  cost until then.
- The first `wgpu` commit will include the surface plumbing, not just a shader.

## Alternatives considered

| Alternative | Why not |
|-------------|---------|
| Add a headless `wgpu` backend now | Tests depend on an adapter; the gate cannot prove it works |
| Put a surface/window in `IBackend::render` | Leaks platform surface types into the platform-independent renderer |
| Add `wgpu` now behind an off-by-default feature | Untested code and a dependency with no production consumer |
| Use a software Vulkan/GL adapter in CI | Not present on every runner, and it turns a unit gate into an environment check |
