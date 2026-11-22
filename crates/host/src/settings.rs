//! `Settings` — the console's configurable state.
//!
//! Ported from `settings.cpp` and `settings.hpp`. Upstream's object also owns
//! `RenderSettings`, the legacy color table, `STARTUPINFO`'s flags, the
//! `CONSOLE_STATE_INFO` round-trip, the registry serialization and the
//! renderer's text-measurement and glyph switches. Those belong to the
//! renderer, the window and the registry layers here (ADR 001); this module is
//! the value record those layers read and write.
//!
//! The fill attributes stay the legacy `WORD` upstream stores, so
//! `TextAttribute::from_legacy_word` is the one conversion a caller needs.

use types::CursorType;
use utils::{Point, Size};

/// The minimum window opacity; below it the console would be invisible.
///
/// Upstream `MIN_WINDOW_OPACITY`: 0x4D is roughly 30% opaque.
pub const MIN_WINDOW_OPACITY: u8 = 0x4D;

/// The default number of commands kept per application.
pub const DEFAULT_NUMBER_OF_COMMANDS: u32 = 25;

/// The default number of history buffers.
pub const DEFAULT_NUMBER_OF_BUFFERS: u32 = 4;

/// The face name that asks the renderer for its default font.
pub const DEFAULT_TT_FONT_FACENAME: &str = "__DefaultTTFont__";

/// The legacy attribute bits that carry foreground and background colour.
const COLOUR_ATTRIBUTES: u16 = 0x00ff;

/// The console settings.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct Settings {
    fill_attribute: u16,
    popup_fill_attribute: u16,
    screen_buffer_size: Size,
    window_size: Size,
    window_origin: Point,
    font_size: Size,
    font_family: u32,
    font_weight: u32,
    face_name: String,
    launch_face_name: String,
    cursor_size: u32,
    quick_edit: bool,
    insert_mode: bool,
    auto_position: bool,
    history_buffer_size: u32,
    number_of_history_buffers: u32,
    history_no_dup: bool,
    code_page: u32,
    scroll_scale: u32,
    trim_leading_zeros: bool,
    enable_color_selection: bool,
    line_selection: bool,
    wrap_text: bool,
    ctrl_key_shortcuts_disabled: bool,
    window_alpha: u8,
    filter_on_paste: bool,
    allow_alt_f4_close: bool,
    virt_term_level: u32,
    cursor_type: CursorType,
    intercept_copy_paste: bool,
    terminal_scrolling: bool,
}

impl Settings {
    /// Creates the OneCore defaults upstream's constructor uses.
    ///
    /// The code page is 0, meaning "the platform default"; upstream reads the
    /// OEM code page from the host, which a portable console does not have.
    pub fn new() -> Self {
        Self {
            // White (not bright) on black, as upstream's field initializers.
            fill_attribute: 0x0007,
            // Purple on bright white.
            popup_fill_attribute: 0x00f5,
            screen_buffer_size: Size::new(80, 25),
            window_size: Size::new(80, 25),
            window_origin: Point { x: 0, y: 0 },
            font_size: Size::new(0, 16),
            font_family: 0,
            font_weight: 0,
            face_name: DEFAULT_TT_FONT_FACENAME.to_string(),
            launch_face_name: String::new(),
            cursor_size: 25,
            quick_edit: true,
            insert_mode: true,
            auto_position: true,
            history_buffer_size: DEFAULT_NUMBER_OF_COMMANDS,
            number_of_history_buffers: DEFAULT_NUMBER_OF_BUFFERS,
            history_no_dup: false,
            code_page: 0,
            scroll_scale: 1,
            trim_leading_zeros: false,
            enable_color_selection: false,
            line_selection: true,
            wrap_text: true,
            ctrl_key_shortcuts_disabled: false,
            window_alpha: u8::MAX,
            filter_on_paste: false,
            allow_alt_f4_close: true,
            virt_term_level: 0,
            cursor_type: CursorType::Legacy,
            intercept_copy_paste: false,
            terminal_scrolling: true,
        }
    }

