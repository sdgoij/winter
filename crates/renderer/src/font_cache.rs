//! A process-wide font collection, ported from `src/renderer/base/FontCache.h`.
//!
//! Upstream caches DirectWrite's system font collection and lets `.ttf` files
//! in the executable's directory take precedence over installed families. The
//! portable shape is the same: build a collection from an ordered list of
//! directories (nearby first), read each file for the family it declares, and
//! keep the first face of a family and style.
//!
//! DirectWrite is replaced by a reader for the four tables a terminal needs to
//! choose a face — `name`, `post`, `OS/2`, `head` (ADR 008). Glyph data stays
//! with the rasteriser. Locating the platform's font directories belongs to the
//! host layer, so this crate stays free of `cfg(windows)`/`cfg(unix)` (ADR 006).

use std::path::{Path, PathBuf};
use std::sync::OnceLock;

/// Extensions the directory scan opens.
const FONT_EXTENSIONS: [&str; 4] = ["ttf", "otf", "ttc", "otc"];

/// Font files larger than this are skipped rather than read into memory.
const MAX_FONT_FILE_BYTES: u64 = 64 * 1024 * 1024;

/// How deep the recursive scan descends below a font directory.
const MAX_SCAN_DEPTH: u32 = 8;

/// Family names tried in order when no family is requested.
///
/// Upstream resolves the `__DefaultTTFont__` sentinel through DirectWrite. A
/// directory scan has no such sentinel, so the portable stand-in is a
/// preference list, then the first fixed-pitch face the collection holds.
pub const PREFERRED_MONOSPACE_FAMILIES: &[&str] = &[
    "Cascadia Mono",
    "Cascadia Code",
    "Consolas",
    "DejaVu Sans Mono",
    "Liberation Mono",
    "Menlo",
    "Monaco",
    "Courier New",
];

/// A font file the collection can open.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct FontFace {
    family: String,
    subfamily: String,
    path: PathBuf,
    index: u32,
    fixed_pitch: bool,
    weight: u16,
    italic: bool,
}

impl FontFace {
    /// Returns the family name (`name` ID 16, else ID 1).
    pub fn family(&self) -> &str {
        &self.family
    }

    /// Returns the style name (`name` ID 17, else ID 2), empty when absent.
    pub fn subfamily(&self) -> &str {
        &self.subfamily
    }

    /// Returns the font file the face lives in.
    pub fn path(&self) -> &Path {
        &self.path
    }

    /// Returns the face index inside a `ttcf` collection, zero otherwise.
    pub const fn index(&self) -> u32 {
        self.index
    }

    /// Returns whether `post.isFixedPitch` marks the face monospace.
    pub const fn is_fixed_pitch(&self) -> bool {
        self.fixed_pitch
    }

    /// Returns the `OS/2` weight class (400 when the table is absent).
    pub const fn weight(&self) -> u16 {
        self.weight
    }

    /// Returns whether the face is italic.
    pub const fn is_italic(&self) -> bool {
        self.italic
    }
}

/// The faces found in the scanned directories, in precedence order.
#[derive(Debug, Clone, Default, PartialEq, Eq)]
pub struct FontCollection {
    faces: Vec<FontFace>,
}

impl FontCollection {
    /// Creates an empty collection.
    pub fn new() -> Self {
        Self::default()
    }

