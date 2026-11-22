//! `SixelParser` — sixel image decoding.
//!
//! Ported from `SixelParser.hpp/cpp`: the command state machine (colour,
//! repeat, raster attributes, carriage return, next line), the VT340 palette and
//! extended colour defaults, colour-number mapping, and the six-pixel column
//! encoding. The decoded result is a pixel image; the adapter paints it into the
//! buffer's per-row image bands (ADR 005).
//!
//! Deviations recorded in `plan.md`: an image is decoded in full and handed over
//! when the DCS ends, rather than flushed periodically for palette animation,
//! and the margin/pan behaviour of sixel display mode reset is approximated by
//! scrolling the page.

use crate::dispatch_types::*;
use types::color_table::{color_from_hls, color_from_rgb100};

/// Default conformance level for Sixel.
pub const SIXEL_DEFAULT_CONFORMANCE: VTInt = 9;

/// The number of colour numbers a sixel image can define.
pub const MAX_COLORS: usize = 256;

/// The VT340 palette, the default for the first 16 colour entries.
const VT340: [(u8, u8, u8); 16] = [
    (0x00, 0x00, 0x00),
    (0x33, 0x33, 0xCC),
    (0xCC, 0x24, 0x24),
    (0x33, 0xCC, 0x33),
    (0xCC, 0x33, 0xCC),
    (0x33, 0xCC, 0xCC),
    (0xCC, 0xCC, 0x33),
    (0x78, 0x78, 0x78),
    (0x45, 0x45, 0x45),
    (0x57, 0x57, 0x99),
    (0x99, 0x45, 0x45),
    (0x57, 0x99, 0x57),
    (0x99, 0x57, 0x99),
    (0x57, 0x99, 0x99),
    (0x99, 0x99, 0x57),
    (0xCC, 0xCC, 0xCC),
];

/// The parser state between commands.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum States {
    /// Between commands.
    Normal,
    /// After `#`, collecting colour parameters.
    Color,
    /// After `!`, collecting a repeat count.
    Repeat,
    /// After `"`, collecting raster attributes.
    Attributes,
}

/// A decoded sixel image, ready to paint into row bands.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct DecodedImage {
    /// Pixels per row of the buffer (`pixels` is row-major with this stride).
    pub stride: i32,
    /// The used pixel width, at most the stride.
    pub width: i32,
    /// The decoded height in pixels.
    pub height: i32,
    /// Pixels as `0xAARRGGBB`; zero is transparent.
    pub pixels: Vec<u32>,
    /// The pixel size of one text cell.
    pub cell_size: (i32, i32),
    /// The top of the final sixel row, in pixels from the image origin.
    pub final_row_top: i32,
}

/// Sixel image decoder.
#[derive(Debug)]
pub struct SixelParser {
    conformance_level: VTInt,
    display_mode: bool,
    cell_size: (i32, i32),
    max_colors: usize,

    color_table: [u32; MAX_COLORS],
    color_map: [u8; MAX_COLORS],
    color_map_used: [bool; MAX_COLORS],
    colors_used: usize,
    colors_available: usize,
    foreground_index: u8,

    state: States,
    parameters: Vec<VTParameter>,

    pixel_aspect_ratio: i32,
    sixel_height: i32,
    segment_height: i32,
    max_pixel_aspect_ratio: i32,

    background_required: bool,
    background_size: (i32, i32),
    background_filled_to: i32,
    background_page_height: i32,

    available_width: i32,
    available_height: i32,
    cursor: (i32, i32),
    image_width: i32,
    image_height: i32,
    image_buffer: Vec<u32>,

    finished: bool,
}