    /// Applies the desktop defaults from `win32k-settings.man`.
    ///
    /// Upstream uses these when the registry is unavailable, so a desktop host
    /// starts with a 120-column buffer rather than the OneCore 80.
    pub fn apply_desktop_specific_defaults(&mut self) {
        self.screen_buffer_size = Size::new(120, 9001);
        self.window_size = Size::new(120, 30);
        self.font_size = Size::new(0, 16);
        self.font_family = 0;
        self.font_weight = 0;
        self.cursor_size = 25;
        self.fill_attribute = 0x0007;
        self.popup_fill_attribute = 0x00f5;
        self.face_name = DEFAULT_TT_FONT_FACENAME.to_string();
        self.insert_mode = true;
        self.ctrl_key_shortcuts_disabled = false;
        self.wrap_text = true;
        self.line_selection = true;
        self.window_alpha = u8::MAX;
        self.filter_on_paste = true;
        self.quick_edit = true;
        self.history_buffer_size = 50;
        self.number_of_history_buffers = 4;
        self.history_no_dup = false;
        self.trim_leading_zeros = false;
        self.enable_color_selection = false;
        self.scroll_scale = 1;
    }

    /// Applies the width and height a command line asked for.
    ///
    /// Non-positive values mean "not given". A PTY that was not told a size
    /// uses its window size as the buffer size, as upstream does.
    pub fn apply_command_line_size(&mut self, width: i32, height: i32, in_vt_io_mode: bool) {
        if width > 0 && height > 0 {
            self.screen_buffer_size = Size::new(width, height);
            self.window_size = Size::new(width, height);
        } else if in_vt_io_mode {
            self.screen_buffer_size = self.window_size;
        }
    }

    /// Clamps the settings into the ranges the console can run with.
    ///
    /// Upstream also finishes the default colour and render settings here;
    /// those live with the renderer in this port (ADR 001).
    pub fn validate(&mut self) {
        self.screen_buffer_size = Size::new(
            self.screen_buffer_size.width().max(1),
            self.screen_buffer_size.height().max(1),
        );
        self.window_size = Size::new(self.window_size.width().max(1), self.window_size.height().max(1));

        // The buffer must be at least as large as the window.
        self.screen_buffer_size = Size::new(
            self.screen_buffer_size.width().max(self.window_size.width()),
            self.screen_buffer_size.height().max(self.window_size.height()),
        );

        if self.window_alpha < MIN_WINDOW_OPACITY {
            self.window_alpha = u8::MAX;
        }

        // Wrapped text has exactly the buffer's width.
        if self.wrap_text {
            self.window_size = Size::new(self.screen_buffer_size.width(), self.window_size.height());
        }

        // Fill attributes carry colour only; inverting or box-drawing bits here
        // would change every cell the console writes.
        self.fill_attribute &= COLOUR_ATTRIBUTES;
        self.popup_fill_attribute &= COLOUR_ATTRIBUTES;
    }

    /// Returns the attribute new cells are filled with.
    pub const fn fill_attribute(&self) -> u16 {
        self.fill_attribute
    }

    /// Sets the fill attribute, keeping only the colour bits.
    pub fn set_fill_attribute(&mut self, fill_attribute: u16) {
        self.fill_attribute = fill_attribute & COLOUR_ATTRIBUTES;
    }

    /// Returns the attribute popups are filled with.
    pub const fn popup_fill_attribute(&self) -> u16 {
        self.popup_fill_attribute
    }

    /// Sets the popup fill attribute, keeping only the colour bits.
    pub fn set_popup_fill_attribute(&mut self, popup_fill_attribute: u16) {
        self.popup_fill_attribute = popup_fill_attribute & COLOUR_ATTRIBUTES;
    }

