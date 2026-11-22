//! # winterm-host
//!
//! The console host: the side of the terminal that owns what a client writes
//! and what it reads back. Ported from `src/host`.
//!
//! Phase 6 starts here with the command history. The input buffer, cooked read,
//! screen buffer and the input parser (`InputStateMachineEngine`, ADR 010) are
//! the modules that follow.
//!
//! ## Modules
//!
//! - [`history`] — `CommandHistory`, the per-application command ring

pub mod history;
