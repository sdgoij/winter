//! `FontBuffer` — builds a soft font from a DECDLD data string.
//!
//! Ported from `FontBuffer.hpp/cpp`. The buffer receives the sixel payload of
//! a DECDLD sequence, works out the cell size the font was designed for, packs
//! the bit patterns to that size, and fills the characters outside the
//! downloaded range with an error glyph (a reverse question mark).

use types::dispatch_types::{
    CharsetSize, DrcsCellMatrix, DrcsEraseControl, DrcsFontSet, DrcsFontUsage, VTID, VTInt, VTParameter,
};

const MAX_WIDTH: VTInt = 16;
const MAX_HEIGHT: VTInt = 32;
const MAX_CHARS: usize = 96;
const BUFFER_LEN: usize = MAX_HEIGHT as usize * MAX_CHARS;

/// Builds and stores one DECDLD soft font.
#[derive(Debug, Clone)]
pub struct FontBuffer {
    cell_matrix: DrcsCellMatrix,
    pending_cell_matrix: DrcsCellMatrix,
    cell_height: VTInt,
    pending_cell_height: VTInt,
    size_declared_as_matrix: bool,
    declared_width: VTInt,
    declared_height: VTInt,
    used_width: VTInt,
    used_height: VTInt,
    full_width: VTInt,
    full_height: VTInt,
    text_width: VTInt,
    text_offset: VTInt,
    text_centering_hint: usize,

    font_set: DrcsFontSet,
    pending_font_set: DrcsFontSet,
    font_usage: DrcsFontUsage,
    pending_font_usage: DrcsFontUsage,
    lines_per_page: VTInt,
    columns_per_page: VTInt,
    is_text_font: bool,

    charset_size: CharsetSize,
    pending_charset_size: CharsetSize,
    charset_id: VTID,
    pending_charset_id: VTID,
    charset_id_initialized: bool,
    charset_id_builder: VTID,
    start_char: VTInt,
    last_char: VTInt,
    current_char: VTInt,

    buffer: Vec<u16>,
    current_char_buffer: usize,
    buffer_cleared: bool,
    sixel_column: VTInt,
    sixel_row: VTInt,
}

impl FontBuffer {
    /// Creates an empty font buffer.
    pub fn new() -> Self {
        let mut buffer = Self {
            cell_matrix: DrcsCellMatrix::Default,
            pending_cell_matrix: DrcsCellMatrix::Default,
            cell_height: 0,
            pending_cell_height: 0,
            size_declared_as_matrix: false,
            declared_width: 0,
            declared_height: 0,
            used_width: 0,
            used_height: 0,
            full_width: MAX_WIDTH,
            full_height: MAX_HEIGHT,
            text_width: MAX_WIDTH,
            text_offset: 0,
            text_centering_hint: 0,
            font_set: DrcsFontSet::Default,
            pending_font_set: DrcsFontSet::Default,
            font_usage: DrcsFontUsage::Default,
            pending_font_usage: DrcsFontUsage::Default,
            lines_per_page: 24,
            columns_per_page: 80,
            is_text_font: true,
            charset_size: CharsetSize::Size94,
            pending_charset_size: CharsetSize::Size94,
            charset_id: VTID::new(),
            pending_charset_id: VTID::new(),
            charset_id_initialized: false,
            charset_id_builder: VTID::new(),
            start_char: 0,
            last_char: 0,
            current_char: 0,
            buffer: vec![0; BUFFER_LEN],
            current_char_buffer: 0,
            buffer_cleared: true,
            sixel_column: 0,
            sixel_row: 0,
        };
        // A new buffer starts as if every rendition had just been erased.
        buffer.set_erase_control(DrcsEraseControl::AllRenditions);
        buffer
    }

    /// Ported from `SetEraseControl`.
    pub fn set_erase_control(&mut self, erase_control: DrcsEraseControl) -> bool {
        match erase_control {
            DrcsEraseControl::AllChars | DrcsEraseControl::AllRenditions => {
                // An invalid matrix is guaranteed to differ from the pending
                // one, which forces the next character to clear the buffer.
                self.cell_matrix = DrcsCellMatrix::Invalid;
                true
            }
            DrcsEraseControl::ReloadedChars => true,
            DrcsEraseControl::Other(_) => false,
        }
    }

