//! `TerminalOutput` — Character set translation management.
//!
//! Ported from `terminalOutput.hpp` in the Windows Terminal C++ codebase.
//! Provides character set translation for displaying special characters.

use crate::charsets;
use crate::dispatch_types::VTID;

/// Terminal output state for character set translation.
#[derive(Debug, Clone)]
pub struct TerminalOutput {
    /// User preference charset ID.
    upss_id: VTID,
    /// User preference charset translation table.
    upss_translation_table: &'static [u8],
    /// Gset translation tables (G0-G3).
    gset_translation_tables: [&'static [u8]; 4],
    /// Gset IDs (G0-G3).
    gset_ids: [VTID; 4],
    /// GL set number (currently active).
    gl_set_number: usize,
    /// GR set number (currently active).
    gr_set_number: usize,
    /// GL translation table.
    gl_translation_table: &'static [u8],
    /// GR translation table.
    gr_translation_table: &'static [u8],
    /// Single shift set number.
    ss_set_number: usize,
    /// Whether GR translation is enabled.
    gr_translation_enabled: bool,
    /// DRCS ID.
    drcs_id: VTID,
    /// DRCS translation table.
    drcs_translation_table: &'static [u8],
}

impl TerminalOutput {
    /// Creates a new TerminalOutput.
    pub fn new(gr_enabled: bool) -> Self {
        Self {
            upss_id: VTID::new(),
            upss_translation_table: charsets::ASCII,
            gset_translation_tables: [charsets::ASCII, charsets::ASCII, charsets::ASCII, charsets::ASCII],
            gset_ids: [VTID::new(); 4],
            gl_set_number: 0,
            gr_set_number: 2,
            gl_translation_table: charsets::ASCII,
            gr_translation_table: charsets::ASCII,
            ss_set_number: 0,
            gr_translation_enabled: gr_enabled,
            drcs_id: VTID::new(),
            drcs_translation_table: charsets::ASCII,
        }
    }

    /// Performs a soft reset.
    pub fn soft_reset(&mut self) {
        self.upss_id = VTID::new();
        self.upss_translation_table = charsets::ASCII;
        self.gset_ids = [VTID::new(); 4];
        self.gset_translation_tables = [charsets::ASCII, charsets::ASCII, charsets::ASCII, charsets::ASCII];
        self.gl_set_number = 0;
        self.gr_set_number = 2;
        self.gl_translation_table = charsets::ASCII;
        self.gr_translation_table = charsets::ASCII;
        self.ss_set_number = 0;
        self.gr_translation_enabled = false;
        self.drcs_id = VTID::new();
        self.drcs_translation_table = charsets::ASCII;
    }

    /// Restores state from a saved TerminalOutput.
    pub fn restore_from(&mut self, saved: &TerminalOutput) {
        self.upss_id = saved.upss_id;
        self.upss_translation_table = saved.upss_translation_table;
        self.gset_translation_tables = saved.gset_translation_tables;
        self.gset_ids = saved.gset_ids;
        self.gl_set_number = saved.gl_set_number;
        self.gr_set_number = saved.gr_set_number;
        self.gl_translation_table = saved.gl_translation_table;
        self.gr_translation_table = saved.gr_translation_table;
        self.ss_set_number = saved.ss_set_number;
        self.gr_translation_enabled = saved.gr_translation_enabled;
        self.drcs_id = saved.drcs_id;
        self.drcs_translation_table = saved.drcs_translation_table;
    }

    /// Assigns user preference charset.
    pub fn assign_user_preference_charset(&mut self, charset_id: VTID, size_96: bool) {
        self.upss_id = charset_id;
        let table = if size_96 {
            self.lookup_translation_table_96(&charset_id)
        } else {
            self.lookup_translation_table_94(&charset_id)
        };
        self.upss_translation_table = table;
    }

    /// Returns the user preference charset ID.
    pub fn get_user_preference_charset_id(&self) -> VTID {
        self.upss_id
    }

    /// Returns the user preference charset size.
    pub fn get_user_preference_charset_size(&self) -> usize {
        if self.upss_id.is_empty() { 94 } else { 96 }
    }

    /// Translates a key character.
    pub fn translate_key(&self, ch: char) -> char {
        if !(' '..='~').contains(&ch) {
            return ch;
        }

        let offset = (ch as usize).saturating_sub(' ' as usize);
        let table = if self.needs_translation() && self.gr_translation_enabled {
            &self.gr_translation_table
        } else {
            &self.gl_translation_table
        };

        if offset < table.len() {
            let translated = table[offset] as char;
            if translated != '\0' {
                return translated;
            }
        }

        ch
    }

    /// Designates a 94-character set.
    pub fn designate_94_charset(&mut self, gset_number: usize, charset_id: VTID) {
        if gset_number >= 4 {
            return;
        }
        self.gset_ids[gset_number] = charset_id;
        self.gset_translation_tables[gset_number] = self.lookup_translation_table_94(&charset_id);
    }

    /// Designates a 96-character set.
    pub fn designate_96_charset(&mut self, gset_number: usize, charset_id: VTID) {
        if gset_number >= 4 {
            return;
        }
        self.gset_ids[gset_number] = charset_id;
        self.gset_translation_tables[gset_number] = self.lookup_translation_table_96(&charset_id);
    }

    /// Sets the DRCS94 designation.
    pub fn set_drcs94_designation(&mut self, charset_id: VTID) {
        self.drcs_id = charset_id;
        let table = self.lookup_translation_table_94(&charset_id);
        self._replace_drcs_table(self.drcs_translation_table, table);
    }

