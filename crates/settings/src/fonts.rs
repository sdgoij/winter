//! The font a pane measures and paints with.
//!
//! Upstream's profile carries `font: { face, size, weight, features }`. Only
//! the size has a consumer here: the egui shell measures its cell from one
//! embedded monospace family, so a face or weight would be configuration no
//! code reads (ADR 022's rule). The schema keeps the object shape, so those
//! fields can arrive without a migration when a font loader exists.

use serde::de::{self, Visitor};
use serde::{Deserialize, Deserializer, Serialize};

/// The smallest point size the shell honours.
pub const MIN_SIZE: f32 = 4.0;
/// The largest point size the shell honours.
pub const MAX_SIZE: f32 = 100.0;
/// The size a file that names none gets.
pub const DEFAULT_SIZE: f32 = 16.0;

/// The grid's type.
#[derive(Debug, Clone, Copy, PartialEq, Serialize, Deserialize)]
#[serde(default)]
pub struct Font {
    /// The point size the grid is measured and painted at.
    #[serde(deserialize_with = "size_from_number")]
    pub size: f32,
}

/// Reads a size written as either an integer or a float.
///
/// TOML distinguishes `size = 16` from `size = 16.0`, and a hand-written file
/// will use the first; a serde derive would refuse it as the wrong type.
fn size_from_number<'de, D>(deserializer: D) -> Result<f32, D::Error>
where
    D: Deserializer<'de>,
{
    struct Number;

    impl Visitor<'_> for Number {
        type Value = f32;

        fn expecting(&self, formatter: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
            formatter.write_str("a point size")
        }

        fn visit_i64<E: de::Error>(self, value: i64) -> Result<f32, E> {
            Ok(value as f32)
        }

        fn visit_u64<E: de::Error>(self, value: u64) -> Result<f32, E> {
            Ok(value as f32)
        }

        fn visit_f64<E: de::Error>(self, value: f64) -> Result<f32, E> {
            Ok(value as f32)
        }
    }

    deserializer.deserialize_any(Number)
}

impl Default for Font {
    fn default() -> Self {
        Self { size: DEFAULT_SIZE }
    }
}

impl Font {
    /// Clamps a size outside the usable range.
    ///
    /// A size is normalized rather than rejected for the same reason a blank
    /// command is: the file stays usable and the window still starts.
    pub(crate) fn normalize(mut self) -> Self {
        self.size = self.size.clamp(MIN_SIZE, MAX_SIZE);
        self
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn the_default_font_is_readable_at_sixteen_points() {
        assert_eq!(Font::default().size, 16.0);
        assert_eq!(Font::default().normalize(), Font::default());
    }

    #[test]
    fn a_size_outside_the_range_is_clamped() {
        assert_eq!(Font { size: 0.0 }.normalize().size, MIN_SIZE);
        assert_eq!(Font { size: -3.0 }.normalize().size, MIN_SIZE);
        assert_eq!(Font { size: 500.0 }.normalize().size, MAX_SIZE);
        assert_eq!(Font { size: 12.5 }.normalize().size, 12.5);
    }
}
