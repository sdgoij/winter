//! The drop-down next to the new-tab button, from Windows Terminal's
//! `TerminalTabStrip` profile menu.
//!
//! Upstream builds this as a `MenuFlyout`. The `windows-reactor-pre` 0.108
//! snapshot exposes no flyout type, so the shell draws the list itself (ADR
//! 052). This module owns the list's contents: every profile in the settings
//! file, in file order, followed by the two window commands.

use settings::Settings;

/// One row of the drop-down.
#[derive(Debug, Clone, PartialEq, Eq)]
pub(super) enum MenuEntry {
    /// Open a new tab on the profile with this name.
    Profile(String),
    /// Open the settings dialog.
    Settings,
    /// Open the About dialog.
    About,
}

/// The rows the menu shows, in order.
///
/// The profiles are taken from the settings in force, not from the defaults:
/// an added, removed or renamed profile is what the menu offers after the file
/// is reloaded. A file with no profiles still offers the two commands.
pub(super) fn entries(settings: &Settings) -> Vec<MenuEntry> {
    settings
        .profiles
        .iter()
        .map(|profile| MenuEntry::Profile(profile.name.clone()))
        .chain([MenuEntry::Settings, MenuEntry::About])
        .collect()
}

#[cfg(test)]
mod tests {
    use settings::Profile;

    use super::*;

    fn settings_with(names: &[&str]) -> Settings {
        Settings {
            profiles: names
                .iter()
                .map(|name| Profile {
                    name: (*name).to_string(),
                    ..Profile::default()
                })
                .collect(),
            ..Settings::default()
        }
    }

    #[test]
    fn every_profile_is_listed_in_file_order_before_the_commands() {
        let settings = settings_with(&["Zeta", "Alpha", "Mu"]);

        assert_eq!(
            entries(&settings),
            vec![
                MenuEntry::Profile("Zeta".to_string()),
                MenuEntry::Profile("Alpha".to_string()),
                MenuEntry::Profile("Mu".to_string()),
                MenuEntry::Settings,
                MenuEntry::About,
            ]
        );
    }

    #[test]
    fn a_file_without_profiles_still_offers_settings_and_about() {
        let settings = settings_with(&[]);

        assert_eq!(entries(&settings), vec![MenuEntry::Settings, MenuEntry::About]);
    }

    #[test]
    fn the_names_come_from_the_file_not_from_the_defaults() {
        let settings = settings_with(&["Custom"]);

        assert_eq!(
            entries(&settings).first(),
            Some(&MenuEntry::Profile("Custom".to_string()))
        );
    }
}
