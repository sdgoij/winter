//! Finding extensions on disk and reading what they ship.
//!
//! Discovery is the union of the roots in [`Options`], in priority order: an
//! extension has one identifier, and the first root that offers it wins. Every
//! path a manifest names is resolved against the extension's own directory and
//! canonicalised, so a `..`, an absolute path or a symlink that points outside
//! is refused rather than followed. A broken extension is reported and skipped;
//! it never stops the others.
//!
//! Nothing here links an engine: the loader reads text and bytes, and the
//! `host` feature compiles them.

use std::collections::{BTreeMap, BTreeSet, HashMap};
use std::fs;
use std::path::{Path, PathBuf};

use crate::manifest::{self, Manifest};

/// How deep the walk for modules may go.
const WALK_DEPTH_MAX: usize = 8;

/// A module an extension ships, named by its path from the extension root.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct Source {
    /// The specifier path, always `/`-separated (`lib/util.js`).
    pub name: String,
    /// The module's text.
    pub text: String,
}

/// One extension, read from disk and ready to be loaded.
#[derive(Debug)]
pub struct Loaded {
    /// The parsed and validated manifest.
    pub manifest: Manifest,
    /// The extension's canonical directory.
    pub dir: PathBuf,
    /// The ES module entry, when the manifest declares one.
    pub entry: Option<Source>,
    /// The other modules the entry may import, named from the root, sorted.
    pub sources: Vec<Source>,
    /// The compiled module's bytes, when the manifest declares one.
    pub module: Option<Vec<u8>>,
    /// Each asset's opaque name and its canonical path.
    pub assets: BTreeMap<String, PathBuf>,
}

impl Loaded {
    /// Read one extension directory.
    pub fn read(dir: &Path) -> Result<Self, Error> {
        let dir = dir.canonicalize().map_err(|source| Error::Io {
            path: dir.to_path_buf(),
            source,
        })?;
        let manifest = Manifest::load(&dir)?;

        let entry = match &manifest.entry {
            Some(relative) => {
                let path = resolve(&dir, "entry", relative)?;
                let text = read_text(&path, manifest::ENTRY_MAX)?;
                Some(Source {
                    name: relative.clone(),
                    text,
                })
            }
            None => None,
        };

        let module = match &manifest.module {
            Some(relative) => {
                let path = resolve(&dir, "module", relative)?;
                Some(manifest::read_capped(&path, manifest::MODULE_MAX)?)
            }
            None => None,
        };

        let mut skip: Vec<PathBuf> = vec![dir.join(manifest::FILE)];
        if let Some(relative) = &manifest.entry {
            skip.push(dir.join(relative));
        }
        if let Some(relative) = &manifest.module {
            skip.push(dir.join(relative));
        }
        let sources = collect_sources(&dir, &skip)?;

        let mut assets = BTreeMap::new();
        for (name, relative) in &manifest.assets {
            let path = resolve(&dir, "assets", relative)?;
            let size = fs::metadata(&path)
                .map_err(|source| Error::Io {
                    path: path.clone(),
                    source,
                })?
                .len();
            if size > manifest::ASSET_MAX {
                return Err(Error::TooLarge {
                    path,
                    size,
                    max: manifest::ASSET_MAX,
                });
            }
            assets.insert(name.clone(), path);
        }

        Ok(Self {
            manifest,
            dir,
            entry,
            sources,
            module,
            assets,
        })
    }
}

/// Where to look, and what the user switched off.
#[derive(Debug, Default, Clone, PartialEq, Eq)]
pub struct Options {
    /// Directories, in priority order. Each is either one extension (it holds
    /// `extension.toml`) or a directory of extensions. A root that does not
    /// exist is ignored, since the standard roots often do not.
    pub roots: Vec<PathBuf>,
    /// Identifiers the user disabled. Comparison is exact: identifiers are
    /// already lowercase.
    pub disabled: Vec<String>,
}

/// What discovery found.
#[derive(Debug, Default)]
pub struct Report {
    /// The extensions to load, in the order `load_after` asks for.
    pub loaded: Vec<Loaded>,
    /// The extensions that were found and not loaded, with the reason.
    pub skipped: Vec<Skipped>,
}

/// An extension that was found but not loaded.
#[derive(Debug)]
pub struct Skipped {
    /// The directory it was found in.
    pub path: PathBuf,
    /// Its identifier, when the manifest could be read far enough to have one.
    pub id: Option<String>,
    /// Why it was skipped, for the log.
    pub reason: String,
}

