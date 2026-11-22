//! Reading and writing the settings document.
//!
//! A missing file is normal — a fresh install has none — and yields the
//! defaults silently. A file that is present but unreadable or invalid also
//! yields the defaults, but the reason is kept for the settings window, which
//! is the only place a user can see it (ADR 022).

use std::fmt;
use std::path::{Path, PathBuf};

use crate::validation::Warning;
use crate::{CURRENT_VERSION, Settings, migration};

/// Parses a settings document, filling in what it leaves out.
///
pub fn parse(text: &str) -> Result<Settings, SettingsError> {
    parse_with_warnings(text).map(|(settings, _)| settings)
}

/// Parses a settings document and returns what normalization changed.
///
/// A document written for a newer schema is refused before it is normalized:
/// the fields whose meaning changed are exactly the ones this build would
/// misread (ADR 026). The warnings are not errors: the file is used, and the
/// window says what had to be decided for it (ADR 027).
pub fn parse_with_warnings(text: &str) -> Result<(Settings, Vec<Warning>), SettingsError> {
    let settings: Settings = toml::from_str(text)?;
    if settings.version > CURRENT_VERSION {
        return Err(SettingsError::NewerVersion {
            found: settings.version,
            supported: CURRENT_VERSION,
        });
    }
    Ok(settings.normalize())
}

/// Why a settings document was not used.
#[derive(Debug)]
pub enum SettingsError {
    /// The document is not TOML for this schema.
    Format(toml::de::Error),
    /// The document names a schema version this build does not know.
    NewerVersion {
        /// The version the document names.
        found: u32,
        /// The version this build writes and understands.
        supported: u32,
    },
}

impl fmt::Display for SettingsError {
    fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        match self {
            Self::Format(error) => error.fmt(f),
            Self::NewerVersion { found, supported } => write!(
                f,
                "the file is version {found}; this build understands up to version {supported}"
            ),
        }
    }
}

impl std::error::Error for SettingsError {
    fn source(&self) -> Option<&(dyn std::error::Error + 'static)> {
        match self {
            Self::Format(error) => Some(error),
            Self::NewerVersion { .. } => None,
        }
    }
}

impl From<toml::de::Error> for SettingsError {
    fn from(error: toml::de::Error) -> Self {
        Self::Format(error)
    }
}

/// Serialises settings as TOML, which is what the file is written in.
pub fn to_toml(settings: &Settings) -> String {
    toml::to_string_pretty(settings).unwrap_or_default()
}

/// Writes `settings` to the platform's settings file.
///
/// The document is written beside the target and renamed over it, so a reader —
/// including this application's own watcher — never sees half a file (ADR 028).
pub fn save(settings: &Settings) -> Result<(), SaveError> {
    let Some(path) = settings_path() else {
        return Err(SaveError::NoPath);
    };
    save_to(&path, settings).map_err(SaveError::Io)
}

/// Writes `settings` to `path`, creating the directory it needs.
pub fn save_to(path: &Path, settings: &Settings) -> std::io::Result<()> {
    if let Some(parent) = path.parent() {
        std::fs::create_dir_all(parent)?;
    }

    let temporary = path.with_extension("toml.tmp");
    std::fs::write(&temporary, to_toml(settings))?;
    if let Err(error) = std::fs::rename(&temporary, path) {
        let _ = std::fs::remove_file(&temporary);
        return Err(error);
    }

    Ok(())
}

/// Why the settings file could not be written.
#[derive(Debug)]
pub enum SaveError {
    /// This platform has no settings directory.
    NoPath,
    /// The file could not be written.
    Io(std::io::Error),
}

impl fmt::Display for SaveError {
    fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        match self {
            Self::NoPath => f.write_str("this platform has no settings directory"),
            Self::Io(error) => write!(f, "the settings file could not be written: {error}"),
        }
    }
}