    /// Builds a collection from `directories`, in precedence order.
    ///
    /// Directories are scanned recursively; a face whose family and style are
    /// already present is ignored, so the first directory that holds it wins.
    pub fn from_directories<'a>(directories: impl IntoIterator<Item = &'a Path>) -> Self {
        let mut collection = Self::new();
        for directory in directories {
            collection.add_directory(directory);
        }
        collection
    }

    /// Builds a collection where `nearby` shadows `system`.
    ///
    /// Upstream looks for `.ttf` files in the executable's directory and adds
    /// them before the system set, so a shipped face takes precedence. The
    /// nearby scan is not recursive and ignores every other extension.
    pub fn with_nearby_fonts<'a>(nearby: &'a Path, system: impl IntoIterator<Item = &'a Path>) -> Self {
        let mut collection = Self::new();
        collection.add_nearby_fonts(nearby);
        for directory in system {
            collection.add_directory(directory);
        }
        collection
    }

    /// Returns the faces, in precedence order.
    pub fn faces(&self) -> &[FontFace] {
        &self.faces
    }

    /// Returns the number of faces.
    pub fn len(&self) -> usize {
        self.faces.len()
    }

    /// Returns whether the collection is empty.
    pub fn is_empty(&self) -> bool {
        self.faces.is_empty()
    }

    /// Adds every font file below `directory`, recursively.
    pub fn add_directory(&mut self, directory: &Path) {
        let mut files = Vec::new();
        collect_font_files(directory, 0, &mut files);
        for file in files {
            self.add_file(&file);
        }
    }

    /// Adds an in-memory font file, for a bundled or embedded face.
    pub fn add_bytes(&mut self, path: impl Into<PathBuf>, data: &[u8]) {
        let path = path.into();
        for (index, meta) in sfnt::faces(data) {
            if self.has_face(&meta.family, &meta.subfamily) {
                continue;
            }
            self.faces.push(FontFace {
                family: meta.family,
                subfamily: meta.subfamily,
                path: path.clone(),
                index,
                fixed_pitch: meta.fixed_pitch,
                weight: meta.weight,
                italic: meta.italic,
            });
        }
    }

    /// Returns whether a family is present, ignoring ASCII case.
    pub fn has_family(&self, family: &str) -> bool {
        self.faces.iter().any(|face| face.family.eq_ignore_ascii_case(family))
    }

    /// Resolves a family request to the closest style.
    ///
    /// Ties are broken by collection order, which makes the result independent
    /// of the directory listing order.
    pub fn resolve(&self, family: &str, weight: u16, italic: bool) -> Option<&FontFace> {
        self.faces
            .iter()
            .filter(|face| face.family.eq_ignore_ascii_case(family))
            .min_by_key(|face| (u8::from(face.italic != italic), face.weight.abs_diff(weight)))
    }

    /// Returns the first fixed-pitch face of each preferred family, else the
    /// first fixed-pitch face in the collection.
    pub fn default_monospace(&self, preferred: &[&str]) -> Option<&FontFace> {
        preferred
            .iter()
            .find_map(|family| self.resolve(family, 400, false).filter(|face| face.is_fixed_pitch()))
            .or_else(|| self.faces.iter().find(|face| face.is_fixed_pitch()))
    }

    /// Returns the first preferred family with both an upright and an italic
    /// fixed-pitch face.
    ///
    /// A family can be installed upright only — Cascadia Mono ships that way on
    /// some machines — and choosing it would leave SGR 3 upright while a later
    /// preference has the face. The pair comes back together so the caller
    /// registers a family that can render both.
    pub fn default_monospace_with_italic(&self, preferred: &[&str]) -> Option<(&FontFace, &FontFace)> {
        for family in preferred {
            let Some(regular) = self
                .resolve(family, 400, false)
                .filter(|face| face.is_fixed_pitch() && !face.is_italic())
            else {
                continue;
            };
            let Some(italic) = self
                .resolve(family, 400, true)
                .filter(|face| face.is_italic() && face.is_fixed_pitch())
            else {
                continue;
            };
            return Some((regular, italic));
        }
        None
    }

    /// Iterates over the fixed-pitch faces.
    pub fn monospace_faces(&self) -> impl Iterator<Item = &FontFace> {
        self.faces.iter().filter(|face| face.is_fixed_pitch())
    }

    fn add_nearby_fonts(&mut self, directory: &Path) {
        for path in sorted_entries(directory) {
            if path.is_file() && has_extension(&path, "ttf") {
                self.add_file(&path);
            }
        }
    }

    fn add_file(&mut self, path: &Path) {
        let Ok(metadata) = std::fs::metadata(path) else {
            return;
        };
        if !metadata.is_file() || metadata.len() > MAX_FONT_FILE_BYTES {
            return;
        }
        let Ok(data) = std::fs::read(path) else {
            return;
        };
        self.add_bytes(path, &data);
    }

    fn has_face(&self, family: &str, subfamily: &str) -> bool {
        self.faces
            .iter()
            .any(|face| face.family.eq_ignore_ascii_case(family) && face.subfamily.eq_ignore_ascii_case(subfamily))
    }
}

/// The face metadata read from a font file before its glyph data is needed.
#[derive(Debug, Clone)]
struct FaceMeta {
    family: String,
    subfamily: String,
    fixed_pitch: bool,
    weight: u16,
    italic: bool,
}

fn collect_font_files(directory: &Path, depth: u32, files: &mut Vec<PathBuf>) {
    if depth > MAX_SCAN_DEPTH {
        return;
    }
    for path in sorted_entries(directory) {
        if path.is_dir() {
            collect_font_files(&path, depth + 1, files);
        } else if FONT_EXTENSIONS.iter().any(|extension| has_extension(&path, extension)) {
            files.push(path);
        }
    }
}