/// Read every extension under `options` and order them.
pub fn discover(options: &Options) -> Report {
    let mut report = Report::default();
    let mut seen: HashMap<String, PathBuf> = HashMap::new();
    let mut candidates: Vec<Loaded> = Vec::new();

    for root in &options.roots {
        for dir in candidates_in(root) {
            let loaded = match Loaded::read(&dir) {
                Ok(loaded) => loaded,
                Err(error) => {
                    report.skipped.push(Skipped {
                        path: dir,
                        id: None,
                        reason: error.to_string(),
                    });
                    continue;
                }
            };
            let id = loaded.manifest.id.clone();
            if options.disabled.contains(&id) {
                report.skipped.push(Skipped {
                    path: loaded.dir,
                    id: Some(id),
                    reason: "disabled by the user".to_string(),
                });
                continue;
            }
            if let Some(first) = seen.get(&id) {
                report.skipped.push(Skipped {
                    path: loaded.dir.clone(),
                    id: Some(id.clone()),
                    reason: format!("already loaded from {}", first.display()),
                });
                continue;
            }
            seen.insert(id, loaded.dir.clone());
            candidates.push(loaded);
        }
    }

    let (ordered, cycles) = order(candidates);
    report.loaded = ordered;
    report.skipped.extend(cycles);
    report
}

/// The directories under `root` that hold a manifest, or `root` itself.
fn candidates_in(root: &Path) -> Vec<PathBuf> {
    if root.join(manifest::FILE).is_file() {
        return vec![root.to_path_buf()];
    }
    let Ok(entries) = fs::read_dir(root) else {
        return Vec::new();
    };
    let mut dirs: Vec<PathBuf> = entries
        .filter_map(Result::ok)
        .map(|entry| entry.path())
        .filter(|path| path.join(manifest::FILE).is_file())
        .collect();
    dirs.sort();
    dirs
}

/// Sort by identifier, then honour `load_after` with a cycle reported.
///
/// The scan restarts from the lowest identifier after each emission, so the
/// result is stable: two extensions with no relation keep identifier order, and
/// a dependency moves only as far as it must. A `load_after` naming an extension
/// that is not installed is ignored, since that extension may be absent.
fn order(candidates: Vec<Loaded>) -> (Vec<Loaded>, Vec<Skipped>) {
    let mut slots: Vec<Option<Loaded>> = candidates.into_iter().map(Some).collect();
    let mut ids: Vec<(String, usize)> = slots
        .iter()
        .enumerate()
        .map(|(index, loaded)| {
            let id = loaded
                .as_ref()
                .map(|loaded| loaded.manifest.id.clone())
                .unwrap_or_default();
            (id, index)
        })
        .collect();
    ids.sort();

    let position: HashMap<&str, usize> = ids.iter().map(|(id, index)| (id.as_str(), *index)).collect();

    let deps: Vec<Vec<usize>> = slots
        .iter()
        .map(|loaded| {
            let Some(loaded) = loaded else {
                return Vec::new();
            };
            let mut unique = BTreeSet::new();
            loaded
                .manifest
                .load_after
                .iter()
                .filter_map(|id| position.get(id.as_str()).copied())
                .filter(|index| unique.insert(*index))
                .collect()
        })
        .collect();

    let mut emitted = vec![false; slots.len()];
    let mut loaded = Vec::with_capacity(slots.len());
    // One emission per pass, restarting from the lowest identifier, so a
    // dependency lands immediately before its dependants rather than after
    // every unrelated extension.
    loop {
        let next = ids
            .iter()
            .find(|(_, index)| !emitted[*index] && deps[*index].iter().all(|dep| emitted[*dep]))
            .map(|(_, index)| *index);
        let Some(index) = next else {
            break;
        };
        emitted[index] = true;
        if let Some(extension) = slots[index].take() {
            loaded.push(extension);
        }
    }

    let cycle: Vec<&str> = ids
        .iter()
        .filter(|(_, index)| !emitted[*index])
        .map(|(id, _)| id.as_str())
        .collect();
    let mut skipped = Vec::new();
    if !cycle.is_empty() {
        let reason = format!("`load_after` cycle among {}", cycle.join(", "));
        for (_, index) in &ids {
            if emitted[*index] {
                continue;
            }
            if let Some(extension) = slots[*index].take() {
                skipped.push(Skipped {
                    path: extension.dir,
                    id: Some(extension.manifest.id),
                    reason: reason.clone(),
                });
            }
        }
    }

    (loaded, skipped)
}