    /// Returns the screen buffer's size in characters.
    pub const fn screen_buffer_size(&self) -> Size {
        self.screen_buffer_size
    }

    /// Sets the screen buffer's size.
    pub const fn set_screen_buffer_size(&mut self, size: Size) {
        self.screen_buffer_size = size;
    }

    /// Returns the window's size in characters.
    pub const fn window_size(&self) -> Size {
        self.window_size
    }

    /// Sets the window's size in characters.
    pub const fn set_window_size(&mut self, size: Size) {
        self.window_size = size;
    }

    /// Returns the window's position, in characters.
    pub const fn window_origin(&self) -> Point {
        self.window_origin
    }

    /// Sets the window's position.
    pub const fn set_window_origin(&mut self, origin: Point) {
        self.window_origin = origin;
    }

    /// Returns the requested cell size in pixels.
    pub const fn font_size(&self) -> Size {
        self.font_size
    }

    /// Sets the requested cell size.
    pub const fn set_font_size(&mut self, size: Size) {
        self.font_size = size;
    }

    /// Returns the requested font family.
    pub const fn font_family(&self) -> u32 {
        self.font_family
    }

    /// Sets the requested font family.
    pub const fn set_font_family(&mut self, family: u32) {
        self.font_family = family;
    }

    /// Returns the requested font weight.
    pub const fn font_weight(&self) -> u32 {
        self.font_weight
    }

    /// Sets the requested font weight.
    pub const fn set_font_weight(&mut self, weight: u32) {
        self.font_weight = weight;
    }

    /// Returns the requested face name.
    pub fn face_name(&self) -> &str {
        &self.face_name
    }

    /// Returns whether a face name was chosen.
    pub fn is_face_name_set(&self) -> bool {
        !self.face_name.is_empty()
    }

    /// Sets the face name.
    ///
    /// Upstream copies into a fixed 32-unit buffer; a portable console keeps
    /// the whole name and lets the renderer reject one it cannot load.
    pub fn set_face_name(&mut self, face_name: &str) {
        self.face_name.clear();
        self.face_name.push_str(face_name);
    }

    /// Returns the face name a shortcut asked for, if any.
    pub fn launch_face_name(&self) -> &str {
        &self.launch_face_name
    }

    /// Sets the face name a shortcut asked for.
    pub fn set_launch_face_name(&mut self, face_name: &str) {
        self.launch_face_name.clear();
        self.launch_face_name.push_str(face_name);
    }

    /// Returns the cursor's size as a percentage of the cell.
    pub const fn cursor_size(&self) -> u32 {
        self.cursor_size
    }

    /// Sets the cursor's size.
    pub const fn set_cursor_size(&mut self, size: u32) {
        self.cursor_size = size;
    }

    /// Returns the cursor's shape.
    pub const fn cursor_type(&self) -> CursorType {
        self.cursor_type
    }

    /// Sets the cursor's shape.
    pub const fn set_cursor_type(&mut self, cursor_type: CursorType) {
        self.cursor_type = cursor_type;
    }

    /// Returns whether quick-edit is enabled.
    pub const fn quick_edit(&self) -> bool {
        self.quick_edit
    }

    /// Sets quick-edit.
    pub const fn set_quick_edit(&mut self, quick_edit: bool) {
        self.quick_edit = quick_edit;
    }

    /// Returns whether a cooked read starts in insert mode.
    pub const fn insert_mode(&self) -> bool {
        self.insert_mode
    }

    /// Sets the starting insert mode.
    pub const fn set_insert_mode(&mut self, insert_mode: bool) {
        self.insert_mode = insert_mode;
    }

    /// Returns whether the window positions itself.
    pub const fn auto_position(&self) -> bool {
        self.auto_position
    }

    /// Sets whether the window positions itself.
    pub const fn set_auto_position(&mut self, auto_position: bool) {
        self.auto_position = auto_position;
    }