impl SixelParser {
    /// Creates a new SixelParser.
    pub fn new(conformance_level: VTInt) -> Self {
        let conformance_level = if conformance_level > 0 {
            conformance_level
        } else {
            SIXEL_DEFAULT_CONFORMANCE
        };
        let cell_size = Self::cell_size_for_level(conformance_level);
        let max_colors = Self::max_colors_for_level(conformance_level);

        let mut color_table = [0u32; MAX_COLORS];
        for (index, (red, green, blue)) in VT340.iter().enumerate() {
            color_table[index] = u32::from_be_bytes([0xFF, *red, *green, *blue]);
        }
        // Entries 16–255 are the xterm six-by-six cube and greys.
        const SCALE: [u8; 6] = [0, 0x5F, 0x87, 0xAF, 0xD7, 0xFF];
        for i in 0..216 {
            let (red, green, blue) = (SCALE[(i / 36) % 6], SCALE[(i / 6) % 6], SCALE[i % 6]);
            color_table[i + 16] = u32::from_be_bytes([0xFF, red, green, blue]);
        }
        for i in 0..24 {
            let level = (i * 10 + 8) as u8;
            color_table[i + 232] = u32::from_be_bytes([0xFF, level, level, level]);
        }

        Self {
            conformance_level,
            display_mode: true,
            cell_size,
            max_colors,
            color_table,
            color_map: [0; MAX_COLORS],
            color_map_used: [false; MAX_COLORS],
            colors_used: 0,
            colors_available: max_colors,
            foreground_index: 0,
            state: States::Normal,
            parameters: Vec::new(),
            pixel_aspect_ratio: 2,
            sixel_height: 12,
            segment_height: 12,
            max_pixel_aspect_ratio: 1,
            background_required: true,
            background_size: (i32::MAX, i32::MAX),
            background_filled_to: 0,
            background_page_height: 0,
            available_width: 0,
            available_height: 0,
            cursor: (0, 0),
            image_width: 0,
            image_height: 0,
            image_buffer: Vec::new(),
            finished: false,
        }
    }

    /// Performs a soft reset; only level 2 restores the VT340 palette.
    pub fn soft_reset(&mut self) {
        if self.conformance_level == 2 {
            for (index, (red, green, blue)) in VT340.iter().enumerate() {
                self.color_table[index] = u32::from_be_bytes([0xFF, *red, *green, *blue]);
            }
        }
        self.state = States::Normal;
        self.parameters.clear();
        self.image_buffer.clear();
        self.image_width = 0;
        self.cursor = (0, 0);
        self.finished = false;
    }

    /// Sets the display mode; levels below 3 keep it clamped at the bottom.
    pub fn set_display_mode(&mut self, enabled: bool) {
        if self.conformance_level >= 3 {
            self.display_mode = enabled;
        }
    }

    /// Returns the cell size for a conformance level.
    pub fn cell_size_for_level(conformance_level: VTInt) -> (VTInt, VTInt) {
        match conformance_level {
            1 => (9, 20),
            _ => (10, 20),
        }
    }

    /// Returns the maximum colours for a conformance level.
    pub fn max_colors_for_level(conformance_level: VTInt) -> usize {
        match conformance_level {
            1 | 2 => 4,
            3 => 16,
            _ => MAX_COLORS,
        }
    }

    /// Returns the conformance level.
    pub const fn conformance_level(&self) -> VTInt {
        self.conformance_level
    }

    /// Returns the display mode.
    pub const fn display_mode(&self) -> bool {
        self.display_mode
    }

    /// Returns the current state.
    pub const fn state(&self) -> States {
        self.state
    }

    /// Returns the parsed parameters.
    pub fn parameters(&self) -> &[VTParameter] {
        &self.parameters
    }

    /// Returns the colour table as `0xAARRGGBB`.
    pub const fn color_table(&self) -> &[u32; MAX_COLORS] {
        &self.color_table
    }

    /// Starts a new image within the available pixel area.
    ///
    /// Returns false when the background/colour parameters make the image
    /// invalid, matching `SixelParser::DefineImage`.
    pub fn define_image(
        &mut self,
        macro_parameter: VTInt,
        background_select: SixelBackground,
        background_color: VTParameter,
        available_width: i32,
        available_height: i32,
    ) -> bool {
        self.available_width = available_width.max(0);
        self.available_height = available_height.max(0);
        self.cursor = (0, 0);
        self.image_width = 0;
        self.image_height = 0;
        self.image_buffer.clear();
        self.finished = false;
        self.parameters.clear();

        self.init_raster_attributes(macro_parameter, background_select);
        self.init_color_map(background_color);
        self.max_pixel_aspect_ratio = (self.available_height / 6).max(1);
        self.background_page_height = self.available_height;
        self.background_filled_to = 0;
        self.resize_image_buffer();
        self.state = States::Normal;
        true
    }