/// Resolve a manifest path against the extension directory.
///
/// The result is canonical and must be a file inside `dir`. Canonicalising is
/// what refuses a symlink that points outside, which a lexical `..` check alone
/// would accept.
fn resolve(dir: &Path, field: &'static str, relative: &str) -> Result<PathBuf, Error> {
    let joined = dir.join(relative);
    let path = match joined.canonicalize() {
        Ok(path) => path,
        Err(source) if source.kind() == std::io::ErrorKind::NotFound => {
            return Err(Error::Missing { path: joined });
        }
        Err(source) => {
            return Err(Error::Io { path: joined, source });
        }
    };
    if !path.starts_with(dir) {
        return Err(Error::Escape {
            field,
            path: joined,
            dir: dir.to_path_buf(),
        });
    }
    if !path.is_file() {
        return Err(Error::NotAFile { path });
    }
    Ok(path)
}

/// Read a module as UTF-8, up to `max` bytes.
fn read_text(path: &Path, max: u64) -> Result<String, Error> {
    let bytes = manifest::read_capped(path, max)?;
    String::from_utf8(bytes).map_err(|_| Error::NotUtf8 {
        path: path.to_path_buf(),
    })
}

/// Collect the `.js` and `.json` files under `dir`, excluding `skip`.
///
/// The walk does not follow symlinks and does not enter a directory whose name
/// starts with `.`; a module the extension ships is a real file in its own
/// tree. The `.json` files are registered too, so `import … with { type:
/// "json" }` works.
fn collect_sources(dir: &Path, skip: &[PathBuf]) -> Result<Vec<Source>, Error> {
    let mut files = Vec::new();
    walk(dir, 0, &mut files)?;

    let mut sources: Vec<Source> = Vec::new();
    let mut total: u64 = 0;
    for path in files {
        if skip.contains(&path) {
            continue;
        }
        let name = path
            .strip_prefix(dir)
            .unwrap_or(&path)
            .components()
            .map(|component| component.as_os_str().to_string_lossy())
            .collect::<Vec<_>>()
            .join("/");
        total = total.saturating_add(
            fs::metadata(&path)
                .map_err(|source| Error::Io {
                    path: path.clone(),
                    source,
                })?
                .len(),
        );
        if total > manifest::SOURCES_MAX {
            return Err(Error::SourcesTooLarge {
                size: total,
                max: manifest::SOURCES_MAX,
            });
        }
        let text = read_text(&path, manifest::SOURCE_MAX)?;
        sources.push(Source { name, text });
    }
    if sources.len() > manifest::SOURCE_COUNT_MAX {
        return Err(Error::TooManySources {
            max: manifest::SOURCE_COUNT_MAX,
        });
    }
    sources.sort_by(|a, b| a.name.cmp(&b.name));
    Ok(sources)
}

/// Walk `dir`, collecting `.js` and `.json` files.
fn walk(dir: &Path, depth: usize, files: &mut Vec<PathBuf>) -> Result<(), Error> {
    let entries = fs::read_dir(dir).map_err(|source| Error::Io {
        path: dir.to_path_buf(),
        source,
    })?;
    let mut entries: Vec<fs::DirEntry> = entries.collect::<Result<Vec<_>, _>>().map_err(|source| Error::Io {
        path: dir.to_path_buf(),
        source,
    })?;
    entries.sort_by_key(|entry| entry.file_name());

    for entry in entries {
        let name = entry.file_name();
        if name.to_string_lossy().starts_with('.') {
            continue;
        }
        let path = entry.path();
        let file_type = entry.file_type().map_err(|source| Error::Io {
            path: path.clone(),
            source,
        })?;
        // A symlink is neither: `file_type` reports the link, not its target.
        if file_type.is_dir() {
            if depth + 1 > WALK_DEPTH_MAX {
                return Err(Error::TooDeep { path });
            }
            walk(&path, depth + 1, files)?;
        } else if file_type.is_file() {
            let extension = path.extension().and_then(|extension| extension.to_str());
            if matches!(extension, Some("js" | "json")) {
                files.push(path);
            }
        }
    }
    Ok(())
}

