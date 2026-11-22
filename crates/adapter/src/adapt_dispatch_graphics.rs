//! SGR — Set Graphics Rendition, and DECSCA character protection.
//!
//! Ported from `adaptDispatchGraphics.cpp` in the Windows Terminal C++ adapter.
//! The option decoder is `apply_graphics_option`; `GraphicsOptions` is the SGR
//! parameter table. Colours are the Windows console palette order, which is what
//! `TextColor::index16` indexes.

use types::dispatch_types::{GraphicsOptions, LogicalAttributeOptions};
use types::underline_style::UnderlineStyle;
use types::{TextAttribute, TextColor, VTParameter, VTParameters, VTSubParameters};

use crate::AdaptDispatch;

/// Windows console palette indices, in the order the DARK_/BRIGHT_ constants use.
const DARK_BLACK: u8 = 0;
const DARK_BLUE: u8 = 1;
const DARK_GREEN: u8 = 2;
const DARK_CYAN: u8 = 3;
const DARK_RED: u8 = 4;
const DARK_MAGENTA: u8 = 5;
const DARK_YELLOW: u8 = 6;
const DARK_WHITE: u8 = 7;
const BRIGHT_BLACK: u8 = 8;
const BRIGHT_BLUE: u8 = 9;
const BRIGHT_GREEN: u8 = 10;
const BRIGHT_CYAN: u8 = 11;
const BRIGHT_RED: u8 = 12;
const BRIGHT_MAGENTA: u8 = 13;
const BRIGHT_YELLOW: u8 = 14;
const BRIGHT_WHITE: u8 = 15;

impl AdaptDispatch {
    /// Ported from `_ApplyGraphicsOptions`: applies each option in order.
    pub(crate) fn apply_graphics_options(options: &VTParameters, attributes: &mut TextAttribute) {
        let mut index = 0;
        while index < options.size() {
            index += Self::apply_graphics_option(options, index, attributes);
        }
    }

