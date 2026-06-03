//! `MarkKind` — cell marking type for terminal buffers.
//!
//! Ported from `MarkKind` in the Windows Terminal C++ buffer module.

/// Represents the kind of semantic mark applied to a terminal cell.
///
/// Ported from `MarkKind` enum in the Windows Terminal C++ `TextAttribute.hpp`.
#[derive(Debug, Clone, Copy, PartialEq, Eq, Hash)]
#[repr(u16)]
pub enum MarkKind {
    /// No mark applied.
    None = 0,
    /// Cell is part of a prompt (user input).
    Prompt = 1,
    /// Cell is part of a command output.
    Command = 2,
    /// Cell is part of program output.
    Output = 3,
}

impl MarkKind {
    /// Returns the name of this mark kind.
    pub const fn name(&self) -> &'static str {
        match self {
            MarkKind::None => "none",
            MarkKind::Prompt => "prompt",
            MarkKind::Command => "command",
            MarkKind::Output => "output",
        }
    }

    /// Creates a MarkKind from a u16 value.
    pub const fn from_u16(value: u16) -> Option<Self> {
        match value {
            0 => Some(MarkKind::None),
            1 => Some(MarkKind::Prompt),
            2 => Some(MarkKind::Command),
            3 => Some(MarkKind::Output),
            _ => None,
        }
    }
}

impl Default for MarkKind {
    fn default() -> Self {
        MarkKind::None
    }
}

impl std::fmt::Display for MarkKind {
    fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        write!(f, "{}", self.name())
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn test_default() {
        assert_eq!(MarkKind::default(), MarkKind::None);
    }

    #[test]
    fn test_from_u16() {
        assert_eq!(MarkKind::from_u16(0), Some(MarkKind::None));
        assert_eq!(MarkKind::from_u16(3), Some(MarkKind::Output));
        assert_eq!(MarkKind::from_u16(10), None);
    }

    #[test]
    fn test_names() {
        assert_eq!(MarkKind::Prompt.name(), "prompt");
        assert_eq!(MarkKind::Command.name(), "command");
    }
}