/// What discovery can be wrong about.
#[derive(Debug, thiserror::Error)]
pub enum Error {
    /// The manifest, or a file the manifest bounds.
    #[error(transparent)]
    Manifest(#[from] manifest::Error),
    /// A directory or file could not be read.
    #[error("cannot read {path}: {source}")]
    Io {
        /// What failed.
        path: PathBuf,
        /// The operating system's reason.
        #[source]
        source: std::io::Error,
    },
    /// A path the manifest names does not exist.
    #[error("{path} does not exist")]
    Missing {
        /// The path as resolved.
        path: PathBuf,
    },
    /// A path the manifest names is not a file.
    #[error("{path} is not a file")]
    NotAFile {
        /// The path as resolved.
        path: PathBuf,
    },
    /// A module is not UTF-8.
    #[error("{path} is not UTF-8")]
    NotUtf8 {
        /// The module that failed.
        path: PathBuf,
    },
    /// A path escapes the extension directory.
    #[error("`{field}` names {path}, which is outside {dir}")]
    Escape {
        /// The manifest field that carries it.
        field: &'static str,
        /// The path as written, joined to the directory.
        path: PathBuf,
        /// The extension's directory.
        dir: PathBuf,
    },
    /// A file is longer than its cap.
    #[error("{path} is {size} bytes, over the {max}-byte cap")]
    TooLarge {
        /// The file that is too long.
        path: PathBuf,
        /// Its length.
        size: u64,
        /// The cap.
        max: u64,
    },
    /// The extension ships more modules than [`manifest::SOURCE_COUNT_MAX`].
    #[error("the extension has more than {max} extra modules")]
    TooManySources {
        /// The cap.
        max: usize,
    },
    /// The extension's extra modules total more than [`manifest::SOURCES_MAX`].
    #[error("the extra modules total {size} bytes, over the {max}-byte cap")]
    SourcesTooLarge {
        /// Their total.
        size: u64,
        /// The cap.
        max: u64,
    },
    /// The tree is deeper than [`WALK_DEPTH_MAX`].
    #[error("{path} is nested deeper than {WALK_DEPTH_MAX} directories")]
    TooDeep {
        /// The directory that is too deep.
        path: PathBuf,
    },
}

#[cfg(test)]
mod tests {
    use super::*;

    fn write(dir: &Path, name: &str, text: &str) {
        let path = dir.join(name);
        if let Some(parent) = path.parent() {
            fs::create_dir_all(parent).expect("parent");
        }
        fs::write(path, text).expect("write");
    }

    fn manifest(id: &str, extra: &str) -> String {
        format!("id = \"{id}\"\nname = \"{id}\"\nversion = \"1\"\napi = 1\nentry = \"main.js\"\n{extra}")
    }

    /// An extension directory named after its id, with a one-line entry.
    fn extension(root: &Path, id: &str, extra: &str) -> PathBuf {
        let dir = root.join(id);
        fs::create_dir_all(&dir).expect("dir");
        write(&dir, manifest::FILE, &manifest(id, extra));
        write(&dir, "main.js", "export const ready = true;\n");
        dir
    }

    fn discover_one(dir: &Path) -> Report {
        discover(&Options {
            roots: vec![dir.to_path_buf()],
            disabled: Vec::new(),
        })
    }

    fn ids(report: &Report) -> Vec<&str> {
        report.loaded.iter().map(|loaded| loaded.manifest.id.as_str()).collect()
    }

    #[test]
    fn a_directory_of_extensions_is_discovered_in_identifier_order() {
        let temp = tempfile::tempdir().expect("tempdir");
        extension(temp.path(), "b.one", "");
        extension(temp.path(), "a.one", "");
        let report = discover_one(temp.path());
        assert_eq!(ids(&report), ["a.one", "b.one"]);
        assert!(report.skipped.is_empty(), "{:?}", report.skipped);
    }

    #[test]
    fn an_explicit_root_may_be_one_extension_itself() {
        let temp = tempfile::tempdir().expect("tempdir");
        let dir = extension(temp.path(), "a.one", "");
        let report = discover_one(&dir);
        assert_eq!(ids(&report), ["a.one"]);
    }

    #[test]
    fn a_root_that_does_not_exist_is_ignored() {
        let report = discover_one(Path::new("/nonexistent/winter/extensions"));
        assert!(report.loaded.is_empty());
        assert!(report.skipped.is_empty());
    }

