//! # rusterm-adapter
//!
//! Terminal adapter layer — VT dispatch, page management, and protocol handling.
//! Ported from Microsoft's Windows Terminal C++ `terminal/adapter` module.
//!
//! ## Modules
//!
//! - [`dispatch_types`] — VT parameter types, enums, and constants
//! - [`interact_dispatch`] — Input state machine callback interface
//! - [`term_dispatch`] — Output state machine callback interface
//! - [`adapt_dispatch`] — Main VT adapter implementation
//! - [`font_buffer`] — Font buffer tracking
//! - [`macro_buffer`] — VT macro support
//! - [`page_manager`] — Alternate/main screen buffer switching
//! - [`sixel_parser`] — Sixel image format parsing
//! - [`terminal_output`] — Character set translation management
//! - [`charsets`] — Character set definitions and translations

pub mod charsets;
pub mod dispatch_types;
pub mod font_buffer;
pub mod interact_dispatch;
pub mod macro_buffer;
pub mod page_manager;
pub mod sixel_parser;
pub mod term_dispatch;
pub mod terminal_output;

// Re-export key types.
pub use dispatch_types::*;
pub use interact_dispatch::IInteractDispatch;
pub use page_manager::{Page, PageManager};
pub use sixel_parser::SixelParser;
pub use term_dispatch::ITermDispatch;
pub use terminal_output::TerminalOutput;
