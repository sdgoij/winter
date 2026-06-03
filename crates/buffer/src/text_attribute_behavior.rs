//! `TextAttributeBehavior` — enumeration of text attribute behaviors.

/// Describes how a text attribute should be applied or combined.
#[derive(Debug, Clone, Copy, PartialEq, Eq, Default)]
pub enum TextAttributeBehavior {
    /// Replace the existing attribute completely.
    #[default]
    Replace,
    /// Add to the existing attribute (bitwise OR).
    Additive,
    /// Remove from the existing attribute (bitwise AND NOT).
    Subtractive,
}

impl TextAttributeBehavior {
    /// Returns whether this behavior adds to existing attributes.
    pub const fn is_additive(&self) -> bool {
        matches!(self, Self::Additive)
    }

    /// Returns whether this behavior replaces existing attributes.
    pub const fn is_replace(&self) -> bool {
        matches!(self, Self::Replace)
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn test_default_is_replace() {
        assert!(TextAttributeBehavior::default().is_replace());
    }

    #[test]
    fn test_additive() {
        let behavior = TextAttributeBehavior::Additive;
        assert!(behavior.is_additive());
        assert!(!behavior.is_replace());
    }

    #[test]
    fn test_subtractive() {
        let behavior = TextAttributeBehavior::Subtractive;
        assert!(!behavior.is_additive());
        assert!(!behavior.is_replace());
    }
}
