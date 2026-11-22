//! `ColorFix` — perceptual colour nudging in the Oklab colour space.
//!
//! Ported from `ColorFix.hpp/.cpp`. Upstream ships a 256-entry sRGB-to-linear
//! table that its generator built from the standard transfer function; this port
//! evaluates the function instead, which is the same mapping without a literal.
//! The `cbrtf_est` cube-root estimator is replaced by the exact `cbrt`, which
//! only improves precision.

/// A linear-light RGB triple.
#[derive(Debug, Clone, Copy)]
struct LinearRgb {
    r: f32,
    g: f32,
    b: f32,
}

/// An Oklab colour.
#[derive(Debug, Clone, Copy)]
struct Oklab {
    l: f32,
    a: f32,
    b: f32,
}

fn srgb_to_linear(channel: u8) -> f32 {
    let value = f32::from(channel) / 255.0;
    if value <= 0.04045 {
        value / 12.92
    } else {
        ((value + 0.055) / 1.055).powf(2.4)
    }
}

fn linear_to_srgb(channel: f32) -> f32 {
    let value = channel.clamp(0.0, 1.0);
    let encoded = if value > 0.003_130_8 {
        1.055 * value.powf(1.0 / 2.4) - 0.055
    } else {
        12.92 * value
    };
    encoded * 255.0
}

/// Splits a `COLORREF` (`0x00BBGGRR`) into linear-light components.
fn colorref_to_linear(color: u32) -> LinearRgb {
    LinearRgb {
        r: srgb_to_linear(color as u8),
        g: srgb_to_linear((color >> 8) as u8),
        b: srgb_to_linear((color >> 16) as u8),
    }
}

/// Packs linear-light components back into a `COLORREF`.
fn linear_to_colorref(color: LinearRgb) -> u32 {
    let round = |channel: f32| linear_to_srgb(channel).round_ties_even() as u32;
    round(color.r) | (round(color.g) << 8) | (round(color.b) << 16)
}

// The Oklab conversions are Björn Ottosson's public-domain reference formulas
// (https://bottosson.github.io/posts/oklab/), as upstream uses them.

fn linear_srgb_to_oklab(color: LinearRgb) -> Oklab {
    let l = 0.412_221_47 * color.r + 0.536_332_54 * color.g + 0.051_445_99 * color.b;
    let m = 0.211_903_5 * color.r + 0.680_699_5 * color.g + 0.107_396_96 * color.b;
    let s = 0.088_302_46 * color.r + 0.281_718_85 * color.g + 0.629_978_7 * color.b;

    let l_ = l.cbrt();
    let m_ = m.cbrt();
    let s_ = s.cbrt();

    Oklab {
        l: 0.210_454_26 * l_ + 0.793_617_8 * m_ - 0.004_072_047 * s_,
        a: 1.977_998_5 * l_ - 2.428_592_2 * m_ + 0.450_593_7 * s_,
        b: 0.025_904_037 * l_ + 0.782_771_77 * m_ - 0.808_675_77 * s_,
    }
}

fn oklab_to_linear_srgb(color: Oklab) -> LinearRgb {
    let l_ = color.l + 0.396_337_78 * color.a + 0.215_803_76 * color.b;
    let m_ = color.l - 0.105_561_346 * color.a - 0.063_854_17 * color.b;
    let s_ = color.l - 0.089_484_18 * color.a - 1.291_485_5 * color.b;

    let l = l_ * l_ * l_;
    let m = m_ * m_ * m_;
    let s = s_ * s_ * s_;

    LinearRgb {
        r: 4.076_741_7 * l - 3.307_711_6 * m + 0.230_969_94 * s,
        g: -1.268_438 * l + 2.609_757_4 * m - 0.341_319_38 * s,
        b: -0.004_196_086_3 * l - 0.703_418_6 * m + 1.707_614_7 * s,
    }
}

/// Changes `color` so that it is visually far enough from `reference` to be
/// readable, using the ΔEOK distance.
pub fn get_perceivable_color(color: u32, reference: u32, min_squared_distance: f32) -> u32 {
    let reference_oklab = linear_srgb_to_oklab(colorref_to_linear(reference));
    let mut color_oklab = linear_srgb_to_oklab(colorref_to_linear(color));

    let dl = reference_oklab.l - color_oklab.l;
    let da = reference_oklab.a - color_oklab.a;
    let db = reference_oklab.b - color_oklab.b;
    let distance = dl * dl + da * da + db * db;
    if distance >= min_squared_distance {
        return color;
    }

    // ΔEOK is a euclidean distance, so the minimum lightness change that makes
    // the distance large enough follows directly.
    let mut delta_l = (min_squared_distance - da * da - db * db).sqrt();

    // Keep the brightness relationship: a darker colour gets darker.
    if color_oklab.l < reference_oklab.l {
        delta_l = -delta_l;
    }

    // This is upstream's simple gamut clip; it can wash out saturated colours,
    // but it is cheap and only runs when the user enabled the mode.
    color_oklab.l = reference_oklab.l + delta_l;
    if !(0.0..=1.0).contains(&color_oklab.l) {
        color_oklab.l = reference_oklab.l - delta_l;
    }

    linear_to_colorref(oklab_to_linear_srgb(color_oklab)) | (color & 0xFF00_0000)
}

/// Adjusts a colour's perceptual lightness by `delta`.
pub fn adjust_lightness(color: u32, delta: f32) -> u32 {
    let mut oklab = linear_srgb_to_oklab(colorref_to_linear(color));
    oklab.l = (oklab.l + delta).clamp(0.0, 1.0);
    linear_to_colorref(oklab_to_linear_srgb(oklab)) | (color & 0xFF00_0000)
}

/// Returns a colour's perceptual lightness in the range `0..=1`.
pub fn get_lightness(color: u32) -> f32 {
    linear_srgb_to_oklab(colorref_to_linear(color)).l
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn lightness_spans_black_to_white() {
        assert!((get_lightness(0x0000_0000) - 0.0).abs() < 0.01);
        assert!((get_lightness(0x00FF_FFFF) - 1.0).abs() < 0.01);
    }

    #[test]
    fn adjusting_lightness_moves_the_colour() {
        let grey = 0x0080_8080;
        assert!(get_lightness(adjust_lightness(grey, 0.2)) > get_lightness(grey));
        assert!(get_lightness(adjust_lightness(grey, -0.2)) < get_lightness(grey));
    }

    #[test]
    fn far_apart_colours_are_left_alone() {
        let color = 0x00FF_0000;
        assert_eq!(get_perceivable_color(color, 0x0000_00FF, 0.25), color);
    }

    #[test]
    fn identical_colours_are_nudged_apart() {
        for color in [0x0000_0000, 0x0080_8080, 0x00FF_FFFF] {
            let fixed = get_perceivable_color(color, color, 0.25);
            assert_ne!(fixed, color, "{color:#010x} should move");
        }
    }

    #[test]
    fn the_alpha_byte_is_preserved() {
        let color = 0xAB00_0000;
        assert_eq!(get_perceivable_color(color, color, 0.25) & 0xFF00_0000, 0xAB00_0000);
        assert_eq!(adjust_lightness(color, 0.1) & 0xFF00_0000, 0xAB00_0000);
    }
}