    /// Returns the number of commands kept per application.
    pub const fn history_buffer_size(&self) -> u32 {
        self.history_buffer_size
    }

    /// Sets the number of commands kept per application.
    pub const fn set_history_buffer_size(&mut self, size: u32) {
        self.history_buffer_size = size;
    }

    /// Returns the number of applications whose history is kept.
    pub const fn number_of_history_buffers(&self) -> u32 {
        self.number_of_history_buffers
    }

    /// Sets the number of history buffers.
    pub const fn set_number_of_history_buffers(&mut self, buffers: u32) {
        self.number_of_history_buffers = buffers;
    }

    /// Returns whether duplicate commands are kept in the history.
    pub const fn history_no_dup(&self) -> bool {
        self.history_no_dup
    }

    /// Sets duplicate suppression for the history.
    pub const fn set_history_no_dup(&mut self, history_no_dup: bool) {
        self.history_no_dup = history_no_dup;
    }

    /// Returns the input code page; 0 means the platform default.
    pub const fn code_page(&self) -> u32 {
        self.code_page
    }

    /// Sets the input code page.
    pub const fn set_code_page(&mut self, code_page: u32) {
        self.code_page = code_page;
    }

    /// Returns how many rows one wheel notch scrolls.
    pub const fn scroll_scale(&self) -> u32 {
        self.scroll_scale
    }

    /// Sets the wheel's scroll scale.
    pub const fn set_scroll_scale(&mut self, scroll_scale: u32) {
        self.scroll_scale = scroll_scale;
    }

    /// Returns whether leading zeros are trimmed from selections.
    pub const fn trim_leading_zeros(&self) -> bool {
        self.trim_leading_zeros
    }

    /// Sets leading-zero trimming.
    pub const fn set_trim_leading_zeros(&mut self, trim: bool) {
        self.trim_leading_zeros = trim;
    }

    /// Returns whether colour selection is enabled.
    pub const fn enable_color_selection(&self) -> bool {
        self.enable_color_selection
    }

    /// Sets colour selection.
    pub const fn set_enable_color_selection(&mut self, enabled: bool) {
        self.enable_color_selection = enabled;
    }

    /// Returns whether a selection extends by line.
    pub const fn line_selection(&self) -> bool {
        self.line_selection
    }

    /// Sets line selection.
    pub const fn set_line_selection(&mut self, line_selection: bool) {
        self.line_selection = line_selection;
    }

    /// Returns whether text wraps when the window is resized.
    pub const fn wrap_text(&self) -> bool {
        self.wrap_text
    }

    /// Sets text wrapping.
    pub const fn set_wrap_text(&mut self, wrap_text: bool) {
        self.wrap_text = wrap_text;
    }

    /// Returns whether Ctrl-key shortcuts are disabled.
    pub const fn ctrl_key_shortcuts_disabled(&self) -> bool {
        self.ctrl_key_shortcuts_disabled
    }

    /// Sets Ctrl-key shortcut disabling.
    pub const fn set_ctrl_key_shortcuts_disabled(&mut self, disabled: bool) {
        self.ctrl_key_shortcuts_disabled = disabled;
    }

    /// Returns the window's opacity; 255 is opaque.
    pub const fn window_alpha(&self) -> u8 {
        self.window_alpha
    }

    /// Sets the window's opacity, treating anything below the minimum as fully
    /// opaque so the window never disappears.
    pub fn set_window_alpha(&mut self, alpha: u8) {
        self.window_alpha = if alpha < MIN_WINDOW_OPACITY { u8::MAX } else { alpha };
    }

    /// Returns whether pasted text is filtered.
    pub const fn filter_on_paste(&self) -> bool {
        self.filter_on_paste
    }

    /// Sets paste filtering.
    pub const fn set_filter_on_paste(&mut self, filter: bool) {
        self.filter_on_paste = filter;
    }

