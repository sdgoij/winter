//! # winterm-adapter
//!
//! Terminal adapter layer — VT dispatch, page management, and protocol handling.
//! Ported from Microsoft's Windows Terminal C++ `terminal/adapter` module.
//!
//! ## Modules
//!
//! - [`dispatch_types`] — re-exported from `types` (ADR 002)
//! - [`interact_dispatch`] — Input state machine callback interface
//! - [`term_dispatch`] — Output state machine callback interface
//! - [`adapt_dispatch`] — Main VT adapter implementation
//! - [`font_buffer`] — DECDLD soft-font construction
//! - [`macro_buffer`] — VT macro support
//! - [`page_manager`] — Alternate/main screen buffer switching
//! - [`sixel_parser`] — Sixel image format parsing
//! - [`terminal_output`] — Character set translation management

pub mod adapt_dispatch;
pub mod adapt_dispatch_graphics;
pub mod font_buffer;
pub mod interact_dispatch;
mod kitty_graphics;
pub mod macro_buffer;
pub mod page_manager;
pub mod sixel_parser;
pub mod term_dispatch;
pub mod terminal_output;

// `dispatch_types` and `charsets` are terminal vocabulary with no storage, so
// they live in `types`. Re-exporting the module keeps `crate::dispatch_types::*`
// and the historical `adapter::VTParameter` paths working.
pub use types::charsets;
pub use types::dispatch_types;
pub use types::dispatch_types::*;

// Re-export key types.
pub use adapt_dispatch::AdaptDispatch;
pub use font_buffer::FontBuffer;
pub use interact_dispatch::IInteractDispatch;
pub use macro_buffer::MacroBuffer;
pub use page_manager::{Page, PageManager};
pub use sixel_parser::SixelParser;
pub use term_dispatch::ITermDispatch;
pub use terminal_output::TerminalOutput;
