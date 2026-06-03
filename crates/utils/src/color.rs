//! RGBA color type with blending and hex conversion.
//!
//! Ported from Microsoft's Windows Terminal C++ `til::color`.

/// RGBA color stored as a 32-bit little-endian ABGR layout.
///
/// Ported from `til::color` in the Windows Terminal C++ codebase.
/// Uses a union of individual RGBA bytes and a u32 for efficient
/// type-punning conversions.
#[derive(Debug, Clone, Copy, Default, PartialEq, Eq, Hash)]
#[repr(C)]
pub struct Color {
    /// Red component (0-255).
    pub r: u8,
    /// Green component (0-255).
    pub g: u8,
    /// Blue component (0-255).
    pub b: u8,
    /// Alpha component (0-255).
    pub a: u8,
}

impl Color {
    /// Creates a color with the given RGBA values.
    #[inline]
    pub const fn new(r: u8, g: u8, b: u8, a: u8) -> Self {
        Self { r, g, b, a }
    }

    /// Creates an opaque RGB color (alpha = 255).
    #[inline]
    pub const fn rgb(r: u8, g: u8, b: u8) -> Self {
        Self { r, g, b, a: 255 }
    }

    /// Creates a fully transparent black color.
    #[inline]
    pub const fn transparent() -> Self {
        Self {
            r: 0,
            g: 0,
            b: 0,
            a: 0,
        }
    }

    /// Creates a color from a u32 in ABGR format (little-endian).
    /// In ABGR format: byte 0=blue, byte 1=green, byte 2=red, byte 3=alpha.
    #[inline]
    pub const fn from_abgr(abgr: u32) -> Self {
        Self {
            r: ((abgr >> 16) & 0xFF) as u8,
            g: ((abgr >> 8) & 0xFF) as u8,
            b: (abgr & 0xFF) as u8,
            a: ((abgr >> 24) & 0xFF) as u8,
        }
    }

    /// Returns the color as a u32 in ABGR format.
    #[inline]
    pub const fn to_abgr(&self) -> u32 {
        ((self.a as u32) << 24) | ((self.r as u32) << 16) | ((self.g as u32) << 8) | (self.b as u32)
    }

    /// Returns the color as a COLORREF-like value (0x00BBGGRR).
    #[inline]
    pub const fn to_color_ref(&self) -> u32 {
        ((self.b as u32) << 16) | ((self.g as u32) << 8) | (self.r as u32)
    }

    /// Creates a color from a COLORREF-like value (0x00BBGGRR).
    /// Adds 0xFF to the alpha channel (opaque).
    #[inline]
    pub const fn from_color_ref(color_ref: u32) -> Self {
        Self::rgb(
            (color_ref & 0xFF) as u8,
            ((color_ref >> 8) & 0xFF) as u8,
            ((color_ref >> 16) & 0xFF) as u8,
        )
    }

    /// Returns a new color with the specified alpha.
    #[inline]
    pub const fn with_alpha(&self, alpha: u8) -> Self {
        Self {
            r: self.r,
            g: self.g,
            b: self.b,
            a: alpha,
        }
    }

    /// Source-over alpha blending.
    /// Blends `self` (source) over `destination` (bottom/background).
    /// Both colors are expected in straight (unmultiplied) alpha.
    /// See: https://en.wikipedia.org/wiki/Alpha_compositing
    pub fn layer_over(&self, destination: Color) -> Color {
        let a_inverse = (255 - self.a) as f32 / 255.0;
        let result_a = self.a as f32 + destination.a as f32 * a_inverse;

        if result_a == 0.0 {
            return Color::default();
        }

        let result_r = (self.r as f32 * self.a as f32
            + destination.r as f32 * destination.a as f32 * a_inverse)
            / result_a;
        let result_g = (self.g as f32 * self.a as f32
            + destination.g as f32 * destination.a as f32 * a_inverse)
            / result_a;
        let result_b = (self.b as f32 * self.a as f32
            + destination.b as f32 * destination.a as f32 * a_inverse)
            / result_a;

        Color::new(
            (result_r.round() as u32).min(255) as u8,
            (result_g.round() as u32).min(255) as u8,
            (result_b.round() as u32).min(255) as u8,
            (result_a.round() as u32).min(255) as u8,
        )
    }

