//! `FontInfoBase` — the face name, family, weight and code page of a font.
//!
//! Ported from `FontInfoBase.hpp/.cpp`. Upstream describes a font with a
//! Windows `LOGFONT`; these are the portable fields the renderer reads.

/// Face name that requests the default TrueType font.
pub const DEFAULT_TT_FONT_FACENAME: &str = "__DefaultTTFont__";
/// Face name of the default raster font.
pub const DEFAULT_RASTER_FONT_FACENAME: &str = "Terminal";

/// Size of the legacy `LOGFONT::lfFaceName` buffer, in UTF-16 units.
pub const LF_FACESIZE: usize = 32;

/// The `TMPF_TRUETYPE` family flag.
pub(crate) const TMPF_TRUETYPE: u8 = 0x04;

/// The portable part of a font description.
#[derive(Debug, Clone, Default, PartialEq, Eq)]
pub struct FontInfoBase {
    face_name: String,
    family: u8,
    weight: u32,
    code_page: u32,
}

impl FontInfoBase {
    /// Returns the requested face name.
    pub fn face_name(&self) -> &str {
        &self.face_name
    }

    /// Returns the family flags (`TMPF_*`).
    pub const fn family(&self) -> u8 {
        self.family
    }

    /// Returns the font weight.
    pub const fn weight(&self) -> u32 {
        self.weight
    }

    /// Returns the code page.
    pub const fn code_page(&self) -> u32 {
        self.code_page
    }

    /// Sets the requested face name.
    pub fn set_face_name(&mut self, face_name: String) {
        self.face_name = face_name;
    }

    /// Sets the family flags.
    pub const fn set_family(&mut self, family: u8) {
        self.family = family;
    }

    /// Sets the font weight.
    pub const fn set_weight(&mut self, weight: u32) {
        self.weight = weight;
    }

    /// Sets the code page.
    pub const fn set_code_page(&mut self, code_page: u32) {
        self.code_page = code_page;
    }

    /// Fills a null-terminated UTF-16 `lfFaceName` buffer, truncating the name
    /// to fit and zeroing the rest.
    pub fn fill_legacy_name_buffer(&self, buffer: &mut [u16; LF_FACESIZE]) {
        let mut units = self.face_name.encode_utf16();
        let limit = LF_FACESIZE - 1;
        for slot in buffer.iter_mut().take(limit) {
            *slot = units.next().unwrap_or(0);
        }
        buffer[limit] = 0;
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn defaults_are_empty() {
        let info = FontInfoBase::default();
        assert_eq!(info.face_name(), "");
        assert_eq!(info.family(), 0);
        assert_eq!(info.weight(), 0);
        assert_eq!(info.code_page(), 0);
    }

    #[test]
    fn setters_round_trip() {
        let mut info = FontInfoBase::default();
        info.set_face_name("Consolas".to_string());
        info.set_family(TMPF_TRUETYPE);
        info.set_weight(700);
        info.set_code_page(65001);

        assert_eq!(info.face_name(), "Consolas");
        assert_eq!(info.family(), TMPF_TRUETYPE);
        assert_eq!(info.weight(), 700);
        assert_eq!(info.code_page(), 65001);
    }

    #[test]
    fn the_legacy_buffer_is_null_terminated() {
        let mut info = FontInfoBase::default();
        info.set_face_name("Consolas".to_string());

        let mut buffer = [0xFFFFu16; LF_FACESIZE];
        info.fill_legacy_name_buffer(&mut buffer);

        assert_eq!(&buffer[..8], "Consolas".encode_utf16().collect::<Vec<_>>());
        assert_eq!(buffer[8], 0);
        assert!(buffer[9..].iter().all(|unit| *unit == 0));
    }

    #[test]
    fn a_long_name_is_truncated_to_thirty_one_units() {
        let mut info = FontInfoBase::default();
        info.set_face_name("A".repeat(40));

        let mut buffer = [0xFFFFu16; LF_FACESIZE];
        info.fill_legacy_name_buffer(&mut buffer);

        assert!(buffer[..31].iter().all(|unit| *unit == u16::from(b'A')));
        assert_eq!(buffer[31], 0);
    }
}