    /// Returns whether Alt+F4 may close the console.
    pub const fn allow_alt_f4_close(&self) -> bool {
        self.allow_alt_f4_close
    }

    /// Sets whether Alt+F4 may close the console.
    pub const fn set_allow_alt_f4_close(&mut self, allow: bool) {
        self.allow_alt_f4_close = allow;
    }

    /// Returns the registry's requested VT level.
    pub const fn virt_term_level(&self) -> u32 {
        self.virt_term_level
    }

    /// Sets the requested VT level.
    pub const fn set_virt_term_level(&mut self, level: u32) {
        self.virt_term_level = level;
    }

    /// Returns whether Ctrl+V is intercepted as paste.
    pub const fn intercept_copy_paste(&self) -> bool {
        self.intercept_copy_paste
    }

    /// Sets Ctrl+V interception.
    pub const fn set_intercept_copy_paste(&mut self, intercept: bool) {
        self.intercept_copy_paste = intercept;
    }

    /// Returns whether the console addresses the buffer as a terminal does.
    pub const fn terminal_scrolling(&self) -> bool {
        self.terminal_scrolling
    }

    /// Sets terminal-scrolling mode.
    pub const fn set_terminal_scrolling(&mut self, enabled: bool) {
        self.terminal_scrolling = enabled;
    }

    /// Applies what a console key holds over the current settings.
    ///
    /// A value the key does not hold leaves the current one, which is upstream's
    /// rule: the registry is a layer over the constructor's defaults. A key that
    /// names a window position also turns automatic placement off, whatever the
    /// coordinate says (`Registry::LoadFromRegistry`).
    pub fn apply_registry(&mut self, from: &crate::registry::ConsoleSettings) {
        if let Some(value) = from.popup_fill_attribute {
            self.popup_fill_attribute = value;
        }
        if let Some(value) = from.insert_mode {
            self.insert_mode = value;
        }
        if let Some(value) = from.line_selection {
            self.line_selection = value;
        }
        if let Some(value) = from.filter_on_paste {
            self.filter_on_paste = value;
        }
        if let Some(value) = from.wrap_text {
            self.wrap_text = value;
        }
        if let Some(value) = from.ctrl_key_shortcuts_disabled {
            self.ctrl_key_shortcuts_disabled = value;
        }
        if let Some(value) = from.quick_edit {
            self.quick_edit = value;
        }
        if let Some(value) = from.window_alpha {
            // Upstream writes the byte as the key gives it; the clamping setter
            // is for the API, not for the registry.
            self.window_alpha = value;
        }
        if let Some(value) = from.font_size {
            self.font_size = value;
        }
        if let Some(value) = from.font_family {
            self.font_family = value;
        }
        if let Some(value) = from.font_weight {
            self.font_weight = value;
        }
        if let Some(value) = &from.face_name {
            self.set_face_name(value);
        }
        if let Some(value) = from.cursor_size {
            self.cursor_size = value;
        }
        if let Some(value) = from.history_buffer_size {
            self.history_buffer_size = value;
        }
        if let Some(value) = from.number_of_history_buffers {
            self.number_of_history_buffers = value;
        }
        if let Some(value) = from.history_no_dup {
            self.history_no_dup = value;
        }
        if let Some(value) = from.scroll_scale {
            self.scroll_scale = value;
        }
        if let Some(value) = from.fill_attribute {
            self.fill_attribute = value;
        }
        if let Some(value) = from.screen_buffer_size {
            self.screen_buffer_size = value;
        }
        if let Some(value) = from.window_size {
            self.window_size = value;
        }
        if let Some(value) = from.trim_leading_zeros {
            self.trim_leading_zeros = value;
        }
        if let Some(value) = from.enable_color_selection {
            self.enable_color_selection = value;
        }
        if let Some(value) = from.window_origin {
            self.window_origin = value;
            self.auto_position = false;
        }
        if let Some(value) = from.cursor_type {
            self.cursor_type = value;
        }
        if let Some(value) = from.intercept_copy_paste {
            self.intercept_copy_paste = value;
        }
        if let Some(value) = from.terminal_scrolling {
            self.terminal_scrolling = value;
        }
        if let Some(value) = from.virt_term_level {
            self.virt_term_level = value;
        }
        if let Some(value) = from.code_page {
            self.code_page = value;
        }
        if let Some(value) = from.allow_alt_f4_close {
            self.allow_alt_f4_close = value;
        }
    }
}

