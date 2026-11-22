//! `RenderSettings` — runtime colour table and render modes.
//!
//! Ported from `RenderSettings.hpp/.cpp`. Upstream's class owns a raw `COLORREF`
//! array; this port owns a [`ColorTable`], the shared colour-table home from
//! `types`, so the adapter's OSC and DEC colour handlers and the renderer can
//! point at one table (ADR 002).

use types::color_fix::get_perceivable_color;
use types::color_table::{ColorAlias, ColorTable};
use types::{TextAttribute, TextColor};

/// Sentinel for "no colour configured", which is upstream's `INVALID_COLOR`.
pub const INVALID_COLOR: u32 = 0xFFFF_FFFF;

/// A renderer mode flag.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
#[repr(u8)]
pub enum Mode {
    /// Only correct indistinguishable default/indexed colours.
    IndexedDistinguishableColors = 0,
    /// Correct every indistinguishable pair.
    AlwaysDistinguishableColors = 1,
    /// Draw intense colours with a bold font.
    IntenseIsBold = 2,
    /// Draw intense colours with the bright palette entries.
    IntenseIsBright = 3,
    /// Invert the screen (`DECSCNM`).
    ScreenReversed = 4,
    /// Hold back presentation until an update completes.
    SynchronizedOutput = 5,
}

impl Mode {
    const fn bit(self) -> u8 {
        1u8 << (self as u8)
    }
}

/// The colour table and mode flags the renderer reads.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct RenderSettings {
    modes: u8,
    color_table: ColorTable,
    default_color_table: ColorTable,
    blink_should_be_faint: bool,
}

impl RenderSettings {
    /// Creates the default settings.
    pub fn new() -> Self {
        let color_table = ColorTable::new();
        let mut settings = Self {
            modes: 0,
            default_color_table: color_table.clone(),
            color_table,
            blink_should_be_faint: false,
        };
        settings.set_render_mode(Mode::IntenseIsBright, true);
        settings
    }

    /// Saves the current table and aliases as the values a reset restores.
    pub fn save_default_settings(&mut self) {
        self.default_color_table = self.color_table.clone();
    }

    /// Restores the saved table and aliases, and the two resettable modes.
    pub fn restore_default_settings(&mut self) {
        self.color_table = self.default_color_table.clone();
        self.modes &= !(Mode::ScreenReversed.bit() | Mode::SynchronizedOutput.bit());
    }

    /// Sets a render mode.
    pub fn set_render_mode(&mut self, mode: Mode, enabled: bool) {
        if enabled {
            self.modes |= mode.bit();
        } else {
            self.modes &= !mode.bit();
        }
    }

    /// Returns whether a render mode is set.
    pub const fn render_mode(&self, mode: Mode) -> bool {
        self.modes & mode.bit() != 0
    }

    /// Returns the active colour table.
    pub const fn color_table(&self) -> &ColorTable {
        &self.color_table
    }

    /// Resets the first sixteen palette entries, as `DECCTR` does.
    pub fn reset_color_table(&mut self) {
        for index in 0..16 {
            self.color_table.reset_entry(index);
        }
    }

    /// Sets one table entry from a `COLORREF`.
    pub fn set_color_table_entry(&mut self, table_index: usize, color: u32) {
        self.color_table.set(table_index, TextColor::from_colorref(color));
    }

    /// Returns one table entry as a `COLORREF`, or [`INVALID_COLOR`] when unset.
    pub fn color_table_entry(&self, table_index: usize) -> u32 {
        self.color_table
            .get(table_index)
            .map_or(INVALID_COLOR, |color| color.get_colorref())
    }

    /// Restores the 256 xterm-addressable entries from the saved defaults.
    pub fn restore_default_indexed_256_color_table(&mut self) {
        for index in 0..256 {
            if let Some(color) = self.default_color_table.get(index) {
                self.color_table.set(index, color);
            }
        }
    }

    /// Restores one entry from the saved defaults.
    pub fn restore_default_color_table_entry(&mut self, table_index: usize) {
        match self.default_color_table.get(table_index) {
            Some(color) => self.color_table.set(table_index, color),
            None => self.color_table.reset_entry(table_index),
        }
    }

    /// Points an alias at `table_index` and gives that entry a new colour.
    pub fn set_color_alias(&mut self, alias: ColorAlias, table_index: usize, color: u32) {
        self.set_color_alias_index(alias, table_index);
        self.set_color_table_entry(table_index, color);
    }

    /// Returns the colour an alias currently resolves to.
    pub fn color_alias(&self, alias: ColorAlias) -> u32 {
        self.color_table_entry(self.color_alias_index(alias))
    }

    /// Points an alias at another table entry.
    pub fn set_color_alias_index(&mut self, alias: ColorAlias, table_index: usize) {
        self.color_table.set_alias_index(alias, table_index);
    }

