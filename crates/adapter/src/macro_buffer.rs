//! `MacroBuffer` — VT macro support.
//!
//! Ported from `MacroBuffer.hpp/cpp` in the Windows Terminal C++ codebase.
//! Manages storage and retrieval of terminal macros.

use crate::dispatch_types::{MacroDeleteControl, MacroEncoding};

/// Maximum number of macros.
const MAX_MACROS: usize = 10;

/// Maximum macro ID.
const MAX_MACRO_ID: i32 = 9;

/// A single macro entry.
#[derive(Debug, Clone)]
struct MacroEntry {
    /// Macro ID (0-9).
    #[allow(dead_code)]
    id: i32,
    /// Whether the macro is defined.
    defined: bool,
    /// Encoding type.
    encoding: MacroEncoding,
    /// Macro content as bytes.
    content: Vec<u8>,
}

impl MacroEntry {
    /// Creates a new empty macro entry.
    fn new(id: i32) -> Self {
        Self {
            id,
            defined: false,
            encoding: MacroEncoding::Text,
            content: Vec::new(),
        }
    }
}

/// Buffer for storing VT macros.
#[derive(Debug)]
pub struct MacroBuffer {
    /// Macro entries.
    macros: [MacroEntry; MAX_MACROS],
}

impl MacroBuffer {
    /// Creates a new MacroBuffer.
    pub fn new() -> Self {
        Self {
            macros: [
                MacroEntry::new(0),
                MacroEntry::new(1),
                MacroEntry::new(2),
                MacroEntry::new(3),
                MacroEntry::new(4),
                MacroEntry::new(5),
                MacroEntry::new(6),
                MacroEntry::new(7),
                MacroEntry::new(8),
                MacroEntry::new(9),
            ],
        }
    }

    /// Resets all macros.
    pub fn reset(&mut self) {
        for entry in &mut self.macros {
            entry.defined = false;
            entry.content.clear();
            entry.encoding = MacroEncoding::Text;
        }
    }

    /// Defines a macro with the given ID.
    /// Returns the writer function that will receive macro content.
    pub fn define_macro(
        &mut self,
        id: i32,
        delete_control: MacroDeleteControl,
        encoding: MacroEncoding,
    ) -> Option<&mut Vec<u8>> {
        if !(0..MAX_MACRO_ID).contains(&id) {
            return None;
        }

        match delete_control {
            MacroDeleteControl::DeleteId => {
                let entry = &mut self.macros[id as usize];
                entry.defined = true;
                entry.content.clear();
                entry.encoding = encoding;
                Some(&mut entry.content)
            }
            MacroDeleteControl::DeleteAll => {
                self.reset();
                Some(&mut self.macros[id as usize].content)
            }
        }
    }

    /// Invokes a macro by ID.
    /// Returns the macro content if the macro is defined.
    pub fn invoke_macro(&self, id: i32) -> Option<&[u8]> {
        if !(0..MAX_MACRO_ID).contains(&id) {
            return None;
        }

        let entry = &self.macros[id as usize];
        if entry.defined { Some(&entry.content) } else { None }
    }

    /// Deletes a specific macro.
    pub fn delete_macro(&mut self, id: i32) {
        if !(0..MAX_MACRO_ID).contains(&id) {
            return;
        }
        let entry = &mut self.macros[id as usize];
        entry.defined = false;
        entry.content.clear();
    }

    /// Deletes all macros.
    pub fn delete_all_macros(&mut self) {
        self.reset();
    }

    /// Returns the number of defined macros.
    pub fn defined_count(&self) -> usize {
        self.macros.iter().filter(|m| m.defined).count()
    }

    /// Returns the total storage used by all macros in bytes.
    pub fn total_size(&self) -> usize {
        self.macros.iter().map(|m| m.content.len()).sum()
    }
}

impl Default for MacroBuffer {
    fn default() -> Self {
        Self::new()
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn test_macro_buffer_new() {
        let buffer = MacroBuffer::new();
        assert_eq!(buffer.defined_count(), 0);
        assert_eq!(buffer.total_size(), 0);
    }

    #[test]
    fn test_define_macro() {
        let mut buffer = MacroBuffer::new();
        let content = buffer
            .define_macro(1, MacroDeleteControl::DeleteId, MacroEncoding::Text)
            .unwrap();
        content.extend_from_slice(b"test macro");
        assert_eq!(buffer.defined_count(), 1);
        assert_eq!(buffer.total_size(), 10);
    }

    #[test]
    fn test_invoke_macro() {
        let mut buffer = MacroBuffer::new();
        let content = buffer
            .define_macro(5, MacroDeleteControl::DeleteId, MacroEncoding::Text)
            .unwrap();
        content.extend_from_slice(b"hello");

        let invoked = buffer.invoke_macro(5).unwrap();
        assert_eq!(invoked, b"hello");
    }

    #[test]
    fn test_invoke_undefined_macro() {
        let buffer = MacroBuffer::new();
        assert!(buffer.invoke_macro(3).is_none());
    }

    #[test]
    fn test_delete_macro() {
        let mut buffer = MacroBuffer::new();
        let content = buffer
            .define_macro(2, MacroDeleteControl::DeleteId, MacroEncoding::Text)
            .unwrap();
        content.extend_from_slice(b"data");
        assert_eq!(buffer.defined_count(), 1);

        buffer.delete_macro(2);
        assert_eq!(buffer.defined_count(), 0);
        assert!(buffer.invoke_macro(2).is_none());
    }

    #[test]
    fn test_delete_all_macros() {
        let mut buffer = MacroBuffer::new();
        for i in 0..5 {
            let content = buffer
                .define_macro(i, MacroDeleteControl::DeleteId, MacroEncoding::Text)
                .unwrap();
            content.extend_from_slice(&[i as u8]);
        }
        assert_eq!(buffer.defined_count(), 5);

        buffer.delete_all_macros();
        assert_eq!(buffer.defined_count(), 0);
    }

    #[test]
    fn test_invalid_macro_id() {
        let mut buffer = MacroBuffer::new();
        assert!(
            buffer
                .define_macro(-1, MacroDeleteControl::DeleteId, MacroEncoding::Text)
                .is_none()
        );
        assert!(
            buffer
                .define_macro(10, MacroDeleteControl::DeleteId, MacroEncoding::Text)
                .is_none()
        );
        assert!(buffer.invoke_macro(-1).is_none());
        assert!(buffer.invoke_macro(10).is_none());
    }
}