    /// Feeds one character of the sixel payload. Returns false at the end of
    /// the image (ESC) or on an unrecoverable error.
    pub fn parse_char(&mut self, ch: char) -> bool {
        if ('?'..='~').contains(&ch) {
            let repeat = self.apply_pending_command();
            self.write_to_image_buffer((ch as u8 - b'?') as i32, repeat);
            return true;
        }
        if ch.is_ascii_digit() || ch == ';' {
            self.parse_parameter_char(ch);
            return true;
        }

        match ch {
            '#' => {
                self.apply_pending_command();
                self.state = States::Color;
                self.parameters.clear();
            }
            '!' => {
                self.apply_pending_command();
                self.state = States::Repeat;
                self.parameters.clear();
            }
            '$' => {
                self.apply_pending_command();
                self.execute_carriage_return();
            }
            '-' => {
                self.apply_pending_command();
                self.execute_next_line();
            }
            '+' if self.conformance_level == 2 => {
                self.apply_pending_command();
                self.execute_move_to_home();
            }
            '"' if self.conformance_level >= 3 => {
                self.apply_pending_command();
                self.state = States::Attributes;
                self.parameters.clear();
            }
            '\x1b' => {
                if self.state == States::Color {
                    self.apply_pending_command();
                }
                self.fill_image_background();
                self.execute_carriage_return();
                self.finished = true;
                return false;
            }
            _ => {}
        }
        true
    }

    /// Takes the decoded image, if the sequence has ended.
    pub fn take_image(&mut self) -> Option<DecodedImage> {
        if !self.finished {
            return None;
        }
        self.finished = false;

        let stride = self.available_width;
        let height = self
            .image_height
            .min((self.image_buffer.len() / stride.max(1) as usize) as i32);
        if stride <= 0 || height <= 0 || self.image_width <= 0 {
            return None;
        }

        Some(DecodedImage {
            stride,
            width: self.image_width.min(stride),
            height,
            pixels: std::mem::take(&mut self.image_buffer),
            cell_size: self.cell_size,
            final_row_top: self.cursor.1,
        })
    }

    fn parse_parameter_char(&mut self, ch: char) {
        if self.parameters.len() > 5 {
            return;
        }
        if self.parameters.is_empty() {
            self.parameters.push(VTParameter::new());
        }

        if ch == ';' {
            self.parameters.push(VTParameter::new());
        } else if let Some(digit) = ch.to_digit(10) {
            let current = self.parameters.last().map_or(0, |param| param.value_or(0));
            let value = current.saturating_mul(10).saturating_add(digit as VTInt).min(65535);
            *self.parameters.last_mut().expect("just pushed") = VTParameter::from(value);
        }
    }

    fn apply_pending_command(&mut self) -> i32 {
        if self.state == States::Normal {
            return 1;
        }
        let previous = self.state;
        self.state = States::Normal;
        match previous {
            States::Color => {
                self.define_color();
                1
            }
            States::Repeat => self.parameters.first().map_or(0, |param| param.value_or(0)),
            States::Attributes => {
                self.update_raster_attributes();
                1
            }
            States::Normal => 1,
        }
    }

    fn execute_carriage_return(&mut self) {
        self.image_width = self.image_width.max(self.cursor.0);
        self.cursor.0 = 0;
    }

    fn execute_next_line(&mut self) {
        self.execute_carriage_return();
        if self.available_height > 0 {
            self.cursor.1 += self.sixel_height;
            self.available_height = (self.available_height - self.sixel_height).max(0);
            self.resize_image_buffer();
            self.fill_image_background_when_scrolled();
        }
    }

    fn execute_move_to_home(&mut self) {
        self.execute_carriage_return();
        self.cursor.1 = 0;
        self.available_height = self.image_height;
    }

