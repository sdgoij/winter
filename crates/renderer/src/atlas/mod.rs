//! The atlas renderer.
//!
//! Ported from `src/renderer/atlas/`. This starts with the portable data model
//! from `common.h`: bitmaps, grid-line runs, cursor and miscellaneous settings
//! and decoration positions. `ShapedRow`, `FontSettings`, `RenderingPayload` and
//! the `IBackend` trait follow once the shaping and backend layers exist.

pub mod common;