impl Default for Settings {
    fn default() -> Self {
        Self::new()
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn the_constructor_uses_the_onecore_defaults() {
        let settings = Settings::new();
        assert_eq!(settings.screen_buffer_size(), Size::new(80, 25));
        assert_eq!(settings.window_size(), Size::new(80, 25));
        assert_eq!(settings.font_size(), Size::new(0, 16));
        assert_eq!(settings.face_name(), DEFAULT_TT_FONT_FACENAME);
        assert!(settings.insert_mode());
        assert!(settings.quick_edit());
        assert!(settings.auto_position());
        assert!(settings.line_selection());
        assert!(settings.wrap_text());
        assert!(!settings.history_no_dup());
        assert!(settings.terminal_scrolling());
        assert_eq!(settings.history_buffer_size(), DEFAULT_NUMBER_OF_COMMANDS);
        assert_eq!(settings.number_of_history_buffers(), DEFAULT_NUMBER_OF_BUFFERS);
        assert_eq!(settings.fill_attribute(), 0x0007);
        assert_eq!(settings.popup_fill_attribute(), 0x00f5);
        assert_eq!(settings.window_alpha(), u8::MAX);
        assert_eq!(settings.cursor_type(), CursorType::Legacy);
    }

    #[test]
    fn the_desktop_defaults_use_the_manifest_values() {
        let mut settings = Settings::new();
        settings.apply_desktop_specific_defaults();

        assert_eq!(settings.screen_buffer_size(), Size::new(120, 9001));
        assert_eq!(settings.window_size(), Size::new(120, 30));
        assert_eq!(settings.history_buffer_size(), 50);
        assert!(settings.filter_on_paste());
        assert_eq!(settings.cursor_size(), 25);
        assert_eq!(settings.face_name(), DEFAULT_TT_FONT_FACENAME);
    }

    #[test]
    fn validation_raises_the_sizes_to_at_least_one() {
        let mut settings = Settings::new();
        settings.set_screen_buffer_size(Size::new(0, 0));
        settings.set_window_size(Size::new(0, 0));
        settings.validate();

        assert_eq!(settings.screen_buffer_size(), Size::new(1, 1));
        assert_eq!(settings.window_size(), Size::new(1, 1));
    }

    #[test]
    fn validation_makes_the_buffer_at_least_the_window() {
        let mut settings = Settings::new();
        settings.set_screen_buffer_size(Size::new(40, 10));
        settings.set_window_size(Size::new(80, 25));
        settings.validate();

        assert_eq!(settings.screen_buffer_size(), Size::new(80, 25));
    }

    #[test]
    fn wrapping_makes_the_window_as_wide_as_the_buffer() {
        let mut settings = Settings::new();
        settings.set_screen_buffer_size(Size::new(100, 40));
        settings.set_window_size(Size::new(80, 25));
        settings.validate();

        assert_eq!(settings.window_size(), Size::new(100, 25));
    }

    #[test]
    fn an_alpha_below_the_minimum_becomes_opaque() {
        let mut settings = Settings::new();
        settings.set_window_alpha(MIN_WINDOW_OPACITY - 1);
        assert_eq!(settings.window_alpha(), u8::MAX);

        settings.set_window_alpha(0x80);
        assert_eq!(settings.window_alpha(), 0x80);
    }

    #[test]
    fn fill_attributes_keep_only_the_colour_bits() {
        let mut settings = Settings::new();
        settings.set_fill_attribute(0xffff);
        assert_eq!(settings.fill_attribute(), 0x00ff);

        settings.set_popup_fill_attribute(0xff5a);
        assert_eq!(settings.popup_fill_attribute(), 0x005a);
    }

    #[test]
    fn validation_clears_non_colour_fill_bits() {
        let mut settings = Settings::new();
        settings.fill_attribute = 0xffff;
        settings.popup_fill_attribute = 0xffff;
        settings.validate();

        assert_eq!(settings.fill_attribute(), 0x00ff);
        assert_eq!(settings.popup_fill_attribute(), 0x00ff);
    }

    #[test]
    fn a_command_line_size_overrides_both_sizes() {
        let mut settings = Settings::new();
        settings.apply_command_line_size(100, 40, false);
        assert_eq!(settings.screen_buffer_size(), Size::new(100, 40));
        assert_eq!(settings.window_size(), Size::new(100, 40));
    }

    #[test]
    fn a_ptys_unspecified_size_follows_the_window() {
        let mut settings = Settings::new();
        settings.set_window_size(Size::new(100, 30));
        settings.apply_command_line_size(0, 0, true);
        assert_eq!(settings.screen_buffer_size(), Size::new(100, 30));
    }

    #[test]
    fn an_unspecified_size_outside_a_pty_changes_nothing() {
        let mut settings = Settings::new();
        settings.apply_command_line_size(0, 0, false);
        assert_eq!(settings.screen_buffer_size(), Size::new(80, 25));
    }

    #[test]
    fn the_settings_round_trip_through_their_accessors() {
        let mut settings = Settings::new();
        settings.set_insert_mode(false);
        settings.set_history_no_dup(true);
        settings.set_history_buffer_size(100);
        settings.set_number_of_history_buffers(8);
        settings.set_scroll_scale(3);
        settings.set_code_page(65001);
        settings.set_virt_term_level(1);
        settings.set_cursor_type(CursorType::VerticalBar);
        settings.set_cursor_size(50);
        settings.set_terminal_scrolling(false);
        settings.set_intercept_copy_paste(true);

        assert!(!settings.insert_mode());
        assert!(settings.history_no_dup());
        assert_eq!(settings.history_buffer_size(), 100);
        assert_eq!(settings.number_of_history_buffers(), 8);
        assert_eq!(settings.scroll_scale(), 3);
        assert_eq!(settings.code_page(), 65001);
        assert_eq!(settings.virt_term_level(), 1);
        assert_eq!(settings.cursor_type(), CursorType::VerticalBar);
        assert_eq!(settings.cursor_size(), 50);
        assert!(!settings.terminal_scrolling());
        assert!(settings.intercept_copy_paste());
    }

    #[test]
    fn a_key_layers_over_the_defaults_and_leaves_the_rest_alone() {
        let defaults = Settings::new();
        let mut settings = Settings::new();
        settings.apply_registry(&crate::registry::load(
            &crate::registry::Map::new()
                .dword("InsertMode", 1)
                .dword("CodePage", 65001),
        ));

        assert!(settings.insert_mode(), "the key's value is applied");
        assert_eq!(settings.code_page(), 65001);
        assert_eq!(
            settings.history_buffer_size(),
            defaults.history_buffer_size(),
            "a value the key does not hold keeps the default"
        );
    }

    #[test]
    fn a_window_position_turns_automatic_placement_off() {
        let mut settings = Settings::new();
        assert!(settings.auto_position());

        settings.apply_registry(&crate::registry::load(
            &crate::registry::Map::new().dword("WindowPosition", (10 << 16) | 20),
        ));

        assert!(!settings.auto_position(), "a position means the user placed it");
        assert_eq!(settings.window_origin().x, 20);
        assert_eq!(settings.window_origin().y, 10);
    }
}