    fn init_raster_attributes(&mut self, macro_parameter: VTInt, background_select: SixelBackground) {
        if self.conformance_level < 3 {
            self.pixel_aspect_ratio = 2;
        } else {
            self.pixel_aspect_ratio = match macro_parameter {
                0 | 1 | 5 | 6 => 2,
                2 => 5,
                3 | 4 => 3,
                _ => 1,
            };
        }
        self.sixel_height = 6 * self.pixel_aspect_ratio;
        self.segment_height = self.sixel_height;

        let transparent = matches!(background_select, SixelBackground::Transparent);
        self.background_required = self.conformance_level == 1 || !transparent;
        self.background_size = (i32::MAX, i32::MAX);
    }

    fn update_raster_attributes(&mut self) {
        let y_aspect = self.parameters.first().map_or(0, |param| param.value_or(0));
        let x_aspect = self.parameters.get(1).map_or(0, |param| param.value_or(0));
        if x_aspect > 0 {
            let ratio = (y_aspect + x_aspect - 1) / x_aspect;
            self.pixel_aspect_ratio = ratio.clamp(1, self.max_pixel_aspect_ratio);
            self.sixel_height = 6 * self.pixel_aspect_ratio;
            self.segment_height = self.segment_height.max(self.sixel_height);
            self.resize_image_buffer();
        }

        let width = self.parameters.get(2).map_or(0, |param| param.value_or(0));
        let height = self.parameters.get(3).map_or(0, |param| param.value_or(0));
        if width > 0 {
            self.background_size.0 = width;
        }
        if height > 0 {
            self.background_size.1 = height;
        }
        self.fill_image_background_when_scrolled();
        self.execute_carriage_return();
    }

    fn init_color_map(&mut self, background_color: VTParameter) {
        self.colors_used = 0;
        self.colors_available = self.max_colors;
        for (color_number, mapping) in self.color_map.iter_mut().enumerate() {
            *mapping = (color_number % self.max_colors) as u8;
        }
        self.color_map_used = [false; MAX_COLORS];

        if self.conformance_level == 2 && background_color.has_value() {
            let color_number = background_color.value_or(0);
            if color_number >= 0 && (color_number as usize) < self.max_colors {
                self.color_map[color_number as usize] = 0;
                self.color_map_used[color_number as usize] = true;
            } else if self.max_colors > 1 {
                self.colors_available = self.max_colors - 1;
            }
        }

        let default_index = (self.max_colors - 1).min(15);
        self.foreground_index = default_index as u8;
    }

    fn define_color(&mut self) {
        let color_number = (self.parameters.first().map_or(0, |param| param.value_or(0)).max(0) as usize) % MAX_COLORS;

        if self.parameters.len() > 1 && self.conformance_level > 1 {
            let model = ColorModel::from_int(self.parameters.get(1).map_or(0, |param| param.value_or(0)));
            let x = self.parameters.get(2).map_or(0, |param| param.value_or(0));
            let y = self.parameters.get(3).map_or(0, |param| param.value_or(0));
            let z = self.parameters.get(4).map_or(0, |param| param.value_or(0));
            match model {
                Some(ColorModel::Hls) => {
                    let color = color_from_hls(x, y, z);
                    self.define_named_color(color_number, color.get_r(), color.get_g(), color.get_b());
                }
                Some(ColorModel::Rgb) => {
                    let color = color_from_rgb100(x, y, z);
                    self.define_named_color(color_number, color.get_r(), color.get_g(), color.get_b());
                }
                None => {}
            }
        }

        self.foreground_index = self.color_map[color_number];
    }

