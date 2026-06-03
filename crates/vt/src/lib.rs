//! # vt
//!
//! Virtual Terminal (VT) escape sequence parser and state machine.
//! Ported from Microsoft's Windows Terminal C++ VT module.
//!
//! ## Modules
//!
//! - [`ascii`] — ASCII constants for VT sequence parsing
//! - [`parameters`] — `VtParameters` for CSI/OSC/DCS parameter parsing
//! - [`dispatch_types`] — `VtId`, `VtSequence`, `DeviceAttributesResponse`
//! - [`state`] — `VtState` enum (19 states of the DFA)
//! - [`state_machine`] — `StateMachine` DFA parser
//! - [`base64`] — Base64 encode/decode utilities
//! - [`charsets`] — VT character set management (G0-G3)

pub mod ascii;
pub mod base64;
pub mod charsets;
pub mod dispatch_types;
pub mod parameters;
pub mod state;
pub mod state_machine;

// Re-export key types.
pub use base64::{Base64Error, decode, encode};
pub use charsets::{Charset, Charsets};
pub use dispatch_types::{DeviceAttributesResponse, DeviceOptions, VtId, VtSequence};
pub use parameters::VtParameters;
pub use state::VtState;
pub use state_machine::{DispatchAction, StateMachine};
