//! Reading and writing the settings file.
//!
//! A missing, unreadable or invalid file is the same to the application: it
//! runs on defaults. Reporting the difference is the settings UI's job and is
//! not implemented yet.

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
    load_from_path(settings_path())
}

fn load_from_path(path: Option<PathBuf>) -> Settings {
    path.and_then(|path| std::fs::read_to_string(path).ok())
        .and_then(|json| parse(&json).ok())
        .unwrap_or_default()
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
    fn a_missing_or_broken_file_loads_as_defaults() {
        assert_eq!(
            load_from_path(Some(PathBuf::from("/definitely/not/here.json"))),
            Settings::default()
        );

        let directory = tempfile::tempdir().expect("temp dir");
        let path = directory.path().join("settings.json");
        std::fs::write(&path, "{").expect("write");
        assert_eq!(load_from_path(Some(path)), Settings::default());
    }

    #[test]
    fn a_valid_file_is_loaded() {
        let directory = tempfile::tempdir().expect("temp dir");
        let path = directory.path().join("settings.json");
        std::fs::write(&path, r#"{"profiles": [{"name": "a", "command": "sh"}]}"#).expect("write");

        let settings = load_from_path(Some(path));
        assert_eq!(settings.profile().map(|profile| profile.name.as_str()), Some("a"));
    }
}