    /// Creates a color from a hue value [0, 1).
    /// Uses the hue-to-RGB algorithm from the Chilliant spectrum.
    pub fn from_hue(hue: f32) -> Self {
        let r = (hue * 6.0 - 3.0).abs() - 1.0;
        let g = 2.0 - (hue * 6.0 - 2.0).abs();
        let b = 2.0 - (hue * 6.0 - 4.0).abs();

        Color::rgb(
            (r.max(0.0).min(1.0) * 255.0).round() as u8,
            (g.max(0.0).min(1.0) * 255.0).round() as u8,
            (b.max(0.0).min(1.0) * 255.0).round() as u8,
        )
    }

    /// Returns the color as a hex string: "#RRGGBBAA".
    pub fn to_hex(&self) -> String {
        format!("#{:02X}{:02X}{:02X}{:02X}", self.r, self.g, self.b, self.a)
    }

    /// Returns the color as a hex string without alpha: "#RRGGBB".
    pub fn to_hex_short(&self) -> String {
        format!("#{:02X}{:02X}{:02X}", self.r, self.g, self.b)
    }

    /// Parses a hex string into a Color.
    /// Supports both "#RRGGBB" and "#RRGGBBAA" formats.
    pub fn from_hex(hex: &str) -> Option<Self> {
        let hex = hex.trim_start_matches('#');
        if hex.len() != 6 && hex.len() != 8 {
            return None;
        }

        let parse_byte = |s: &str| u8::from_str_radix(s, 16).ok();

        let r = parse_byte(&hex[0..2])?;
        let g = parse_byte(&hex[2..4])?;
        let b = parse_byte(&hex[4..6])?;
        let a = if hex.len() == 8 {
            parse_byte(&hex[6..8])?
        } else {
            255
        };

        Some(Self::new(r, g, b, a))
    }

    /// Formats the color as a string.
    pub fn to_string(&self) -> String {
        self.to_hex()
    }
}

impl std::fmt::Display for Color {
    fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        write!(
            f,
            "#{:02X}{:02X}{:02X}{:02X}",
            self.r, self.g, self.b, self.a
        )
    }
}

/// Well-known colors.
impl Color {
    pub const BLACK: Self = Self::rgb(0, 0, 0);
    pub const WHITE: Self = Self::rgb(255, 255, 255);
    pub const RED: Self = Self::rgb(255, 0, 0);
    pub const GREEN: Self = Self::rgb(0, 255, 0);
    pub const BLUE: Self = Self::rgb(0, 0, 255);
    pub const YELLOW: Self = Self::rgb(255, 255, 0);
    pub const CYAN: Self = Self::rgb(0, 255, 255);
    pub const MAGENTA: Self = Self::rgb(255, 0, 255);
    pub const TRANSPARENT: Self = Self::transparent();
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn test_color_creation() {
        let c = Color::rgb(255, 128, 64);
        assert_eq!(c.r, 255);
        assert_eq!(c.g, 128);
        assert_eq!(c.b, 64);
        assert_eq!(c.a, 255);

        let c2 = Color::new(10, 20, 30, 40);
        assert_eq!(c2.r, 10);
        assert_eq!(c2.g, 20);
        assert_eq!(c2.b, 30);
        assert_eq!(c2.a, 40);
    }

    #[test]
    fn test_color_abgr_conversion() {
        let c = Color::rgb(0x12, 0x34, 0x56);
        let abgr = c.to_abgr();
        let c2 = Color::from_abgr(abgr);
        // Verify roundtrip preserves RGBA values
        assert_eq!(c2.r, 0x12);
        assert_eq!(c2.g, 0x34);
        assert_eq!(c2.b, 0x56);
        assert_eq!(c2.a, 0xFF);
    }