    /// Returns the table entry an alias points at.
    pub const fn color_alias_index(&self, alias: ColorAlias) -> usize {
        self.color_table.alias_index(alias)
    }

    /// Restores an alias to the index saved in the defaults.
    pub fn restore_default_color_alias_index(&mut self, alias: ColorAlias) {
        let index = self.default_color_table.alias_index(alias);
        self.color_table.set_alias_index(alias, index);
    }

    /// Replaces the colours attributes resolve against.
    ///
    /// The shell owns the table (a scheme is applied to the adapter's copy), so
    /// it hands the renderer the same colours each frame.
    pub fn set_color_table(&mut self, table: ColorTable) {
        self.color_table = table;
    }

    /// Returns the foreground and background `COLORREF`s for an attribute.
    pub fn get_attribute_colors(&self, attr: &TextAttribute) -> (u32, u32) {
        let foreground_text = attr.get_foreground();
        let background_text = attr.get_background();
        let default_fg_index = self.color_alias_index(ColorAlias::DefaultForeground);
        let default_bg_index = self.color_alias_index(ColorAlias::DefaultBackground);

        let brighten = attr.is_intense() && self.render_mode(Mode::IntenseIsBright);
        let dim = attr.is_faint() || (self.blink_should_be_faint && attr.is_blinking());
        let swap = attr.is_reverse_video() ^ self.render_mode(Mode::ScreenReversed);

        let mut fg = foreground_text.get_color(&self.color_table, default_fg_index, brighten);
        let mut bg = background_text.get_color(&self.color_table, default_bg_index, false);

        if dim {
            // Divide each foreground component by two.
            fg = (fg >> 1) & 0x007F_7F7F;
        }
        if swap {
            std::mem::swap(&mut fg, &mut bg);
        }
        if attr.is_invisible() {
            fg = bg;
        }

        if self.distinguishable_colors_enabled()
            && fg != bg
            && (self.render_mode(Mode::AlwaysDistinguishableColors)
                || (foreground_text.is_default_or_legacy() && background_text.is_default_or_legacy()))
        {
            fg = get_perceivable_color(fg, bg, 0.25);
        }

        (fg, bg)
    }

    /// Returns the attribute colours with the alpha byte set.
    ///
    /// The background keeps a zero alpha only when it is the default background
    /// and neither reverse video nor invisibility applies, which is what enables
    /// an acrylic backdrop.
    pub fn get_attribute_colors_with_alpha(&self, attr: &TextAttribute) -> (u32, u32) {
        let (mut fg, mut bg) = self.get_attribute_colors(attr);
        fg |= 0xFF00_0000;

        let swap = attr.is_reverse_video() ^ self.render_mode(Mode::ScreenReversed);
        if !attr.get_background().is_default() || swap || attr.is_invisible() {
            bg |= 0xFF00_0000;
        }

        (fg, bg)
    }

    /// Returns the underline colour for an attribute, falling back to the
    /// foreground when no underline colour is set.
    pub fn get_attribute_underline_color(&self, attr: &TextAttribute) -> u32 {
        let (fg, bg) = self.get_attribute_colors(attr);
        let underline_text = attr.get_underline_color();
        if underline_text.is_default() {
            return fg;
        }

        let default_index = self.color_alias_index(ColorAlias::DefaultForeground);
        let mut underline = underline_text.get_color(&self.color_table, default_index, true);
        if attr.is_invisible() {
            underline = bg;
        }

        if underline != bg
            && (self.render_mode(Mode::AlwaysDistinguishableColors)
                || (self.render_mode(Mode::IndexedDistinguishableColors)
                    && underline_text.is_default_or_legacy()
                    && attr.get_background().is_default_or_legacy()))
        {
            underline = get_perceivable_color(underline, bg, 0.25);
        }

        underline
    }

    /// Flips which blink phase the renderer is in, so a blink can dim.
    pub const fn toggle_blink_rendition(&mut self) {
        self.blink_should_be_faint = !self.blink_should_be_faint;
    }

    const fn distinguishable_colors_enabled(&self) -> bool {
        self.render_mode(Mode::IndexedDistinguishableColors) || self.render_mode(Mode::AlwaysDistinguishableColors)
    }
}

