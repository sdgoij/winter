//! `MacroBuffer` — VT macro storage and definition parser.
//!
//! Ported from `MacroBuffer.hpp/cpp`: DECDMAC definitions in text or hex-pair
//! encoding (with `!count;…;` repeats), a shared 256 KiB budget, and the DEC
//! VT420 checksum used by DECCKSR.
//!
//! Invocation differs from upstream because this crate owns no state machine:
//! upstream feeds the sequence back into its parser, while
//! [`crate::AdaptDispatch::take_macro_input`] hands the bytes to the
//! application, which feeds them to the parser it already owns.

use crate::dispatch_types::{MacroDeleteControl, MacroEncoding};

/// The macro identifier space, ported from upstream `_macros` (64 entries).
pub const MAX_MACROS: usize = 64;

/// The shared macro memory budget, in bytes. Upstream counts UTF-16 units; the
/// sequences are control bytes in practice, so the units coincide.
pub const MAX_SPACE: usize = 0x40000;

/// The largest repeat count, matching the parser's parameter cap.
const MAX_PARAMETER_VALUE: usize = 65535;

/// Which part of a macro definition the next character belongs to.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
enum ParseState {
    Text,
    HexDigit,
    SecondHexDigit,
    RepeatCount,
}

/// Stores macro definitions and parses DECDMAC payloads.
#[derive(Debug)]
pub struct MacroBuffer {
    /// The parser state for the definition currently being received.
    parse_state: ParseState,
    /// The byte assembled from the two hex digits seen so far.
    decoded_char: u8,
    /// Whether a repeat sequence is waiting for its closing `;`.
    repeat_pending: bool,
    /// The repeat count being accumulated.
    repeat_count: usize,
    /// Where the repeated sequence starts in the active macro.
    repeat_start: usize,
    /// Macro content by id; empty means undefined.
    macros: Vec<Vec<u8>>,
    /// The macro being defined.
    active_macro_id: usize,
    /// Total bytes held across all macros.
    space_used: usize,
}

impl MacroBuffer {
    /// Creates a new MacroBuffer.
    pub fn new() -> Self {
        Self {
            parse_state: ParseState::Text,
            decoded_char: 0,
            repeat_pending: false,
            repeat_count: 0,
            repeat_start: 0,
            macros: vec![Vec::new(); MAX_MACROS],
            active_macro_id: 0,
            space_used: 0,
        }
    }

    /// Discards every macro, which is what a hard reset does.
    pub fn reset(&mut self) {
        for macro_content in &mut self.macros {
            macro_content.clear();
        }
        self.space_used = 0;
        self.parse_state = ParseState::Text;
        self.decoded_char = 0;
        self.repeat_pending = false;
        self.repeat_count = 0;
        self.repeat_start = 0;
    }

    /// Prepares to receive a definition, deleting the macro (or all macros)
    /// first as `delete_control` asks.
    ///
    /// Ported from `MacroBuffer::InitParser`. Returns false for an out-of-range
    /// id or an encoding this parser does not implement.
    pub fn init_parser(&mut self, macro_id: i32, delete_control: MacroDeleteControl, encoding: MacroEncoding) -> bool {
        let Ok(macro_id) = usize::try_from(macro_id) else {
            return false;
        };
        if macro_id >= MAX_MACROS {
            return false;
        }

        self.active_macro_id = macro_id;
        self.decoded_char = 0;
        self.repeat_pending = false;

        self.parse_state = match encoding {
            MacroEncoding::HexPair => ParseState::HexDigit,
            MacroEncoding::Text => ParseState::Text,
        };

        match delete_control {
            MacroDeleteControl::DeleteId => self.delete_macro(macro_id),
            MacroDeleteControl::DeleteAll => self.delete_all_macros(),
        }
        true
    }

    /// Feeds one character of a definition.
    ///
    /// Ported from `MacroBuffer::ParseDefinition`: an ESC ends the definition,
    /// other control characters are ignored, and any syntax error clears what
    /// has been received so far. Returns whether parsing should continue.
    pub fn parse_definition(&mut self, ch: char) -> bool {
        if ch == '\x1b' {
            if self.repeat_pending && !self.apply_pending_repeat() {
                self.delete_macro(self.active_macro_id);
            }
            return false;
        }

        if ch < ' ' {
            return true;
        }

        let mut success = true;
        match self.parse_state {
            ParseState::Text => {
                success = self.append_to_active_macro(ch);
            }
            ParseState::HexDigit => {
                if self.decode_hex_digit(ch) {
                    self.parse_state = ParseState::SecondHexDigit;
                } else if ch == '!' && !self.repeat_pending {
                    self.parse_state = ParseState::RepeatCount;
                    self.repeat_count = 0;
                } else if ch == ';' && self.repeat_pending {
                    success = self.apply_pending_repeat();
                } else {
                    success = false;
                }
            }
            ParseState::SecondHexDigit => {
                success = self.decode_hex_digit(ch) && self.append_byte(self.decoded_char);
                self.decoded_char = 0;
                self.parse_state = ParseState::HexDigit;
            }
            ParseState::RepeatCount => {
                if let Some(digit) = ch.to_digit(10) {
                    self.repeat_count = self
                        .repeat_count
                        .saturating_mul(10)
                        .saturating_add(digit as usize)
                        .min(MAX_PARAMETER_VALUE);
                } else if ch == ';' {
                    self.repeat_pending = true;
                    self.repeat_start = self.macros[self.active_macro_id].len();
                    self.parse_state = ParseState::HexDigit;
                } else {
                    success = false;
                }
            }
        }

        if !success {
            self.delete_macro(self.active_macro_id);
        }
        success
    }