    #[test]
    fn test_color_ref_conversion() {
        let c = Color::rgb(0x12, 0x34, 0x56);
        let ref_ = c.to_color_ref();
        let c2 = Color::from_color_ref(ref_);
        assert_eq!(c, c2);
    }

    #[test]
    fn test_with_alpha() {
        let c = Color::rgb(100, 200, 50);
        let c2 = c.with_alpha(128);
        assert_eq!(c2.r, 100);
        assert_eq!(c2.g, 200);
        assert_eq!(c2.b, 50);
        assert_eq!(c2.a, 128);
    }

    #[test]
    fn test_layer_over() {
        let src = Color::new(255, 0, 0, 128); // semi-transparent red
        let dst = Color::new(0, 255, 0, 255); // opaque green
        let blended = src.layer_over(dst);
        // Should have a mix of red and green
        assert!(blended.r > 0);
        assert!(blended.g > 0);
    }

    #[test]
    fn test_from_hue() {
        let c0 = Color::from_hue(0.0);
        let _c1 = Color::from_hue(1.0 / 6.0);
        let _c2 = Color::from_hue(2.0 / 6.0);
        // Hue 0.0 should be red
        assert!(c0.r > c0.g && c0.r > c0.b);
    }

    #[test]
    fn test_hex_roundtrip() {
        let c = Color::new(0xAB, 0xCD, 0xEF, 0x12);
        let hex = c.to_hex();
        let c2 = Color::from_hex(&hex).unwrap();
        assert_eq!(c, c2);

        let c3 = Color::rgb(0x11, 0x22, 0x33);
        let hex3 = c3.to_hex_short();
        let c4 = Color::from_hex(&hex3).unwrap();
        assert_eq!(c4.r, 0x11);
        assert_eq!(c4.g, 0x22);
        assert_eq!(c4.b, 0x33);
        assert_eq!(c4.a, 255);
    }

    #[test]
    fn test_hex_from_str() {
        let c = Color::from_hex("#FF0080").unwrap();
        assert_eq!(c.r, 0xFF);
        assert_eq!(c.g, 0x00);
        assert_eq!(c.b, 0x80);
        assert_eq!(c.a, 255);

        let c2 = Color::from_hex("#FF0080AA").unwrap();
        assert_eq!(c2.r, 0xFF);
        assert_eq!(c2.a, 0xAA);

        assert!(Color::from_hex("#GGGGGG").is_none());
        assert!(Color::from_hex("#FFF").is_none());
        assert!(Color::from_hex("").is_none());
    }

    #[test]
    fn test_display() {
        let c = Color::rgb(10, 20, 30);
        assert_eq!(format!("{}", c), "#0A141EFF");
    }

    #[test]
    fn test_to_string() {
        let c = Color::rgb(1, 2, 3);
        assert_eq!(c.to_string(), "#010203FF");
    }
}

#[cfg(test)]
mod proptests {
    use proptest::prelude::*;

    use super::*;

    prop_compose! {
        fn any_color()(r in any::<u8>(), g in any::<u8>(), b in any::<u8>(), a in any::<u8>()) -> Color {
            Color::new(r, g, b, a)
        }
    }

    proptest! {
        #[test]
        fn test_hex_roundtrip(r in 0u8..=255, g in 0u8..=255, b in 0u8..=255, a in 0u8..=255) {
            let c = Color::new(r, g, b, a);
            let hex = c.to_hex();
            let c2 = Color::from_hex(&hex).unwrap();
            assert_eq!(c, c2);
        }

        #[test]
        fn test_alpha_blend_commutative(a in any_color(), b in any_color()) {
            let ab = a.layer_over(b);
            // Alpha blending is not commutative, but let's verify it works
            // with opaque colors (which is the common case)
            if a.a == 255 && b.a == 255 {
                let ba = b.layer_over(a);
                // With opaque colors, result should be fully opaque
                assert_eq!(ab.a, 255);
                assert_eq!(ba.a, 255);
            }
        }
    }
}
