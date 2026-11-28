//! The `[extensions]` table: which extensions this installation does not load.

use serde::{Deserialize, Serialize};

/// The `[extensions]` table.
///
/// An identifier is the `id` from the extension's own manifest: lowercase and
/// dotted. A name this installation does not have is harmless — an extension
/// may have been uninstalled — so nothing here is checked against what is on
/// disk; discovery reports an identifier that matched nothing it found.
///
/// Both switches are read when the window opens and again whenever a running
/// shell sees the file change; a change rebuilds the engine without a restart
/// (ADR 088). The chords are read with the rest of the file and take effect
/// immediately.
#[derive(Debug, Clone, Default, PartialEq, Eq, Serialize, Deserialize)]
#[serde(default, deny_unknown_fields)]
pub struct Extensions {
    /// Whether the shell loads extensions at all. Off by default: the feature
    /// is new, and `WINTER_EXT` overrides this for one run either way.
    pub enabled: bool,
    /// Whether a running shell re-reads extension files when they change on
    /// disk. Off by default: an install and a switch rebuild without it, and a
    /// watcher costs a metadata poll per second. `WINTER_WATCH` overrides this
    /// for one run.
    pub watch: bool,
    /// Identifiers the shell skips.
    pub disabled: Vec<String>,
    /// Chords that run an extension command directly.
    #[serde(skip_serializing_if = "Vec::is_empty")]
    pub commands: Vec<ExtensionCommand>,
    /// The registry URLs this installation reads. `None` means the built-in
    /// default (ADR 088); `Some([])` means none. Nothing is fetched until the
    /// Search panel is opened.
    #[serde(skip_serializing_if = "Option::is_none")]
    pub registries: Option<Vec<String>>,
}

/// One chord bound to an extension command.
///
/// The command is named the way a palette row is — `<extension id>:<command
/// id>` — so the two surfaces share one key. The window's own bindings are
/// checked first, so a chord that already runs something cannot be shadowed by
/// an extension.
#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
#[serde(deny_unknown_fields)]
pub struct ExtensionCommand {
    /// The chord that runs the command.
    pub keys: crate::Chord,
    /// The command's key, as the palette or the file spells it.
    pub command: String,
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
        assert!(!extensions.watch);
        assert!(extensions.disabled.is_empty());
        assert!(!extensions.is_disabled("com.example.x"));
    }

    #[test]
    fn the_switch_the_list_and_the_chords_are_read_from_the_file() {
        let parsed: crate::Settings = toml::from_str(
            r#"
            [extensions]
            enabled = true
            watch = true
            disabled = ["com.example.noisy", "org.example.chatty"]

            [[extensions.commands]]
            keys = "ctrl+g"
            command = "winter.failed-command:where-am-i"
            "#,
        )
        .expect("settings");
        assert!(parsed.extensions.enabled);
        assert!(parsed.extensions.watch);
        assert_eq!(parsed.extensions.disabled, ["com.example.noisy", "org.example.chatty"]);
        assert!(parsed.extensions.is_disabled("org.example.chatty"));
        assert!(!parsed.extensions.is_disabled("com.example.quiet"));
        assert_eq!(parsed.extensions.commands.len(), 1);
        assert_eq!(
            parsed.extensions.commands[0].command,
            "winter.failed-command:where-am-i"
        );
        assert_eq!(parsed.extensions.commands[0].keys.to_string(), "ctrl+g");
    }

    #[test]
    fn a_chord_without_a_command_is_refused() {
        let error = toml::from_str::<crate::Settings>(
            r#"
            [[extensions.commands]]
            keys = "ctrl+g"
            "#,
        )
        .expect_err("a binding needs its command");
        assert!(error.to_string().contains("command"), "{error}");
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
        assert!(!written.extensions.watch);
        assert!(written.extensions.disabled.is_empty());
        assert!(written.extensions.registries.is_none());
    }

    #[test]
    fn an_absent_registry_list_means_the_built_in_default() {
        assert!(Extensions::default().registries.is_none());
        let parsed: crate::Settings = toml::from_str("[extensions]\nenabled = true\n").expect("settings");
        assert!(parsed.extensions.registries.is_none());
    }

    #[test]
    fn a_registry_list_round_trips() {
        let parsed: crate::Settings = toml::from_str(
            r#"
            [extensions]
            registries = ["https://one.example/registry.toml", "file:///tmp/registry.toml"]
            "#,
        )
        .expect("settings");
        assert_eq!(
            parsed.extensions.registries.as_deref(),
            Some(
                &[
                    "https://one.example/registry.toml".to_string(),
                    "file:///tmp/registry.toml".to_string()
                ][..]
            )
        );

        let written = crate::to_toml(&parsed);
        assert!(written.contains("registries"), "{written}");
        assert_eq!(
            crate::parse(&written).expect("reads back").extensions.registries,
            parsed.extensions.registries
        );
    }

    #[test]
    fn an_empty_registry_list_is_an_explicit_none() {
        let parsed: crate::Settings = toml::from_str("[extensions]\nregistries = []\n").expect("settings");
        assert_eq!(parsed.extensions.registries.as_deref(), Some(&[][..]));

        let written = crate::to_toml(&parsed);
        assert!(written.contains("registries = []"), "{written}");
    }
}