/// Returns a directory's entries sorted by path, so a scan is reproducible.
fn sorted_entries(directory: &Path) -> Vec<PathBuf> {
    let Ok(entries) = std::fs::read_dir(directory) else {
        return Vec::new();
    };
    let mut paths: Vec<PathBuf> = entries
        .filter_map(|entry| entry.ok())
        .map(|entry| entry.path())
        .collect();
    paths.sort();
    paths
}

fn has_extension(path: &Path, wanted: &str) -> bool {
    path.extension()
        .and_then(std::ffi::OsStr::to_str)
        .is_some_and(|extension| extension.eq_ignore_ascii_case(wanted))
}

/// The process-wide collection, built once by the host.
static CACHED: OnceLock<FontCollection> = OnceLock::new();

/// Returns the process-wide collection, building it on first use.
///
/// Later builders are not run, matching upstream's cache-on-first-use.
pub fn get_or_build(build: impl FnOnce() -> FontCollection) -> &'static FontCollection {
    CACHED.get_or_init(build)
}

/// Returns the process-wide collection, when the host has built one.
pub fn cached() -> Option<&'static FontCollection> {
    CACHED.get()
}

/// Reads face metadata from SFNT font files.
///
/// Only the tables a face choice needs are parsed. Every offset and length is
/// checked against the buffer, so truncated or hostile files yield no faces
/// rather than slicing out of range.
mod sfnt {
    use super::FaceMeta;

    const TAG_NAME: &[u8; 4] = b"name";
    const TAG_POST: &[u8; 4] = b"post";
    const TAG_OS2: &[u8; 4] = b"OS/2";
    const TAG_HEAD: &[u8; 4] = b"head";
    const TAG_TTCF: &[u8; 4] = b"ttcf";

    /// The Windows `name` table platform ID.
    const PLATFORM_WINDOWS: u16 = 3;
    /// The Unicode `name` table platform ID.
    const PLATFORM_UNICODE: u16 = 0;
    /// The Classic Macintosh `name` table platform ID.
    const PLATFORM_MACINTOSH: u16 = 1;
    /// `0x0409`, English (United States).
    const LANGUAGE_EN_US: u16 = 0x0409;

    /// `OS/2.fsSelection` bit 0.
    const FS_SELECTION_ITALIC: u16 = 0x0001;
    /// `head.macStyle` bit 1.
    const MAC_STYLE_ITALIC: u16 = 0x0002;
    /// `usWeightClass` for a regular face, used when `OS/2` is absent.
    const WEIGHT_NORMAL: u16 = 400;

    /// Returns the faces in a font file, with their collection index.
    pub(super) fn faces(data: &[u8]) -> Vec<(u32, FaceMeta)> {
        let mut faces = Vec::new();
        for (index, directory) in directories(data).into_iter().enumerate() {
            if let Some(meta) = face_meta(data, directory) {
                faces.push((index as u32, meta));
            }
        }
        faces
    }

    /// Returns the table-directory offsets in a font file.
    fn directories(data: &[u8]) -> Vec<usize> {
        if data.get(0..4) == Some(&TAG_TTCF[..]) {
            let available = data.len().saturating_sub(12) / 4;
            let count = be_u32(data, 8).map_or(0, |count| count as usize).min(available);
            (0..count)
                .filter_map(|index| be_u32(data, 12 + index * 4))
                .map(|offset| offset as usize)
                .collect()
        } else if is_sfnt(data) {
            vec![0]
        } else {
            Vec::new()
        }
    }

    fn is_sfnt(data: &[u8]) -> bool {
        match data.get(0..4) {
            Some([0x00, 0x01, 0x00, 0x00]) => true,
            Some(tag) => tag == &b"OTTO"[..] || tag == &b"true"[..] || tag == &b"typ1"[..],
            None => false,
        }
    }

    fn face_meta(data: &[u8], directory: usize) -> Option<FaceMeta> {
        let name = table(data, directory, TAG_NAME)?;
        let family = name_string(name, 16).or_else(|| name_string(name, 1))?;
        let subfamily = name_string(name, 17)
            .or_else(|| name_string(name, 2))
            .unwrap_or_default();
        Some(FaceMeta {
            family,
            subfamily,
            fixed_pitch: fixed_pitch(data, directory),
            weight: weight(data, directory),
            italic: italic(data, directory),
        })
    }

