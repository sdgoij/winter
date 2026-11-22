//! Reading and writing the settings file.
//!
//! A missing file is normal — a fresh install has none — and yields the
//! defaults silently. A file that is present but unreadable or invalid also
//! yields the defaults, but the reason is kept for the settings window, which
//! is the only place a user can see it (ADR 022).

use std::fmt;
use std::path::{Path, PathBuf};

use crate::Settings;

/// Parses a settings file, filling in what it leaves out.
pub fn parse(json: &str) -> Result<Settings, serde_json::Error> {
    let settings: Settings = serde_json::from_str(json)?;
    Ok(settings.normalize())
}

/// Serialises settings for writing.
pub fn to_json(settings: &Settings) -> String {
    serde_json::to_string_pretty(settings).unwrap_or_default()
}

/// Returns where this platform keeps the settings file.
pub fn settings_path() -> Option<PathBuf> {
    Some(settings_path_in(&config_base()?))
}

/// Loads the settings file, or returns the defaults.
pub fn load() -> Settings {
    load_with_report().settings
}

/// Loads the settings file and keeps why it could not be used.
///
/// The application starts on the returned settings either way; the report is
/// what the settings window shows.
pub fn load_with_report() -> Loaded {
    load_from_path(settings_path())
}

/// The settings in force and the state of the file they came from.
#[derive(Debug, Clone, PartialEq)]
pub struct Loaded {
    /// The settings the application should run with.
    pub settings: Settings,
    /// The file that was read, when the platform has a settings path.
    pub path: Option<PathBuf>,
    /// Why the file was not used. A missing file is not an error.
    pub error: Option<LoadError>,
}

/// Why a settings file was not used.
#[derive(Debug, Clone, PartialEq, Eq)]
pub enum LoadError {
    /// The file exists but could not be read.
    Unreadable {
        /// The file that was tried.
        path: PathBuf,
        /// The operating system's message.
        detail: String,
    },
    /// The file was read but is not valid settings JSON.
    Invalid {
        /// The file that was tried.
        path: PathBuf,
        /// The parser's message.
        detail: String,
    },
}

impl fmt::Display for LoadError {
    fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        match self {
            Self::Unreadable { path, detail } => {
                write!(f, "{} could not be read: {detail}", path.display())
            }
            Self::Invalid { path, detail } => {
                write!(f, "{} does not parse: {detail}", path.display())
            }
        }
    }
}

impl std::error::Error for LoadError {}

fn load_from_path(path: Option<PathBuf>) -> Loaded {
    let Some(path) = path else {
        return Loaded {
            settings: Settings::default(),
            path: None,
            error: None,
        };
    };

    let json = match std::fs::read_to_string(&path) {
        Ok(json) => json,
        Err(error) if error.kind() == std::io::ErrorKind::NotFound => {
            return Loaded {
                settings: Settings::default(),
                path: Some(path),
                error: None,
            };
        }
        Err(error) => {
            return Loaded {
                settings: Settings::default(),
                path: Some(path.clone()),
                error: Some(LoadError::Unreadable {
                    path,
                    detail: error.to_string(),
                }),
            };
        }
    };

    match parse(&json) {
        Ok(settings) => Loaded {
            settings,
            path: Some(path),
            error: None,
        },
        Err(error) => Loaded {
            settings: Settings::default(),
            path: Some(path.clone()),
            error: Some(LoadError::Invalid {
                path,
                detail: error.to_string(),
            }),
        },
    }
}

fn config_base() -> Option<PathBuf> {
    #[cfg(windows)]
    {
        std::env::var_os("APPDATA").map(PathBuf::from)
    }
    #[cfg(unix)]
    {
        std::env::var_os("XDG_CONFIG_HOME")
            .map(PathBuf::from)
            .or_else(|| std::env::var_os("HOME").map(|home| PathBuf::from(home).join(".config")))
    }
}

fn settings_path_in(base: &Path) -> PathBuf {
    base.join("winterm").join("settings.json")
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn the_settings_file_sits_under_the_app_directory() {
        let base = Path::new("/tmp/config");
        assert_eq!(
            settings_path_in(base),
            PathBuf::from("/tmp/config/winterm/settings.json")
        );
    }

    #[test]
    fn a_missing_file_is_not_an_error() {
        let loaded = load_from_path(Some(PathBuf::from("/definitely/not/here.json")));
        assert_eq!(loaded.settings, Settings::default());
        assert_eq!(loaded.error, None);
        assert!(loaded.path.is_some());

        let loaded = load_from_path(None);
        assert_eq!(loaded.settings, Settings::default());
        assert_eq!(loaded.error, None);
        assert_eq!(loaded.path, None);
    }

    #[test]
    fn a_broken_file_loads_the_defaults_and_says_why() {
        let directory = tempfile::tempdir().expect("temp dir");
        let path = directory.path().join("settings.json");
        std::fs::write(&path, "{").expect("write");

        let loaded = load_from_path(Some(path.clone()));
        assert_eq!(loaded.settings, Settings::default());
        assert_eq!(loaded.path, Some(path.clone()));
        assert!(matches!(loaded.error, Some(LoadError::Invalid { .. })));
        match &loaded.error {
            Some(LoadError::Invalid { detail, .. }) => assert!(!detail.is_empty(), "the parser's message is kept"),
            other => panic!("expected an invalid-file error, got {other:?}"),
        }
        assert!(loaded.error.expect("error").to_string().contains("does not parse"));
    }

    #[test]
    fn a_path_that_cannot_be_read_is_reported() {
        // A directory is readable as a path but not as a file on every
        // supported platform.
        let directory = tempfile::tempdir().expect("temp dir");
        let path = directory.path().to_path_buf();

        let loaded = load_from_path(Some(path.clone()));
        assert_eq!(loaded.settings, Settings::default());
        assert!(matches!(loaded.error, Some(LoadError::Unreadable { .. })));
        assert!(loaded.error.expect("error").to_string().contains("could not be read"));
    }

    #[test]
    fn a_valid_file_is_loaded() {
        let directory = tempfile::tempdir().expect("temp dir");
        let path = directory.path().join("settings.json");
        std::fs::write(&path, r#"{"profiles": [{"name": "a", "command": "sh"}]}"#).expect("write");

        let loaded = load_from_path(Some(path));
        assert_eq!(loaded.error, None);
        assert_eq!(
            loaded.settings.profile().map(|profile| profile.name.as_str()),
            Some("a")
        );
    }
}
