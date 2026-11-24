//! POSIX pty — the Unix pseudo terminal.
//!
//! This is the port's own backend (ADR 016); upstream has no pty. The master
//! side is a file descriptor; the slave side is opened for the client's
//! standard streams, and the child becomes a session leader on it so it has a
//! controlling terminal.
//!
//! `unsafe` is confined to this module by the allow on its declaration in
//! `lib.rs`.

use std::ffi::{CStr, OsStr};
use std::fs::{File, OpenOptions};
use std::io::{self, Read, Write};
use std::os::unix::ffi::OsStrExt;
use std::os::unix::io::{AsRawFd, FromRawFd};
use std::os::unix::process::CommandExt;
use std::path::PathBuf;
use std::process::{Child, Command, ExitStatus, Stdio};
use std::sync::Mutex;

use crate::device_comm::DeviceComm;
use crate::pty::{PtyCommand, PtySize};

/// A running client attached to a pty.
pub struct Pty {
    /// The master side; reads are the client's output, writes its input.
    master: File,
    /// The client process.
    child: Child,
    /// Whether the client has been waited for.
    reaped: bool,
}

/// Serialises opening a master and naming its slave.
///
/// `ptsname` answers from a buffer shared by every thread in the process, so
/// two spawns at the same moment can read each other's terminal name. Both
/// children then claim one pty, and the second `TIOCSCTTY` fails with `EPERM`:
/// CI hit exactly that as a flake in `the_exit_status_is_reported`. The lock
/// covers only the naming; the child starts outside it.
static NAMING: Mutex<()> = Mutex::new(());

impl Pty {
    /// Starts `command` on a new pty of `size`.
    pub fn spawn(command: &PtyCommand, size: PtySize) -> io::Result<Self> {
        let naming = NAMING.lock().unwrap_or_else(|error| error.into_inner());

        // SAFETY: opens a new master descriptor that we own immediately.
        let master_fd = unsafe { libc::posix_openpt(libc::O_RDWR | libc::O_NOCTTY) };
        if master_fd < 0 {
            return Err(io::Error::last_os_error());
        }
        // SAFETY: the descriptor is fresh and owned by us.
        let master = unsafe { File::from_raw_fd(master_fd) };

        // SAFETY: the descriptor is a live pty master.
        if unsafe { libc::grantpt(master_fd) } == -1 {
            return Err(io::Error::last_os_error());
        }
        // SAFETY: as above.
        if unsafe { libc::unlockpt(master_fd) } == -1 {
            return Err(io::Error::last_os_error());
        }

        // `ptsname` writes to a static buffer; the path is copied out before
        // any other call can touch it.
        // SAFETY: the call returns a valid NUL-terminated string for a live
        // master, or null.
        let name = unsafe { libc::ptsname(master_fd) };
        if name.is_null() {
            return Err(io::Error::last_os_error());
        }
        // SAFETY: the pointer was just checked and is NUL-terminated.
        let slave_path = PathBuf::from(OsStr::from_bytes(unsafe { CStr::from_ptr(name) }.to_bytes()));
        drop(naming);

        let slave = OpenOptions::new().read(true).write(true).open(&slave_path)?;
        let slave_fd = slave.as_raw_fd();

        let mut child_command = Command::new(command.program());
        child_command.args(command.arguments());
        child_command.stdin(Stdio::from(slave.try_clone()?));
        child_command.stdout(Stdio::from(slave.try_clone()?));
        child_command.stderr(Stdio::from(slave));

        // SAFETY: the closure only calls async-signal-safe functions between
        // fork and exec.
        unsafe {
            child_command.pre_exec(move || {
                // A new session makes the pty the controlling terminal.
                if libc::setsid() == -1 {
                    return Err(io::Error::last_os_error());
                }

                // The slave descriptor may already have been replaced by fd 0
                // by the time this runs, so try both.
                let request = libc::TIOCSCTTY as libc::c_ulong;
                if libc::ioctl(slave_fd, request, 0) == -1 && libc::ioctl(0, request, 0) == -1 {
                    return Err(io::Error::last_os_error());
                }
                Ok(())
            });
        }

        let child = child_command.spawn()?;
        let mut pty = Self {
            master,
            child,
            reaped: false,
        };
        pty.resize(size)?;
        Ok(pty)
    }

    /// Returns an independent handle for reading the client's output.
    ///
    /// A terminal reads on its own thread while it waits for the client, so
    /// the master descriptor can be duplicated and moved out of the `Pty`.
    pub fn try_clone_reader(&self) -> io::Result<PtyReader> {
        Ok(PtyReader(self.master.try_clone()?))
    }

    /// Closes the read side.
    ///
    /// On Unix a cloned reader ends on its own when the client exits, so this
    /// only drops the `Pty`'s own descriptor.
    pub fn close(&mut self) {
        // Nothing to do: the client's exit closes the slave side.
    }

    /// Resizes the pty.
    pub fn resize(&mut self, size: PtySize) -> io::Result<()> {
        let window = libc::winsize {
            ws_row: size.rows,
            ws_col: size.cols,
            ws_xpixel: 0,
            ws_ypixel: 0,
        };

        // SAFETY: the descriptor is live and the window is a valid winsize.
        let result = unsafe { libc::ioctl(self.master.as_raw_fd(), libc::TIOCSWINSZ as libc::c_ulong, &window) };
        if result == -1 {
            Err(io::Error::last_os_error())
        } else {
            Ok(())
        }
    }

    /// Waits for the client to exit and returns its status.
    pub fn wait(&mut self) -> io::Result<ExitStatus> {
        self.reaped = true;
        self.child.wait()
    }

