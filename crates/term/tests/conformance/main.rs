//! The VT conformance corpus.
//!
//! Every module drives the public [`term::Terminal`] API with the bytes a
//! program would write and asserts what a user would see: the screen, the
//! cursor, or the response the host owes the program.
//!
//! Expected values come from ECMA-48, DEC's terminal manuals, or xterm's
//! documented behaviour — never from what this code happens to do. When a case
//! here disagrees with the reference, the reference wins.

mod csi;
mod private_modes;
mod support;
