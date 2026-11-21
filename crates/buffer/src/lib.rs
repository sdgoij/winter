//! # winterm-buffer
//!
//! Terminal text buffer — rows, cells, and text attributes.
//! Ported from Microsoft's Windows Terminal C++ buffer module.
//!
//! ## Modules
//!
//! - [`text_attribute_behavior`] — Attribute behavior enumeration
//! - [`output_cell`] — `OutputCell` — the fundamental cell type
//! - [`row`] — `Row` — a row of terminal cells
//! - [`text_buffer`] — `TextBuffer` — the circular scrollback buffer
//! - [`cursor`] — `Cursor` — terminal cursor state
//! - [`marks`] — Scroll/command marks for shell integration
//! - [`hyperlinks`] — Hyperlink map for OSC 8 hyperlinks
//! - [`selection`] — Text selection types
//! - [`cell_width`] — Character-to-column mapping
//! - [`line_rendition_ext`] — Screen/buffer line conversion helpers
//! - [`output_cell_iterator`] — Cell iterators
//! - [`output_cell_rect`] — Rectangular cell regions
//! - [`image_slice`] — Image slice for sixel rendering
//! - [`text_buffer_text_iterator`] — Text iterators
//! - [`search`] — Text search with simple regex support

pub mod cell_width;
pub mod cursor;
pub mod dbcs_attribute;
pub mod hyperlinks;
pub mod image_slice;
pub mod line_rendition_ext;
pub mod marks;
pub mod output_cell;
pub mod output_cell_iterator;
pub mod output_cell_rect;
pub mod row;
pub mod row_attributes;
pub mod search;
pub mod selection;
pub mod text_attribute_behavior;
pub mod text_buffer;
pub mod text_buffer_text_iterator;

// Re-export key types.
pub use cell_width::CharToColumnMapper;
pub use cursor::Cursor;
pub use dbcs_attribute::DbcsAttribute;
pub use hyperlinks::HyperlinkMap;
pub use image_slice::ImageSlice;
pub use marks::{BufferMark, MarkExtents, Marks};
pub use output_cell::{CellBehavior, OutputCell};
pub use output_cell_iterator::{OutputCellIterator, OutputCellMutIterator};
pub use output_cell_rect::OutputCellRect;
pub use row::Row;
pub use row_attributes::RowAttributes;
pub use search::{SearchMatch, SearchOptions};
pub use selection::{Selection, SelectionPoint, SelectionType};
// `TextAttribute` is vocabulary shared by the buffer and the adapter, so it
// lives in `types`; the re-export keeps the historical `buffer::TextAttribute`
// path working (ADR 002).
pub use text_attribute_behavior::TextAttributeBehavior;
pub use text_buffer::TextBuffer;
pub use text_buffer_text_iterator::{TextBufferRowIterator, TextBufferTextIterator};
pub use types::TextAttribute;
