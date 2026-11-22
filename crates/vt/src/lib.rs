//! # winterm-vt
//!
//! Virtual Terminal (VT) escape sequence parser and state machine.
//! Ported from Microsoft's Windows Terminal C++ VT module.
//!
//! ## Modules
//!
//! - [`ascii`] — ASCII constants for VT sequence parsing
//! - [`i_state_machine_engine`] — the callbacks the parser drives
//! - [`state`] — `VtState` enum (the DFA's states)
//! - [`state_machine`] — `StateMachine` DFA parser
//! - [`base64`] — Base64 encode/decode utilities
//! - [`charsets`] — VT character set management (G0-G3)

pub mod ascii;
pub mod base64;
pub mod charsets;
pub mod i_state_machine_engine;
pub mod state;
pub mod state_machine;

// Re-export key types.
pub use base64::{Base64Error, decode, encode};
pub use charsets::{Charset, Charsets};
pub use i_state_machine_engine::{IStateMachineEngine, StringHandler};
pub use state::VtState;
pub use state_machine::{MAX_PARAMETER_COUNT, MAX_PARAMETER_VALUE, MAX_SUBPARAMETER_COUNT, ParserMode, StateMachine};
