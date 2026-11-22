//! `FontInfoDesired` — the font size a caller asks the engine for.
//!
//! Ported from `FontInfoDesired.hpp/.cpp`. Both cell dimensions may be unset,
//! because the engine is allowed to choose them.

use crate::css_length::CSSLengthPercentage;
use crate::font_info_base::{FontInfoBase, TMPF_TRUETYPE};
use utils::Size;

/// The requested cell metrics and attributes.
#[derive(Debug, Clone, PartialEq)]
pub struct FontInfoDesired {
    base: FontInfoBase,
    cell_width: CSSLengthPercentage,
    cell_height: CSSLengthPercentage,
    font_size_in_pt: f32,
    builtin_glyphs: bool,
    color_glyphs: bool,
}

impl Default for FontInfoDesired {
    fn default() -> Self {
        Self {
            base: FontInfoBase::default(),
            cell_width: CSSLengthPercentage::default(),
            cell_height: CSSLengthPercentage::default(),
            font_size_in_pt: 0.0,
            builtin_glyphs: false,
            // Colour glyphs are on by default, as upstream initialises them.
            color_glyphs: true,
        }
    }
}

impl FontInfoDesired {
    /// Returns the requested face name.
    pub fn face_name(&self) -> &str {
        self.base.face_name()
    }

    /// Returns the family flags.
    pub const fn family(&self) -> u8 {
        self.base.family()
    }

    /// Returns the font weight.
    pub const fn weight(&self) -> u32 {
        self.base.weight()
    }

    /// Returns the code page.
    pub const fn code_page(&self) -> u32 {
        self.base.code_page()
    }

    /// Sets the requested face name.
    pub fn set_face_name(&mut self, face_name: String) {
        self.base.set_face_name(face_name);
    }

    /// Sets the family flags.
    pub const fn set_family(&mut self, family: u8) {
        self.base.set_family(family);
    }

    /// Sets the font weight.
    pub const fn set_weight(&mut self, weight: u32) {
        self.base.set_weight(weight);
    }

    /// Sets the code page.
    pub const fn set_code_page(&mut self, code_page: u32) {
        self.base.set_code_page(code_page);
    }

    /// Fills a null-terminated legacy face-name buffer.
    pub fn fill_legacy_name_buffer(&self, buffer: &mut [u16; crate::font_info_base::LF_FACESIZE]) {
        self.base.fill_legacy_name_buffer(buffer);
    }

    /// Returns the requested cell width.
    pub const fn cell_width(&self) -> CSSLengthPercentage {
        self.cell_width
    }

    /// Returns the requested cell height.
    pub const fn cell_height(&self) -> CSSLengthPercentage {
        self.cell_height
    }

    /// Returns the requested font size in points.
    pub const fn font_size_in_pt(&self) -> f32 {
        self.font_size_in_pt
    }

    /// Returns whether built-in glyphs are enabled.
    pub const fn enable_builtin_glyphs(&self) -> bool {
        self.builtin_glyphs
    }

    /// Returns whether colour glyphs are enabled.
    pub const fn enable_color_glyphs(&self) -> bool {
        self.color_glyphs
    }

    /// Sets the requested cell width.
    pub fn set_cell_width(&mut self, cell_width: CSSLengthPercentage) {
        self.cell_width = cell_width;
    }

    /// Sets the requested cell height.
    pub fn set_cell_height(&mut self, cell_height: CSSLengthPercentage) {
        self.cell_height = cell_height;
    }

    /// Sets the requested font size in points.
    pub fn set_font_size_in_pt(&mut self, font_size_in_pt: f32) {
        self.font_size_in_pt = font_size_in_pt;
    }

    /// Enables or disables built-in glyphs.
    pub const fn set_enable_builtin_glyphs(&mut self, builtin_glyphs: bool) {
        self.builtin_glyphs = builtin_glyphs;
    }

