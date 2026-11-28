//! When the files a discovery read last changed, for a shell's opt-in watcher.
//!
//! A watcher is a poll rather than an operating-system notification: the shell
//! already ticks once a second to notice a saved settings file, and a handful of
//! metadata calls on the same tick keeps that one loop the only watcher in the
//! process. The set is exactly what [`discover`](crate::discover) read — the
//! roots, each extension's manifest, entry, modules and assets — plus the
//! manifest of a directory discovery refused, so fixing a broken one is noticed
//! too.
//!
//! The watcher is opt-in (ADR 088): an install and a switch rebuild the engine
//! without it.

use std::path::{Path, PathBuf};
use std::time::SystemTime;

use crate::discover::Report;
use crate::manifest;

/// The last-modified time of each path a discovery read.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct Stamps {
    entries: Vec<(PathBuf, Option<SystemTime>)>,
}

impl Stamps {
    /// Reads the stamps of everything `report` was built from, under `roots`.
    pub fn of(report: &Report, roots: &[PathBuf]) -> Self {
        let mut paths: Vec<PathBuf> = roots.to_vec();
        for loaded in &report.loaded {
            paths.push(loaded.dir.clone());
            paths.push(loaded.dir.join(manifest::FILE));
            if let Some(entry) = &loaded.entry {
                paths.push(loaded.dir.join(&entry.name));
            }
            if let Some(module) = &loaded.manifest.module {
                paths.push(loaded.dir.join(module));
            }
            for source in &loaded.sources {
                paths.push(loaded.dir.join(&source.name));
            }
            paths.extend(loaded.assets.values().cloned());
        }
        for skipped in &report.skipped {
            paths.push(skipped.path.clone());
            paths.push(skipped.path.join(manifest::FILE));
        }
        paths.sort();
        paths.dedup();

        let entries = paths
            .into_iter()
            .map(|path| {
                let stamp = modified(&path);
                (path, stamp)
            })
            .collect();
        Self { entries }
    }

    /// Whether anything changed since the last call, refreshing the stamps when
    /// it did.
    ///
    /// A path that appeared or disappeared differs from a stamp that was taken
    /// when it did the opposite, so an added extension directory or a removed
    /// module counts as a change.
    pub fn changed(&mut self) -> bool {
        let mut changed = false;
        for (path, stamp) in &mut self.entries {
            let now = modified(path);
            if now != *stamp {
                *stamp = now;
                changed = true;
            }
        }
        changed
    }
}

fn modified(path: &Path) -> Option<SystemTime> {
    std::fs::metadata(path).and_then(|metadata| metadata.modified()).ok()
}

#[cfg(test)]
mod tests {
    use std::time::{Duration, UNIX_EPOCH};

    use super::*;

    fn write(dir: &Path, name: &str, text: &str) {
        let path = dir.join(name);
        if let Some(parent) = path.parent() {
            std::fs::create_dir_all(parent).expect("parent");
        }
        std::fs::write(path, text).expect("write");
    }

    fn extension(root: &Path, id: &str) {
        let dir = root.join(id);
        std::fs::create_dir_all(&dir).expect("dir");
        write(
            &dir,
            "extension.toml",
            &format!("id = \"{id}\"\nname = \"{id}\"\nversion = \"1.0.0\"\napi = 1\nentry = \"main.js\"\n"),
        );
        write(&dir, "main.js", "export {};\n");
    }

    fn report(root: &Path) -> Report {
        crate::discover(&crate::Options {
            roots: vec![root.to_path_buf()],
            disabled: Vec::new(),
        })
    }

    /// Sets a file's modification time to a fixed instant, so a test does not
    /// depend on filesystem timestamp granularity.
    fn touch(path: &Path, seconds: u64) {
        let file = std::fs::OpenOptions::new().write(true).open(path).expect("open");
        file.set_times(std::fs::FileTimes::new().set_modified(UNIX_EPOCH + Duration::from_secs(seconds)))
            .expect("times");
    }

    #[test]
    fn a_quiet_tree_reports_nothing() {
        let temp = tempfile::tempdir().expect("temp");
        extension(temp.path(), "a.b");
        let roots = vec![temp.path().to_path_buf()];
        let mut stamps = Stamps::of(&report(temp.path()), &roots);

        assert!(!stamps.changed());
        assert!(!stamps.changed());
    }

    #[test]
    fn an_edited_entry_is_reported_once() {
        let temp = tempfile::tempdir().expect("temp");
        extension(temp.path(), "a.b");
        let roots = vec![temp.path().to_path_buf()];
        let mut stamps = Stamps::of(&report(temp.path()), &roots);

        touch(&temp.path().join("a.b").join("main.js"), 1_000_000);

        assert!(stamps.changed());
        assert!(!stamps.changed());
    }

    #[test]
    fn a_root_that_appears_is_reported() {
        let temp = tempfile::tempdir().expect("temp");
        let root = temp.path().join("extensions");
        let roots = vec![root.clone()];
        let mut stamps = Stamps::of(&report(temp.path()), &roots);
        assert!(!stamps.changed());

        std::fs::create_dir_all(&root).expect("root");
        extension(&root, "a.b");

        assert!(stamps.changed());
    }

    #[test]
    fn a_repaired_manifest_is_reported() {
        let temp = tempfile::tempdir().expect("temp");
        let broken = temp.path().join("x.y");
        std::fs::create_dir_all(&broken).expect("dir");
        write(&broken, "extension.toml", "not a manifest");
        let roots = vec![temp.path().to_path_buf()];
        let found = report(temp.path());
        assert_eq!(found.skipped.len(), 1);
        let mut stamps = Stamps::of(&found, &roots);

        write(
            &broken,
            "extension.toml",
            "id = \"x.y\"\nname = \"X\"\nversion = \"1.0.0\"\napi = 1\nentry = \"main.js\"\n",
        );
        touch(&broken.join("extension.toml"), 1_000_000);

        assert!(stamps.changed());
    }

    #[test]
    fn a_removed_file_is_reported() {
        let temp = tempfile::tempdir().expect("temp");
        extension(temp.path(), "a.b");
        let roots = vec![temp.path().to_path_buf()];
        let mut stamps = Stamps::of(&report(temp.path()), &roots);

        std::fs::remove_file(temp.path().join("a.b").join("main.js")).expect("remove");

        assert!(stamps.changed());
    }
}
