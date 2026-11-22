//! `FontInfo` — the font metrics an engine actually produced.
//!
//! Ported from `FontInfo.hpp` and `fontinfo.cpp`. Unlike `FontInfoDesired`, the
//! pixel cell size here is always a measured, non-zero value.

use crate::font_info_base::{FontInfoBase, TMPF_TRUETYPE};
use utils::Size;

/// A cell size in device-independent pixels.
#[derive(Debug, Clone, Copy, Default, PartialEq)]
pub struct CellSizeInDIP {
    /// Cell width in DIPs.
    pub width: f32,
    /// Cell height in DIPs.
    pub height: f32,
}

impl CellSizeInDIP {
    /// Truncates the DIP size to whole units.
    ///
    /// Upstream calls this `AsInteger_DoNotUse`: the conversion is lossy and
    /// does not round-trip, and the name is a marker for legacy callers.
    pub fn as_integer_do_not_use(&self) -> Size {
        Size::new(
            self.width.round_ties_even() as i32,
            self.height.round_ties_even() as i32,
        )
    }
}

/// The measured font metrics.
#[derive(Debug, Clone, Default, PartialEq)]
pub struct FontInfo {
    base: FontInfoBase,
    font_size_in_pt: f32,
    cell_size_in_dip: CellSizeInDIP,
    cell_size_in_physical_px: Size,
}

impl FontInfo {
    /// Returns the face name the engine used.
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

    /// Returns the font size in points.
    pub const fn font_size_in_pt(&self) -> f32 {
        self.font_size_in_pt
    }

    /// Returns the cell size in DIPs.
    pub const fn cell_size_in_dip(&self) -> CellSizeInDIP {
        self.cell_size_in_dip
    }

    /// Returns the cell size in physical pixels.
    pub const fn cell_size_in_physical_px(&self) -> Size {
        self.cell_size_in_physical_px
    }

    /// Records the metrics the engine produced.
    #[allow(clippy::too_many_arguments)]
    pub fn set_from_engine(
        &mut self,
        face_name: String,
        family: u8,
        weight: u32,
        code_page: u32,
        font_size_in_pt: f32,
        cell_size_in_dip: CellSizeInDIP,
        cell_size_in_physical_px: Size,
    ) {
        self.base = FontInfoBase::default();
        self.base.set_face_name(face_name);
        self.base.set_family(family);
        self.base.set_weight(weight);
        self.base.set_code_page(code_page);
        self.font_size_in_pt = font_size_in_pt;
        self.cell_size_in_dip = cell_size_in_dip;
        self.cell_size_in_physical_px = cell_size_in_physical_px;
    }

    /// Returns whether the engine used a TrueType font.
    pub const fn is_true_type_font(&self) -> bool {
        self.base.family() & TMPF_TRUETYPE != 0
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn engine_metrics_round_trip() {
        let mut info = FontInfo::default();
        info.set_from_engine(
            "Cascadia Mono".to_string(),
            TMPF_TRUETYPE,
            400,
            65001,
            12.0,
            CellSizeInDIP {
                width: 8.0,
                height: 16.0,
            },
            Size::new(8, 16),
        );

        assert_eq!(info.face_name(), "Cascadia Mono");
        assert_eq!(info.weight(), 400);
        assert_eq!(info.code_page(), 65001);
        assert_eq!(info.font_size_in_pt(), 12.0);
        assert_eq!(info.cell_size_in_physical_px(), Size::new(8, 16));
        assert!(info.is_true_type_font());
    }

    #[test]
    fn dip_to_integer_rounds_half_to_even() {
        let size = CellSizeInDIP {
            width: 7.5,
            height: 8.5,
        };
        // 7.5 -> 8 and 8.5 -> 8 under the default floating-point rounding mode.
        assert_eq!(size.as_integer_do_not_use(), Size::new(8, 8));
    }

    #[test]
    fn default_is_not_true_type() {
        assert!(!FontInfo::default().is_true_type_font());
    }
}