    /// Ported from `SetAttributes`.
    pub fn set_attributes(
        &mut self,
        cell_matrix: DrcsCellMatrix,
        cell_height: VTParameter,
        font_set: DrcsFontSet,
        font_usage: DrcsFontUsage,
    ) -> bool {
        let (columns_per_page, lines_per_page) = match font_set {
            DrcsFontSet::Default | DrcsFontSet::Size80x24 => (80, 24),
            DrcsFontSet::Size80x36 => (80, 36),
            DrcsFontSet::Size80x48 => (80, 48),
            DrcsFontSet::Size132x24 => (132, 24),
            DrcsFontSet::Size132x36 => (132, 36),
            DrcsFontSet::Size132x48 => (132, 48),
            DrcsFontSet::Other(_) => return false,
        };
        self.columns_per_page = columns_per_page;
        self.lines_per_page = lines_per_page;

        self.is_text_font = match font_usage {
            DrcsFontUsage::Default | DrcsFontUsage::Text => true,
            DrcsFontUsage::FullCell => false,
            DrcsFontUsage::Other(_) => return false,
        };

        let valid = match cell_matrix {
            DrcsCellMatrix::Invalid => false,
            DrcsCellMatrix::Size5x10 => {
                self.size_declared_as_matrix = true;
                self.declared_width = 5;
                self.declared_height = 10;
                self.is_text_font
            }
            DrcsCellMatrix::Size6x10 => {
                self.size_declared_as_matrix = true;
                self.declared_width = 6;
                self.declared_height = 10;
                self.is_text_font || self.columns_per_page == 132
            }
            DrcsCellMatrix::Size7x10 => {
                self.size_declared_as_matrix = true;
                self.declared_width = 7;
                self.declared_height = 10;
                self.is_text_font
            }
            DrcsCellMatrix::Default | DrcsCellMatrix::Other(_) => {
                // Anything else is a pixel width; height comes from the
                // height parameter.
                let width = match cell_matrix {
                    DrcsCellMatrix::Other(value) => value,
                    _ => 0,
                };
                self.size_declared_as_matrix = false;
                self.declared_width = width;
                self.declared_height = cell_height.value_or(0);
                self.declared_width <= MAX_WIDTH && self.declared_height <= MAX_HEIGHT
            }
        };

        self.pending_cell_matrix = cell_matrix;
        self.pending_cell_height = cell_height.value_or(0);
        self.pending_font_set = font_set;
        self.pending_font_usage = font_usage;
        self.used_width = 0;
        self.used_height = 0;

        valid
    }

    /// Ported from `SetStartChar`.
    pub fn set_start_char(&mut self, start_char: VTParameter, charset_size: CharsetSize) -> bool {
        self.start_char = match charset_size {
            CharsetSize::Size94 => start_char.value_or(1),
            CharsetSize::Size96 => start_char.value_or(0),
            CharsetSize::Other(_) => return false,
        };

        self.current_char = self.start_char;
        self.pending_charset_size = charset_size;
        self.charset_id_initialized = false;
        self.charset_id_builder = VTID::new();
        true
    }

    /// Feeds one character of the DECDLD data string.
    ///
    /// Returns false when the sixel data runs past the end of the buffer, which
    /// is upstream's `E_OUTOFMEMORY` path; the caller then ignores the rest of
    /// the string.
    pub fn add_sixel_data(&mut self, ch: char) -> bool {
        if !self.charset_id_initialized {
            self.build_charset_id(ch);
            return true;
        }

        let value = ch as u32;
        if (0x3F..=0x7E).contains(&value) {
            self.add_sixel_value(value as VTInt - 0x3F);
            true
        } else if ch == '/' {
            self.end_of_sixel_line()
        } else if ch == ';' {
            if !self.end_of_sixel_line() {
                return false;
            }
            self.current_char += 1;
            self.prepare_next_character();
            true
        } else {
            true
        }
    }

