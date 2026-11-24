//! The window's icon, from the drawing every shell and the packager share.
//!
//! Upstream embeds `.ico` resources through the app packaging; this port has no
//! resource pipeline, and a binary asset in the tree would be the only one
//! nobody can read. The pixels are computed instead (ADR 058), by the
//! dependency-free `winter-icon` crate so a build script can compile the same
//! drawing (ADR 080).

pub use winter_icon::{SIZE, rgba};