    /// Sets the DRCS96 designation.
    pub fn set_drcs96_designation(&mut self, charset_id: VTID) {
        self.drcs_id = charset_id;
        let table = self.lookup_translation_table_96(&charset_id);
        self._replace_drcs_table(self.drcs_translation_table, table);
    }

    /// Gets the charset ID for a given G set number.
    pub fn get_charset_id(&self, gset_number: usize) -> VTID {
        if gset_number >= 4 {
            return VTID::new();
        }
        self.gset_ids[gset_number]
    }

    /// Gets the charset size for a given G set number.
    pub fn get_charset_size(&self, gset_number: usize) -> usize {
        if gset_number >= 4 {
            return 94;
        }
        let id = self.gset_ids[gset_number];
        if id.is_empty() { 94 } else { 96 }
    }

    /// Performs a locking shift.
    pub fn locking_shift(&mut self, gset_number: usize) {
        if gset_number >= 4 {
            return;
        }
        self.gl_set_number = gset_number;
    }

    /// Performs a locking shift right.
    pub fn locking_shift_right(&mut self, gset_number: usize) {
        if gset_number >= 4 {
            return;
        }
        self.gr_set_number = gset_number;
    }

    /// Performs a single shift.
    pub fn single_shift(&mut self, gset_number: usize) {
        if gset_number >= 4 {
            return;
        }
        self.ss_set_number = gset_number;
    }

    /// Gets the left set number (GL).
    pub fn get_left_set_number(&self) -> usize {
        self.gl_set_number
    }

    /// Gets the right set number (GR).
    pub fn get_right_set_number(&self) -> usize {
        self.gr_set_number
    }

    /// Returns whether a single shift is pending.
    pub fn is_single_shift_pending(&self, gset_number: usize) -> bool {
        if gset_number >= 4 {
            return false;
        }
        self.ss_set_number == gset_number
    }

    /// Returns whether translation is needed.
    pub fn needs_translation(&self) -> bool {
        !self.gl_translation_table.is_empty() && self.gl_translation_table != charsets::ASCII
    }

    /// Enables GR translation.
    pub fn enable_gr_translation(&mut self, enabled: bool) {
        self.gr_translation_enabled = enabled;
    }

    fn lookup_translation_table_94(&self, charset_id: &VTID) -> &'static [u8] {
        match charset_id.as_bytes() {
            [b'C'] => charsets::DEC_SPECIAL_GRAPHICS,
            [b'0'] => charsets::LATIN1,
            [b'A'] => charsets::LATIN1,
            [b'B'] => charsets::ASCII,
            [b'4'] => charsets::LATIN2,
            [b'E'] => charsets::LATIN2,
            [b'6'] => charsets::LATIN_CYRILLIC,
            [b'G'] => charsets::LATIN_CYRILLIC,
            [b'8'] => charsets::LATIN_GREEK,
            [b'H'] => charsets::LATIN_GREEK,
            [b'\\'] => charsets::LATIN_HEBREW,
            [b'N'] => charsets::LATIN_HEBREW,
            [b'Q'] => charsets::LATIN5,
            [b'R'] => charsets::LATIN5,
            [b'Y'] => charsets::LATIN_TURKISH,
            [b'Z'] => charsets::LATIN_TURKISH,
            _ => charsets::ASCII,
        }
    }

    fn lookup_translation_table_96(&self, charset_id: &VTID) -> &'static [u8] {
        match charset_id.as_bytes() {
            [b'C'] => charsets::DEC_SPECIAL_GRAPHICS,
            [b'0'] => charsets::LATIN1,
            [b'A'] => charsets::LATIN1,
            [b'B'] => charsets::ASCII,
            [b'2'] => charsets::LATIN1_BASED_96,
            [b'3'] => charsets::LATIN2_BASED_96,
            [b'4'] => charsets::LATIN_CYRILLIC_BASED_96,
            [b'Q'] => charsets::GREEK_BASED_96,
            [b'\\'] => charsets::HEBREW_BASED_96,
            _ => charsets::ASCII,
        }
    }

    fn _replace_drcs_table(&mut self, old_table: &'static [u8], new_table: &'static [u8]) {
        // In the C++ implementation, this replaces DRCS entries.
        // For now, just update the translation table directly.
        let _ = old_table;
        self.drcs_translation_table = new_table;
    }
}

impl Default for TerminalOutput {
    fn default() -> Self {
        Self::new(false)
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn test_terminal_output_new() {
        let output = TerminalOutput::new(false);
        assert!(!output.needs_translation());
        assert!(!output.gr_translation_enabled);
    }

    #[test]
    fn test_terminal_output_translate() {
        let output = TerminalOutput::new(false);
        // Without GR translation, characters should pass through
        assert_eq!(output.translate_key('A'), 'A');
        assert_eq!(output.translate_key('a'), 'a');
        assert_eq!(output.translate_key('0'), '0');
    }

    #[test]
    fn test_terminal_output_soft_reset() {
        let mut output = TerminalOutput::new(true);
        output.enable_gr_translation(true);
        output.soft_reset();
        assert!(!output.gr_translation_enabled);
    }

    #[test]
    fn test_terminal_output_locking_shift() {
        let mut output = TerminalOutput::new(false);
        output.locking_shift(1);
        assert_eq!(output.get_left_set_number(), 1);
    }

    #[test]
    fn test_terminal_output_locking_shift_right() {
        let mut output = TerminalOutput::new(false);
        output.locking_shift_right(3);
        assert_eq!(output.get_right_set_number(), 3);
    }
}