    /// Decodes one hex digit into `decoded_char`.
    fn decode_hex_digit(&mut self, ch: char) -> bool {
        let Some(digit) = ch.to_digit(16) else {
            return false;
        };
        self.decoded_char = self.decoded_char.wrapping_shl(4) | digit as u8;
        true
    }

    /// Appends a text character, encoded as UTF-8.
    fn append_to_active_macro(&mut self, ch: char) -> bool {
        let mut buffer = [0u8; 4];
        let bytes = ch.encode_utf8(&mut buffer).as_bytes();
        if bytes.len() > self.space_available() {
            return false;
        }
        self.macros[self.active_macro_id].extend_from_slice(bytes);
        self.space_used += bytes.len();
        true
    }

    /// Appends one decoded byte.
    fn append_byte(&mut self, byte: u8) -> bool {
        if self.space_available() == 0 {
            return false;
        }
        self.macros[self.active_macro_id].push(byte);
        self.space_used += 1;
        true
    }

    /// Repeats the sequence since `repeat_start` as the pending count asks.
    ///
    /// Ported from `MacroBuffer::_applyPendingRepeat`: the sequence has already
    /// been stored once, so only the extra copies are appended.
    fn apply_pending_repeat(&mut self) -> bool {
        if self.repeat_count > 1 {
            let content = &self.macros[self.active_macro_id];
            let sequence_length = content.len() - self.repeat_start;
            let Some(space_required) = (self.repeat_count - 1).checked_mul(sequence_length) else {
                return false;
            };
            if space_required > self.space_available() {
                return false;
            }

            let sequence = content[self.repeat_start..].to_vec();
            for _ in 1..self.repeat_count {
                self.macros[self.active_macro_id].extend_from_slice(&sequence);
                self.space_used += sequence_length;
            }
        }
        self.repeat_pending = false;
        true
    }

    /// Returns the content of a macro, or None for an out-of-range id.
    pub fn macro_bytes(&self, id: usize) -> Option<&[u8]> {
        self.macros.get(id).map(Vec::as_slice)
    }

    /// Deletes a specific macro.
    pub fn delete_macro(&mut self, id: usize) {
        if let Some(content) = self.macros.get_mut(id) {
            self.space_used = self.space_used.saturating_sub(content.len());
            content.clear();
        }
    }

    /// Deletes all macros.
    pub fn delete_all_macros(&mut self) {
        for content in &mut self.macros {
            content.clear();
        }
        self.space_used = 0;
    }

    /// Returns the bytes still available.
    pub fn space_available(&self) -> usize {
        MAX_SPACE - self.space_used
    }

    /// Returns the total bytes held.
    pub const fn space_used(&self) -> usize {
        self.space_used
    }

    /// Returns the DEC VT420 checksum of every stored macro byte.
    ///
    /// Ported from `MacroBuffer::CalculateChecksum`: bytes are summed and
    /// subtracted from zero, so the result wraps like the terminal's.
    pub fn checksum(&self) -> u16 {
        let mut checksum: u16 = 0;
        for content in &self.macros {
            for byte in content {
                checksum = checksum.wrapping_sub(u16::from(*byte));
            }
        }
        checksum
    }

    /// Returns the number of non-empty macros.
    pub fn defined_count(&self) -> usize {
        self.macros.iter().filter(|content| !content.is_empty()).count()
    }

