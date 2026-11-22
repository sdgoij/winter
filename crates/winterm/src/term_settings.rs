//! What one terminal starts with, ported from `TerminalSettings.cpp`.
//!
//! Upstream's `TerminalSettings` is the resolved copy of every core and control
//! setting a profile names, inherited from the profile defaults and handed to
//! the connection and the control when a tab opens. This port resolves the
//! subset the shell consumes at that moment — the command, the tab's title and
//! the colour scheme — and leaves the rest of the file where the window reads it
//! (ADR 036).

use std::path::Path;

use server::pty::PtyCommand;
use settings::Settings;

/// The command, title and colours a new terminal starts with.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct TerminalSettings {
    /// The shell the connection runs.
    pub command: PtyCommand,
    /// The title the tab shows until the program sets one with OSC 0/2.
    pub title: String,
    /// The scheme the pane's palette starts from.
    pub scheme: Option<settings::Scheme>,
}

impl TerminalSettings {
    /// Resolves the default profile's terminal.
    pub fn resolve(settings: &Settings) -> Self {
        Self::for_profile(settings, settings.profile())
    }

    /// Resolves one profile's terminal.
    ///
    /// A profile that names a title keeps it; otherwise the tab is titled after
    /// the program, which is what `command_title` gives.
    pub fn for_profile(settings: &Settings, profile: Option<&settings::Profile>) -> Self {
        let profile = profile.cloned().unwrap_or_default();
        let command = PtyCommand::new(&profile.command).args(profile.args.iter().cloned());
        let title = profile.title.clone().unwrap_or_else(|| command_title(&command));

        Self {
            command,
            title,
            scheme: settings.scheme().cloned(),
        }
    }
}

/// The tab title for a command: its program's name without the extension.
fn command_title(command: &PtyCommand) -> String {
    Path::new(command.program())
        .file_stem()
        .and_then(std::ffi::OsStr::to_str)
        .unwrap_or("shell")
        .to_string()
}

#[cfg(test)]
mod tests {
    use super::*;

    fn settings(toml: &str) -> Settings {
        settings::parse(toml).expect("settings")
    }

    #[test]
    fn the_default_profile_resolves_its_command_and_title() {
        let settings = settings(
            r##"
                [[profiles]]
                name = "only"
                command = "zsh"
                args = ["-l"]
            "##,
        );
        let resolved = TerminalSettings::resolve(&settings);

        assert_eq!(resolved.command.program(), "zsh");
        assert_eq!(resolved.title, "zsh");
        assert_eq!(resolved.scheme, None);
    }

    #[test]
    fn a_profile_title_wins_over_the_program_name() {
        let settings = settings(
            r##"
                [[profiles]]
                name = "only"
                command = "pwsh.exe"
                title = "Shell"
            "##,
        );
        assert_eq!(TerminalSettings::resolve(&settings).title, "Shell");
    }

    #[test]
    fn the_scheme_is_resolved_with_the_settings() {
        let settings = settings(
            r##"
                default_scheme = "night"

                [[profiles]]
                name = "only"
                command = "sh"

                [[schemes]]
                name = "night"
                background = "#101010"
            "##,
        );
        let resolved = TerminalSettings::resolve(&settings);
        assert_eq!(resolved.scheme.map(|scheme| scheme.name), Some("night".to_string()));
    }

    #[test]
    fn a_named_profile_is_resolved_by_name() {
        let settings = settings(
            r##"
                [[profiles]]
                name = "one"
                command = "one.exe"

                [[profiles]]
                name = "two"
                command = "two.exe"
            "##,
        );
        let resolved = TerminalSettings::for_profile(&settings, settings.profile_named("two"));
        assert_eq!(resolved.command.program(), "two.exe");
        assert_eq!(resolved.title, "two");
    }

    #[test]
    fn a_program_without_a_name_is_still_titled() {
        let settings = settings("[[profiles]]\nname = \"a\"\ncommand = \"sh\"\n");
        assert!(!TerminalSettings::resolve(&settings).title.is_empty());
    }
}
