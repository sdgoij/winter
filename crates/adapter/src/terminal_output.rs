//! `TerminalOutput` — Character set translation management.
//!
//! Ported from `terminalOutput.hpp` in the Windows Terminal C++ codebase.
//! Provides character set translation for displaying special characters.

use crate::dispatch_types::VTID;
use types::charsets;

/// Terminal output state for character set translation.
#[derive(Debug, Clone)]
pub struct TerminalOutput {
    /// User preference charset ID.
    upss_id: VTID,
    /// User preference charset translation table.
    upss_translation_table: &'static [char],
    /// Gset translation tables (G0-G3).
    gset_translation_tables: [&'static [char]; 4],
    /// Gset IDs (G0-G3).
    gset_ids: [VTID; 4],
    /// Whether each Gset is a 96-character set.
    gset_sizes_96: [bool; 4],
    /// Whether the user preference charset is a 96-character set.
    upss_size_96: bool,
    /// GL set number (currently active).
    gl_set_number: usize,
    /// GR set number (currently active).
    gr_set_number: usize,
    /// GL translation table.
    gl_translation_table: &'static [char],
    /// GR translation table.
    gr_translation_table: &'static [char],
    /// Single shift set number.
    ss_set_number: usize,
    /// Whether GR translation is enabled.
    gr_translation_enabled: bool,
    /// DRCS ID.
    drcs_id: VTID,
    /// DRCS translation table.
    drcs_translation_table: &'static [char],
}

impl TerminalOutput {
    /// Creates a new TerminalOutput.
    pub fn new(gr_enabled: bool) -> Self {
        Self {
            upss_id: VTID::new(),
            upss_translation_table: charsets::ASCII,
            gset_translation_tables: [charsets::ASCII, charsets::ASCII, charsets::ASCII, charsets::ASCII],
            gset_ids: [VTID::from_byte(b'B'); 4],
            gset_sizes_96: [false; 4],
            upss_size_96: false,
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
        self.upss_size_96 = false;
        self.upss_translation_table = charsets::ASCII;
        self.gset_ids = [VTID::from_byte(b'B'); 4];
        self.gset_sizes_96 = [false; 4];
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
        self.gset_sizes_96 = saved.gset_sizes_96;
        self.upss_size_96 = saved.upss_size_96;
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
        self.upss_size_96 = size_96;
        let table = if size_96 {
            self.lookup_translation_table_96(&charset_id)
        } else {
            self.lookup_translation_table_94(&charset_id)
        };
        self.upss_translation_table = table;
        // Every G-set mapped to the user-preference set follows it.
        for gset in 0..4 {
            if self.gset_ids[gset].as_bytes() == b"<" {
                self.gset_translation_tables[gset] = table;
            }
        }
        self.apply_locking_shifts();
    }

    /// Returns the user preference charset ID.
    pub fn get_user_preference_charset_id(&self) -> VTID {
        self.upss_id
    }

    /// Returns the user preference charset size.
    pub fn get_user_preference_charset_size(&self) -> usize {
        if self.upss_size_96 { 96 } else { 94 }
    }

    /// Translates a key character.
    /// Translates a character through the active character sets.
    ///
    /// GL covers `0x20..=0x7E` and GR covers `0xA0..=0xFF`, as upstream's
    /// `TranslateKey` does. A single shift applies to one character and is
    /// then cleared.
    pub fn translate_key(&mut self, ch: char) -> char {
        if (2..=3).contains(&self.ss_set_number) {
            let table = self.gset_translation_tables[self.ss_set_number];
            self.ss_set_number = 0;
            let translated = translate_with(table, ch, 0x20);
            if translated != ch {
                return translated;
            }
            return translate_with(table, ch, 0xA0);
        }

        let code = ch as u32;
        if (0x20..=0x7E).contains(&code) {
            return translate_with(self.gl_translation_table, ch, 0x20);
        }
        // The default GR table is the ASCII set, whose entries are the low half's
        // characters. Applying it to 0xA0..=0xFF would fold `ç` (0xE7) to `g`
        // (0x67); only a designated GR character set translates that range.
        if (0xA0..=0xFF).contains(&code) && self.gr_translation_table != charsets::ASCII {
            return translate_with(self.gr_translation_table, ch, 0xA0);
        }
        ch
    }

    /// Designates a 94-character set.
    pub fn designate_94_charset(&mut self, gset_number: usize, charset_id: VTID) {
        if gset_number >= 4 {
            return;
        }
        self.gset_ids[gset_number] = charset_id;
        self.gset_sizes_96[gset_number] = false;
        self.gset_translation_tables[gset_number] = self.lookup_translation_table_94(&charset_id);
        self.apply_locking_shifts();
    }

    /// Designates a 96-character set.
    pub fn designate_96_charset(&mut self, gset_number: usize, charset_id: VTID) {
        if gset_number >= 4 {
            return;
        }
        self.gset_ids[gset_number] = charset_id;
        self.gset_sizes_96[gset_number] = true;
        self.gset_translation_tables[gset_number] = self.lookup_translation_table_96(&charset_id);
        self.apply_locking_shifts();
    }

    /// Re-reads the active GL and GR tables from their G-sets.
    ///
    /// Upstream's `_SetTranslationTable` does this after every designation, and
    /// `LockingShift`/`LockingShiftRight` do it after every shift: without it a
    /// designation has no effect on what is printed.
    fn apply_locking_shifts(&mut self) {
        let gl = self.gl_set_number;
        let gr = self.gr_set_number;
        self.locking_shift(gl);
        self.locking_shift_right(gr);
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
        if self.gset_sizes_96[gset_number] { 96 } else { 94 }
    }

    /// Performs a locking shift.
    pub fn locking_shift(&mut self, gset_number: usize) {
        if gset_number >= 4 {
            return;
        }
        self.gl_set_number = gset_number;
        self.gl_translation_table = self.gset_translation_tables[gset_number];
    }

    /// Performs a locking shift right.
    pub fn locking_shift_right(&mut self, gset_number: usize) {
        if gset_number >= 4 {
            return;
        }
        self.gr_set_number = gset_number;
        // Upstream clears the table while GR translation is disabled.
        self.gr_translation_table = if self.gr_translation_enabled {
            self.gset_translation_tables[gset_number]
        } else {
            charsets::ASCII
        };
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
        self.gl_translation_table != charsets::ASCII || self.gr_translation_table != charsets::ASCII
    }

    /// Enables GR translation.
    pub fn enable_gr_translation(&mut self, enabled: bool) {
        self.gr_translation_enabled = enabled;
        // G2 and G3 default to the user-preference set while GR translation is
        // enabled, and to ASCII otherwise.
        let table = if enabled {
            self.upss_translation_table
        } else {
            charsets::ASCII
        };
        self.gset_translation_tables[2] = table;
        self.gset_translation_tables[3] = table;
        let id = if enabled {
            VTID::from_byte(b'<')
        } else {
            VTID::from_byte(b'B')
        };
        self.gset_ids[2] = id;
        self.gset_ids[3] = id;
        self.apply_locking_shifts();
    }

    fn lookup_translation_table_94(&self, charset_id: &VTID) -> &'static [char] {
        match charset_id.as_bytes() {
            [b'B'] | [b'1'] => charsets::ASCII,
            [b'0'] | [b'2'] => charsets::DEC_SPECIAL_GRAPHICS,
            [b'A'] => charsets::LATIN1,
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

    fn lookup_translation_table_96(&self, charset_id: &VTID) -> &'static [char] {
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

    fn _replace_drcs_table(&mut self, old_table: &'static [char], new_table: &'static [char]) {
        // In the C++ implementation, this replaces DRCS entries.
        // For now, just update the translation table directly.
        let _ = old_table;
        self.drcs_translation_table = new_table;
    }
}

/// Applies a 94- or 96-character table to `ch`, returning it unchanged when
/// the table has no mapping.
fn translate_with(table: &[char], ch: char, base: u32) -> char {
    let Some(offset) = (ch as u32).checked_sub(base) else {
        return ch;
    };
    match table.get(offset as usize) {
        Some(&translated) if translated != '\0' => translated,
        _ => ch,
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
        let mut output = TerminalOutput::new(false);
        // Without a designated set, characters should pass through
        assert_eq!(output.translate_key('A'), 'A');
        assert_eq!(output.translate_key('a'), 'a');
        assert_eq!(output.translate_key('0'), '0');
    }

    #[test]
    fn the_default_gr_table_leaves_non_ascii_alone() {
        // The default GR table is the ASCII set, whose entries are 0x20..=0x7E.
        // Applying it to 0xA0..=0xFF folded 'ç' (0xE7) to 'g' (0x67).
        let mut output = TerminalOutput::new(false);
        assert_eq!(output.translate_key('\u{e7}'), '\u{e7}');
        assert_eq!(output.translate_key('\u{e2}'), '\u{e2}');
        assert_eq!(output.translate_key('\u{2014}'), '\u{2014}');
        assert_eq!(output.translate_key('\u{30cd}'), '\u{30cd}');
    }

    #[test]
    fn a_designated_gr_set_still_translates_its_range() {
        let mut output = TerminalOutput::new(true);
        output.designate_94_charset(1, VTID::from_byte(b'0'));
        output.locking_shift_right(1);

        // 0xF1 is the GR form of the drawing set's 'q'.
        assert_eq!(output.translate_key('\u{f1}'), '\u{2500}');
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
    fn the_graphics_set_maps_the_drawing_characters() {
        let mut output = TerminalOutput::new(false);
        output.designate_94_charset(0, VTID::from_byte(b'0'));

        assert_eq!(output.translate_key('q'), '\u{2500}');
        assert_eq!(output.translate_key('x'), '\u{2502}');
        assert_eq!(output.translate_key('l'), '\u{250c}');
        assert_eq!(output.translate_key('~'), '\u{00b7}');
        // Below the drawing block the set is ASCII.
        assert_eq!(output.translate_key('A'), 'A');
    }

    #[test]
    fn a_single_shift_applies_to_one_character() {
        let mut output = TerminalOutput::new(false);
        output.designate_94_charset(2, VTID::from_byte(b'0'));
        output.single_shift(2);

        assert_eq!(output.translate_key('q'), '\u{2500}');
        // The shift was consumed, so the next character uses GL again.
        assert_eq!(output.translate_key('q'), 'q');
    }

    #[test]
    fn designating_a_charset_changes_what_is_printed() {
        let mut output = TerminalOutput::new(false);
        // `A` is the British set, which differs from ASCII at `#`.
        output.designate_94_charset(0, VTID::from_byte(b'A'));
        assert!(output.needs_translation());
        assert_eq!(output.translate_key('#'), '\u{a3}');

        output.designate_94_charset(0, VTID::from_byte(b'B'));
        assert_eq!(output.translate_key('#'), '#');
        assert!(!output.needs_translation());
    }

    #[test]
    fn a_locking_shift_selects_the_designated_set() {
        let mut output = TerminalOutput::new(false);
        output.designate_94_charset(1, VTID::from_byte(b'A'));
        assert_eq!(output.translate_key('#'), '#', "G1 is not active yet");

        output.locking_shift(1);
        assert_eq!(output.translate_key('#'), '\u{a3}');

        output.locking_shift(0);
        assert_eq!(output.translate_key('#'), '#');
    }

    #[test]
    fn test_terminal_output_locking_shift_right() {
        let mut output = TerminalOutput::new(false);
        output.locking_shift_right(3);
        assert_eq!(output.get_right_set_number(), 3);
    }
}