    /// Ported from `_ApplyGraphicsOption`: returns how many options were consumed,
    /// which is more than one for the `38`/`48` extended-colour forms.
    pub(crate) fn apply_graphics_option(options: &VTParameters, index: usize, attributes: &mut TextAttribute) -> usize {
        if options.has_sub_params_for(index) {
            if let Some(sub_params) = options.sub_params_for(index) {
                let collected = collect_sub_params(sub_params);
                apply_graphics_option_with_sub_params(options.at(index), &collected, attributes);
            }
            return 1;
        }

        let Some(option) = GraphicsOptions::from_int(options.at(index).value_or(0)) else {
            return 1;
        };

        match option {
            GraphicsOptions::Off => {
                attributes.set_default_foreground();
                attributes.set_default_background();
                attributes.set_default_underline_color();
                attributes.set_default_rendition_attributes();
            }
            GraphicsOptions::ForegroundDefault => attributes.set_default_foreground(),
            GraphicsOptions::BackgroundDefault => attributes.set_default_background(),
            GraphicsOptions::UnderlineColorDefault => attributes.set_default_underline_color(),
            GraphicsOptions::Intense => attributes.set_intense(true),
            GraphicsOptions::RGBColorOrFaint => attributes.set_faint(true),
            GraphicsOptions::NotIntenseOrFaint => {
                attributes.set_intense(false);
                attributes.set_faint(false);
            }
            GraphicsOptions::Italics => attributes.set_italic(true),
            GraphicsOptions::NotItalics => attributes.set_italic(false),
            GraphicsOptions::BlinkOrXterm256Index | GraphicsOptions::RapidBlink => attributes.set_blinking(true),
            GraphicsOptions::Steady => attributes.set_blinking(false),
            GraphicsOptions::Invisible => attributes.set_invisible(true),
            GraphicsOptions::Visible => attributes.set_invisible(false),
            GraphicsOptions::CrossedOut => attributes.set_crossed_out(true),
            GraphicsOptions::NotCrossedOut => attributes.set_crossed_out(false),
            GraphicsOptions::Negative => attributes.set_reverse_video(true),
            GraphicsOptions::Positive => attributes.set_reverse_video(false),
            GraphicsOptions::Underline => attributes.set_underline_style(UnderlineStyle::SinglyUnderlined),
            GraphicsOptions::DoublyUnderlined => attributes.set_underline_style(UnderlineStyle::DoublyUnderlined),
            GraphicsOptions::NoUnderline => attributes.set_underline_style(UnderlineStyle::NoUnderline),
            GraphicsOptions::Overline => attributes.set_overlined(true),
            GraphicsOptions::NoOverline => attributes.set_overlined(false),

            GraphicsOptions::ForegroundBlack => attributes.set_indexed_foreground(DARK_BLACK),
            GraphicsOptions::ForegroundBlue => attributes.set_indexed_foreground(DARK_BLUE),
            GraphicsOptions::ForegroundGreen => attributes.set_indexed_foreground(DARK_GREEN),
            GraphicsOptions::ForegroundCyan => attributes.set_indexed_foreground(DARK_CYAN),
            GraphicsOptions::ForegroundRed => attributes.set_indexed_foreground(DARK_RED),
            GraphicsOptions::ForegroundMagenta => attributes.set_indexed_foreground(DARK_MAGENTA),
            GraphicsOptions::ForegroundYellow => attributes.set_indexed_foreground(DARK_YELLOW),
            GraphicsOptions::ForegroundWhite => attributes.set_indexed_foreground(DARK_WHITE),

            GraphicsOptions::BackgroundBlack => attributes.set_indexed_background(DARK_BLACK),
            GraphicsOptions::BackgroundBlue => attributes.set_indexed_background(DARK_BLUE),
            GraphicsOptions::BackgroundGreen => attributes.set_indexed_background(DARK_GREEN),
            GraphicsOptions::BackgroundCyan => attributes.set_indexed_background(DARK_CYAN),
            GraphicsOptions::BackgroundRed => attributes.set_indexed_background(DARK_RED),
            GraphicsOptions::BackgroundMagenta => attributes.set_indexed_background(DARK_MAGENTA),
            GraphicsOptions::BackgroundYellow => attributes.set_indexed_background(DARK_YELLOW),
            GraphicsOptions::BackgroundWhite => attributes.set_indexed_background(DARK_WHITE),

            GraphicsOptions::BrightForegroundBlack => attributes.set_indexed_foreground(BRIGHT_BLACK),
            GraphicsOptions::BrightForegroundBlue => attributes.set_indexed_foreground(BRIGHT_BLUE),
            GraphicsOptions::BrightForegroundGreen => attributes.set_indexed_foreground(BRIGHT_GREEN),
            GraphicsOptions::BrightForegroundCyan => attributes.set_indexed_foreground(BRIGHT_CYAN),
            GraphicsOptions::BrightForegroundRed => attributes.set_indexed_foreground(BRIGHT_RED),
            GraphicsOptions::BrightForegroundMagenta => attributes.set_indexed_foreground(BRIGHT_MAGENTA),
            GraphicsOptions::BrightForegroundYellow => attributes.set_indexed_foreground(BRIGHT_YELLOW),
            GraphicsOptions::BrightForegroundWhite => attributes.set_indexed_foreground(BRIGHT_WHITE),

            GraphicsOptions::BrightBackgroundBlack => attributes.set_indexed_background(BRIGHT_BLACK),
            GraphicsOptions::BrightBackgroundBlue => attributes.set_indexed_background(BRIGHT_BLUE),
            GraphicsOptions::BrightBackgroundGreen => attributes.set_indexed_background(BRIGHT_GREEN),
            GraphicsOptions::BrightBackgroundCyan => attributes.set_indexed_background(BRIGHT_CYAN),
            GraphicsOptions::BrightBackgroundRed => attributes.set_indexed_background(BRIGHT_RED),
            GraphicsOptions::BrightBackgroundMagenta => attributes.set_indexed_background(BRIGHT_MAGENTA),
            GraphicsOptions::BrightBackgroundYellow => attributes.set_indexed_background(BRIGHT_YELLOW),
            GraphicsOptions::BrightBackgroundWhite => attributes.set_indexed_background(BRIGHT_WHITE),

            GraphicsOptions::ForegroundExtended => {
                return 1 + set_rgb_colors_helper(options, index + 1, attributes, true);
            }
            GraphicsOptions::BackgroundExtended => {
                return 1 + set_rgb_colors_helper(options, index + 1, attributes, false);
            }
            GraphicsOptions::UnderlineColor => {}
        }

        1
    }

    /// Ported from `SetCharacterProtectionAttribute` (DECSCA).
    pub(crate) fn apply_logical_attribute(options: &VTParameters, attributes: &mut TextAttribute) {
        for index in 0..options.size() {
            match options.at(index).value_or(0) {
                1 => attributes.set_protected(true),
                // Default and Unprotected both clear it; every other value is
                // ignored, as upstream does.
                0 | 2 => attributes.set_protected(false),
                _ => {}
            }
        }
        let _ = LogicalAttributeOptions::Default;
    }
}

