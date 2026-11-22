//! One runnable profile: the command a tab starts.
//!
//! Profiles are named so a future settings UI can list them and so the root
//! can name a default. The fields are exactly what the application consumes
//! today; a profile gains a colour scheme, font and starting directory when
//! those settings have consumers.

use serde::{Deserialize, Serialize};

/// A command a tab can start.
///
/// Every field defaults to absent rather than to a runnable value: a profile
/// that omits its command is something normalization has to fill in, and the
/// window should say so (ADR 027). `Profile::default()` is the runnable one.
#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
pub struct Profile {
    /// The name the default profile refers to.
    #[serde(default)]
    pub name: String,
    /// The program to run.
    #[serde(default)]
    pub command: String,
    /// Its arguments.
    #[serde(default)]
    pub args: Vec<String>,
    /// The title to show instead of the program's name.
    pub title: Option<String>,
}

impl Default for Profile {
    fn default() -> Self {
        Self {
            name: "default".to_string(),
            command: default_shell(),
            args: Vec::new(),
            title: None,
        }
    }
}

/// The shell this platform starts when the settings do not name one.
///
/// Windows starts PowerShell, which every supported release ships; the command
/// prompt and Bash are ordinary profiles, not fallbacks. Elsewhere the user's
/// `SHELL` wins.
pub fn default_shell() -> String {
    #[cfg(windows)]
    {
        "powershell.exe".to_string()
    }
    #[cfg(unix)]
    {
        std::env::var("SHELL").unwrap_or_else(|_| "/bin/sh".to_string())
    }
}

/// The profiles a fresh installation offers.
///
/// Windows offers the three shells the port targets, PowerShell first so a
/// fresh installation opens it. Bash resolves through `PATH`, which is where a
/// WSL or Git Bash installation puts it. Other platforms keep the one profile
/// their `SHELL` names.
pub fn default_profiles() -> Vec<Profile> {
    #[cfg(windows)]
    {
        vec![
            named("PowerShell", &default_shell()),
            named("Command Prompt", &command_prompt()),
            named("Bash", "bash.exe"),
        ]
    }
    #[cfg(unix)]
    {
        vec![Profile::default()]
    }
}

/// A default profile whose tab title is its name.
#[cfg(windows)]
fn named(name: &str, command: &str) -> Profile {
    Profile {
        name: name.to_string(),
        command: command.to_string(),
        args: Vec::new(),
        title: Some(name.to_string()),
    }
}

/// The command prompt's path as the environment spells it.
#[cfg(windows)]
fn command_prompt() -> String {
    std::env::var("COMSPEC").unwrap_or_else(|_| "cmd.exe".to_string())
}