impl Default for RenderSettings {
    fn default() -> Self {
        Self::new()
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use types::TextColor;

    fn colorref(table: &ColorTable, index: usize) -> u32 {
        table.get(index).expect("default entry").get_colorref()
    }

    #[test]
    fn intense_is_bright_is_on_by_default() {
        let settings = RenderSettings::new();
        assert!(settings.render_mode(Mode::IntenseIsBright));
        assert!(!settings.render_mode(Mode::IntenseIsBold));
        assert!(!settings.render_mode(Mode::ScreenReversed));
    }

    #[test]
    fn modes_can_be_set_and_cleared() {
        let mut settings = RenderSettings::new();
        settings.set_render_mode(Mode::ScreenReversed, true);
        assert!(settings.render_mode(Mode::ScreenReversed));
        settings.set_render_mode(Mode::ScreenReversed, false);
        assert!(!settings.render_mode(Mode::ScreenReversed));
    }

    #[test]
    fn a_restore_keeps_user_modes_but_clears_screen_reverse() {
        let mut settings = RenderSettings::new();
        settings.set_render_mode(Mode::ScreenReversed, true);
        settings.set_render_mode(Mode::SynchronizedOutput, true);
        settings.set_render_mode(Mode::AlwaysDistinguishableColors, true);
        settings.set_color_table_entry(1, 0x0012_3456);

        settings.restore_default_settings();

        assert!(!settings.render_mode(Mode::ScreenReversed));
        assert!(!settings.render_mode(Mode::SynchronizedOutput));
        assert!(settings.render_mode(Mode::AlwaysDistinguishableColors));
        assert_eq!(settings.color_table_entry(1), colorref(settings.color_table(), 1));
    }

    #[test]
    fn entries_round_trip_and_reserved_slots_are_invalid() {
        let mut settings = RenderSettings::new();
        settings.set_color_table_entry(4, 0x0012_3456);
        assert_eq!(settings.color_table_entry(4), 0x0012_3456);
        assert_eq!(settings.color_table_entry(261), INVALID_COLOR);
    }

    #[test]
    fn default_attribute_colours_resolve_through_the_aliases() {
        let settings = RenderSettings::new();
        let (fg, bg) = settings.get_attribute_colors(&TextAttribute::default());

        assert_eq!(fg, colorref(settings.color_table(), 7));
        assert_eq!(bg, colorref(settings.color_table(), 0));
    }

    #[test]
    fn intense_colours_use_the_bright_entries() {
        let settings = RenderSettings::new();
        let mut attr = TextAttribute::default();
        attr.set_foreground(TextColor::index16(1));
        attr.set_intense(true);

        let (fg, _) = settings.get_attribute_colors(&attr);
        assert_eq!(fg, colorref(settings.color_table(), 9));
    }

    #[test]
    fn faint_halves_the_foreground() {
        let settings = RenderSettings::new();
        let mut attr = TextAttribute::default();
        attr.set_faint(true);

        let (fg, _) = settings.get_attribute_colors(&attr);
        assert_eq!(fg, (colorref(settings.color_table(), 7) >> 1) & 0x007F_7F7F);
    }

    #[test]
    fn reverse_video_swaps_the_colours() {
        let settings = RenderSettings::new();
        let mut attr = TextAttribute::default();
        attr.set_reverse_video(true);

        let (fg, bg) = settings.get_attribute_colors(&attr);
        assert_eq!(fg, colorref(settings.color_table(), 0));
        assert_eq!(bg, colorref(settings.color_table(), 7));
    }

    #[test]
    fn invisible_text_uses_the_background() {
        let settings = RenderSettings::new();
        let mut attr = TextAttribute::default();
        attr.set_invisible(true);

        let (fg, bg) = settings.get_attribute_colors(&attr);
        assert_eq!(fg, bg);
    }

    #[test]
    fn screen_reverse_inverts_an_unreversed_attribute() {
        let mut settings = RenderSettings::new();
        settings.set_render_mode(Mode::ScreenReversed, true);

        let (fg, bg) = settings.get_attribute_colors(&TextAttribute::default());
        assert_eq!(fg, colorref(settings.color_table(), 0));
        assert_eq!(bg, colorref(settings.color_table(), 7));
    }

    #[test]
    fn the_default_underline_colour_follows_the_foreground() {
        let settings = RenderSettings::new();
        let attr = TextAttribute::default();
        let (fg, _) = settings.get_attribute_colors(&attr);
        assert_eq!(settings.get_attribute_underline_color(&attr), fg);
    }

    #[test]
    fn an_explicit_underline_colour_is_used() {
        let settings = RenderSettings::new();
        let mut attr = TextAttribute::default();
        attr.set_underline_color(TextColor::rgb(1, 2, 3));

        assert_eq!(
            settings.get_attribute_underline_color(&attr),
            TextColor::rgb(1, 2, 3).get_colorref()
        );
    }

    #[test]
    fn the_alpha_byte_is_only_added_to_the_background_when_non_default() {
        let settings = RenderSettings::new();

        let (fg, bg) = settings.get_attribute_colors_with_alpha(&TextAttribute::default());
        assert_eq!(fg & 0xFF00_0000, 0xFF00_0000);
        assert_eq!(bg & 0xFF00_0000, 0);

        let mut attr = TextAttribute::default();
        attr.set_background(TextColor::rgb(1, 2, 3));
        let (_, bg) = settings.get_attribute_colors_with_alpha(&attr);
        assert_eq!(bg & 0xFF00_0000, 0xFF00_0000);
    }
}