/// Ported from `_SetRgbColorsHelper`: the semicolon-separated `38;2;r;g;b` /
/// `38;5;n` forms. Returns the options consumed, not counting the 38/48 itself.
fn set_rgb_colors_helper(
    options: &VTParameters,
    index: usize,
    attributes: &mut TextAttribute,
    foreground: bool,
) -> usize {
    match GraphicsOptions::from_int(options.at(index).value_or(0)) {
        Some(GraphicsOptions::RGBColorOrFaint) => {
            let red = options.at(index + 1).value_or(0);
            let green = options.at(index + 2).value_or(0);
            let blue = options.at(index + 3).value_or(0);
            if (0..=255).contains(&red) && (0..=255).contains(&green) && (0..=255).contains(&blue) {
                apply_rgb(attributes, foreground, red as u8, green as u8, blue as u8, false);
            }
            4
        }
        Some(GraphicsOptions::BlinkOrXterm256Index) => {
            let table_index = options.at(index + 1).value_or(0);
            if (0..=255).contains(&table_index) {
                apply_indexed(attributes, foreground, table_index as u8, false);
            }
            2
        }
        _ => 1,
    }
}

/// Ported from `_SetRgbColorsHelperFromSubParams`: the `38:2::r:g:b` form.
fn apply_graphics_option_with_sub_params(
    option: VTParameter,
    sub_params: &[VTParameter],
    attributes: &mut TextAttribute,
) {
    let Some(option) = GraphicsOptions::from_int(option.value_or(0)) else {
        return;
    };

    match option {
        GraphicsOptions::Underline => {
            // SGR 4:x — only the five styled underlines exist.
            let style = sub_params.first().map_or(0, |param| param.value_or(0));
            if let Some(style) = underline_style_from_int(style) {
                attributes.set_underline_style(style);
            }
        }
        GraphicsOptions::ForegroundExtended | GraphicsOptions::BackgroundExtended | GraphicsOptions::UnderlineColor => {
            let foreground = option == GraphicsOptions::ForegroundExtended;
            let underline = option == GraphicsOptions::UnderlineColor;

            match GraphicsOptions::from_int(sub_params.first().map_or(0, |param| param.value_or(0))) {
                Some(GraphicsOptions::RGBColorOrFaint) => {
                    // :2:<colour-space-id>:<r>:<g>:<b>; a non-zero colour space
                    // means the values are not RGB, so the colour is dropped.
                    let valid_color_space = sub_params.get(1).map_or(0, |param| param.value_or(0)) == 0;
                    let red = sub_params.get(2).map_or(0, |param| param.value_or(0));
                    let green = sub_params.get(3).map_or(0, |param| param.value_or(0));
                    let blue = sub_params.get(4).map_or(0, |param| param.value_or(0));
                    if valid_color_space
                        && (0..=255).contains(&red)
                        && (0..=255).contains(&green)
                        && (0..=255).contains(&blue)
                    {
                        if underline {
                            attributes.set_underline_color(TextColor::rgb(red as u8, green as u8, blue as u8));
                        } else {
                            apply_rgb(attributes, foreground, red as u8, green as u8, blue as u8, true);
                        }
                    }
                }
                Some(GraphicsOptions::BlinkOrXterm256Index) => {
                    let table_index = sub_params.get(1).map_or(0, |param| param.value_or(0));
                    if (0..=255).contains(&table_index) {
                        let color = TextColor::index256(table_index as u8);
                        if underline {
                            attributes.set_underline_color(color);
                        } else if foreground {
                            attributes.set_foreground(color);
                        } else {
                            attributes.set_background(color);
                        }
                    }
                }
                _ => {}
            }
        }
        _ => {}
    }
}

fn apply_rgb(attributes: &mut TextAttribute, foreground: bool, red: u8, green: u8, blue: u8, _from_sub_params: bool) {
    let color = TextColor::rgb(red, green, blue);
    if foreground {
        attributes.set_foreground(color);
    } else {
        attributes.set_background(color);
    }
}

fn apply_indexed(attributes: &mut TextAttribute, foreground: bool, index: u8, _from_sub_params: bool) {
    if foreground {
        attributes.set_indexed_foreground_256(index);
    } else {
        attributes.set_indexed_background_256(index);
    }
}

fn underline_style_from_int(value: i32) -> Option<UnderlineStyle> {
    Some(match value {
        0 => UnderlineStyle::NoUnderline,
        1 => UnderlineStyle::SinglyUnderlined,
        2 => UnderlineStyle::DoublyUnderlined,
        3 => UnderlineStyle::CurlyUnderlined,
        4 => UnderlineStyle::DottedUnderlined,
        5 => UnderlineStyle::DashedUnderlined,
        _ => return None,
    })
}

