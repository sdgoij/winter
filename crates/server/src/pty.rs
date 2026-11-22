//! Pseudo terminal — spawn a client and talk to it through a pty.
//!
//! This is the port's own layer (ADR 016): upstream has no pty because conhost
//! *is* the console. The shared types live here; the two backends are
//! `conpty` (Windows) and `posix_pty` (Unix), and each supplies a `Pty` with
//! the same API.
//!
//! The API is blocking. An event loop splits the handle in the application
//! layer rather than here.

use std::ffi::{OsStr, OsString};

pub use crate::device_comm::DeviceComm;

#[cfg(windows)]
pub use crate::conpty::{Pty, PtyReader};
#[cfg(unix)]
pub use crate::posix_pty::{Pty, PtyReader};

/// The size of a pseudo terminal, in character cells.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub struct PtySize {
    /// The width in columns.
    pub cols: u16,
    /// The height in rows.
    pub rows: u16,
}

impl PtySize {
    /// The default terminal size.
    pub const DEFAULT: Self = Self { cols: 80, rows: 24 };

    /// Creates a size.
    pub const fn new(cols: u16, rows: u16) -> Self {
        Self { cols, rows }
    }
}

impl Default for PtySize {
    fn default() -> Self {
        Self::DEFAULT
    }
}

/// The program and arguments to run in the pseudo terminal.
///
/// Environment and working directory are inherited from this process, which is
/// what a terminal wants when it starts a shell. They become fields here if a
/// caller needs to change them.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct PtyCommand {
    program: OsString,
    args: Vec<OsString>,
}

impl PtyCommand {
    /// Creates a command for `program`.
    pub fn new(program: impl Into<OsString>) -> Self {
        Self {
            program: program.into(),
            args: Vec::new(),
        }
    }

    /// Adds one argument.
    pub fn arg(mut self, arg: impl Into<OsString>) -> Self {
        self.args.push(arg.into());
        self
    }

    /// Adds several arguments.
    pub fn args<I, S>(mut self, args: I) -> Self
    where
        I: IntoIterator<Item = S>,
        S: Into<OsString>,
    {
        self.args.extend(args.into_iter().map(Into::into));
        self
    }

    /// Returns the program.
    pub fn program(&self) -> &OsStr {
        &self.program
    }

    /// Returns the arguments.
    pub fn arguments(&self) -> impl Iterator<Item = &OsStr> {
        self.args.iter().map(OsString::as_os_str)
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn a_command_keeps_its_program_and_arguments() {
        let command = PtyCommand::new("/bin/sh").args(["-c", "echo hi"]);
        assert_eq!(command.program(), OsStr::new("/bin/sh"));
        assert_eq!(
            command.arguments().collect::<Vec<_>>(),
            vec![OsStr::new("-c"), OsStr::new("echo hi")]
        );
    }

    #[test]
    fn the_default_size_is_eighty_by_twenty_four() {
        assert_eq!(PtySize::default(), PtySize::new(80, 24));
    }
}
