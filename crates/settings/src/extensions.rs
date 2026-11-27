//! The `[extensions]` table: which extensions this installation does not load.

use serde::{Deserialize, Serialize};

/// The `[extensions]` table.
///
/// An identifier is the `id` from the extension's own manifest: lowercase and
/// dotted. A name this installation does not have is harmless — an extension
/// may have been uninstalled — so nothing here is checked against what is on
/// disk; discovery reports an identifier that matched nothing it found.
///
/// The table is read when the window opens, so a change to it takes effect on
/// the next start, not the next settings reload: unloading code that holds
/// registrations is a different operation from re-reading a document.
#[derive(Debug, Clone, Default, PartialEq, Eq, Serialize, Deserialize)]
#[serde(default, deny_unknown_fields)]
pub struct Extensions {
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
        assert!(extensions.disabled.is_empty());
        assert!(!extensions.is_disabled("com.example.x"));
    }

    #[test]
    fn the_table_reads_the_identifiers_the_file_names() {
        let parsed: crate::Settings = toml::from_str(
            r#"
            [extensions]
            disabled = ["com.example.noisy", "org.example.chatty"]
            "#,
        )
        .expect("settings");
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
        assert!(crate::parse(&text).expect("reads back").extensions.disabled.is_empty());
    }
}