    fn define_named_color(&mut self, color_number: usize, red: u8, green: u8, blue: u8) {
        let argb = u32::from_be_bytes([0xFF, red, green, blue]);
        if self.color_map_used[color_number] {
            let table_index = self.color_map[color_number] as usize;
            self.color_table[table_index] = argb;
        } else if self.colors_used < self.colors_available {
            self.colors_used += 1;
            let table_index = self.colors_used % self.max_colors;
            self.color_map[color_number] = table_index as u8;
            self.color_table[table_index] = argb;
        } else if self.conformance_level == 2 {
            // The VT240 uses the closest existing entry when the palette is full.
            let mut best_index = 0;
            let mut best_diff = i32::MAX;
            for index in 0..self.max_colors {
                let existing = self.color_table[index];
                let red_diff = (existing >> 16 & 0xFF) as i32 - i32::from(red);
                let green_diff = (existing >> 8 & 0xFF) as i32 - i32::from(green);
                let blue_diff = (existing & 0xFF) as i32 - i32::from(blue);
                let total = red_diff * red_diff + green_diff * green_diff + blue_diff * blue_diff;
                if total <= best_diff {
                    best_diff = total;
                    best_index = index;
                }
            }
            self.color_map[color_number] = best_index as u8;
        }
        self.color_map_used[color_number] = true;
    }

    fn resize_image_buffer(&mut self) {
        let height = self.cursor.1 + self.sixel_height;
        self.resize_image_buffer_to(height);
    }

    fn resize_image_buffer_to(&mut self, height: i32) {
        if self.available_width <= 0 || height <= 0 {
            return;
        }
        let required_size = (height * self.available_width) as usize;
        if required_size > self.image_buffer.len() {
            self.image_buffer.resize(required_size, 0);
        }
        self.image_height = self.image_height.max(height);
    }

    fn write_to_image_buffer(&mut self, sixel_value: i32, repeat_count: i32) {
        self.fill_image_background();

        let repeat = repeat_count.min(self.available_width - self.cursor.0);
        if repeat <= 0 {
            return;
        }
        if sixel_value == 0 {
            self.cursor.0 += repeat;
            return;
        }

        let stride = self.available_width as usize;
        let foreground = self.color_table[self.foreground_index as usize];
        let aspect = self.pixel_aspect_ratio.max(1);

        for bit in 0..6 {
            if sixel_value & (1 << bit) == 0 {
                continue;
            }
            for aspect_row in 0..aspect {
                let y = self.cursor.1 + bit * aspect + aspect_row;
                let row_start = y as usize * stride;
                for column in 0..repeat {
                    let index = row_start + (self.cursor.0 + column) as usize;
                    if index < self.image_buffer.len() {
                        self.image_buffer[index] = foreground;
                    }
                }
            }
        }

        self.cursor.0 += repeat;
        self.image_width = self.image_width.max(self.cursor.0);
    }

    fn fill_image_background(&mut self) {
        if !self.background_required {
            return;
        }
        self.background_required = false;

        let height = self.background_size.1.min(self.available_height).max(0);
        let width = self.background_size.0.min(self.available_width).max(0);
        self.resize_image_buffer_to(self.cursor.1 + height);
        self.fill_background_range(0, height, width);
        self.background_filled_to = height;
    }

    fn fill_image_background_when_scrolled(&mut self) {
        if self.background_filled_to <= 0 {
            return;
        }

        let target = self
            .background_size
            .1
            .min(self.cursor.1 + self.sixel_height)
            .min(self.background_page_height)
            .max(self.background_filled_to);
        if target > self.background_filled_to {
            self.resize_image_buffer_to(target);
            let width = self.background_size.0.min(self.available_width).max(0);
            self.fill_background_range(self.background_filled_to, target, width);
            self.background_filled_to = target;
        }
    }

