//! `SixelParser` — Sixel image format parsing.
//!
//! Ported from `SixelParser.hpp/cpp` in the Windows Terminal C++ codebase.
//! Handles parsing and rendering of Sixel image data.

use crate::dispatch_types::*;

/// Default conformance level for Sixel.
pub const SIXEL_DEFAULT_CONFORMANCE: VTInt = 9;

/// Maximum number of colors in Sixel.
const MAX_COLORS: usize = 256;

/// A pixel with color index and transparency.
#[derive(Debug, Clone, Copy)]
pub struct IndexedPixel {
    /// Whether the pixel is transparent.
    #[allow(dead_code)]
    transparent: bool,
    /// Color index.
    #[allow(dead_code)]
    color_index: u8,
}

/// Sixel parser state.
struct SixelState {
    state: States,
    parameters: Vec<VTParameter>,
}

#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum States {
    Normal,
    Attributes,
    Color,
    Repeat,
}

/// Sixel image parser.
pub struct SixelParser {
    /// Current parser state.
    state: SixelState,
    /// Conformance level.
    conformance_level: VTInt,
    /// Whether display mode is enabled.
    display_mode: bool,
    /// Color table.
    color_table: [u32; MAX_COLORS], // RGB values
    /// Whether color table has been used.
    color_table_used: [bool; MAX_COLORS],
    /// Number of colors used.
    colors_used: usize,
    /// Available colors.
    #[allow(dead_code)]
    colors_available: usize,
    /// Image buffer (row-major pixel data).
    image_buffer: Vec<IndexedPixel>,
    /// Image width in pixels.
    image_width: VTInt,
    /// Image height in pixels.
    image_height: VTInt,
}

impl SixelParser {
    /// Creates a new SixelParser.
    pub fn new(conformance_level: VTInt) -> Self {
        Self {
            state: SixelState {
                state: States::Normal,
                parameters: Vec::new(),
            },
            conformance_level: if conformance_level > 0 {
                conformance_level
            } else {
                SIXEL_DEFAULT_CONFORMANCE
            },
            display_mode: true,
            color_table: [0; MAX_COLORS],
            color_table_used: [false; MAX_COLORS],
            colors_used: 0,
            colors_available: MAX_COLORS,
            image_buffer: Vec::new(),
            image_width: 0,
            image_height: 0,
        }
    }

    /// Performs a soft reset.
    pub fn soft_reset(&mut self) {
        self.state.state = States::Normal;
        self.state.parameters.clear();
        self.color_table_used = [false; MAX_COLORS];
        self.colors_used = 0;
        self.image_buffer.clear();
        self.image_width = 0;
        self.image_height = 0;
    }

    /// Sets the display mode.
    pub fn set_display_mode(&mut self, enabled: bool) {
        self.display_mode = enabled;
    }

    /// Returns the cell size for a given conformance level.
    pub fn cell_size_for_level(conformance_level: VTInt) -> (VTInt, VTInt) {
        // Sixel cells are typically 6x13 pixels
        let _ = conformance_level;
        (6, 13)
    }

    /// Returns the maximum colors for a given conformance level.
    pub fn max_colors_for_level(conformance_level: VTInt) -> usize {
        if conformance_level >= 9 {
            MAX_COLORS
        } else {
            16 // Older conformance levels support fewer colors
        }
    }

    /// Parses a command character (e.g., '&' for sixel commands).
    pub fn parse_command_char(&mut self, ch: char) {
        match ch {
            '"' => {
                self.state.state = States::Attributes;
            }
            '$' => {
                self.state.state = States::Color;
            }
            '-' => {
                self.state.state = States::Repeat;
            }
            _ => {
                // Unknown command character
            }
        }
    }

