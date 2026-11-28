//! The Search tab's state: a lazy fetch, the rows a user sees, and install and
//! uninstall work moved off the UI thread.
//!
//! A shell owns one [`Screen`] while its extensions dialog is open. Opening the
//! Search tab calls [`Screen::open`], which is the first and only place a
//! registry is read (ADR 088); the Installed tab never fetches. A user action
//! starts a worker and the shell polls [`Screen::poll`] from its tick, so a
//! download never blocks a frame.
//!
//! The rows are owned data rather than borrows of the index, because a shell
//! renders them in its own toolkit and cannot hold the index across a frame.

use std::collections::HashMap;
use std::path::{Path, PathBuf};
use std::sync::mpsc::{Receiver, TryRecvError, channel};
use std::thread;

use semver::Version;

use crate::fetch::Network;
use crate::install::{self, Provenance};
use crate::job::{Job, State};

/// The copy of an identifier already under the settings root.
#[derive(Debug, Clone, PartialEq, Eq)]
pub enum Local {
    /// Installed by hand; an install refuses to replace it.
    Manual,
    /// Installed by a registry, with the record that says which one.
    Registry(Provenance),
}

/// One row of the Search tab.
///
/// Two registries that list the same identifier are two rows, not one: the user
/// picks which registry to install from and the provenance record keeps the
/// answer (ADR 088).
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct SearchRow {
    /// The registry's URL, which is its identity.
    pub registry: String,
    /// The registry's display name.
    pub registry_name: String,
    /// The manifest identifier.
    pub id: String,
    /// The name a user sees.
    pub name: String,
    /// One line for the row.
    pub description: Option<String>,
    /// The project's page.
    pub homepage: Option<String>,
    /// The declared license.
    pub license: Option<String>,
    /// Words the search matches.
    pub tags: Vec<String>,
    /// Identifiers this extension expects beside it, shown but not installed.
    pub dependencies: Vec<String>,
    /// The releases this build can install, newest first.
    pub versions: Vec<Version>,
    /// The copy already on disk, when there is one.
    pub installed: Option<Local>,
}

impl SearchRow {
    /// Whether the row can be installed without replacing another registry's
    /// copy or a hand-made directory.
    pub fn installable(&self) -> bool {
        match &self.installed {
            None => true,
            Some(Local::Manual) => false,
            Some(Local::Registry(found)) => found.registry == self.registry,
        }
    }
}

/// What the screen's last install or uninstall did.
#[derive(Debug, Clone, PartialEq, Eq)]
pub enum Status {
    /// A worker is running.
    Working,
    /// It finished, and this is what the user should read.
    Done(String),
    /// It failed, and this is why.
    Failed(String),
}

/// What [`Screen::poll`] found.
#[derive(Debug, Default, Clone, Copy, PartialEq, Eq)]
pub struct Changed {
    /// The rows or the status changed, so the screen must be drawn again.
    pub display: bool,
    /// The extension tree changed on disk, so the engine must be rebuilt.
    pub tree: bool,
}

/// The Search tab, the registry fetches behind it, and their workers.
#[derive(Debug)]
pub struct Screen {
    urls: Vec<String>,
    cache: PathBuf,
    extensions: PathBuf,
    job: Option<Job>,
    query: String,
    /// The version a user picked, keyed by registry and identifier.
    versions: HashMap<(String, String), Version>,
    /// What is on disk, keyed by the manifest identifier.
    on_disk: HashMap<String, Local>,
    worker: Option<Receiver<Result<String, String>>>,
    status: Option<Status>,
}

impl Screen {
    /// A screen for `urls`, writing the cache under `cache` and installing
    /// under `extensions`.
    ///
    /// Nothing is fetched here: [`open`](Self::open) is what reads the network.
    pub fn new(urls: Vec<String>, cache: PathBuf, extensions: PathBuf) -> Self {
        let mut screen = Self {
            urls,
            cache,
            extensions,
            job: None,
            query: String::new(),
            versions: HashMap::new(),
            on_disk: HashMap::new(),
            worker: None,
            status: None,
        };
        screen.refresh_on_disk();
        screen
    }

