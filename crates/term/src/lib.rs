//! # winterm-term
//!
//! The glue between the VT parser and the terminal adapter.
//!
//! `vt` owns the grammar and knows nothing of terminal vocabulary; `adapter`
//! owns the vocabulary and knows nothing of parsing. [`OutputStateMachineEngine`]
//! implements `IStateMachineEngine` by translating each completed sequence into
//! `ITermDispatch` calls, which is upstream's `OutputStateMachineEngine`.
//!
//! The parser feeds bytes. Printed text is decoded here, through one UTF-8
//! decoder with an explicit replacement policy, before it reaches the buffer.

pub mod output_engine;
pub mod terminal;

pub use output_engine::OutputStateMachineEngine;
pub use terminal::Terminal;