    #[test]
    fn modules_are_collected_at_any_depth_and_named_from_the_root() {
        let temp = tempfile::tempdir().expect("tempdir");
        let dir = extension(temp.path(), "a.one", "");
        write(&dir, "lib/util.js", "export const x = 1;\n");
        write(&dir, "lib/deep/more.json", "{\"k\": 1}\n");
        let report = discover_one(temp.path());
        let names: Vec<&str> = report.loaded[0]
            .sources
            .iter()
            .map(|source| source.name.as_str())
            .collect();
        assert_eq!(names, ["lib/deep/more.json", "lib/util.js"]);
    }

    #[test]
    fn the_entry_is_not_registered_as_another_module() {
        let temp = tempfile::tempdir().expect("tempdir");
        extension(temp.path(), "a.one", "");
        let report = discover_one(temp.path());
        let loaded = &report.loaded[0];
        assert_eq!(loaded.entry.as_ref().map(|entry| entry.name.as_str()), Some("main.js"));
        assert!(loaded.sources.is_empty(), "{:?}", loaded.sources);
    }

    #[test]
    fn the_compiled_module_and_assets_are_read_beside_the_entry() {
        let temp = tempfile::tempdir().expect("tempdir");
        let dir = extension(
            temp.path(),
            "a.one",
            "module = \"code.wasm\"\n[assets]\nicon = \"assets/icon.png\"\n",
        );
        write(&dir, "code.wasm", "\0asm");
        write(&dir, "assets/icon.png", "png");
        let report = discover_one(temp.path());
        let loaded = &report.loaded[0];
        assert_eq!(loaded.module.as_deref(), Some(&b"\0asm"[..]));
        assert!(loaded.assets.contains_key("icon"));
        let names: Vec<&str> = loaded.sources.iter().map(|s| s.name.as_str()).collect();
        assert!(!names.contains(&"code.wasm"), "{names:?}");
    }

    #[test]
    fn a_hidden_directory_is_not_walked() {
        let temp = tempfile::tempdir().expect("tempdir");
        let dir = extension(temp.path(), "a.one", "");
        write(&dir, ".git/objects/x.js", "export {};\n");
        let report = discover_one(temp.path());
        assert!(report.loaded[0].sources.is_empty());
    }

    #[test]
    fn an_entry_that_escapes_the_directory_is_refused() {
        let temp = tempfile::tempdir().expect("tempdir");
        let outside = temp.path().join("outside.js");
        fs::write(&outside, "export {};\n").expect("outside");
        let dir = temp.path().join("a.one");
        fs::create_dir_all(&dir).expect("dir");
        write(
            &dir,
            manifest::FILE,
            &manifest("a.one", "").replace("\"main.js\"", "\"../outside.js\""),
        );
        let report = discover_one(temp.path());
        assert!(report.loaded.is_empty());
        assert_eq!(report.skipped.len(), 1);
        assert!(
            report.skipped[0].reason.contains("outside"),
            "{}",
            report.skipped[0].reason
        );
    }

    #[test]
    fn a_missing_entry_skips_that_extension_and_not_its_neighbours() {
        let temp = tempfile::tempdir().expect("tempdir");
        let broken = extension(temp.path(), "a.broken", "");
        fs::remove_file(broken.join("main.js")).expect("remove");
        extension(temp.path(), "b.fine", "");
        let report = discover_one(temp.path());
        assert_eq!(ids(&report), ["b.fine"]);
        assert_eq!(report.skipped.len(), 1);
    }

    #[test]
    fn duplicate_identifiers_keep_the_first_root() {
        let temp = tempfile::tempdir().expect("tempdir");
        let first = temp.path().join("first");
        let second = temp.path().join("second");
        fs::create_dir_all(&first).expect("first");
        fs::create_dir_all(&second).expect("second");
        extension(&first, "a.one", "");
        extension(&second, "a.one", "");
        let report = discover(&Options {
            roots: vec![first, second],
            disabled: Vec::new(),
        });
        assert_eq!(report.loaded.len(), 1);
        assert_eq!(report.skipped.len(), 1);
        assert!(report.skipped[0].reason.contains("already loaded"));
    }

