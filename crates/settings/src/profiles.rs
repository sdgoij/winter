//! One runnable profile: the command a tab starts.
//!
//! Profiles are named so a future settings UI can list them and so the root
//! can name a default. The fields are exactly what the application consumes
//! today; a profile gains a colour scheme, font and starting directory when
//! those settings have consumers.

use serde::{Deserialize, Serialize};

/// A command a tab can start.
#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
#[serde(default)]
pub struct Profile {
    /// The name the default profile refers to.
    pub name: String,
    /// The program to run.
    pub command: String,
    /// Its arguments.
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
pub fn default_shell() -> String {
    #[cfg(windows)]
    {
        std::env::var("COMSPEC").unwrap_or_else(|_| "cmd.exe".to_string())
    }
    #[cfg(unix)]
    {
        std::env::var("SHELL").unwrap_or_else(|_| "/bin/sh".to_string())
    }
}
