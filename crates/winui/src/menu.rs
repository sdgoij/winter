//! The drop-down next to the new-tab button, from Windows Terminal's
//! `TerminalTabStrip` profile menu.
//!
//! Upstream builds this as a `MenuFlyout`. The `windows-reactor-pre` 0.108
//! snapshot exposes no flyout type, so the shell draws the list itself (ADR
//! 052). This module owns the list's contents: every profile in the settings
//! file, in file order, followed by the two window commands.

use settings::Profile;

/// One row of the drop-down.
#[derive(Debug, Clone, PartialEq, Eq)]
pub(super) enum MenuEntry {
    /// Open a new tab on the profile with this name.
    Profile(String),
    /// Open the settings dialog.
    Settings,
    /// Open the extensions dialog.
    Extensions,
    /// Open the About dialog.
    About,
}

/// The rows the menu shows, in order.
///
/// The profiles are taken from the settings in force, not from the defaults:
/// an added, removed or renamed profile is what the menu offers after the file
/// is reloaded. A file with no profiles still offers the two commands.
pub(super) fn entries(profiles: &[Profile]) -> Vec<MenuEntry> {
    profiles
        .iter()
        .map(|profile| MenuEntry::Profile(profile.name.clone()))
        .chain([MenuEntry::Settings, MenuEntry::Extensions, MenuEntry::About])
        .collect()
}

#[cfg(test)]
mod tests {
    use super::*;

    fn profiles_named(names: &[&str]) -> Vec<Profile> {
        names
            .iter()
            .map(|name| Profile {
                name: (*name).to_string(),
                ..Profile::default()
            })
            .collect()
    }

    #[test]
    fn every_profile_is_listed_in_order_before_the_commands() {
        let profiles = profiles_named(&["Zeta", "Alpha", "Mu"]);

        assert_eq!(
            entries(&profiles),
            vec![
                MenuEntry::Profile("Zeta".to_string()),
                MenuEntry::Profile("Alpha".to_string()),
                MenuEntry::Profile("Mu".to_string()),
                MenuEntry::Settings,
                MenuEntry::Extensions,
                MenuEntry::About,
            ]
        );
    }

    #[test]
    fn a_file_without_profiles_still_offers_settings_extensions_and_about() {
        assert_eq!(
            entries(&[]),
            vec![MenuEntry::Settings, MenuEntry::Extensions, MenuEntry::About]
        );
    }

    #[test]
    fn the_names_come_from_the_profiles_in_force() {
        let profiles = profiles_named(&["Custom"]);

        assert_eq!(
            entries(&profiles).first(),
            Some(&MenuEntry::Profile("Custom".to_string()))
        );
    }
}