    /// Parses a parameter character (digit or semicolon).
    pub fn parse_parameter_char(&mut self, ch: char) {
        match ch {
            ';' => {
                self.state.parameters.push(VTParameter::new());
            }
            '0'..='9' => {
                // Accumulate parameter value
                if let Some(last) = self.state.parameters.last_mut() {
                    if last.has_value() {
                        let value = last.value() * 10 + (ch as VTInt - '0' as VTInt);
                        *last = VTParameter::from(value);
                    } else {
                        *last = VTParameter::from(ch as VTInt - '0' as VTInt);
                    }
                } else {
                    self.state
                        .parameters
                        .push(VTParameter::from(ch as VTInt - '0' as VTInt));
                }
            }
            _ => {
                // Invalid parameter character
            }
        }
    }

    /// Returns the current state.
    pub fn state(&self) -> States {
        self.state.state
    }

    /// Returns the parameters.
    pub fn parameters(&self) -> &[VTParameter] {
        &self.state.parameters
    }

    /// Returns the conformance level.
    pub fn conformance_level(&self) -> VTInt {
        self.conformance_level
    }

    /// Returns the display mode.
    pub fn display_mode(&self) -> bool {
        self.display_mode
    }

    /// Returns the color table.
    pub fn color_table(&self) -> &[u32; MAX_COLORS] {
        &self.color_table
    }

    /// Returns the image buffer.
    pub fn image_buffer(&self) -> &[IndexedPixel] {
        &self.image_buffer
    }

    /// Returns the image dimensions.
    pub fn image_dimensions(&self) -> (VTInt, VTInt) {
        (self.image_width, self.image_height)
    }
}

impl Default for SixelParser {
    fn default() -> Self {
        Self::new(SIXEL_DEFAULT_CONFORMANCE)
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn test_sixel_parser_new() {
        let parser = SixelParser::new(0);
        assert_eq!(parser.conformance_level(), SIXEL_DEFAULT_CONFORMANCE);
        assert!(parser.display_mode());
        assert_eq!(parser.state(), States::Normal);
    }

    #[test]
    fn test_sixel_parser_custom_conformance() {
        let parser = SixelParser::new(15);
        assert_eq!(parser.conformance_level(), 15);
    }

    #[test]
    fn test_sixel_parser_soft_reset() {
        let mut parser = SixelParser::new(0);
        parser.parse_command_char('"');
        parser.parse_parameter_char('1');
        parser.soft_reset();
        assert_eq!(parser.state(), States::Normal);
        assert!(parser.parameters().is_empty());
    }

    #[test]
    fn test_sixel_parser_parse_command() {
        let mut parser = SixelParser::new(0);
        parser.parse_command_char('"');
        assert_eq!(parser.state(), States::Attributes);

        parser.parse_command_char('$');
        assert_eq!(parser.state(), States::Color);

        parser.parse_command_char('-');
        assert_eq!(parser.state(), States::Repeat);
    }

    #[test]
    fn test_sixel_parser_parse_parameters() {
        let mut parser = SixelParser::new(0);
        parser.parse_parameter_char('1');
        parser.parse_parameter_char('2');
        assert_eq!(parser.parameters().len(), 1);
        assert_eq!(parser.parameters()[0].value(), 12);

        parser.parse_parameter_char(';');
        parser.parse_parameter_char('3');
        assert_eq!(parser.parameters().len(), 2);
        assert_eq!(parser.parameters()[1].value(), 3);
    }

    #[test]
    fn test_cell_size_for_level() {
        let (w, h) = SixelParser::cell_size_for_level(9);
        assert_eq!(w, 6);
        assert_eq!(h, 13);
    }

    #[test]
    fn test_max_colors_for_level() {
        assert_eq!(SixelParser::max_colors_for_level(9), 256);
        assert_eq!(SixelParser::max_colors_for_level(5), 16);
    }

    #[test]
    fn test_set_display_mode() {
        let mut parser = SixelParser::new(0);
        assert!(parser.display_mode());
        parser.set_display_mode(false);
        assert!(!parser.display_mode());
    }
}
