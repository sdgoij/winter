//! Cross-platform file I/O.
//!
//! Ported from Microsoft's Windows Terminal C++ `til::io`.

use std::fs;
use std::path::{Path, PathBuf};

/// Error type for I/O operations.
#[derive(Debug, thiserror::Error)]
pub enum IoError {
    #[error("file not found: {0}")]
    NotFound(PathBuf),
    #[error("I/O error: {0}")]
    Io(#[from] std::io::Error),
    #[error("not a UTF-8 file: {0}")]
    Utf8(#[from] std::string::FromUtf8Error),
}

/// Reads a file as UTF-8 string, stripping BOM if present.
/// Returns an empty string if the file doesn't exist.
pub fn read_file_utf8(path: &Path) -> Result<String, IoError> {
    match fs::read(path) {
        Ok(data) => {
            let s = String::from_utf8(data)?;
            // Strip UTF-8 BOM
            if s.starts_with('\u{feff}') {
                Ok(s[1..].to_string())
            } else {
                Ok(s)
            }
        }
        Err(e) if e.kind() == std::io::ErrorKind::NotFound => Ok(String::new()),
        Err(e) => Err(e.into()),
    }
}

/// Reads a file as bytes.
/// Returns an empty vec if the file doesn't exist.
pub fn read_file_bytes(path: &Path) -> Result<Vec<u8>, IoError> {
    match fs::read(path) {
        Ok(data) => Ok(data),
        Err(e) if e.kind() == std::io::ErrorKind::NotFound => Ok(Vec::new()),
        Err(e) => Err(e.into()),
    }
}

/// Writes a UTF-8 string to a file.
pub fn write_file_utf8(path: &Path, content: &str) -> Result<(), IoError> {
    fs::write(path, content.as_bytes())?;
    Ok(())
}

/// Writes bytes to a file.
pub fn write_file_bytes(path: &Path, data: &[u8]) -> Result<(), IoError> {
    fs::write(path, data)?;
    Ok(())
}

/// Atomically writes to a file by writing to a temp file and renaming.
pub fn write_file_atomic(path: &Path, content: &str) -> Result<(), IoError> {
    let tmp_path = path.with_extension("tmp");
    fs::write(&tmp_path, content.as_bytes())?;
    fs::rename(&tmp_path, path)?;
    Ok(())
}

/// Checks if a path exists.
pub fn path_exists(path: &Path) -> bool {
    path.exists()
}

/// Creates a temporary file and returns its path.
///
/// Behind the `temp-file` feature: `uuid` is what names the file, and a build
/// with no filesystem has no use for either (ADR 082).
#[cfg(feature = "temp-file")]
pub fn create_temp_file(prefix: &str, suffix: &str) -> Result<PathBuf, IoError> {
    let tmp = std::env::temp_dir();
    let name = format!("{}-{}-{}", prefix, uuid::Uuid::new_v4(), suffix);
    let path = tmp.join(name);
    Ok(path)
}

/// Reads a file and returns it as a string.
pub fn read_file(path: &Path) -> Result<String, IoError> {
    read_file_utf8(path)
}

#[cfg(test)]
mod tests {
    use super::*;
    use tempfile::tempdir;

    #[test]
    fn test_write_read_file() {
        let dir = tempdir().unwrap();
        let path = dir.path().join("test.txt");

        write_file_utf8(&path, "hello world").unwrap();
        let content = read_file_utf8(&path).unwrap();
        assert_eq!(content, "hello world");
    }

    #[test]
    fn test_read_nonexistent_file() {
        let result = read_file_utf8(Path::new("/nonexistent/file.txt"));
        assert!(result.unwrap().is_empty());
    }

    #[test]
    fn test_write_file_atomic() {
        let dir = tempdir().unwrap();
        let path = dir.path().join("atomic.txt");

        write_file_atomic(&path, "atomic content").unwrap();
        let content = read_file_utf8(&path).unwrap();
        assert_eq!(content, "atomic content");
    }

    #[test]
    fn test_path_exists() {
        assert!(path_exists(Path::new("/")));
        assert!(!path_exists(Path::new("/nonexistent_xyz")));
    }
}
