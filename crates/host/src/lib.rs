//! # winterm-host
//!
//! The console host: the side of the terminal that owns what a client writes
//! and what it reads back. Ported from `src/host`.
//!
//! Phase 6 starts here with the command history. The input buffer, cooked read,
//! screen buffer and the input parser (`InputStateMachineEngine`, ADR 010) are
//! the modules that follow.
//!
//! ## Modules
//!
//! - [`history`] — `CommandHistory`, the per-application command ring
//! - [`input_buffer`] — `InputBuffer`, the records a client has written but not
//!   yet read, their modes and their coalescing rules
//! - [`read_data_cooked`] — `CookedReadData`, the edit line and history popups
//!   behind a cooked read
//! - [`input_state_machine_engine`] — `InputStateMachineEngine`, the grammar
//!   that turns an application's VT input into console records
//! - [`console_information`] — `ConsoleInformation`, the console's shared state

pub mod console_information;
pub mod history;
pub mod input_buffer;
pub mod input_state_machine_engine;
pub mod read_data_cooked;
