//! # winter
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
//!   that name a toolkit. `linux` is the GTK 4 shell (ADR 075): GTK hosts the
//!   window, the menus, the find bar and the settings window, while the tab
//!   strip, the scrollbar and the grid stay [`renderer`]'s pixels, drawn through
//!   GTK's OpenGL context with the software blit as the fallback (ADR 076). The
//!   egui modules above remain the fallback when the `gtk` feature is off.
//!   `macos` is the AppKit shell (ADR 072): AppKit hosts the window, its menu
//!   bar, its find field and its settings window, and the tab strip, the
//!   scrollbar and the grid are `renderer`'s pixels there too.
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
#[cfg(all(target_os = "linux", feature = "gtk"))]
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
pub mod trace;
// The renderer's view of a live terminal belongs to the shells that blit its
// pixels: WinUI, the macOS shell, and the GTK shell. Its tests run on every
// platform, which is why `test` is part of the condition.
#[cfg(any(windows, target_os = "macos", feature = "gtk", test))]
pub mod terminal_render;
pub mod terminal_widget;
pub mod view;
pub mod visual_bell;
pub mod window;
#[cfg(windows)]
pub mod winui;