    /// Ported from `FinalizeSixelData`.
    pub fn finalize_sixel_data(&mut self) -> bool {
        if !self.charset_id_initialized {
            return false;
        }

        if !self.end_of_sixel_line() {
            return false;
        }

        if self.buffer_cleared {
            let (full_width, full_height, text_width) = self.calculate_dimensions();
            self.full_width = full_width;
            self.full_height = full_height;
            self.text_width = text_width;
            self.pack_and_center_bit_patterns();
            self.fill_unused_characters();
        }

        true
    }

    /// Returns the packed bit patterns, `MAX_CHARS * cell height` words.
    pub fn bit_pattern(&self) -> &[u16] {
        let length = (MAX_CHARS * self.full_height.max(0) as usize).min(self.buffer.len());
        &self.buffer[..length]
    }

    /// Returns the full cell size in pixels.
    pub const fn cell_size(&self) -> (VTInt, VTInt) {
        (self.full_width, self.full_height)
    }

    /// Returns the renderer's centering hint.
    pub const fn text_centering_hint(&self) -> usize {
        self.text_centering_hint
    }

    /// Returns the charset ID this font is designated as.
    pub const fn designation(&self) -> VTID {
        self.charset_id
    }

    fn build_charset_id(&mut self, ch: char) {
        let value = ch as u32;
        if (0x20..=0x2F).contains(&value) {
            self.charset_id_builder = self.charset_id_builder.with_intermediate(value as u8);
        } else if (0x30..=0x7E).contains(&value) {
            self.pending_charset_id = self.charset_id_builder.finalize_with(value as u8);
            self.charset_id_initialized = true;
            self.prepare_character_buffer();
        }
    }

    fn prepare_character_buffer(&mut self) {
        if self.cell_matrix != self.pending_cell_matrix
            || self.cell_height != self.pending_cell_height
            || self.font_set != self.pending_font_set
            || self.font_usage != self.pending_font_usage
            || self.charset_size != self.pending_charset_size
            || self.charset_id != self.pending_charset_id
        {
            self.cell_matrix = self.pending_cell_matrix;
            self.cell_height = self.pending_cell_height;
            self.font_set = self.pending_font_set;
            self.font_usage = self.pending_font_usage;
            self.charset_size = self.pending_charset_size;
            self.charset_id = self.pending_charset_id;

            // The final size is not known until the sixel data arrives.
            self.full_width = MAX_WIDTH;
            self.full_height = MAX_HEIGHT;
            self.text_width = MAX_WIDTH;
            self.text_offset = 0;
            self.buffer.fill(0);
            self.buffer_cleared = true;
        } else {
            self.buffer_cleared = false;
        }

        self.prepare_next_character();
    }

    fn prepare_next_character(&mut self) {
        self.last_char = self.current_char;
        self.current_char_buffer = self.current_char.max(0) as usize * self.full_height.max(0) as usize;
        self.sixel_column = 0;
        self.sixel_row = 0;

        if !self.buffer_cleared && self.current_char < MAX_CHARS as VTInt {
            let start = self.current_char_buffer;
            let end = (start + self.full_height.max(0) as usize).min(self.buffer.len());
            if start < end {
                self.buffer[start..end].fill(0);
            }
        }
    }

    fn add_sixel_value(&mut self, value: VTInt) {
        if self.current_char < MAX_CHARS as VTInt && self.sixel_column < self.text_width {
            let shift = (self.sixel_column + self.text_offset).max(0) as u32;
            if shift < 16 {
                let output_column_bit = 0x8000u16 >> shift;

                for i in 0..6 {
                    if self.sixel_row + i >= self.full_height {
                        break;
                    }
                    let index = self.current_char_buffer + i as usize;
                    if index >= self.buffer.len() {
                        break;
                    }
                    if value & (1 << i) != 0 {
                        self.buffer[index] |= output_column_bit;
                    }
                }
            }
        }
        self.sixel_column += 1;
    }

