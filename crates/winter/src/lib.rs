//! # winter
//!
//! The terminal's shell-independent core: a control over a pseudo terminal, the
//! state a shell paints, and nothing that names a toolkit.
//!
//! The crate is split so a toolkit never reaches the terminal (ADR 038):
//!
//! - [`session`] composes `server`'s pty with a pump thread; it knows nothing
//!   of a window.
//! - [`grid`] turns the active buffer into rows of attribute runs and a
//!   cursor; it knows nothing of a window either.
//! - [`input`] is the shell's event vocabulary and the rules that turn it into
//!   `term` keys, shared by every window backend (ADRs 009, 040).
//! - [`term_control`] wraps a terminal and its connection;
//!   [`term_connection`] is the backend trait and [`term_events`] is what it
//!   reports. [`shell_integration`] is the hook a PowerShell profile is given
//!   so the shell reports its prompts and exit codes (ADR 060).
//! - [`sound`] plays the bell and DECPS tones on a thread of its own, and is
//!   the only module allowed `unsafe` (ADR 033).
//! - [`config`] holds the settings draft every editor edits, and [`fonts`] the
//!   directories a face is composed from.
//!
//! The shells are crates of their own and depend on this one (ADR 080):
//! `winter-gtk4` is the GTK 4 shell (ADRs 075, 076), `winter-cocoa` the AppKit
//! one (ADRs 072, 073) and `winter-winui` the WinUI 3 one (ADR 040). `app` is
//! the binary that picks between them.
//!
//! The emulation and console crates are portable and are tested without a
//! window.

pub mod config;
pub mod find;
pub mod fonts;
pub mod grid;
pub mod icon;
pub mod input;
pub mod notifications;
pub mod scrollbar;
pub mod session;
pub mod shell_integration;
#[allow(unsafe_code)]
pub mod sound;
pub mod tabs;
pub mod term_connection;
pub mod term_control;
pub mod term_events;
pub mod term_settings;
pub mod terminal_render;
pub mod trace;
pub mod view;
pub mod visual_bell;
