//! # rusterm-buffer
//!
//! Terminal text buffer — rows, cells, and text attributes.
//! Ported from Microsoft's Windows Terminal C++ buffer module.
//!
//! ## Modules
//!
//! - [`text_attribute`] — `TextAttribute` for character rendering attributes
//! - [`row_attributes`] — Per-row metadata
//! - [`dbcs_attribute`] — Double-byte character support
//! - [`text_attribute_behavior`] — Attribute behavior enumeration

pub mod dbcs_attribute;
pub mod row_attributes;
pub mod text_attribute;
pub mod text_attribute_behavior;

// Re-export key types.
pub use dbcs_attribute::DbcsAttribute;
pub use row_attributes::RowAttributes;
pub use text_attribute::TextAttribute;
pub use text_attribute_behavior::TextAttributeBehavior;
