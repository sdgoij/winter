//! The atlas renderer.
//!
//! Ported from `src/renderer/atlas/`. This starts with the portable data model
//! from `common.h` and the shaping trait, with a per-cell fallback. The atlas
//! engine, `FontSettings`, `RenderingPayload` and the `IBackend` trait follow
//! once a backend exists.

pub mod common;
pub mod engine;
pub mod font;
pub mod shaping;
pub mod software;
