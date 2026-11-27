//! The `[extensions]` table: which extensions this installation does not load.

use serde::{Deserialize, Serialize};

/// The `[extensions]` table.
///
/// An identifier is the `id` from the extension's own manifest: lowercase and
/// dotted. A name this installation does not have is harmless — an extension
/// may have been uninstalled — so nothing here is checked against what is on
/// disk; discovery reports an identifier that matched nothing it found.
///
/// Both keys are read when the window opens, so a change takes effect on the
/// next start, not the next settings reload: unloading code that holds
/// registrations is a different operation from re-reading a document.
#[derive(Debug, Clone, Default, PartialEq, Eq, Serialize, Deserialize)]
#[serde(default, deny_unknown_fields)]
pub struct Extensions {
    /// Whether the shell loads extensions at all. Off by default: the feature
    /// is new, and `WINTER_EXT` overrides this for one run either way.
    pub enabled: bool,
    /// Identifiers the shell skips.
    pub disabled: Vec<String>,
}

impl Extensions {
    /// Whether `id` is switched off.
    pub fn is_disabled(&self, id: &str) -> bool {
        self.disabled.iter().any(|disabled| disabled == id)
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn an_absent_table_disables_nothing() {
        let extensions = Extensions::default();
        assert!(!extensions.enabled);
        assert!(extensions.disabled.is_empty());
        assert!(!extensions.is_disabled("com.example.x"));
    }

    #[test]
    fn the_switch_and_the_list_are_read_from_the_file() {
        let parsed: crate::Settings = toml::from_str(
            r#"
            [extensions]
            enabled = true
            disabled = ["com.example.noisy", "org.example.chatty"]
            "#,
        )
        .expect("settings");
        assert!(parsed.extensions.enabled);
        assert_eq!(parsed.extensions.disabled, ["com.example.noisy", "org.example.chatty"]);
        assert!(parsed.extensions.is_disabled("org.example.chatty"));
        assert!(!parsed.extensions.is_disabled("com.example.quiet"));
    }

    #[test]
    fn an_unknown_key_in_the_table_is_refused() {
        let error = toml::from_str::<crate::Settings>(
            r#"
            [extensions]
            disable = ["com.example.noisy"]
            "#,
        )
        .expect_err("a typo must be refused");
        assert!(error.to_string().contains("disable"), "{error}");
    }

    #[test]
    fn the_written_defaults_carry_the_table() {
        let text = crate::to_toml(&crate::Settings::default());
        assert!(text.contains("[extensions]"), "{text}");
        let written = crate::parse(&text).expect("reads back");
        assert!(!written.extensions.enabled);
        assert!(written.extensions.disabled.is_empty());
    }
}