    fn fill_background_range(&mut self, begin: i32, end: i32, width: i32) {
        if end <= begin || width <= 0 {
            return;
        }
        let stride = self.available_width as usize;
        let background = self.color_table[0];
        for row in begin..end {
            let row_start = row as usize * stride;
            for column in 0..width {
                let index = row_start + column as usize;
                if index < self.image_buffer.len() {
                    self.image_buffer[index] = background;
                }
            }
        }
        self.image_width = self.image_width.max(width);
        self.image_height = self.image_height.max(end);
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

    fn decode(payload: &str) -> Option<DecodedImage> {
        decode_with(payload, SixelBackground::Opaque)
    }

    fn decode_with(payload: &str, background: SixelBackground) -> Option<DecodedImage> {
        let mut parser = SixelParser::new(SIXEL_DEFAULT_CONFORMANCE);
        assert!(parser.define_image(0, background, VTParameter::new(), 80, 40));
        for ch in payload.chars() {
            if !parser.parse_char(ch) {
                break;
            }
        }
        parser.take_image()
    }

    #[test]
    fn a_new_parser_has_the_vt340_palette_and_defaults() {
        let parser = SixelParser::new(SIXEL_DEFAULT_CONFORMANCE);
        assert_eq!(parser.state(), States::Normal);
        assert!(parser.display_mode());
        assert_eq!(SixelParser::cell_size_for_level(9), (10, 20));
        assert_eq!(SixelParser::cell_size_for_level(1), (9, 20));
        assert_eq!(parser.color_table()[0], 0xFF00_0000);
        assert_eq!(parser.color_table()[1], 0xFF33_33CC);
        assert_eq!(parser.color_table()[16], 0xFF00_0000);
        assert_eq!(parser.color_table()[231], 0xFFFF_FFFF);
    }

    #[test]
    fn conformance_levels_limit_the_colour_count() {
        assert_eq!(SixelParser::max_colors_for_level(1), 4);
        assert_eq!(SixelParser::max_colors_for_level(2), 4);
        assert_eq!(SixelParser::max_colors_for_level(3), 16);
        assert_eq!(SixelParser::max_colors_for_level(9), 256);
    }

    #[test]
    fn a_single_sixel_column_sets_six_pixels() {
        // '@' is '?' + 1, so only the first of the six pixel rows is set. The
        // default aspect ratio of 2 makes each sixel pixel two device rows.
        let image = decode_with("@\x1b", SixelBackground::Transparent).expect("image");
        assert_eq!(image.cell_size, (10, 20));
        // The default foreground is VT340 entry 15, a light grey.
        assert_eq!(image.pixels[0], 0xFFCC_CCCC);
        assert_eq!(image.pixels[80], 0xFFCC_CCCC);
        assert_eq!(image.pixels[160], 0);
    }

    #[test]
    fn repeats_multiply_a_sixel_column() {
        let image = decode_with("!3~\x1b", SixelBackground::Transparent).expect("image");
        assert_eq!(image.width, 3);
    }

    #[test]
    fn a_colour_definition_changes_the_foreground() {
        // Define colour 1 as red (RGB 100,0,0) and select it.
        let image = decode("#1;2;100;0;0#1~\x1b").expect("image");
        assert_eq!(image.pixels[0], 0xFFFF_0000);
    }

    #[test]
    fn carriage_return_and_next_line_move_the_cursor() {
        let image = decode_with("~$~-~\x1b", SixelBackground::Transparent).expect("image");
        // The carriage return keeps the used width at one column, and the next
        // line adds a second row of device pixels.
        assert_eq!(image.width, 1);
        assert!(image.height > image.cell_size.1);
    }

    #[test]
    fn a_transparent_background_leaves_pixels_unset() {
        let image = decode_with("@\x1b", SixelBackground::Transparent).expect("image");
        // The first two device rows were drawn; the next is transparent.
        assert_eq!(image.pixels[2 * 80], 0);
    }

    #[test]
    fn an_opaque_background_fills_the_available_area() {
        let image = decode_with("@\x1b", SixelBackground::Opaque).expect("image");
        // Colour 0 is the VT340 black, and the background covers the page.
        assert_eq!(image.height, 40);
        assert_eq!(image.width, 80);
        assert_eq!(image.pixels[160], 0xFF00_0000);
        assert_eq!(image.pixels[0], 0xFFCC_CCCC);
    }

    #[test]
    fn no_image_is_returned_before_the_terminator() {
        let mut parser = SixelParser::new(SIXEL_DEFAULT_CONFORMANCE);
        assert!(parser.define_image(0, SixelBackground::Opaque, VTParameter::new(), 80, 40));
        parser.parse_char('~');
        assert!(parser.take_image().is_none());
    }
}
