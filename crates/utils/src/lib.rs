//! # winterm-utils
//!
//! Terminal Implementation Library — geometry, types, and utilities.
//! Ported from Microsoft's Windows Terminal C++ `til` namespace.
//!
//! ## Modules
//!
//! - [`point`] — 2D point with checked arithmetic
//! - [`size`] — 2D size with scaling and area computation
//! - [`rect`] — Rectangle with union/intersection/difference
//! - [`inclusive_rect`] — Rectangle with inclusive bounds
//! - [`color`] — RGBA color type
//! - [`small_vec`] — Stack-optimized small vector
//! - [`enum_set`] — Fixed-size enum bitmask
//! - [`hash`] — Portable wyhash-based hasher
//! - [`mutex`] — Cross-platform mutex
//! - [`atomic`] — Atomic wait/notify primitives
//! - [`ticket_lock`] — Fair FIFO ticket lock
//! - [`throttled`] — Debounced function calls
//! - [`io`] — Cross-platform file I/O
//! - [`unicode`] — Unicode helpers
//! - [`string`] — String utilities
//! - [`replace`] — String replacement helpers
//! - [`math`] — Checked arithmetic utilities
//! - [`colorbrewer`] — Color palette constants
//! - [`rle`] — Run-length encoding
//! - [`flat_set`] — Linear probing flat set
//! - [`generational`] — Generational indices
//! - [`spsc`] — Single-producer single-consumer queue
//! - [`latch`] — One-shot synchronization barrier
//! - [`bit`] — Bit manipulation utilities
//! - [`bytes`] — Byte slice helpers

// This is the workspace's unsafe-primitives crate: the lock-free queue, the
// stack-optimised vector, and the manual mutex/latch wrappers are implemented
// here and nowhere else. Every other crate denies `unsafe_code` outright.
#![allow(unsafe_code)]

pub mod atomic;
pub mod bit;
pub mod bytes;
pub mod color;
pub mod colorbrewer;
pub mod enum_set;
pub mod flat_set;
pub mod generational;
pub mod hash;
pub mod inclusive_rect;
pub mod io;
pub mod latch;
pub mod math;
pub mod mutex;
pub mod point;
pub mod rect;
pub mod replace;
pub mod rle;
pub mod size;
pub mod small_vec;
pub mod spsc;
pub mod string;
pub mod throttled;
pub mod ticket_lock;
pub mod unicode;

// Re-export key types at crate root for convenience.
pub use color::Color;
pub use enum_set::EnumSet;
pub use hash::Hasher;
pub use inclusive_rect::InclusiveRect;
pub use math::checked_add;
pub use math::checked_div;
pub use math::checked_mul;
pub use math::checked_sub;
pub use point::{CoordType, Point, PointSpan};
pub use rect::Rect;
pub use size::Size;
pub use small_vec::SmallVec;