    /// Returns the total bytes held, matching [`MacroBuffer::space_used`].
    pub fn total_size(&self) -> usize {
        self.space_used
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

    fn define(buffer: &mut MacroBuffer, id: i32, encoding: MacroEncoding, payload: &str) {
        assert!(buffer.init_parser(id, MacroDeleteControl::DeleteId, encoding));
        for ch in payload.chars() {
            buffer.parse_definition(ch);
        }
        buffer.parse_definition('\x1b');
    }

    #[test]
    fn a_new_buffer_holds_nothing() {
        let buffer = MacroBuffer::new();
        assert_eq!(buffer.defined_count(), 0);
        assert_eq!(buffer.total_size(), 0);
        assert_eq!(buffer.space_available(), MAX_SPACE);
        assert_eq!(buffer.checksum(), 0);
        assert_eq!(buffer.macro_bytes(0), Some(&[][..]));
    }

    #[test]
    fn text_definitions_store_their_payload() {
        let mut buffer = MacroBuffer::new();

        define(&mut buffer, 2, MacroEncoding::Text, "hello");

        assert_eq!(buffer.macro_bytes(2), Some(&b"hello"[..]));
        assert_eq!(buffer.defined_count(), 1);
        assert_eq!(buffer.total_size(), 5);
    }

    #[test]
    fn hex_definitions_decode_pairs() {
        let mut buffer = MacroBuffer::new();

        define(&mut buffer, 3, MacroEncoding::HexPair, "1b5b41");

        assert_eq!(buffer.macro_bytes(3), Some(&[0x1b, 0x5b, 0x41][..]));
    }

    #[test]
    fn a_hex_repeat_appends_the_extra_copies() {
        let mut buffer = MacroBuffer::new();

        // `!count;sequence;` repeats `sequence` count times in total.
        define(&mut buffer, 4, MacroEncoding::HexPair, "!3;4142;");

        assert_eq!(buffer.macro_bytes(4), Some(&b"ABABAB"[..]));
    }

    #[test]
    fn a_malformed_hex_definition_is_discarded() {
        let mut buffer = MacroBuffer::new();
        assert!(buffer.init_parser(1, MacroDeleteControl::DeleteId, MacroEncoding::HexPair));

        assert!(buffer.parse_definition('4'));
        assert!(!buffer.parse_definition('z'));

        assert_eq!(buffer.macro_bytes(1), Some(&[][..]));
        assert_eq!(buffer.defined_count(), 0);
    }

    #[test]
    fn control_characters_are_ignored_but_esc_ends_the_definition() {
        let mut buffer = MacroBuffer::new();
        assert!(buffer.init_parser(0, MacroDeleteControl::DeleteId, MacroEncoding::Text));

        assert!(buffer.parse_definition('\n'));
        assert!(buffer.parse_definition('A'));
        assert!(!buffer.parse_definition('\x1b'));

        assert_eq!(buffer.macro_bytes(0), Some(&b"A"[..]));
    }

    #[test]
    fn the_budget_is_shared_and_overflow_aborts_the_definition() {
        let mut buffer = MacroBuffer::new();
        assert!(buffer.init_parser(0, MacroDeleteControl::DeleteId, MacroEncoding::Text));
        for _ in 0..MAX_SPACE {
            assert!(buffer.parse_definition('x'));
        }

        assert!(!buffer.parse_definition('y'));
        // A failed definition is discarded, returning the space.
        assert_eq!(buffer.space_available(), MAX_SPACE);
    }

    #[test]
    fn delete_id_clears_only_the_named_macro() {
        let mut buffer = MacroBuffer::new();
        define(&mut buffer, 0, MacroEncoding::Text, "zero");
        define(&mut buffer, 1, MacroEncoding::Text, "one");

        define(&mut buffer, 0, MacroEncoding::Text, "two");

        assert_eq!(buffer.macro_bytes(0), Some(&b"two"[..]));
        assert_eq!(buffer.macro_bytes(1), Some(&b"one"[..]));
        assert_eq!(buffer.total_size(), 6);
    }

    #[test]
    fn delete_all_clears_every_macro_first() {
        let mut buffer = MacroBuffer::new();
        define(&mut buffer, 0, MacroEncoding::Text, "zero");
        define(&mut buffer, 1, MacroEncoding::Text, "one");

        assert!(buffer.init_parser(2, MacroDeleteControl::DeleteAll, MacroEncoding::Text));
        buffer.parse_definition('x');

        assert_eq!(buffer.defined_count(), 1);
        assert_eq!(buffer.macro_bytes(2), Some(&b"x"[..]));
    }

    #[test]
    fn the_checksum_subtracts_every_byte_from_zero() {
        let mut buffer = MacroBuffer::new();
        define(&mut buffer, 0, MacroEncoding::Text, "AB");

        assert_eq!(buffer.checksum(), 0u16.wrapping_sub(0x41).wrapping_sub(0x42));
    }

    #[test]
    fn ids_outside_the_macro_space_are_rejected() {
        let mut buffer = MacroBuffer::new();
        assert!(!buffer.init_parser(-1, MacroDeleteControl::DeleteId, MacroEncoding::Text));
        assert!(!buffer.init_parser(MAX_MACROS as i32, MacroDeleteControl::DeleteId, MacroEncoding::Text));
        assert!(buffer.macro_bytes(MAX_MACROS).is_none());
    }

    #[test]
    fn reset_discards_everything() {
        let mut buffer = MacroBuffer::new();
        define(&mut buffer, 0, MacroEncoding::Text, "zero");

        buffer.reset();

        assert_eq!(buffer.defined_count(), 0);
        assert_eq!(buffer.space_available(), MAX_SPACE);
    }
}
