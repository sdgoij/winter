//! The `winter-frame` binary: a shell on a pty, and its pixels on stdout.
//!
//! An editor starts this, writes commands to its stdin and reads frames from its
//! stdout. The protocol is [`winter_demo::parse`] and
//! [`winter_demo::write_frame`]; the terminal itself is `winter` (ADR 083).

use std::io::{self, BufWriter, Write};
use std::path::PathBuf;

use server::pty::{PtyCommand, PtySize};
use winter::embed::Surface;
use winter::session::Session;

/// The grid the frame starts at; a host resizes it with a command.
const COLUMNS: i32 = 100;
/// The grid the frame starts at.
const ROWS: i32 = 30;
/// The point size the face is measured at.
const FONT_SIZE: f32 = 14.0;

fn main() -> io::Result<()> {
    let setup = winter_demo::measure(FONT_SIZE, COLUMNS, ROWS)
        .ok_or_else(|| io::Error::other("no monospace face was found in the host's font directories"))?;

    let size = PtySize::new(COLUMNS as u16, ROWS as u16);
    let session = Session::spawn(&PtyCommand::new(shell()), size)?;
    let mut surface = Surface::new(setup, Box::new(session));

    let stdin = io::stdin();
    let stdout = io::stdout();
    let mut output = BufWriter::new(stdout.lock());
    winter_demo::serve(stdin.lock(), &mut output, &mut surface)?;
    output.flush()
}

/// The shell a demo starts: `COMSPEC` on Windows, the user's shell elsewhere.
fn shell() -> PathBuf {
    #[cfg(windows)]
    if let Some(shell) = std::env::var_os("COMSPEC") {
        return PathBuf::from(shell);
    }

    #[cfg(unix)]
    if let Some(shell) = std::env::var_os("SHELL") {
        return PathBuf::from(shell);
    }

    if cfg!(windows) {
        PathBuf::from("cmd.exe")
    } else {
        PathBuf::from("/bin/sh")
    }
}