    fn end_of_sixel_line(&mut self) -> bool {
        if self.current_char_buffer >= self.buffer.len() || self.sixel_row >= MAX_HEIGHT {
            return false;
        }

        self.current_char_buffer += 6;
        self.sixel_row += 6;
        self.used_width = self.used_width.max(self.sixel_column);
        self.used_height = self.used_height.max(self.sixel_row);
        self.sixel_column = 0;
        true
    }

    fn calculate_dimensions(&self) -> (VTInt, VTInt, VTInt) {
        if self.size_declared_as_matrix {
            if self.columns_per_page == 132 && self.declared_width <= 6 {
                return (6, 10, 0);
            }
            return (10, 10, 8);
        }

        if self.declared_width != 0 && self.declared_height != 0 && !self.is_text_font {
            return (self.declared_width, self.declared_height, 0);
        }

        let text_width = if self.is_text_font { self.declared_width } else { 0 };

        if self.lines_per_page != 24 {
            let cell_width = if self.columns_per_page == 132 { 6 } else { 10 };
            let cell_height = if self.lines_per_page == 48 { 8 } else { 10 };
            return (cell_width, cell_height, text_width);
        }

        let in_range = |cell_width: VTInt, cell_height: VTInt| {
            let sixel_height = (cell_height + 5) / 6 * 6;
            let height_in_range = if self.declared_height != 0 {
                self.declared_height <= cell_height
            } else {
                self.used_height <= sixel_height
            };
            let width_in_range = if self.declared_width != 0 {
                self.declared_width <= cell_width
            } else {
                self.used_width <= cell_width
            };
            height_in_range && width_in_range
        };
        let no_declared_size = self.declared_width == 0 && self.declared_height == 0;

        if self.columns_per_page == 80 {
            if in_range(8, 10) && no_declared_size {
                (10, 10, 8)
            } else if in_range(15, 12) {
                (15, 12, text_width)
            } else if in_range(10, 16) {
                (10, 16, text_width)
            } else if in_range(10, 20) {
                (10, 20, text_width)
            } else if in_range(12, 30) {
                (12, 30, text_width)
            } else {
                (MAX_WIDTH, MAX_HEIGHT, text_width)
            }
        } else if in_range(6, 10) && no_declared_size {
            (6, 10, 0)
        } else if in_range(9, 12) {
            (9, 12, text_width)
        } else if in_range(6, 16) {
            (6, 16, text_width)
        } else if in_range(6, 20) {
            (6, 20, text_width)
        } else if in_range(7, 30) {
            (7, 30, text_width)
        } else {
            (MAX_WIDTH, MAX_HEIGHT, text_width)
        }
    }

    fn pack_and_center_bit_patterns(&mut self) {
        if self.text_width == 0 {
            self.text_width = self.full_width;
        }
        self.text_width = self.text_width.min(self.full_width);
        self.text_offset = (self.full_width - self.text_width) / 2;
        let clip_shift = self.text_width.max(0) as u32;
        let clipping_mask = if clip_shift >= 16 {
            0
        } else {
            !(0xFFFFu16 >> clip_shift)
        };

        self.text_centering_hint = if self.declared_width != 0 {
            (self.full_width - (self.declared_width + self.text_offset * 2)).max(0) as usize
        } else {
            0
        };

        let mut destination = 0;
        for source in 0..self.buffer.len() {
            if (source as VTInt % MAX_HEIGHT) < self.full_height {
                let mut line = self.buffer[source];
                line &= clipping_mask;
                line >>= self.text_offset.max(0) as u32;
                self.buffer[destination] = line;
                destination += 1;
            }
        }
    }

    fn fill_unused_characters(&mut self) {
        let pattern = self.generate_error_glyph();
        for ch in 0..MAX_CHARS as VTInt {
            if ch < self.start_char || ch > self.last_char {
                let start = ch.max(0) as usize * self.full_height.max(0) as usize;
                for (offset, line) in pattern.iter().take(self.full_height.max(0) as usize).enumerate() {
                    if let Some(slot) = self.buffer.get_mut(start + offset) {
                        *slot = *line;
                    }
                }
            }
        }
    }