    /// The registry URLs this screen reads, in configured order.
    pub fn urls(&self) -> &[String] {
        &self.urls
    }

    /// The filter the Search tab shows.
    pub fn query(&self) -> &str {
        &self.query
    }

    /// Replaces the filter. An empty query keeps every row.
    pub fn set_query(&mut self, query: impl Into<String>) {
        self.query = query.into();
    }

    /// Starts the fetch if it has not started. Returns whether it did.
    ///
    /// The Search tab calls this when it is first shown, which is the first
    /// network access an installation makes (ADR 088).
    pub fn open(&mut self) -> bool {
        if self.job.is_some() {
            return false;
        }
        self.job = Some(Job::start(self.urls.clone(), self.cache.clone()));
        true
    }

    /// Whether any registry is still being read.
    pub fn is_loading(&self) -> bool {
        self.job.as_ref().is_some_and(Job::is_loading)
    }

    /// One state per URL, in configured order; empty before [`open`](Self::open).
    pub fn states(&self) -> &[State] {
        self.job.as_ref().map_or(&[], Job::states)
    }

    /// The last install or uninstall's status.
    pub fn status(&self) -> Option<&Status> {
        self.status.as_ref()
    }

    /// Whether an install or uninstall worker is running.
    pub fn is_working(&self) -> bool {
        self.worker.is_some()
    }

    /// What is on disk for `id`, when anything is.
    pub fn local(&self, id: &str) -> Option<&Local> {
        self.on_disk.get(id)
    }

    /// The rows matching the query, registry by registry.
    pub fn rows(&self) -> Vec<SearchRow> {
        let Some(job) = &self.job else {
            return Vec::new();
        };
        job.index()
            .matching(&self.query)
            .into_iter()
            .map(|row| {
                let mut versions: Vec<Version> = row
                    .extension
                    .installable(extensions::manifest::API)
                    .map(|release| release.version.clone())
                    .collect();
                versions.sort_by(|left, right| right.cmp(left));
                SearchRow {
                    registry: row.registry.url.clone(),
                    registry_name: row.registry.document.name.clone(),
                    id: row.extension.id.clone(),
                    name: row.extension.name.clone(),
                    description: row.extension.description.clone(),
                    homepage: row.extension.homepage.clone(),
                    license: row.extension.license.clone(),
                    tags: row.extension.tags.clone(),
                    dependencies: row.extension.dependencies.clone(),
                    installed: self.on_disk.get(&row.extension.id).cloned(),
                    versions,
                }
            })
            .collect()
    }

