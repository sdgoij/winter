//! # winterm-types
//!
//! Core terminal types — color, character attributes, rendering metadata, and viewport.
//! Ported from Microsoft's Windows Terminal C++ types and buffer modules.
//!
//! ## Modules
//!
//! - [`text_color`] — `TextColor` enum for default/indexed/RGB color states
//! - [`cursor_type`] — `CursorType` enum for cursor shapes
//! - [`character_attrs`] — `CharacterAttributes` bitflags
//! - [`underline_style`] — `UnderlineStyle` enum
//! - [`mark_kind`] — `MarkKind` enum for cell marking
//! - [`text_attribute`] — `TextAttribute` for character rendering attributes
//! - [`charsets`] — 94/96-character translation tables
//! - [`dispatch_types`] — VT parameter types, dispatch enums, and mode flags
//! - [`input_modes`] — the snapshot of input-affecting modes an input engine reads
//! - [`input_record`] — console input records and their control-key state
//! - [`paste`] — filtering clipboard text before it reaches the pty
//! - [`sgr_stack`] — `SgrStack` for XTPUSHSGR / XTPOPSGR
//! - [`viewport`] — `Viewport` screen dimension type
//! - [`line_rendition`] — `LineRendition` for DECDHL/DECSWL/DECDWL
//! - [`codepoint_width`] — Unicode width detection
//! - [`color_fix`] — Oklab perceptual colour nudging
//! - [`theme_utils`] — Light/dark theme detection
//! - [`color_table`] — 16-color palette management
//! - [`virtual_key`] — the virtual-key codes a console input record carries

pub mod character_attrs;
pub mod charsets;
pub mod codepoint_width;
pub mod color_fix;
pub mod color_table;
pub mod cursor_type;
pub mod dispatch_types;
pub mod input_modes;
pub mod input_record;
pub mod line_rendition;
pub mod mark_kind;
pub mod paste;
pub mod sgr_stack;
pub mod text_attribute;
pub mod text_color;
pub mod theme_utils;
pub mod underline_style;
pub mod viewport;
pub mod virtual_key;

// Re-export key types.
pub use character_attrs::CharacterAttributes;
pub use codepoint_width::codepoint_width;
pub use color_table::ColorTable;
pub use cursor_type::CursorType;
pub use dispatch_types::*;
pub use input_modes::InputModes;
pub use input_record::{
    ControlKeyState, FocusEventRecord, InputRecord, KeyEventRecord, MenuEventRecord, MouseEventFlags, MouseEventRecord,
    WindowSizeEventRecord,
};
pub use line_rendition::LineRendition;
pub use mark_kind::MarkKind;
pub use paste::{FilterOption, filter_string_for_paste};
pub use sgr_stack::SgrStack;
pub use text_attribute::TextAttribute;
pub use text_color::{ColorType, TextColor};
pub use underline_style::UnderlineStyle;
pub use viewport::Viewport;
