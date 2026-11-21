//! `SgrStack` — save and restore text attributes for XTPUSHSGR / XTPOPSGR.
//!
//! Ported from `sgrStack.cpp` in the Windows Terminal C++ `types` module.
//! The stack is ten deep, like xterm's, and each entry records which parts of the
//! attribute a later pop may restore; a push with no options saves everything.

use crate::dispatch_types::SgrSaveRestoreStackOptions;
use crate::text_attribute::TextAttribute;
use crate::underline_style::UnderlineStyle;

/// Xterm allows the save stack to go ten deep.
const MAX_STORED: usize = 10;

#[derive(Debug, Clone, Copy)]
struct Saved {
    attributes: TextAttribute,
    /// Bit `n` is set when option `n` was saved; bit 0 means "everything".
    parts: u32,
}

/// A bounded stack of saved text attributes.
#[derive(Debug)]
pub struct SgrStack {
    entries: [Saved; MAX_STORED],
    next_push: usize,
    count: usize,
}

impl SgrStack {
    /// Creates an empty stack.
    pub const fn new() -> Self {
        Self {
            entries: [Saved {
                attributes: TextAttribute::new(),
                parts: 0,
            }; MAX_STORED],
            next_push: 0,
            count: 0,
        }
    }

    /// Saves `current`. With no options, every attribute is saved; otherwise
    /// only the named parts, and unsupported values are ignored.
    pub fn push(&mut self, current: TextAttribute, options: &[i32]) {
        let mut parts = 0u32;
        if options.is_empty() {
            parts |= 1;
        } else {
            for &option in options {
                if option > 0 && option <= SgrSaveRestoreStackOptions::MAX {
                    parts |= 1 << option;
                }
            }
        }

        if self.count < MAX_STORED {
            self.count += 1;
        }
        self.entries[self.next_push] = Saved {
            attributes: current,
            parts,
        };
        self.next_push = (self.next_push + 1) % MAX_STORED;
    }

    /// Restores the most recent save, combining it with `current` when only
    /// parts of the attribute were saved. Popping an empty stack is a no-op.
    pub fn pop(&mut self, current: TextAttribute) -> TextAttribute {
        if self.count == 0 {
            return current;
        }
        self.count -= 1;
        self.next_push = if self.next_push == 0 {
            MAX_STORED - 1
        } else {
            self.next_push - 1
        };

        let saved = self.entries[self.next_push];
        if saved.parts & 1 != 0 {
            saved.attributes
        } else {
            combine_with_current(current, saved.attributes, saved.parts)
        }
    }
}

impl Default for SgrStack {
    fn default() -> Self {
        Self::new()
    }
}

fn combine_with_current(mut result: TextAttribute, saved: TextAttribute, parts: u32) -> TextAttribute {
    let has = |option: SgrSaveRestoreStackOptions| parts & (1 << option as u32) != 0;

    if has(SgrSaveRestoreStackOptions::Intense) {
        result.set_intense(saved.is_intense());
    }
    if has(SgrSaveRestoreStackOptions::Faintness) {
        result.set_faint(saved.is_faint());
    }
    if has(SgrSaveRestoreStackOptions::Italics) {
        result.set_italic(saved.is_italic());
    }

    // Doubly underlined has its own option (21) even though it shares the
    // underline-style field with option 4, so the two are merged as upstream does.
    let underline_saved = has(SgrSaveRestoreStackOptions::Underline);
    let double_saved = has(SgrSaveRestoreStackOptions::DoublyUnderlined);
    if underline_saved && double_saved {
        result.set_underline_style(saved.get_underline_style());
    } else if underline_saved {
        let saved_style = saved.get_underline_style();
        if saved_style != UnderlineStyle::NoUnderline && saved_style != UnderlineStyle::DoublyUnderlined {
            result.set_underline_style(saved_style);
        } else if result.get_underline_style() != UnderlineStyle::DoublyUnderlined {
            result.set_underline_style(UnderlineStyle::NoUnderline);
        }
    } else if double_saved {
        if saved.get_underline_style() == UnderlineStyle::DoublyUnderlined {
            result.set_underline_style(UnderlineStyle::DoublyUnderlined);
        } else if result.get_underline_style() == UnderlineStyle::DoublyUnderlined {
            result.set_underline_style(UnderlineStyle::NoUnderline);
        }
    }

    if has(SgrSaveRestoreStackOptions::Blink) {
        result.set_blinking(saved.is_blinking());
    }
    if has(SgrSaveRestoreStackOptions::Negative) {
        result.set_reverse_video(saved.is_reverse_video());
    }
    if has(SgrSaveRestoreStackOptions::Invisible) {
        result.set_invisible(saved.is_invisible());
    }
    if has(SgrSaveRestoreStackOptions::CrossedOut) {
        result.set_crossed_out(saved.is_crossed_out());
    }
    if has(SgrSaveRestoreStackOptions::SaveForegroundColor) {
        result.set_foreground(saved.get_foreground());
    }
    if has(SgrSaveRestoreStackOptions::SaveBackgroundColor) {
        result.set_background(saved.get_background());
    }

    result
}

#[cfg(test)]
mod tests {
    use crate::text_color::TextColor;

    use super::*;

    fn intense() -> TextAttribute {
        let mut attr = TextAttribute::new();
        attr.set_intense(true);
        attr
    }

    #[test]
    fn push_then_pop_restores_the_whole_attribute() {
        let mut stack = SgrStack::new();
        let saved = intense();
        stack.push(saved, &[]);

        let mut current = TextAttribute::new();
        current.set_italic(true);
        assert_eq!(stack.pop(current), saved);
    }

    #[test]
    fn partial_push_only_restores_the_named_parts() {
        let mut stack = SgrStack::new();
        let mut saved = TextAttribute::new();
        saved.set_foreground(TextColor::rgb(1, 2, 3));

        // Option 1 saves only the intense bit.
        stack.push(saved, &[1]);

        let mut current = TextAttribute::new();
        current.set_foreground(TextColor::rgb(9, 9, 9));
        let restored = stack.pop(current);

        // The colour was not saved, so the current one survives.
        assert_eq!(restored.get_foreground(), TextColor::rgb(9, 9, 9));
        assert!(!restored.is_intense());
    }

    #[test]
    fn popping_an_empty_stack_returns_the_current_attribute() {
        let mut stack = SgrStack::new();
        let current = intense();
        assert_eq!(stack.pop(current), current);
    }

    #[test]
    fn the_stack_is_ten_deep_and_drops_the_oldest() {
        let mut stack = SgrStack::new();
        for i in 0..12 {
            let mut attr = TextAttribute::new();
            attr.set_indexed_foreground_256(i as u8);
            stack.push(attr, &[]);
        }

        // Ten pops succeed; the last two return whatever was passed in.
        for _ in 0..10 {
            assert_ne!(stack.pop(TextAttribute::new()).get_foreground(), TextColor::new());
        }
        let fallback = intense();
        assert_eq!(stack.pop(fallback), fallback);
    }
}
