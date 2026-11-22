//! # winterm
//!
//! The application: a shell on a pseudo terminal, painted in a window.
//!
//! The crate is split so the GUI can be replaced without touching the rest
//! (ADR 018):
//!
//! - [`session`] composes `server`'s pty with a pump thread; it knows nothing
//!   of a window.
//! - [`grid`] turns the active buffer into rows of attribute runs and a
//!   cursor; it knows nothing of a window either.
//! - [`ui`] is the egui shell. It is the only module that names a toolkit, and
//!   it translates events into `term`'s vocabulary and snapshots into paint
//!   calls.
//!
//! The window layer is the last piece of Phase 8; the emulation and console
//! crates are portable and are tested without it.

pub mod grid;
pub mod session;
pub mod ui;
pub mod view;