    /// Enables or disables colour glyphs.
    pub const fn set_enable_color_glyphs(&mut self, color_glyphs: bool) {
        self.color_glyphs = color_glyphs;
    }

    /// Resolves the requested cell size to whole pixels at 96 DPI.
    pub fn pixel_cell_size(&self) -> Size {
        Size::new(
            self.cell_width.resolve(0.0, 96.0, 12.0, 0.0).round_ties_even() as i32,
            self.cell_height.resolve(0.0, 96.0, 12.0, 0.0).round_ties_even() as i32,
        )
    }

    /// Sets the cell size in pixels.
    pub fn set_pixel_cell_size(&mut self, size: Size) {
        self.cell_width = CSSLengthPercentage::from_pixel(size.width() as f32);
        self.cell_height = CSSLengthPercentage::from_pixel(size.height() as f32);
    }

    /// Returns whether the requested font is a TrueType font.
    pub const fn is_true_type_font(&self) -> bool {
        self.base.family() & TMPF_TRUETYPE != 0
    }

    /// Returns whether this describes the default raster font.
    ///
    /// Upstream treats an empty face name with either of two size pairs as the
    /// default raster font: the engine's `(0, 0)`, or the classic `(8, 12)`.
    pub fn is_default_raster_font(&self) -> bool {
        let zero = CSSLengthPercentage::from_pixel(0.0);
        let eight = CSSLengthPercentage::from_pixel(8.0);
        let twelve = CSSLengthPercentage::from_pixel(12.0);
        self.face_name().is_empty()
            && ((self.cell_width == zero && self.cell_height == zero)
                || (self.cell_width == eight && self.cell_height == twelve))
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::css_length::ReferenceFrame;

    #[test]
    fn colour_glyphs_default_to_on() {
        let desired = FontInfoDesired::default();
        assert!(desired.enable_color_glyphs());
        assert!(!desired.enable_builtin_glyphs());
        assert_eq!(desired.cell_width().reference(), ReferenceFrame::None);
    }

    #[test]
    fn a_point_cell_size_resolves_to_pixels() {
        let mut desired = FontInfoDesired::default();
        desired.set_cell_width(CSSLengthPercentage::from_string("12pt"));
        desired.set_cell_height(CSSLengthPercentage::from_string("18pt"));

        assert_eq!(desired.pixel_cell_size(), Size::new(16, 24));
    }

    #[test]
    fn a_pixel_cell_size_round_trips() {
        let mut desired = FontInfoDesired::default();
        desired.set_pixel_cell_size(Size::new(8, 12));

        assert_eq!(desired.pixel_cell_size(), Size::new(8, 12));
    }

    #[test]
    fn the_default_raster_font_is_recognised() {
        let mut desired = FontInfoDesired::default();
        desired.set_pixel_cell_size(Size::new(8, 12));
        assert!(desired.is_default_raster_font());

        desired.set_pixel_cell_size(Size::new(0, 0));
        assert!(desired.is_default_raster_font());

        desired.set_pixel_cell_size(Size::new(9, 12));
        assert!(!desired.is_default_raster_font());

        desired.set_pixel_cell_size(Size::new(8, 12));
        desired.set_face_name("Consolas".to_string());
        assert!(!desired.is_default_raster_font());
    }

    #[test]
    fn the_true_type_flag_is_read_from_the_family() {
        let mut desired = FontInfoDesired::default();
        assert!(!desired.is_true_type_font());
        desired.set_family(crate::font_info_base::TMPF_TRUETYPE);
        assert!(desired.is_true_type_font());
    }

    #[test]
    fn base_fields_are_delegated() {
        let mut desired = FontInfoDesired::default();
        desired.set_face_name("Cascadia Mono".to_string());
        desired.set_weight(400);
        desired.set_code_page(65001);

        assert_eq!(desired.face_name(), "Cascadia Mono");
        assert_eq!(desired.weight(), 400);
        assert_eq!(desired.code_page(), 65001);
    }
}