    /// The version a user picked for `row`, or its newest when none was picked.
    pub fn version<'a>(&self, row: &'a SearchRow) -> Option<&'a Version> {
        let key = (row.registry.clone(), row.id.clone());
        match self.versions.get(&key) {
            Some(picked) => row.versions.iter().find(|version| *version == picked),
            None => row.versions.first(),
        }
    }

    /// Picks the version at `slot` in the row at `row` from [`rows`](Self::rows).
    pub fn select_index(&mut self, row: usize, slot: usize) -> bool {
        let rows = self.rows();
        let Some(found) = rows.get(row) else {
            return false;
        };
        let Some(version) = found.versions.get(slot).cloned() else {
            return false;
        };
        self.versions
            .insert((found.registry.clone(), found.id.clone()), version);
        true
    }

    /// Installs the row at `row` from [`rows`](Self::rows) with its picked
    /// version. Returns whether a worker was started.
    pub fn install_index(&mut self, row: usize) -> bool {
        let rows = self.rows();
        let Some(found) = rows.get(row) else {
            return false;
        };
        let Some(version) = self.version(found).cloned() else {
            return false;
        };
        let Some(job) = &self.job else {
            return false;
        };
        let Some(registry) = job
            .index()
            .registries()
            .iter()
            .find(|loaded| loaded.url == found.registry)
        else {
            return false;
        };
        let Some(extension) = registry
            .document
            .extensions
            .iter()
            .find(|extension| extension.id == found.id)
        else {
            return false;
        };
        let Some(release) = extension.release(&version) else {
            return false;
        };

        let extension = extension.clone();
        let release = release.clone();
        let registry = found.registry.clone();
        self.work(move |extensions| {
            install::install(&extensions, &registry, &extension, &release, &Network)
                .map(|installed| {
                    if installed.replaced {
                        format!("{} {} was updated", extension.name, release.version)
                    } else {
                        format!("{} {} was installed", extension.name, release.version)
                    }
                })
                .map_err(|error| error.to_string())
        })
    }

    /// Removes `id`. A hand-made directory needs `force`, which the Installed
    /// tab only offers as an explicit action.
    pub fn uninstall(&mut self, id: &str, force: bool) -> bool {
        let id = id.to_string();
        self.work(move |extensions| {
            let result = if force {
                install::uninstall_any(&extensions, &id)
            } else {
                install::uninstall(&extensions, &id)
            };
            result
                .map(|()| format!("{id} was removed"))
                .map_err(|error| error.to_string())
        })
    }

    /// Takes every finished fetch and worker, returning what changed.
    ///
    /// Call it from the shell's tick; the workers never touch the rows.
    pub fn poll(&mut self) -> Changed {
        let mut changed = Changed::default();
        if let Some(job) = self.job.as_mut() {
            changed.display |= job.poll();
        }

        if let Some(receiver) = self.worker.take() {
            match receiver.try_recv() {
                Ok(result) => {
                    match result {
                        Ok(message) => {
                            // The tree changed: re-read it so the rows and the
                            // Installed tab agree with the disk.
                            self.refresh_on_disk();
                            self.status = Some(Status::Done(message));
                            changed.tree = true;
                        }
                        Err(message) => self.status = Some(Status::Failed(message)),
                    }
                    changed.display = true;
                }
                Err(TryRecvError::Empty) => self.worker = Some(receiver),
                Err(TryRecvError::Disconnected) => {
                    self.status = Some(Status::Failed("the worker stopped".to_string()));
                    changed.display = true;
                }
            }
        }
        changed
    }

    /// Runs `job` on a worker thread, reporting its message on the next poll.
    fn work(&mut self, job: impl FnOnce(PathBuf) -> Result<String, String> + Send + 'static) -> bool {
        if self.worker.is_some() {
            return false;
        }
        let extensions = self.extensions.clone();
        let (sender, receiver) = channel();
        thread::spawn(move || {
            let _ = sender.send(job(extensions));
        });
        self.worker = Some(receiver);
        self.status = Some(Status::Working);
        true
    }

    /// Re-reads the settings root, recording which identifiers are installed
    /// and where from.
    fn refresh_on_disk(&mut self) {
        self.on_disk.clear();
        for (id, record) in installs(&self.extensions) {
            let local = record.map_or(Local::Manual, Local::Registry);
            self.on_disk.insert(id, local);
        }
    }
}

/// Every extension under `extensions`, by manifest identifier, with its
/// provenance record when a registry wrote one.
fn installs(extensions: &Path) -> Vec<(String, Option<Provenance>)> {
    let Ok(entries) = std::fs::read_dir(extensions) else {
        return Vec::new();
    };
    entries
        .flatten()
        .filter_map(|entry| {
            let dir = entry.path();
            if !dir.is_dir() {
                return None;
            }
            let manifest = extensions::Manifest::load(&dir).ok()?;
            Some((manifest.id, install::provenance(&dir)))
        })
        .collect()
}

#[cfg(test)]
mod tests {
    use std::io::Write;
    use std::time::Duration;

    use sha2::{Digest, Sha256};
    use tempfile::TempDir;
    use zip::CompressionMethod;
    use zip::write::SimpleFileOptions;

