//! The `winter-frame` binary: a shell on a pty, and its pixels on stdout.
//!
//! An editor starts this, writes commands to its stdin and reads frames from its
//! stdout. The protocol is [`winter_demo::parse`] and [`winter_demo::Frames`];
//! the terminal itself is `winter` (ADR 083).
//!
//! The shell is PowerShell on Windows — 7 before 5.1 — and `$SHELL` on macOS and
//! Linux, falling back to `/bin/sh`. `--shell <program>` names one instead.

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
    let program = match requested_shell(std::env::args().skip(1))? {
        Some(program) => program,
        None => default_shell(),
    };

    let setup = winter_demo::measure(FONT_SIZE, COLUMNS, ROWS)
        .ok_or_else(|| io::Error::other("no monospace face was found in the host's font directories"))?;

    let size = PtySize::new(COLUMNS as u16, ROWS as u16);
    let session = Session::spawn(&PtyCommand::new(&program), size)?;
    let mut surface = Surface::new(setup, Box::new(session));

    let stdin = io::stdin();
    let stdout = io::stdout();
    let mut output = BufWriter::new(stdout.lock());
    winter_demo::serve(stdin.lock(), &mut output, &mut surface)?;
    output.flush()
}

/// Reads `--shell <program>`, or `None` when the host named none.
fn requested_shell(mut args: impl Iterator<Item = String>) -> io::Result<Option<PathBuf>> {
    match args.next().as_deref() {
        None => Ok(None),
        Some("--shell") => {
            let program = args.next().ok_or_else(|| io::Error::other("--shell needs a program"))?;
            Ok(Some(PathBuf::from(program)))
        }
        Some(other) => Err(io::Error::other(format!("unknown argument: {other}"))),
    }
}

/// The shell this platform starts when the host names none.
fn default_shell() -> PathBuf {
    #[cfg(windows)]
    {
        // PowerShell 7 first: it is the current one, and a host that installed it
        // expects it. Then Windows PowerShell, whose directory a locked-down
        // `PATH` may omit. Then whatever `COMSPEC` names.
        if let Some(program) = first_found(&["pwsh.exe", "powershell.exe"], on_path) {
            return program;
        }
        if let Some(program) = windows_powershell() {
            return program;
        }
        if let Some(program) = std::env::var_os("COMSPEC") {
            return PathBuf::from(program);
        }
        PathBuf::from("cmd.exe")
    }

    #[cfg(not(windows))]
    {
        std::env::var_os("SHELL")
            .map(PathBuf::from)
            .unwrap_or_else(|| PathBuf::from("/bin/sh"))
    }
}

/// The first of `names` that `find` resolves, in order.
fn first_found(names: &[&str], find: impl Fn(&str) -> Option<PathBuf>) -> Option<PathBuf> {
    names.iter().find_map(|name| find(name))
}

/// A program on `PATH`, if the name resolves to a file there.
#[cfg(windows)]
fn on_path(name: &str) -> Option<PathBuf> {
    let path = std::env::var_os("PATH")?;
    std::env::split_paths(&path)
        .map(|directory| directory.join(name))
        .find(|candidate| candidate.is_file())
}

/// Windows PowerShell's own directory, which a host's `PATH` may not carry.
#[cfg(windows)]
fn windows_powershell() -> Option<PathBuf> {
    let root = std::env::var_os("SystemRoot").or_else(|| std::env::var_os("WINDIR"))?;
    let candidate = PathBuf::from(root)
        .join("System32")
        .join("WindowsPowerShell")
        .join("v1.0")
        .join("powershell.exe");
    candidate.is_file().then_some(candidate)
}

#[cfg(test)]
mod tests {
    use std::path::Path;

    use super::*;

    #[test]
    fn the_first_name_that_resolves_wins() {
        let found = first_found(&["pwsh.exe", "powershell.exe"], |name| {
            (name == "powershell.exe").then(|| PathBuf::from(name))
        });
        assert_eq!(found.as_deref(), Some(Path::new("powershell.exe")));
    }

    #[test]
    fn no_candidate_resolving_is_no_shell() {
        assert_eq!(first_found(&["pwsh.exe", "powershell.exe"], |_| None), None);
    }

    #[test]
    fn a_shell_argument_names_the_program() {
        let args = vec!["--shell".to_string(), "pwsh.exe".to_string()];
        let program = requested_shell(args.into_iter()).expect("a shell");
        assert_eq!(program.as_deref(), Some(Path::new("pwsh.exe")));
    }

    #[test]
    fn no_argument_means_the_platform_default() {
        assert_eq!(requested_shell(std::iter::empty::<String>()).expect("none"), None);
    }

    #[test]
    fn a_shell_without_a_program_or_an_unknown_argument_is_refused() {
        assert!(requested_shell(vec!["--shell".to_string()].into_iter()).is_err());
        assert!(requested_shell(vec!["--nope".to_string()].into_iter()).is_err());
    }
}
