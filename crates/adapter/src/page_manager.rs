//! `PageManager` — Alternate/main screen buffer switching.
//!
//! Ported from `PageManager.hpp/cpp` in the Windows Terminal C++ codebase.
//! Manages the text buffers required by VT paging operations.

use utils::inclusive_rect::InclusiveRect;

/// Maximum number of pages.
const MAX_PAGES: usize = 6;

/// A page (screen buffer) in the terminal.
pub struct Page {
    /// The text buffer for this page.
    buffer: std::sync::Arc<std::sync::RwLock<TextBuffer>>,
    /// The viewport rectangle.
    viewport: InclusiveRect,
    /// Page number (1-based).
    number: i32,
}

impl Page {
    /// Creates a new page.
    pub fn new(
        buffer: std::sync::Arc<std::sync::RwLock<TextBuffer>>,
        viewport: InclusiveRect,
        number: i32,
    ) -> Self {
        Self {
            buffer,
            viewport,
            number,
        }
    }

    /// Returns the text buffer.
    pub fn buffer(&self) -> std::sync::Arc<std::sync::RwLock<TextBuffer>> {
        self.buffer.clone()
    }

    /// Returns the viewport.
    pub fn viewport(&self) -> InclusiveRect {
        self.viewport
    }

    /// Returns the page number.
    pub fn number(&self) -> i32 {
        self.number
    }

    /// Returns the width in characters.
    pub fn width(&self) -> i32 {
        self.viewport.width()
    }

    /// Returns the height in characters.
    pub fn height(&self) -> i32 {
        self.viewport.height()
    }
}

/// Text buffer for a page.
#[derive(Debug, Clone)]
pub struct TextBuffer {
    /// Width in characters.
    width: i32,
    /// Height in characters.
    height: i32,
    /// Whether this is the alternate screen buffer.
    is_alternate: bool,
}

impl TextBuffer {
    /// Creates a new text buffer.
    pub fn new(width: i32, height: i32, is_alternate: bool) -> Self {
        Self {
            width,
            height,
            is_alternate,
        }
    }

    /// Returns the width.
    pub fn width(&self) -> i32 {
        self.width
    }

    /// Returns the height.
    pub fn height(&self) -> i32 {
        self.height
    }

    /// Returns whether this is the alternate screen buffer.
    pub fn is_alternate(&self) -> bool {
        self.is_alternate
    }
}

/// Manages page switching between main and alternate buffers.
pub struct PageManager {
    /// Active page number (1-based).
    active_page_number: i32,
    /// Visible page number (1-based).
    visible_page_number: i32,
    /// Page buffers.
    pages: [Option<Page>; MAX_PAGES],
}

impl PageManager {
    /// Creates a new PageManager.
    pub fn new() -> Self {
        Self {
            active_page_number: 1,
            visible_page_number: 1,
            pages: Default::default(),
        }
    }

    /// Resets the page manager.
    pub fn reset(&mut self) {
        self.active_page_number = 1;
        self.visible_page_number = 1;
        self.pages = Default::default();
    }

    /// Gets the page with the given number.
    pub fn get(&mut self, page_number: i32) -> &mut Page {
        let idx = (page_number - 1) as usize;
        if idx >= MAX_PAGES {
            panic!("Page number {} out of range (1-{})", page_number, MAX_PAGES);
        }

        if self.pages[idx].is_none() {
            // Create default viewport for new page
            let viewport = InclusiveRect::new(0, 0, 79, 23); // 80x24 default
            let buffer = TextBuffer::new(80, 24, false);
            self.pages[idx] = Some(Page::new(
                std::sync::Arc::new(std::sync::RwLock::new(buffer)),
                viewport,
                page_number,
            ));
        }

        self.pages[idx].as_mut().expect("Page should exist")
    }

    /// Gets the active page.
    pub fn active_page(&mut self) -> &mut Page {
        self.get(self.active_page_number)
    }

    /// Gets the visible page.
    pub fn visible_page(&mut self) -> &mut Page {
        self.get(self.visible_page_number)
    }

    /// Moves to the specified page.
    pub fn move_to(&mut self, page_number: i32, make_visible: bool) {
        if page_number < 1 || page_number > MAX_PAGES as i32 {
            return;
        }

        self.active_page_number = page_number;

        if make_visible {
            self.visible_page_number = page_number;
        }
    }

    /// Moves relative to the current active page.
    pub fn move_relative(&mut self, page_count: i32, make_visible: bool) {
        let new_page = self.active_page_number + page_count;
        if new_page >= 1 && new_page <= MAX_PAGES as i32 {
            self.move_to(new_page, make_visible);
        }
    }

    /// Makes the active page visible.
    pub fn make_active_page_visible(&mut self) {
        self.visible_page_number = self.active_page_number;
    }

    /// Returns whether we're using the alternate screen buffer (page 2).
    pub fn is_alternate_active(&self) -> bool {
        self.active_page_number == 2
    }

    /// Activates the alternate screen buffer.
    pub fn activate_alternate(&mut self) {
        self.move_to(2, true);
    }

    /// Deactivates the alternate screen buffer (returns to main).
    pub fn deactivate_alternate(&mut self) {
        self.move_to(1, true);
    }
}

impl Default for PageManager {
    fn default() -> Self {
        Self::new()
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn test_page_manager_new() {
        let pm = PageManager::new();
        assert_eq!(pm.active_page_number, 1);
        assert_eq!(pm.visible_page_number, 1);
        assert!(!pm.is_alternate_active());
    }

    #[test]
    fn test_page_manager_reset() {
        let mut pm = PageManager::new();
        pm.move_to(3, true);
        pm.reset();
        assert_eq!(pm.active_page_number, 1);
        assert_eq!(pm.visible_page_number, 1);
    }

    #[test]
    fn test_page_manager_switch_to_alternate() {
        let mut pm = PageManager::new();
        pm.activate_alternate();
        assert!(pm.is_alternate_active());
        assert_eq!(pm.active_page_number, 2);
        assert_eq!(pm.visible_page_number, 2);
    }

    #[test]
    fn test_page_manager_return_to_main() {
        let mut pm = PageManager::new();
        pm.activate_alternate();
        pm.deactivate_alternate();
        assert_eq!(pm.active_page_number, 1);
        assert_eq!(pm.visible_page_number, 1);
    }

    #[test]
    fn test_page_manager_relative_move() {
        let mut pm = PageManager::new();
        pm.move_to(1, true);
        pm.move_relative(1, false); // Move to page 2 but don't make visible
        assert_eq!(pm.active_page_number, 2);
        assert_eq!(pm.visible_page_number, 1);
    }

    #[test]
    #[should_panic(expected = "out of range")]
    fn test_page_manager_invalid_page() {
        let mut pm = PageManager::new();
        pm.get(7); // MAX_PAGES is 6
    }

    #[test]
    fn test_text_buffer() {
        let buffer = TextBuffer::new(80, 24, false);
        assert_eq!(buffer.width(), 80);
        assert_eq!(buffer.height(), 24);
        assert!(!buffer.is_alternate());

        let alt = TextBuffer::new(80, 24, true);
        assert!(alt.is_alternate());
    }
}