    use super::*;

    const SHA: &str = "aaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaa";

    fn file_url(path: &Path) -> String {
        let text = path.display().to_string().replace('\\', "/");
        if text.starts_with('/') {
            format!("file://{text}")
        } else {
            format!("file:///{text}")
        }
    }

    fn write_registry(dir: &Path, name: &str, extensions: &str) -> String {
        let path = dir.join(format!("{name}.toml"));
        std::fs::write(
            &path,
            format!("schema = 1\nname = \"{name}\"\nupdated = \"2026-11-28T12:00:00Z\"\n\n{extensions}"),
        )
        .expect("write");
        file_url(&path)
    }

    fn extension(id: &str, releases: &[(&str, &str)]) -> String {
        let mut text = format!(
            "[[extensions]]\nid = \"{id}\"\nname = \"{id}\"\ndescription = \"does a thing\"\ntags = [\"tool\"]\n"
        );
        for (version, url) in releases {
            text.push_str(&format!(
                "[[extensions.versions]]\nversion = \"{version}\"\napi = 1\nsource = {{ type = \"archive\", url = \"{url}\", sha256 = \"{SHA}\" }}\n"
            ));
        }
        text
    }

    fn sha256(path: &Path) -> String {
        let bytes = std::fs::read(path).expect("read");
        Sha256::digest(bytes).iter().map(|byte| format!("{byte:02x}")).collect()
    }

    fn zip_at(dir: &Path, name: &str, manifest: &str) -> PathBuf {
        let path = dir.join(name);
        let file = std::fs::File::create(&path).expect("create");
        let mut writer = zip::ZipWriter::new(file);
        let options = SimpleFileOptions::default().compression_method(CompressionMethod::Stored);
        writer.start_file("extension.toml", options).expect("start");
        writer.write_all(manifest.as_bytes()).expect("write");
        writer.start_file("main.js", options).expect("start");
        writer.write_all(b"export {};\n").expect("write");
        writer.finish().expect("finish");
        path
    }

