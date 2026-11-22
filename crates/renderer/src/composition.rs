//! `Composition` — the IME composition preview.
//!
//! Ported from the `Composition` and `CompositionRange` structs in
//! `IRenderData.hpp`. Upstream stores the attributes in a small-vector inline
//! capacity of two; this port uses a `Vec`, which keeps the type simple.

use types::TextAttribute;

/// The attribute applied to a run of composition text.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub struct CompositionRange {
    /// The number of characters in the composition this attribute covers.
    pub len: usize,
    /// The attribute for those characters.
    pub attr: TextAttribute,
}

/// The text an input method is composing, with per-run attributes.
#[derive(Debug, Clone, Default, PartialEq, Eq)]
pub struct Composition {
    /// The composition text.
    pub text: String,
    /// The attribute runs, in order.
    pub attributes: Vec<CompositionRange>,
    /// The caret position in characters.
    pub cursor_pos: usize,
}

#[cfg(test)]
mod tests {
    use super::*;
    use types::TextColor;

    #[test]
    fn default_is_empty() {
        let composition = Composition::default();
        assert!(composition.text.is_empty());
        assert!(composition.attributes.is_empty());
        assert_eq!(composition.cursor_pos, 0);
    }

    #[test]
    fn a_composition_carries_text_runs_and_a_caret() {
        let mut attr = TextAttribute::default();
        attr.set_foreground(TextColor::index16(3));

        let composition = Composition {
            text: "nihao".to_string(),
            attributes: vec![CompositionRange { len: 5, attr }],
            cursor_pos: 5,
        };

        assert_eq!(composition.text, "nihao");
        assert_eq!(composition.attributes.len(), 1);
        assert_eq!(composition.attributes[0].len, 5);
        assert_eq!(composition.attributes[0].attr.get_foreground(), TextColor::index16(3));
        assert_eq!(composition.cursor_pos, 5);
    }
}
