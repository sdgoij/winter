//! # winter
//!
//! The application: a shell on a pseudo terminal, painted in a window.
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
//! The native shells own the window and the toolkit: `linux` is the GTK 4
//! shell (ADR 075), where GTK hosts the menus, the find bar and the settings
//! window while the tab strip, the scrollbar and the grid stay `renderer`'s
//! pixels, drawn through GTK's OpenGL context with the software blit as the
//! fallback (ADR 076); `macos` is the AppKit shell (ADR 072); `winui` is the
//! WinUI 3 shell (ADR 040).
//!
//! The emulation and console crates are portable and are tested without a
//! window.

pub mod config;
pub(crate) mod find;
pub mod fonts;
pub mod grid;
pub mod icon;
pub mod input;
#[cfg(target_os = "linux")]
pub mod linux;
#[cfg(target_os = "macos")]
pub mod macos;
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
#[cfg(windows)]
pub mod winui;