    #[test]
    fn a_disabled_extension_is_skipped_and_remembered_by_identifier() {
        let temp = tempfile::tempdir().expect("tempdir");
        extension(temp.path(), "a.noisy", "");
        extension(temp.path(), "b.quiet", "");
        let report = discover(&Options {
            roots: vec![temp.path().to_path_buf()],
            disabled: vec!["a.noisy".to_string()],
        });
        assert_eq!(ids(&report), ["b.quiet"]);
        assert_eq!(report.skipped[0].id.as_deref(), Some("a.noisy"));
    }

    #[test]
    fn load_after_moves_a_dependency_ahead_without_disturbing_the_rest() {
        let temp = tempfile::tempdir().expect("tempdir");
        extension(temp.path(), "a.needs-b", "load_after = [\"b.base\"]\n");
        extension(temp.path(), "b.base", "");
        extension(temp.path(), "c.free", "");
        let report = discover_one(temp.path());
        assert_eq!(ids(&report), ["b.base", "a.needs-b", "c.free"]);
    }

    #[test]
    fn a_load_after_cycle_drops_its_members_and_keeps_the_others() {
        let temp = tempfile::tempdir().expect("tempdir");
        extension(temp.path(), "a.one", "load_after = [\"b.two\"]\n");
        extension(temp.path(), "b.two", "load_after = [\"a.one\"]\n");
        extension(temp.path(), "c.three", "");
        let report = discover_one(temp.path());
        assert_eq!(ids(&report), ["c.three"]);
        assert_eq!(report.skipped.len(), 2);
        assert!(
            report.skipped[0].reason.contains("cycle"),
            "{}",
            report.skipped[0].reason
        );
    }

    #[test]
    fn a_missing_load_after_target_is_ignored() {
        let temp = tempfile::tempdir().expect("tempdir");
        extension(temp.path(), "a.one", "load_after = [\"z.absent\"]\n");
        let report = discover_one(temp.path());
        assert_eq!(ids(&report), ["a.one"]);
    }

    #[test]
    fn a_self_dependency_is_a_cycle() {
        let temp = tempfile::tempdir().expect("tempdir");
        extension(temp.path(), "a.one", "load_after = [\"a.one\"]\n");
        let report = discover_one(temp.path());
        assert!(report.loaded.is_empty());
        assert!(report.skipped[0].reason.contains("cycle"));
    }

    #[test]
    fn an_asset_that_is_missing_skips_the_extension() {
        let temp = tempfile::tempdir().expect("tempdir");
        extension(temp.path(), "a.one", "[assets]\nicon = \"nope.png\"\n");
        let report = discover_one(temp.path());
        assert!(report.loaded.is_empty());
        assert!(report.skipped[0].reason.contains("nope.png"));
    }

    #[test]
    fn an_oversized_manifest_is_refused_before_it_is_parsed() {
        let temp = tempfile::tempdir().expect("tempdir");
        let dir = temp.path().join("a.one");
        fs::create_dir_all(&dir).expect("dir");
        let padding = " ".repeat(manifest::MANIFEST_MAX as usize + 1);
        write(&dir, manifest::FILE, &format!("{}\n{padding}", manifest("a.one", "")));
        let report = discover_one(temp.path());
        assert!(report.loaded.is_empty());
        assert!(report.skipped[0].reason.contains("cap"), "{}", report.skipped[0].reason);
    }

    #[cfg(unix)]
    #[test]
    fn a_symlinked_module_inside_the_tree_is_not_followed() {
        use std::os::unix::fs::symlink;
        let temp = tempfile::tempdir().expect("tempdir");
        let dir = extension(temp.path(), "a.one", "");
        let outside = temp.path().join("outside.js");
        fs::write(&outside, "export const leaked = 1;\n").expect("outside");
        symlink(&outside, dir.join("link.js")).expect("symlink");
        let report = discover_one(temp.path());
        assert!(report.loaded[0].sources.is_empty());
    }

    #[cfg(unix)]
    #[test]
    fn a_symlinked_entry_that_points_outside_is_refused() {
        use std::os::unix::fs::symlink;
        let temp = tempfile::tempdir().expect("tempdir");
        let dir = temp.path().join("a.one");
        fs::create_dir_all(&dir).expect("dir");
        write(&dir, manifest::FILE, &manifest("a.one", ""));
        let outside = temp.path().join("outside.js");
        fs::write(&outside, "export {};\n").expect("outside");
        symlink(&outside, dir.join("main.js")).expect("symlink");
        let report = discover_one(temp.path());
        assert!(report.loaded.is_empty());
        assert!(report.skipped[0].reason.contains("outside"));
    }
}