    /// Returns a table's bytes, or `None` when it is absent or out of range.
    fn table<'a>(data: &'a [u8], directory: usize, wanted: &[u8; 4]) -> Option<&'a [u8]> {
        let count = usize::from(be_u16(data, directory.checked_add(4)?)?);
        let records = directory.checked_add(12)?;
        let end = records.checked_add(count.checked_mul(16)?)?;
        let directory_bytes = data.get(directory..end)?;
        for record in directory_bytes[12..].as_chunks::<16>().0 {
            if &record[0..4] != wanted {
                continue;
            }
            let offset = u32::from_be_bytes([record[8], record[9], record[10], record[11]]) as usize;
            let length = u32::from_be_bytes([record[12], record[13], record[14], record[15]]) as usize;
            return data.get(offset..offset.checked_add(length)?);
        }
        None
    }

    /// Reads a string out of the `name` table.
    fn name_string(name: &[u8], name_id: u16) -> Option<String> {
        let count = usize::from(be_u16(name, 2)?);
        let strings = usize::from(be_u16(name, 4)?);
        name.get(6..6usize.checked_add(count.checked_mul(12)?)?)?;

        let mut best: Option<(u8, u32, String)> = None;
        for index in 0..count {
            let record = 6 + index * 12;
            if be_u16(name, record + 6)? != name_id {
                continue;
            }
            let platform = be_u16(name, record)?;
            let language = be_u16(name, record + 4)?;
            let length = usize::from(be_u16(name, record + 8)?);
            let offset = usize::from(be_u16(name, record + 10)?);
            let start = strings.checked_add(offset)?;
            let Some(bytes) = name.get(start..start.checked_add(length)?) else {
                continue;
            };
            let Some(text) = decode(platform, bytes) else {
                continue;
            };
            if text.is_empty() {
                continue;
            }
            let rank = (platform_rank(platform), language_rank(platform, language));
            let better = match best.as_ref() {
                Some((best_platform, best_language, _)) => rank < (*best_platform, *best_language),
                None => true,
            };
            if better {
                best = Some((rank.0, rank.1, text));
            }
        }
        best.map(|(_, _, text)| text)
    }

    /// Ranks a `name` record; lower is preferred.
    fn platform_rank(platform: u16) -> u8 {
        match platform {
            PLATFORM_WINDOWS => 0,
            PLATFORM_UNICODE => 1,
            PLATFORM_MACINTOSH => 2,
            _ => 3,
        }
    }

    fn language_rank(platform: u16, language: u16) -> u32 {
        if platform == PLATFORM_WINDOWS && language != LANGUAGE_EN_US {
            1
        } else {
            0
        }
    }

    /// Decodes a `name` record's bytes.
    ///
    /// Windows and Unicode records are UTF-16BE. Macintosh records are
    /// MacRoman; they are mapped byte-for-byte to Latin-1, which is exact for
    /// the ASCII family names a terminal cares about.
    fn decode(platform: u16, bytes: &[u8]) -> Option<String> {
        match platform {
            PLATFORM_WINDOWS | PLATFORM_UNICODE => {
                let units: Vec<u16> = bytes
                    .as_chunks::<2>()
                    .0
                    .iter()
                    .map(|pair| u16::from_be_bytes(*pair))
                    .collect();
                Some(
                    char::decode_utf16(units)
                        .map(|unit| unit.unwrap_or(char::REPLACEMENT_CHARACTER))
                        .collect(),
                )
            }
            PLATFORM_MACINTOSH => Some(bytes.iter().map(|byte| char::from(*byte)).collect()),
            _ => None,
        }
    }

    fn fixed_pitch(data: &[u8], directory: usize) -> bool {
        table(data, directory, TAG_POST)
            .and_then(|post| be_u32(post, 12))
            .is_some_and(|value| value != 0)
    }

    fn weight(data: &[u8], directory: usize) -> u16 {
        table(data, directory, TAG_OS2)
            .and_then(|os2| be_u16(os2, 4))
            .unwrap_or(WEIGHT_NORMAL)
    }

    fn italic(data: &[u8], directory: usize) -> bool {
        if let Some(selection) = table(data, directory, TAG_OS2).and_then(|os2| be_u16(os2, 62)) {
            return selection & FS_SELECTION_ITALIC != 0;
        }
        table(data, directory, TAG_HEAD)
            .and_then(|head| be_u16(head, 44))
            .is_some_and(|style| style & MAC_STYLE_ITALIC != 0)
    }

    fn be_u16(data: &[u8], offset: usize) -> Option<u16> {
        let bytes = data.get(offset..offset.checked_add(2)?)?;
        Some(u16::from_be_bytes([bytes[0], bytes[1]]))
    }

    fn be_u32(data: &[u8], offset: usize) -> Option<u32> {
        let bytes = data.get(offset..offset.checked_add(4)?)?;
        Some(u32::from_be_bytes([bytes[0], bytes[1], bytes[2], bytes[3]]))
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use std::sync::atomic::{AtomicU32, Ordering};

    /// A temporary directory that removes itself, with no dependency.
    struct TestDirectory {
        path: PathBuf,
    }

    impl TestDirectory {
        fn new(label: &str) -> Self {
            static COUNTER: AtomicU32 = AtomicU32::new(0);
            let counter = COUNTER.fetch_add(1, Ordering::Relaxed);
            let path =
                std::env::temp_dir().join(format!("winterm-font-cache-{}-{label}-{counter}", std::process::id()));
            match std::fs::remove_dir_all(&path) {
                Ok(()) => {}
                Err(error) if error.kind() == std::io::ErrorKind::NotFound => {}
                Err(error) => panic!("cannot clear {path:?}: {error}"),
            }
            match std::fs::create_dir_all(&path) {
                Ok(()) => Self { path },
                Err(error) => panic!("cannot create {path:?}: {error}"),
            }
        }

        fn path(&self) -> &Path {
            &self.path
        }

        fn write(&self, name: &str, bytes: &[u8]) {
            let path = self.path.join(name);
            match std::fs::write(&path, bytes) {
                Ok(()) => {}
                Err(error) => panic!("cannot write {path:?}: {error}"),
            }
        }
    }

    impl Drop for TestDirectory {
        fn drop(&mut self) {
            let _ = std::fs::remove_dir_all(&self.path);
        }
    }

    fn name_table(entries: &[(u16, &str)]) -> Vec<u8> {
        let mut records = Vec::with_capacity(entries.len() * 12);
        let mut strings = Vec::new();
        for (name_id, text) in entries {
            let encoded: Vec<u8> = text.encode_utf16().flat_map(u16::to_be_bytes).collect();
            let offset = strings.len() as u16;
            strings.extend_from_slice(&encoded);
            records.extend_from_slice(&3u16.to_be_bytes());
            records.extend_from_slice(&1u16.to_be_bytes());
            records.extend_from_slice(&0x0409u16.to_be_bytes());
            records.extend_from_slice(&name_id.to_be_bytes());
            records.extend_from_slice(&(encoded.len() as u16).to_be_bytes());
            records.extend_from_slice(&offset.to_be_bytes());
        }
        let mut table = Vec::new();
        table.extend_from_slice(&0u16.to_be_bytes());
        table.extend_from_slice(&(entries.len() as u16).to_be_bytes());
        table.extend_from_slice(&((6 + records.len()) as u16).to_be_bytes());
        table.extend_from_slice(&records);
        table.extend_from_slice(&strings);
        table
    }

    fn post_table(fixed_pitch: bool) -> Vec<u8> {
        let mut table = vec![0u8; 32];
        table[0..4].copy_from_slice(&0x0002_0000u32.to_be_bytes());
        table[12..16].copy_from_slice(&u32::from(fixed_pitch).to_be_bytes());
        table
    }

    fn os2_table(weight: u16, italic: bool) -> Vec<u8> {
        let mut table = vec![0u8; 78];
        table[4..6].copy_from_slice(&weight.to_be_bytes());
        table[62..64].copy_from_slice(&u16::from(italic).to_be_bytes());
        table
    }

    fn head_table(italic: bool) -> Vec<u8> {
        let mut table = vec![0u8; 54];
        table[44..46].copy_from_slice(&(u16::from(italic) << 1).to_be_bytes());
        table
    }

    fn sfnt(tables: &[(&[u8; 4], Vec<u8>)]) -> Vec<u8> {
        let directory_size = 12 + tables.len() * 16;
        let mut directory = Vec::with_capacity(directory_size);
        directory.extend_from_slice(&[0x00, 0x01, 0x00, 0x00]);
        directory.extend_from_slice(&(tables.len() as u16).to_be_bytes());
        directory.extend_from_slice(&0u16.to_be_bytes());
        directory.extend_from_slice(&0u16.to_be_bytes());
        directory.extend_from_slice(&0u16.to_be_bytes());

        let mut body = Vec::new();
        for (tag, data) in tables {
            let offset = directory_size + body.len();
            directory.extend_from_slice(*tag);
            directory.extend_from_slice(&0u32.to_be_bytes());
            directory.extend_from_slice(&(offset as u32).to_be_bytes());
            directory.extend_from_slice(&(data.len() as u32).to_be_bytes());
            body.extend_from_slice(data);
            while body.len() % 4 != 0 {
                body.push(0);
            }
        }
        directory.extend_from_slice(&body);
        directory
    }

    fn test_face(family: &str, subfamily: &str, fixed_pitch: bool, weight: u16, italic: bool) -> Vec<u8> {
        sfnt(&[
            (b"name", name_table(&[(1, family), (2, subfamily)])),
            (b"post", post_table(fixed_pitch)),
            (b"OS/2", os2_table(weight, italic)),
            (b"head", head_table(italic)),
        ])
    }

    /// Builds a `ttcf` file out of complete SFNT buffers.
    fn font_collection(faces: &[Vec<u8>]) -> Vec<u8> {
        let header_size = 12 + faces.len() * 4;
        let mut offsets = Vec::with_capacity(faces.len());
        let mut base = header_size;
        for face in faces {
            offsets.push(base as u32);
            base += face.len();
        }

        let mut out = Vec::with_capacity(base);
        out.extend_from_slice(b"ttcf");
        out.extend_from_slice(&0x0001_0000u32.to_be_bytes());
        out.extend_from_slice(&(faces.len() as u32).to_be_bytes());
        for offset in &offsets {
            out.extend_from_slice(&offset.to_be_bytes());
        }
        for (face, offset) in faces.iter().zip(offsets) {
            out.extend_from_slice(&with_shifted_table_offsets(face, offset as usize));
        }
        out
    }

    /// Rewrites a standalone SFNT's table offsets so it can sit inside a `ttcf`.
    fn with_shifted_table_offsets(face: &[u8], base: usize) -> Vec<u8> {
        let mut face = face.to_vec();
        let count = usize::from(u16::from_be_bytes([face[4], face[5]]));
        for index in 0..count {
            let record = 12 + index * 16;
            let offset = u32::from_be_bytes([face[record + 8], face[record + 9], face[record + 10], face[record + 11]]);
            face[record + 8..record + 12].copy_from_slice(&(offset + base as u32).to_be_bytes());
        }
        face
    }

    fn collection_of(path: &str, data: &[u8]) -> FontCollection {
        let mut collection = FontCollection::new();
        collection.add_bytes(PathBuf::from(path), data);
        collection
    }

    #[test]
    fn family_and_style_are_read_from_the_name_table() {
        let collection = collection_of("cascadia.ttf", &test_face("Cascadia Mono", "Regular", true, 400, false));

        assert_eq!(collection.len(), 1);
        let face = &collection.faces()[0];
        assert_eq!(face.family(), "Cascadia Mono");
        assert_eq!(face.subfamily(), "Regular");
        assert!(face.is_fixed_pitch());
        assert_eq!(face.weight(), 400);
        assert!(!face.is_italic());
        assert_eq!(face.index(), 0);
        assert_eq!(face.path(), Path::new("cascadia.ttf"));
    }

    #[test]
    fn the_os2_weight_and_style_are_read() {
        let collection = collection_of(
            "bold-italic.ttf",
            &test_face("DejaVu Sans Mono", "Bold Oblique", true, 700, true),
        );

        let face = &collection.faces()[0];
        assert_eq!(face.weight(), 700);
        assert!(face.is_italic());
        assert_eq!(face.subfamily(), "Bold Oblique");
    }

    #[test]
    fn a_face_without_a_post_table_is_not_fixed_pitch() {
        let data = sfnt(&[
            (b"name", name_table(&[(1, "Proportional"), (2, "Regular")])),
            (b"OS/2", os2_table(400, false)),
        ]);
        let collection = collection_of("proportional.ttf", &data);

        assert_eq!(collection.len(), 1);
        assert!(!collection.faces()[0].is_fixed_pitch());
        assert_eq!(collection.faces()[0].weight(), 400);
    }

    #[test]
    fn a_collection_file_yields_every_face() {
        let data = font_collection(&[
            test_face("First Mono", "Regular", true, 400, false),
            test_face("Second Mono", "Bold", true, 700, false),
        ]);
        let collection = collection_of("bundle.ttc", &data);

        assert_eq!(collection.len(), 2);
        assert_eq!(collection.faces()[0].family(), "First Mono");
        assert_eq!(collection.faces()[0].index(), 0);
        assert_eq!(collection.faces()[1].family(), "Second Mono");
        assert_eq!(collection.faces()[1].index(), 1);
    }

    #[test]
    fn a_truncated_or_foreign_file_yields_no_faces() {
        let data = test_face("Cascadia Mono", "Regular", true, 400, false);
        let count = usize::from(u16::from_be_bytes([data[4], data[5]]));
        let directory_end = 12 + count * 16;

        // A file that ends before its table directory is complete names no face.
        for length in [0usize, 4, 8, 11, 12, directory_end - 1] {
            let collection = collection_of("truncated.ttf", &data[..length]);
            assert!(collection.is_empty(), "length {length} produced faces");
        }

        // No truncation may panic, whatever it cuts through.
        for length in 0..=data.len() {
            let collection = collection_of("truncated.ttf", &data[..length]);
            for face in collection.faces() {
                assert!(!face.family().is_empty());
            }
        }

        let collection = collection_of("not-a-font.bin", b"this is not a font file");
        assert!(collection.is_empty());
    }

    #[test]
    fn out_of_range_table_offsets_are_not_followed() {
        let mut data = test_face("Cascadia Mono", "Regular", true, 400, false);
        // Point the first table record (the name table) past the end of the file.
        let count = usize::from(u16::from_be_bytes([data[4], data[5]]));
        assert!(count > 0);
        data[12 + 8..12 + 12].copy_from_slice(&u32::MAX.to_be_bytes());

        let collection = collection_of("broken.ttf", &data);
        assert!(collection.is_empty());
    }

    #[test]
    fn a_family_resolves_to_the_closest_style() {
        let mut collection = FontCollection::new();
        collection.add_bytes("regular.ttf", &test_face("Cascadia Mono", "Regular", true, 400, false));
        collection.add_bytes("bold.ttf", &test_face("Cascadia Mono", "Bold", true, 700, false));
        collection.add_bytes("italic.ttf", &test_face("Cascadia Mono", "Italic", true, 400, true));

        let bold = match collection.resolve("cascadia mono", 700, false) {
            Some(face) => face,
            None => panic!("bold face missing"),
        };
        assert_eq!(bold.weight(), 700);
        assert!(!bold.is_italic());

        let italic = match collection.resolve("CASCADIA MONO", 400, true) {
            Some(face) => face,
            None => panic!("italic face missing"),
        };
        assert!(italic.is_italic());

        let nearest = match collection.resolve("Cascadia Mono", 900, false) {
            Some(face) => face,
            None => panic!("nearest face missing"),
        };
        assert_eq!(nearest.weight(), 700);

        assert!(collection.resolve("Missing Family", 400, false).is_none());
    }

    #[test]
    fn a_family_that_is_already_present_is_not_added_twice() {
        let mut collection = FontCollection::new();
        collection.add_bytes("first.ttf", &test_face("Cascadia Mono", "Regular", true, 400, false));
        collection.add_bytes("second.ttf", &test_face("cascadia mono", "regular", true, 700, false));

        assert_eq!(collection.len(), 1);
        assert_eq!(collection.faces()[0].weight(), 400);
        assert_eq!(collection.faces()[0].path(), Path::new("first.ttf"));
    }

    #[test]
    fn nearby_fonts_shadow_installed_ones_and_system_styles_remain() {
        let nearby = TestDirectory::new("nearby");
        let system = TestDirectory::new("system");
        nearby.write(
            "CascadiaMono.ttf",
            &test_face("Cascadia Mono", "Regular", true, 400, false),
        );
        system.write(
            "CascadiaMono-Regular.ttf",
            &test_face("Cascadia Mono", "Regular", true, 700, false),
        );
        system.write(
            "CascadiaMono-Bold.ttf",
            &test_face("Cascadia Mono", "Bold", true, 700, false),
        );
        system.write(
            "DejaVuSansMono.ttf",
            &test_face("DejaVu Sans Mono", "Regular", true, 400, false),
        );
        // The nearby scan ignores everything but `.ttf` and does not recurse.
        nearby.write("Skipped.otf", &test_face("Skipped OTF", "Regular", true, 400, false));

        let collection = FontCollection::with_nearby_fonts(nearby.path(), [system.path()]);

        assert_eq!(collection.len(), 3);
        assert_eq!(collection.faces()[0].family(), "Cascadia Mono");
        assert_eq!(collection.faces()[0].subfamily(), "Regular");
        assert_eq!(collection.faces()[0].weight(), 400);
        assert_eq!(collection.faces()[1].subfamily(), "Bold");
        assert_eq!(collection.faces()[2].family(), "DejaVu Sans Mono");
        assert!(!collection.has_family("Skipped OTF"));
    }

    #[test]
    fn directories_are_scanned_recursively_and_deterministically() {
        let fonts = TestDirectory::new("recursive");
        let nested = fonts.path().join("dejavu");
        match std::fs::create_dir_all(&nested) {
            Ok(()) => {}
            Err(error) => panic!("cannot create {nested:?}: {error}"),
        }
        fonts.write("Alpha.ttf", &test_face("Alpha Mono", "Regular", true, 400, false));
        match std::fs::write(
            nested.join("Beta.ttf"),
            test_face("Beta Mono", "Regular", true, 400, false),
        ) {
            Ok(()) => {}
            Err(error) => panic!("cannot write nested face: {error}"),
        }

        let collection = FontCollection::from_directories([fonts.path()]);
        assert_eq!(collection.len(), 2);
        assert_eq!(collection.faces()[0].family(), "Alpha Mono");
        assert_eq!(collection.faces()[1].family(), "Beta Mono");
    }

    #[test]
    fn the_default_monospace_prefers_the_list_then_any_fixed_face() {
        let mut collection = FontCollection::new();
        collection.add_bytes(
            "proportional.ttf",
            &test_face("Proportional", "Regular", false, 400, false),
        );
        collection.add_bytes(
            "dejavu.ttf",
            &test_face("DejaVu Sans Mono", "Regular", true, 400, false),
        );
        collection.add_bytes("consolas.ttf", &test_face("Consolas", "Regular", true, 400, false));

        let preferred = match collection.default_monospace(&["Missing", "Consolas"]) {
            Some(face) => face,
            None => panic!("preferred face missing"),
        };
        assert_eq!(preferred.family(), "Consolas");

        let fallback = match collection.default_monospace(&[]) {
            Some(face) => face,
            None => panic!("fallback face missing"),
        };
        assert_eq!(fallback.family(), "DejaVu Sans Mono");

        let mut proportional_only = FontCollection::new();
        proportional_only.add_bytes(
            "proportional.ttf",
            &test_face("Proportional", "Regular", false, 400, false),
        );
        assert!(proportional_only.default_monospace(&[]).is_none());
    }

    #[test]
    fn a_family_without_an_italic_is_skipped_for_one_with_it() {
        let mut collection = FontCollection::new();
        collection.add_bytes("cascadia.ttf", &test_face("Cascadia Mono", "Roman", true, 400, false));
        collection.add_bytes("consolas.ttf", &test_face("Consolas", "Regular", true, 400, false));
        collection.add_bytes("consolas-italic.ttf", &test_face("Consolas", "Italic", true, 400, true));

        let (regular, italic) = match collection.default_monospace_with_italic(&["Cascadia Mono", "Consolas"]) {
            Some(pair) => pair,
            None => panic!("no family with an italic"),
        };
        assert_eq!(regular.family(), "Consolas");
        assert!(!regular.is_italic());
        assert_eq!(italic.subfamily(), "Italic");
    }

    #[test]
    fn a_family_without_an_italic_yields_no_pair() {
        let mut collection = FontCollection::new();
        collection.add_bytes("cascadia.ttf", &test_face("Cascadia Mono", "Roman", true, 400, false));

        assert!(collection.default_monospace_with_italic(&["Cascadia Mono"]).is_none());
    }

    #[test]
    fn the_first_build_is_cached() {
        let mut first = FontCollection::new();
        first.add_bytes("first.ttf", &test_face("First Mono", "Regular", true, 400, false));

        let installed = get_or_build(|| first.clone());
        assert_eq!(installed.faces()[0].family(), "First Mono");

        let again = get_or_build(|| {
            let mut later = FontCollection::new();
            later.add_bytes("later.ttf", &test_face("Later Mono", "Regular", true, 400, false));
            later
        });
        assert_eq!(again, installed);
        match cached() {
            Some(collection) => assert_eq!(collection, installed),
            None => panic!("the collection was not cached"),
        }
    }

    #[test]
    fn arbitrary_bytes_never_panic() {
        let mut seed = 0x5EED_1234u32;
        for length in 0..=512usize {
            let mut data = Vec::with_capacity(length);
            for _ in 0..length {
                seed ^= seed << 13;
                seed ^= seed >> 17;
                seed ^= seed << 5;
                data.push((seed & 0xFF) as u8);
            }
            let mut collection = FontCollection::new();
            collection.add_bytes("random.bin", &data);
            for face in collection.faces() {
                assert!(!face.family().is_empty());
            }
        }
    }

    #[test]
    #[ignore = "reads the host's installed fonts; run with `cargo test -p renderer -- --ignored`"]
    fn a_host_monospace_face_loads_with_fontdue() {
        let candidates = ["C:/Windows/Fonts", "/usr/share/fonts", "/System/Library/Fonts"];
        let directories: Vec<&Path> = candidates.iter().map(Path::new).filter(|path| path.is_dir()).collect();
        assert!(!directories.is_empty(), "no known font directory on this host");

        let collection = FontCollection::from_directories(directories);
        assert!(!collection.is_empty());

        let face = match collection.default_monospace(PREFERRED_MONOSPACE_FAMILIES) {
            Some(face) => face,
            None => panic!("no fixed-pitch face among {} faces", collection.len()),
        };
        let rasteriser = match crate::atlas::font::FontDueRasterizer::from_face(face, 16.0) {
            Ok(rasteriser) => rasteriser,
            Err(error) => panic!("cannot load {}: {error}", face.path().display()),
        };
        assert!(rasteriser.glyph_index('M') != 0);
    }
}
