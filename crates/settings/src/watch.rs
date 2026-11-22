//! Watching the settings file.
//!
//! The watcher is a poll, not a platform notification: the application asks
//! for the file's stamp about once a second and reloads when it changes. That
//! is one `metadata` call per second, needs no dependency or thread, and works
//! the same on every platform (ADR 025).
//!
//! The stamp is the modification time and the length. Together they catch every
//! save that changes the file in practice; a rewrite that changes neither
//! within the filesystem's timestamp granularity is invisible until the next
//! change.

use std::path::Path;
use std::time::SystemTime;

/// A cheap identity for a file, for detecting that it changed.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub struct Stamp {
    /// When the file was last written.
    pub modified: SystemTime,
    /// How many bytes it holds.
    pub len: u64,
}

/// Returns the stamp of the file at `path`, or None when it is not there.
///
/// None also covers a filesystem that cannot report a modification time. The
/// application treats "was there, now is not" as a change, so a file that
/// disappears reloads the defaults.
pub fn stamp(path: &Path) -> Option<Stamp> {
    let metadata = std::fs::metadata(path).ok()?;
    Some(Stamp {
        modified: metadata.modified().ok()?,
        len: metadata.len(),
    })
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn a_stamp_changes_when_the_file_changes() {
        let directory = tempfile::tempdir().expect("temp dir");
        let path = directory.path().join("settings.toml");
        assert_eq!(stamp(&path), None, "a missing file has no stamp");

        std::fs::write(&path, "{}").expect("write");
        let first = stamp(&path).expect("stamp");

        std::fs::write(&path, "{\"font\": {}}").expect("write");
        let second = stamp(&path).expect("stamp");
        assert_ne!(first, second);

        // Removing it is a change too: the defaults are in force again.
        std::fs::remove_file(&path).expect("remove");
        assert_eq!(stamp(&path), None);
    }
}
