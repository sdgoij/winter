//! User input translated into the bytes a program reads.
//!
//! Upstream splits this across two files, `terminalInput.cpp` for keys and
//! `mouseInput.cpp` for the mouse; the modules here mirror that split.

pub mod terminal_input;

pub use terminal_input::{Key, KeyEvent, Modifiers, TerminalInput};
