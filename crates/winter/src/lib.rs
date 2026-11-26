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
//!   directories a face is composed from. [`palette`] is the command palette's
//!   entries, filter and selection; each shell draws it. [`about`] is the name,
//!   description, version line and repository every About dialog shows.
//! - [`embed`] composes the terminal and its frame for a host that is not a
//!   shell — an editor panel, a wasm canvas, an agent's tool — and is the one
//!   door such a host needs (ADR 082).
//!
//! The `pty`, `notifications` and `settings` features are on by default and are
//! what a shell needs. An embedding turns them off: it owns the process, it
//! decides what a bell or a desktop notification means, and it starts from a
//! measured font and a grid rather than a settings file.
//!
//! The shells are crates of their own and depend on this one (ADR 080):
//! `winter-gtk4` is the GTK 4 shell (ADRs 075, 076), `winter-cocoa` the AppKit
//! one (ADRs 072, 073) and `winter-winui` the WinUI 3 one (ADR 040). `app` is
//! the binary that picks between them.
//!
//! The emulation and console crates are portable and are tested without a
//! window.

pub mod about;
#[cfg(feature = "settings")]
pub mod config;
pub mod embed;
pub mod find;
pub mod fonts;
pub mod grid;
pub mod icon;
pub mod input;
#[cfg(feature = "notifications")]
pub mod notifications;
#[cfg(feature = "settings")]
pub mod palette;
pub mod scrollbar;
#[cfg(feature = "pty")]
pub mod session;
#[cfg(feature = "pty")]
pub mod shell_integration;
#[allow(unsafe_code)]
pub mod sound;
pub mod tabs;
pub mod term_connection;
pub mod term_control;
pub mod term_events;
#[cfg(all(feature = "pty", feature = "settings"))]
pub mod term_settings;
pub mod terminal_render;
pub mod trace;
pub mod view;
pub mod visual_bell;