    fn manifest() -> &'static str {
        "id = \"a.b\"\nname = \"B\"\nversion = \"0.1.0\"\napi = 1\nentry = \"main.js\"\n"
    }

    fn screen(temp: &TempDir, urls: Vec<String>) -> Screen {
        Screen::new(urls, temp.path().join("registry-cache"), temp.path().join("extensions"))
    }

    /// Polls until the fetch and any worker are done. The wait is bounded and
    /// only covers a worker thread finishing, as `job`'s own tests do.
    fn settled(screen: &mut Screen) {
        for _ in 0..500 {
            screen.poll();
            if !screen.is_loading() && !screen.is_working() {
                return;
            }
            std::thread::sleep(Duration::from_millis(10));
        }
        panic!("the screen did not settle");
    }

    #[test]
    fn nothing_is_read_until_the_search_tab_opens() {
        let temp = tempfile::tempdir().expect("temp");
        let url = write_registry(temp.path(), "one", &extension("a.b", &[("0.1.0", "https://x/y.zip")]));
        let mut screen = screen(&temp, vec![url]);

        assert!(!screen.is_loading());
        assert!(screen.rows().is_empty());
        assert!(screen.open());
        assert!(!screen.open(), "a second open must not fetch again");
        settled(&mut screen);
        assert_eq!(screen.rows().len(), 1);
    }

    #[test]
    fn two_registries_listing_one_identifier_are_two_rows() {
        let temp = tempfile::tempdir().expect("temp");
        let block = extension("a.b", &[("0.1.0", "https://x/y.zip")]);
        let one = write_registry(temp.path(), "One", &block);
        let two = write_registry(temp.path(), "Two", &block);
        let mut screen = screen(&temp, vec![one.clone(), two.clone()]);
        screen.open();
        settled(&mut screen);

        let rows = screen.rows();
        assert_eq!(rows.len(), 2);
        assert_eq!(rows[0].registry, one);
        assert_eq!(rows[1].registry, two);
        assert_eq!(rows[0].id, rows[1].id);
    }

    #[test]
    fn the_newest_release_is_the_default_and_another_can_be_picked() {
        let temp = tempfile::tempdir().expect("temp");
        let url = write_registry(
            temp.path(),
            "one",
            &extension("a.b", &[("0.1.0", "https://x/y.zip"), ("0.2.0", "https://x/z.zip")]),
        );
        let mut screen = screen(&temp, vec![url]);
        screen.open();
        settled(&mut screen);

        let row = screen.rows().remove(0);
        assert_eq!(
            row.versions,
            ["0.2.0".parse().expect("version"), "0.1.0".parse().expect("version")]
        );
        assert_eq!(screen.version(&row), Some(&"0.2.0".parse().expect("version")));

        assert!(screen.select_index(0, 1));
        let row = screen.rows().remove(0);
        assert_eq!(screen.version(&row), Some(&"0.1.0".parse().expect("version")));
    }

    #[test]
    fn an_install_lands_and_can_be_removed() {
        let temp = tempfile::tempdir().expect("temp");
        let archive = zip_at(temp.path(), "one.zip", manifest());
        let url = write_registry(
            temp.path(),
            "one",
            &extension("a.b", &[("0.1.0", &file_url(&archive))]).replace(SHA, &sha256(&archive)),
        );
        let mut screen = screen(&temp, vec![url]);
        screen.open();
        settled(&mut screen);

        assert!(screen.install_index(0));
        settled(&mut screen);
        assert!(
            matches!(screen.status(), Some(Status::Done(_))),
            "{:?}",
            screen.status()
        );
        assert!(temp.path().join("extensions/a.b/extension.toml").is_file());
        assert!(matches!(screen.local("a.b"), Some(Local::Registry(_))));

        assert!(screen.uninstall("a.b", false));
        settled(&mut screen);
        assert!(!temp.path().join("extensions/a.b").exists());
    }

    #[test]
    fn a_hand_made_directory_is_marked_manual_and_cannot_be_replaced() {
        let temp = tempfile::tempdir().expect("temp");
        let dir = temp.path().join("extensions/a.b");
        std::fs::create_dir_all(&dir).expect("dir");
        std::fs::write(dir.join("extension.toml"), manifest()).expect("write");
        std::fs::write(dir.join("main.js"), "export {};\n").expect("write");

        let url = write_registry(temp.path(), "one", &extension("a.b", &[("0.1.0", "https://x/y.zip")]));
        let mut screen = screen(&temp, vec![url]);
        assert_eq!(screen.local("a.b"), Some(&Local::Manual));

        screen.open();
        settled(&mut screen);
        let row = screen.rows().remove(0);
        assert_eq!(row.installed, Some(Local::Manual));
        assert!(!row.installable());
    }

    #[test]
    fn another_registrys_copy_is_shown_but_not_replaced() {
        let temp = tempfile::tempdir().expect("temp");
        let dir = temp.path().join("extensions/a.b");
        std::fs::create_dir_all(&dir).expect("dir");
        std::fs::write(dir.join("extension.toml"), manifest()).expect("write");
        std::fs::write(dir.join("main.js"), "export {};\n").expect("write");
        std::fs::write(
            dir.join("winter-installed.toml"),
            format!(
                "registry = \"https://one.example/registry.toml\"\nversion = \"0.1.0\"\narchive = \"https://x/y.zip\"\nsha256 = \"{SHA}\"\ninstalled = 1\n"
            ),
        )
        .expect("write");

        let url = write_registry(temp.path(), "two", &extension("a.b", &[("0.1.0", "https://x/y.zip")]));
        let mut screen = screen(&temp, vec![url]);
        screen.open();
        settled(&mut screen);

        let row = screen.rows().remove(0);
        assert!(matches!(row.installed, Some(Local::Registry(_))));
        assert!(!row.installable());
    }
}