    fn generate_error_glyph(&self) -> [u16; MAX_HEIGHT as usize] {
        const INPUT_PATTERN: [u16; MAX_HEIGHT as usize] = [
            0b000000000000000,
            0b000000000000000,
            0b000000000000000,
            0b000000000000000,
            0b000000000000000,
            0b000000000000000,
            0b001111111110000,
            0b011111111111000,
            0b111000000011100,
            0b111000000011100,
            0b111000000000000,
            0b111000000000000,
            0b111100000000000,
            0b011111000000000,
            0b000011110000000,
            0b000001110000000,
            0b000001110000000,
            0b000001110000000,
            0b000001110000000,
            0b000000000000000,
            0b000001110000000,
            0b000001110000000,
            0b000001110000000,
            0,
            0,
            0,
            0,
            0,
            0,
            0,
            0,
            0,
        ];
        const WIDTH_MASKS: [u32; MAX_WIDTH as usize + 1] = [
            0, 1, 3, 32771, 8457, 9481, 9545, 9673, 42441, 26061, 58829, 28141, 60909, 63453, 63485, 65533, 65535,
        ];
        const HEIGHT_MASKS: [u32; MAX_HEIGHT as usize + 1] = [
            0, 1, 3, 7, 15, 1613952, 10002560, 10002816, 10068352, 10068353, 26845569, 26847617, 26847619, 26864003,
            28961155, 28961219, 28961731, 62516163, 62516167, 129625031, 129625039, 129756111, 263973839, 263974863,
            268169167, 536604623, 536608719, 536608735, 536870879, 1073741791, 2147483615, 2147483647, 4294967295,
        ];

        let width_mask = WIDTH_MASKS[(self.full_width.max(0) as usize).min(MAX_WIDTH as usize)];
        let height_mask = HEIGHT_MASKS[(self.full_height.max(0) as usize).min(MAX_HEIGHT as usize)];

        let mut output = [0u16; MAX_HEIGHT as usize];
        let mut output_index = 0;
        for (y, input) in INPUT_PATTERN.iter().enumerate() {
            if height_mask & (1 << y) == 0 {
                continue;
            }

            let mut line = 0u16;
            for x in (0..MAX_WIDTH).rev() {
                if width_mask & (1 << x) != 0 {
                    line <<= 1;
                    line |= u16::from(*input & (1 << x) != 0);
                }
            }
            line <<= (MAX_WIDTH - self.full_width.max(0)) as u32;
            output[output_index] = line;
            output_index += 1;
        }
        output
    }
}