    /// Terminates the client.
    pub fn kill(&mut self) -> io::Result<()> {
        self.child.kill()?;
        let _ = self.wait();
        Ok(())
    }

    /// Returns the client's exit status once it has exited, without blocking.
    pub fn try_wait(&mut self) -> io::Result<Option<ExitStatus>> {
        let status = self.child.try_wait()?;
        if status.is_some() {
            self.reaped = true;
        }
        Ok(status)
    }

    /// Returns the client's process id.
    pub fn process_id(&self) -> u32 {
        self.child.id()
    }
}

impl Read for Pty {
    fn read(&mut self, buf: &mut [u8]) -> io::Result<usize> {
        read_master(&mut self.master, buf)
    }
}

/// The read half of a [`Pty`], safe to move to another thread.
pub struct PtyReader(File);

impl Read for PtyReader {
    fn read(&mut self, buf: &mut [u8]) -> io::Result<usize> {
        read_master(&mut self.0, buf)
    }
}

/// Reads the master side, treating `EIO` as the pty's end of file.
fn read_master(master: &mut File, buf: &mut [u8]) -> io::Result<usize> {
    match master.read(buf) {
        // Once every slave descriptor is closed the master reports EIO; that is
        // this pty's end of file.
        Err(error) if error.raw_os_error() == Some(libc::EIO) => Ok(0),
        other => other,
    }
}

impl Write for Pty {
    fn write(&mut self, buf: &[u8]) -> io::Result<usize> {
        self.master.write(buf)
    }

    fn flush(&mut self) -> io::Result<()> {
        self.master.flush()
    }
}

impl DeviceComm for Pty {
    fn resize(&mut self, size: PtySize) -> io::Result<()> {
        Self::resize(self, size)
    }
}

impl Drop for Pty {
    fn drop(&mut self) {
        if !self.reaped {
            let _ = self.child.kill();
            let _ = self.child.wait();
        }
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    fn shell() -> &'static str {
        "/bin/sh"
    }

    fn read_to_string(pty: &mut Pty) -> String {
        let mut output = Vec::new();
        pty.read_to_end(&mut output).expect("read");
        String::from_utf8_lossy(&output).into_owned()
    }

    #[test]
    fn a_command_writes_to_its_output() {
        let mut pty = Pty::spawn(
            &PtyCommand::new(shell()).args(["-c", "printf hello"]),
            PtySize::default(),
        )
        .expect("spawn");

        let text = read_to_string(&mut pty);
        assert!(text.contains("hello"), "output was {text:?}");
        assert!(pty.wait().expect("wait").success());
    }

    #[test]
    fn the_window_size_is_visible_to_the_client() {
        let mut pty = Pty::spawn(
            &PtyCommand::new(shell()).args(["-c", "stty size"]),
            PtySize::new(80, 24),
        )
        .expect("spawn");

        let text = read_to_string(&mut pty);
        assert!(text.contains("24 80"), "output was {text:?}");
        assert!(pty.wait().expect("wait").success());
    }

    #[test]
    fn the_exit_status_is_reported() {
        let mut pty = Pty::spawn(&PtyCommand::new(shell()).args(["-c", "exit 3"]), PtySize::default()).expect("spawn");

        let _ = read_to_string(&mut pty);
        assert_eq!(pty.wait().expect("wait").code(), Some(3));
    }

    #[test]
    fn concurrent_spawns_each_get_their_own_terminal() {
        // CI hit `spawn` failing with EPERM while its sibling tests passed.
        // `ptsname` answers from a buffer shared by the whole process, so two
        // spawns at the same moment can name the same slave, and the second
        // child's `TIOCSCTTY` then fails because another session owns it.
        //
        // Each pty is held until every name is in. A thread that drops its own
        // gives the number back, and the next spawn is handed `/dev/pts/1`
        // again — three of the eight did on the arm runner — which is reuse,
        // not two clients on one terminal.
        let threads: Vec<_> = (0..8)
            .map(|_| {
                std::thread::spawn(|| {
                    let mut pty =
                        Pty::spawn(&PtyCommand::new(shell()).args(["-c", "tty"]), PtySize::default()).expect("spawn");
                    let name = read_to_string(&mut pty).trim().to_owned();
                    (pty, name)
                })
            })
            .collect();

        let spawned: Vec<(Pty, String)> = threads.into_iter().map(|handle| handle.join().expect("join")).collect();
        let names: Vec<String> = spawned.iter().map(|(_, name)| name.clone()).collect();
        let unique: std::collections::HashSet<&String> = names.iter().collect();
        assert_eq!(unique.len(), names.len(), "terminals: {names:?}");
    }

    #[test]
    fn input_reaches_the_client() {
        let mut pty = Pty::spawn(
            &PtyCommand::new(shell()).args(["-c", "read x; printf 'got:%s' \"$x\""]),
            PtySize::default(),
        )
        .expect("spawn");

        pty.write_all(b"abc\n").expect("write");
        let text = read_to_string(&mut pty);
        assert!(text.contains("got:abc"), "output was {text:?}");
        assert!(pty.wait().expect("wait").success());
    }

    #[test]
    fn resize_is_accepted_while_the_client_runs() {
        let mut pty = Pty::spawn(&PtyCommand::new(shell()).args(["-c", "read x"]), PtySize::default()).expect("spawn");

        pty.resize(PtySize::new(100, 30)).expect("resize");
        pty.write_all(b"\n").expect("write");
        let _ = read_to_string(&mut pty);
        pty.wait().expect("wait");
    }
}
