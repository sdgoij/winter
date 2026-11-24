//! The trace a bug report can carry.
//!
//! A terminal hangs inside a callback: there is no panic, and a GUI-subsystem
//! process has no console to print to, so a hang leaves nothing behind. With
//! `WINTER_TRACE` set (to a path, or to anything for
//! `%TEMP%\winter-trace.log`) the shell writes what crossed the pty and what the
//! pane did with it. It is off unless asked for, and the lines are short.
//!
//! The questions it exists to answer: did the pty deliver anything after the
//! last thing on screen, did the pane still paint, and did the grid change.

use std::fmt::{Arguments, Write as _};
use std::fs::File;
use std::io::Write as _;
use std::path::PathBuf;
use std::sync::{Mutex, OnceLock};

/// The open trace file, or `None` when tracing is off.
fn file() -> Option<&'static Mutex<File>> {
    static FILE: OnceLock<Option<Mutex<File>>> = OnceLock::new();
    FILE.get_or_init(|| {
        let requested = std::env::var_os("WINTER_TRACE")?;
        let path = PathBuf::from(&requested);
        let path = if path.as_os_str().len() > 1 {
            path
        } else {
            std::env::temp_dir().join("winter-trace.log")
        };
        let file = std::fs::OpenOptions::new().create(true).append(true).open(&path).ok()?;
        let _ = writeln!(&file, "--- winter trace: {}", path.display());
        Some(Mutex::new(file))
    })
    .as_ref()
}

/// Writes one line, when tracing is on.
pub fn line(arguments: Arguments<'_>) {
    let Some(file) = file() else {
        return;
    };
    if let Ok(mut file) = file.lock() {
        let _ = writeln!(file, "{arguments}");
    }
}

/// Writes `label` and `data` with everything that is not printable escaped.
pub fn bytes(label: &str, data: &[u8]) {
    if file().is_none() {
        return;
    }
    line(format_args!("{label} {}", escaped(data)));
}

/// Writes `label` and `text` with everything that is not printable escaped.
pub fn text(label: &str, text: &str) {
    bytes(label, text.as_bytes());
}

/// `data` with its escape sequences and line ends visible.
///
/// Control bytes become their usual escapes and everything else outside
/// printable ASCII becomes `\xNN`, so one line of the log is one line of the
/// wire.
fn escaped(data: &[u8]) -> String {
    let mut out = String::with_capacity(data.len());
    for &byte in data {
        match byte {
            b'\r' => out.push_str("\\r"),
            b'\n' => out.push_str("\\n"),
            b'\t' => out.push_str("\\t"),
            b'\x1b' => out.push_str("\\e"),
            0x20..=0x7e => out.push(char::from(byte)),
            _ => {
                let _ = write!(out, "\\x{byte:02x}");
            }
        }
    }
    out
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn escape_sequences_and_line_ends_stay_on_one_line() {
        assert_eq!(escaped(b"ab\r\n"), "ab\\r\\n");
        assert_eq!(escaped(b"\x1b[?9001h"), "\\e[?9001h");
        assert_eq!(escaped(&[0x00, 0xff]), "\\x00\\xff");
        assert_eq!(escaped(b"~"), "~");
    }
}