impl Default for FontBuffer {
    fn default() -> Self {
        Self::new()
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    fn load(
        cell_matrix: DrcsCellMatrix,
        cell_height: VTInt,
        font_set: DrcsFontSet,
        font_usage: DrcsFontUsage,
    ) -> Option<(VTInt, VTInt)> {
        let mut buffer = FontBuffer::new();
        assert!(buffer.set_erase_control(DrcsEraseControl::AllChars));
        if !buffer.set_attributes(cell_matrix, VTParameter::from(cell_height), font_set, font_usage) {
            return None;
        }
        assert!(buffer.set_start_char(VTParameter::from(0), CharsetSize::Size94));

        assert!(buffer.add_sixel_data('B'));
        assert!(buffer.add_sixel_data('C'));
        assert!(buffer.add_sixel_data(';'));
        if !buffer.finalize_sixel_data() {
            return None;
        }
        Some(buffer.cell_size())
    }

    #[test]
    fn matrix_sizes_choose_the_vt2xx_cell() {
        assert_eq!(
            load(DrcsCellMatrix::Size5x10, 0, DrcsFontSet::Size80x24, DrcsFontUsage::Text),
            Some((10, 10))
        );
        assert_eq!(
            load(DrcsCellMatrix::Size6x10, 0, DrcsFontSet::Size80x24, DrcsFontUsage::Text),
            Some((10, 10))
        );
        assert_eq!(
            load(
                DrcsCellMatrix::Size7x10,
                0,
                DrcsFontSet::Size132x24,
                DrcsFontUsage::Text
            ),
            Some((10, 10))
        );
        assert_eq!(
            load(
                DrcsCellMatrix::Size6x10,
                0,
                DrcsFontSet::Size132x24,
                DrcsFontUsage::Text
            ),
            Some((6, 10))
        );
    }

    #[test]
    fn matrix_sizes_require_a_text_font_unless_132_columns() {
        assert_eq!(
            load(
                DrcsCellMatrix::Size5x10,
                0,
                DrcsFontSet::Size80x24,
                DrcsFontUsage::FullCell
            ),
            None
        );
        assert_eq!(
            load(
                DrcsCellMatrix::Size6x10,
                0,
                DrcsFontSet::Size132x24,
                DrcsFontUsage::FullCell
            ),
            Some((6, 10))
        );
        assert_eq!(
            load(
                DrcsCellMatrix::Size7x10,
                0,
                DrcsFontSet::Size132x24,
                DrcsFontUsage::FullCell
            ),
            None
        );
        assert_eq!(
            load(DrcsCellMatrix::Invalid, 0, DrcsFontSet::Size80x24, DrcsFontUsage::Text),
            None
        );
    }

    #[test]
    fn explicit_full_cell_dimensions_are_the_cell_size() {
        assert_eq!(
            load(
                DrcsCellMatrix::Other(13),
                17,
                DrcsFontSet::Size80x24,
                DrcsFontUsage::FullCell
            ),
            Some((13, 17))
        );
        assert_eq!(
            load(
                DrcsCellMatrix::Other(18),
                38,
                DrcsFontSet::Size80x24,
                DrcsFontUsage::FullCell
            ),
            None
        );
    }

    #[test]
    fn a_downloaded_character_reaches_the_bit_pattern() {
        let mut buffer = FontBuffer::new();
        assert!(buffer.set_erase_control(DrcsEraseControl::AllChars));
        assert!(buffer.set_attributes(
            DrcsCellMatrix::Other(13),
            VTParameter::from(17),
            DrcsFontSet::Size80x24,
            DrcsFontUsage::FullCell
        ));
        assert!(buffer.set_start_char(VTParameter::from(0), CharsetSize::Size94));
        for ch in "B~;".chars() {
            assert!(buffer.add_sixel_data(ch));
        }
        assert!(buffer.finalize_sixel_data());

        assert_eq!(buffer.designation().as_bytes(), b"B");
        assert_eq!(buffer.cell_size(), (13, 17));
        assert!(buffer.bit_pattern()[..17].iter().any(|line| *line != 0));
    }

    #[test]
    fn characters_outside_the_range_receive_the_error_glyph() {
        let mut buffer = FontBuffer::new();
        assert!(buffer.set_erase_control(DrcsEraseControl::AllChars));
        assert!(buffer.set_attributes(
            DrcsCellMatrix::Other(13),
            VTParameter::from(17),
            DrcsFontSet::Size80x24,
            DrcsFontUsage::FullCell
        ));
        // Start at 0, download one character, so the rest are error glyphs.
        assert!(buffer.set_start_char(VTParameter::from(0), CharsetSize::Size94));
        for ch in "B~;".chars() {
            assert!(buffer.add_sixel_data(ch));
        }
        assert!(buffer.finalize_sixel_data());

        let pattern = buffer.bit_pattern();
        let height = buffer.cell_size().1 as usize;
        // `last_char` tracks the next character, so character 1 is still the
        // cleared slot; character 2 onward receives the error glyph.
        assert!(pattern[height * 2..height * 3].iter().any(|line| *line != 0));
    }

    #[test]
    fn an_uninitialized_charset_id_cannot_finalize() {
        let mut buffer = FontBuffer::new();
        assert!(buffer.set_erase_control(DrcsEraseControl::AllRenditions));
        assert!(!buffer.finalize_sixel_data());
    }
}
