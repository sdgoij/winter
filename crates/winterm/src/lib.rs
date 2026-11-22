//! # winterm
//!
//! The application: a shell on a pseudo terminal, painted in a window.
//!
//! The crate is split so the GUI can be replaced without touching the rest
//! (ADRs 018, 038):
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
//! - [`app`] owns the tabs and the window's chords, [`terminal_widget`] is the
//!   egui viewport over one terminal, [`config`] is the settings window, and
//!   [`window`] the platform commands. Those four, with [`icon`], are the ones
//!   that name a toolkit.
//!
//! The emulation and console crates are portable and are tested without a
//! window.

pub mod app;
pub mod blink;
pub mod config;
pub(crate) mod find;
pub mod grid;
pub mod icon;
pub mod input;
pub mod layout;
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
// The renderer's view of a live terminal is the WinUI pane's: the egui pane
// paints through `grid` instead. Its tests run on every platform, which is why
// `test` is part of the condition rather than `windows` alone.
#[cfg(any(windows, test))]
pub mod terminal_render;
pub mod terminal_widget;
pub mod view;
pub mod visual_bell;
pub mod window;
#[cfg(windows)]
pub mod winui;