fn collect_sub_params(sub_params: VTSubParameters<'_>) -> Vec<VTParameter> {
    sub_params.iter().copied().collect()
}

#[cfg(test)]
mod tests {
    use types::UnderlineStyle;
    use types::dispatch_types::SgrSaveRestoreStackOptions;

    use super::*;

    fn params(values: &[i32]) -> VTParameters {
        VTParameters::from_values(values.to_vec())
    }

    fn apply(values: &[i32]) -> TextAttribute {
        let mut attributes = TextAttribute::new();
        AdaptDispatch::apply_graphics_options(&params(values), &mut attributes);
        attributes
    }

    #[test]
    fn sgr_reset_clears_everything() {
        let mut attributes = TextAttribute::new();
        attributes.set_intense(true);
        attributes.set_foreground(TextColor::rgb(1, 2, 3));

        AdaptDispatch::apply_graphics_options(&params(&[0]), &mut attributes);

        assert_eq!(attributes, TextAttribute::new());
    }

    #[test]
    fn intensity_and_italics_are_independent() {
        let attributes = apply(&[1, 3]);
        assert!(attributes.is_intense());
        assert!(attributes.is_italic());
        assert!(!attributes.is_faint());
    }

    #[test]
    fn two_is_faint_and_twenty_two_clears_both() {
        let faint = apply(&[2]);
        assert!(faint.is_faint());
        assert!(!faint.is_intense());

        let cleared = apply(&[1, 2, 22]);
        assert!(!cleared.is_intense());
        assert!(!cleared.is_faint());
    }

    #[test]
    fn sixteen_colour_palette_uses_the_console_order() {
        let attributes = apply(&[31]);
        assert_eq!(attributes.get_foreground(), TextColor::index16(DARK_RED));

        let bright = apply(&[94]);
        assert_eq!(bright.get_foreground(), TextColor::index16(BRIGHT_BLUE));

        let background = apply(&[46]);
        assert_eq!(background.get_background(), TextColor::index16(DARK_CYAN));
    }

    #[test]
    fn extended_rgb_is_read_as_three_more_options() {
        let attributes = apply(&[38, 2, 10, 20, 30]);
        assert_eq!(attributes.get_foreground(), TextColor::rgb(10, 20, 30));
    }

    #[test]
    fn extended_rgb_out_of_range_is_ignored() {
        let attributes = apply(&[38, 2, 300, 0, 0]);
        assert!(attributes.get_foreground().is_default());
    }

    #[test]
    fn extended_palette_index_sets_a_256_colour() {
        let attributes = apply(&[48, 5, 200]);
        assert_eq!(attributes.get_background(), TextColor::index256(200));
    }

    #[test]
    fn underline_styles_use_the_style_field() {
        assert_eq!(apply(&[4]).get_underline_style(), UnderlineStyle::SinglyUnderlined);
        assert_eq!(apply(&[21]).get_underline_style(), UnderlineStyle::DoublyUnderlined);
        assert_eq!(apply(&[4, 24]).get_underline_style(), UnderlineStyle::NoUnderline);
    }

    #[test]
    fn inverse_and_crossed_out_toggle() {
        let attributes = apply(&[7, 9]);
        assert!(attributes.is_reverse_video());
        assert!(attributes.is_crossed_out());

        let cleared = apply(&[7, 9, 27, 29]);
        assert!(!cleared.is_reverse_video());
        assert!(!cleared.is_crossed_out());
    }

    #[test]
    fn decsca_marks_the_attribute_protected() {
        let mut attributes = TextAttribute::new();
        AdaptDispatch::apply_logical_attribute(&params(&[1]), &mut attributes);
        assert!(attributes.is_protected());

        AdaptDispatch::apply_logical_attribute(&params(&[0]), &mut attributes);
        assert!(!attributes.is_protected());
    }

    // The save-stack option values are xterm parameter numbers, not ordinals.
    #[test]
    fn save_stack_option_values_match_xterm() {
        assert_eq!(SgrSaveRestoreStackOptions::Negative as i32, 7);
        assert_eq!(SgrSaveRestoreStackOptions::DoublyUnderlined as i32, 21);
        assert_eq!(SgrSaveRestoreStackOptions::SaveForegroundColor as i32, 30);
        assert_eq!(SgrSaveRestoreStackOptions::SaveBackgroundColor as i32, 31);
    }
}