impl std::error::Error for SaveError {
    fn source(&self) -> Option<&(dyn std::error::Error + 'static)> {
        match self {
            Self::NoPath => None,
            Self::Io(error) => Some(error),
        }
    }
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
    /// The earlier file these settings were migrated from, if they were.
    pub migrated_from: Option<PathBuf>,
    /// Why the file was not used. A missing file is not an error.
    pub error: Option<LoadError>,
    /// What normalization had to decide for a file that was used.
    pub warnings: Vec<Warning>,
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
    /// The file was read but is not valid settings TOML.
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

pub(crate) fn load_from_path(path: Option<PathBuf>) -> Loaded {
    let Some(path) = path else {
        return Loaded {
            settings: Settings::default(),
            path: None,
            migrated_from: None,
            error: None,
            warnings: Vec::new(),
        };
    };

    let text = match std::fs::read_to_string(&path) {
        Ok(text) => text,
        Err(error) if error.kind() == std::io::ErrorKind::NotFound => {
            return migrate_beside(&path).unwrap_or(Loaded {
                settings: Settings::default(),
                path: Some(path),
                migrated_from: None,
                error: None,
                warnings: Vec::new(),
            });
        }
        Err(error) => {
            return Loaded {
                settings: Settings::default(),
                path: Some(path.clone()),
                migrated_from: None,
                error: Some(LoadError::Unreadable {
                    path,
                    detail: error.to_string(),
                }),
                warnings: Vec::new(),
            };
        }
    };

    match parse_with_warnings(&text) {
        Ok((settings, warnings)) => Loaded {
            settings,
            path: Some(path),
            migrated_from: None,
            error: None,
            warnings,
        },
        Err(error) => Loaded {
            settings: Settings::default(),
            path: Some(path.clone()),
            migrated_from: None,
            error: Some(LoadError::Invalid {
                path,
                detail: error.to_string(),
            }),
            warnings: Vec::new(),
        },
    }
}

/// Reads the earlier JSON file beside `path`, when one is there.
///
/// The migrated settings are written out as TOML, so the next start reads the
/// current file; a failure to write is not a reason to lose them. The old file
/// stays: deleting a user's file is not a migration.
fn migrate_beside(path: &Path) -> Option<Loaded> {
    let legacy = migration::legacy_path(path);
    let text = match std::fs::read_to_string(&legacy) {
        Ok(text) => text,
        Err(error) if error.kind() == std::io::ErrorKind::NotFound => return None,
        Err(error) => {
            return Some(Loaded {
                settings: Settings::default(),
                path: Some(path.to_path_buf()),
                migrated_from: None,
                error: Some(LoadError::Unreadable {
                    path: legacy,
                    detail: error.to_string(),
                }),
                warnings: Vec::new(),
            });
        }
    };

    match migration::from_json(&text) {
        Ok((settings, warnings)) => {
            let _ = save_to(path, &settings);
            Some(Loaded {
                settings,
                path: Some(path.to_path_buf()),
                migrated_from: Some(legacy),
                error: None,
                warnings,
            })
        }
        Err(error) => Some(Loaded {
            settings: Settings::default(),
            path: Some(path.to_path_buf()),
            migrated_from: None,
            error: Some(LoadError::Invalid {
                path: legacy,
                detail: error.to_string(),
            }),
            warnings: Vec::new(),
        }),
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
    base.join("winterm").join("settings.toml")
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn the_settings_file_sits_under_the_app_directory() {
        let base = Path::new("/tmp/config");
        assert_eq!(
            settings_path_in(base),
            PathBuf::from("/tmp/config/winterm/settings.toml")
        );
    }

    #[test]
    fn a_missing_file_is_not_an_error() {
        let loaded = load_from_path(Some(PathBuf::from("/definitely/not/here.toml")));
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
        let path = directory.path().join("settings.toml");
        std::fs::write(&path, "[[").expect("write");

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
    fn a_file_from_a_newer_build_is_reported() {
        let directory = tempfile::tempdir().expect("temp dir");
        let path = directory.path().join("settings.toml");
        std::fs::write(&path, "version = 7").expect("write");

        let loaded = load_from_path(Some(path));
        assert_eq!(loaded.settings, Settings::default());
        match loaded.error {
            Some(LoadError::Invalid { detail, .. }) => {
                assert!(detail.contains("version 7"), "{detail}");
                assert!(detail.contains("version 1"), "{detail}");
            }
            other => panic!("expected an invalid-file error, got {other:?}"),
        }
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
    fn a_save_creates_the_directory_and_writes_a_loadable_file() {
        let directory = tempfile::tempdir().expect("temp dir");
        let path = directory.path().join("winterm").join("settings.toml");

        let settings = Settings::default();
        save_to(&path, &settings).expect("save");

        let loaded = load_from_path(Some(path.clone()));
        assert_eq!(loaded.error, None);
        assert_eq!(loaded.settings, settings);
        assert!(!path.with_extension("toml.tmp").exists(), "the temporary file is gone");
    }

    #[test]
    fn a_second_save_replaces_the_first() {
        let directory = tempfile::tempdir().expect("temp dir");
        let path = directory.path().join("settings.toml");

        let first = parse("version = 1\n\n[font]\nsize = 20\n").expect("parse");
        save_to(&path, &first).expect("save");

        let second = parse("version = 1\n\n[font]\nsize = 24\n").expect("parse");
        save_to(&path, &second).expect("save");

        assert_eq!(load_from_path(Some(path)).settings.font.size, 24.0);
    }

    #[test]
    fn a_valid_file_is_loaded() {
        let directory = tempfile::tempdir().expect("temp dir");
        let path = directory.path().join("settings.toml");
        std::fs::write(&path, "[[profiles]]\nname = \"a\"\ncommand = \"sh\"\n").expect("write");

        let loaded = load_from_path(Some(path));
        assert_eq!(loaded.error, None);
        assert_eq!(
            loaded.settings.profile().map(|profile| profile.name.as_str()),
            Some("a")
        );
    }

    #[test]
    fn an_earlier_json_file_is_migrated_and_written_as_toml() {
        let directory = tempfile::tempdir().expect("temp dir");
        let path = directory.path().join("settings.toml");
        let legacy = directory.path().join("settings.json");
        std::fs::write(
            &legacy,
            r#"{ "profiles": [{ "name": "old", "command": "cmd.exe" }], "default_profile": "old" }"#,
        )
        .expect("write");

        let loaded = load_from_path(Some(path.clone()));

        assert_eq!(loaded.error, None);
        assert_eq!(loaded.migrated_from, Some(legacy.clone()));
        assert_eq!(
            loaded.settings.profile().map(|profile| profile.name.as_str()),
            Some("old")
        );
        assert!(path.exists(), "the migration is written so it runs once");
        assert!(legacy.exists(), "the earlier file is left alone");

        // The second start reads the TOML and does not migrate again.
        let loaded = load_from_path(Some(path));
        assert_eq!(loaded.migrated_from, None);
        assert_eq!(
            loaded.settings.profile().map(|profile| profile.name.as_str()),
            Some("old")
        );
    }

    #[test]
    fn a_settings_file_wins_over_the_earlier_one() {
        let directory = tempfile::tempdir().expect("temp dir");
        let path = directory.path().join("settings.toml");
        std::fs::write(&path, "[[profiles]]\nname = \"new\"\ncommand = \"sh\"\n").expect("write");
        std::fs::write(directory.path().join("settings.json"), "{}").expect("write");

        let loaded = load_from_path(Some(path));
        assert_eq!(loaded.migrated_from, None);
        assert_eq!(
            loaded.settings.profile().map(|profile| profile.name.as_str()),
            Some("new")
        );
    }

    #[test]
    fn an_earlier_file_that_does_not_parse_is_reported() {
        let directory = tempfile::tempdir().expect("temp dir");
        let path = directory.path().join("settings.toml");
        let legacy = directory.path().join("settings.json");
        std::fs::write(&legacy, "{ not json").expect("write");

        let loaded = load_from_path(Some(path.clone()));

        assert_eq!(loaded.settings, Settings::default());
        assert_eq!(loaded.migrated_from, None);
        match loaded.error {
            Some(LoadError::Invalid { path, detail }) => {
                assert_eq!(path, legacy);
                assert!(!detail.is_empty());
            }
            other => panic!("expected an invalid-file error, got {other:?}"),
        }
        assert!(!path.exists(), "a failed migration writes nothing");
    }
}
