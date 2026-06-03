//! `RowAttributes` — per-row metadata for terminal buffer rows.

/// Metadata associated with a single row in the terminal buffer.
#[derive(Debug, Clone, Copy, Default, PartialEq, Eq)]
pub struct RowAttributes {
    /// Whether this row has been modified since last render.
    dirty: bool,
    /// Whether this row contains any wide (double-width) characters.
    has_wide: bool,
}

impl RowAttributes {
    /// Creates new row attributes.
    pub const fn new() -> Self {
        Self {
            dirty: false,
            has_wide: false,
        }
    }

    /// Marks the row as dirty (modified since last render).
    pub fn mark_dirty(&mut self) {
        self.dirty = true;
    }

    /// Returns whether this row is dirty.
    pub const fn is_dirty(&self) -> bool {
        self.dirty
    }

    /// Clears the dirty flag.
    pub fn clear_dirty(&mut self) {
        self.dirty = false;
    }

    /// Marks that this row contains wide characters.
    pub fn mark_wide(&mut self) {
        self.has_wide = true;
    }

    /// Returns whether this row contains wide characters.
    pub const fn has_wide(&self) -> bool {
        self.has_wide
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn test_default() {
        let attrs = RowAttributes::new();
        assert!(!attrs.is_dirty());
        assert!(!attrs.has_wide());
    }

    #[test]
    fn test_dirty_flag() {
        let mut attrs = RowAttributes::new();
        assert!(!attrs.is_dirty());
        attrs.mark_dirty();
        assert!(attrs.is_dirty());
        attrs.clear_dirty();
        assert!(!attrs.is_dirty());
    }

    #[test]
    fn test_wide_flag() {
        let mut attrs = RowAttributes::new();
        assert!(!attrs.has_wide());
        attrs.mark_wide();
        assert!(attrs.has_wide());
    }
}
