//! A shell on a pseudo terminal, and the bytes it emits.
//!
//! `server` owns the pty and `term` owns the parser; this module is the
//! composition that gives the pty layer its consumer (ADR 017). It is
//! toolkit-free: the window layer asks for bytes and hands bytes back.
//!
//! Reading blocks, so it happens on one thread that forwards chunks into a
//! channel. The UI thread never blocks; it drains the channel once per frame.

use std::io::{self, Read, Write};
use std::process::ExitStatus;
use std::sync::mpsc::{self, Receiver, TryRecvError};
use std::thread;

use server::pty::{Pty, PtyCommand, PtySize};

/// The output stream of a session.
struct Output {
    chunks: Receiver<Vec<u8>>,
    ended: bool,
}

impl Output {
    /// Appends everything received since the last call.
    ///
    /// Returns whether the stream has ended. Once it has, later calls keep
    /// returning `true` and append nothing.
    fn drain_into(&mut self, out: &mut Vec<u8>) -> bool {
        loop {
            match self.chunks.try_recv() {
                Ok(chunk) => out.extend_from_slice(&chunk),
                Err(TryRecvError::Empty) => return self.ended,
                Err(TryRecvError::Disconnected) => {
                    self.ended = true;
                    return true;
                }
            }
        }
    }
}

/// A shell attached to a pseudo terminal.
pub struct Session {
    pty: Pty,
    output: Output,
}

impl Session {
    /// Spawns `command` at `size` and starts forwarding its output.
    pub fn spawn(command: &PtyCommand, size: PtySize) -> io::Result<Self> {
        let pty = Pty::spawn(command, size)?;
        let reader = pty.try_clone_reader()?;
        Ok(Self {
            pty,
            output: Output {
                chunks: spawn_pump(reader),
                ended: false,
            },
        })
    }

    /// Appends everything the client wrote since the last call.
    ///
    /// Returns whether the output has ended.
    pub fn drain_into(&mut self, out: &mut Vec<u8>) -> bool {
        self.output.drain_into(out)
    }

    /// Writes bytes to the client, as if the user had typed them.
    pub fn write(&mut self, bytes: &[u8]) -> io::Result<()> {
        self.pty.write_all(bytes)
    }

    /// Resizes the terminal the client sees.
    pub fn resize(&mut self, size: PtySize) -> io::Result<()> {
        self.pty.resize(size)
    }

    /// Waits for the client to exit.
    ///
    /// On Windows the pseudoconsole keeps the output pipe open after the
    /// client exits, so [`close`](Self::close) must run before the stream
    /// ends.
    pub fn wait(&mut self) -> io::Result<ExitStatus> {
        self.pty.wait()
    }

    /// Ends the terminal and releases the output pipe.
    pub fn close(&mut self) {
        self.pty.close();
    }

    /// Terminates the client.
    pub fn kill(&mut self) -> io::Result<()> {
        self.pty.kill()
    }
}

/// Forwards everything `reader` yields into a channel, then closes it.
///
/// The thread stops at end of input, on an error, or when the receiver is
/// dropped. Interrupted reads are retried, as a signal delivering no data is
/// not the end of the stream.
pub fn spawn_pump<R>(mut reader: R) -> Receiver<Vec<u8>>
where
    R: Read + Send + 'static,
{
    let (sender, receiver) = mpsc::channel();
    thread::spawn(move || {
        let mut buffer = [0u8; 8192];
        loop {
            match reader.read(&mut buffer) {
                Ok(0) => return,
                Ok(count) => {
                    if sender.send(buffer[..count].to_vec()).is_err() {
                        return;
                    }
                }
                Err(error) if error.kind() == io::ErrorKind::Interrupted => continue,
                Err(_) => return,
            }
        }
    });
    receiver
}

#[cfg(test)]
mod tests {
    use super::*;
    use std::io::Cursor;

    #[test]
    fn a_drain_appends_queued_chunks_and_reports_the_end() {
        let (sender, receiver) = mpsc::channel();
        sender.send(b"one".to_vec()).expect("send");
        sender.send(b"two".to_vec()).expect("send");
        drop(sender);

        let mut output = Output {
            chunks: receiver,
            ended: false,
        };
        let mut drained = Vec::new();
        assert!(output.drain_into(&mut drained));
        assert_eq!(drained, b"onetwo");

        // The end is sticky and does not repeat bytes.
        assert!(output.drain_into(&mut drained));
        assert_eq!(drained, b"onetwo");
    }

    #[test]
    fn a_drain_with_an_open_live_stream_reports_no_end() {
        let (sender, receiver) = mpsc::channel();
        sender.send(b"hi".to_vec()).expect("send");

        let mut output = Output {
            chunks: receiver,
            ended: false,
        };
        let mut drained = Vec::new();
        assert!(!output.drain_into(&mut drained));
        assert_eq!(drained, b"hi");
        drop(sender);
    }

    #[test]
    fn the_pump_forwards_every_byte_and_then_closes() {
        let chunks = spawn_pump(Cursor::new(b"one two".to_vec()));
        let mut received = Vec::new();
        while let Ok(chunk) = chunks.recv() {
            received.extend_from_slice(&chunk);
        }
        assert_eq!(received, b"one two");
    }

    #[test]
    fn a_read_error_ends_the_stream_and_keeps_what_came_before() {
        struct HalfBroken {
            sent: bool,
        }

        impl Read for HalfBroken {
            fn read(&mut self, buffer: &mut [u8]) -> io::Result<usize> {
                if self.sent {
                    return Err(io::Error::other("broken"));
                }
                self.sent = true;
                buffer[..4].copy_from_slice(b"kept");
                Ok(4)
            }
        }

        let chunks = spawn_pump(HalfBroken { sent: false });
        let mut received = Vec::new();
        while let Ok(chunk) = chunks.recv() {
            received.extend_from_slice(&chunk);
        }
        assert_eq!(received, b"kept");
    }

    #[cfg(windows)]
    fn shell() -> std::ffi::OsString {
        std::env::var_os("COMSPEC").unwrap_or_else(|| "cmd.exe".into())
    }

    #[cfg(unix)]
    fn shell() -> std::ffi::OsString {
        "/bin/sh".into()
    }

    #[test]
    fn a_session_runs_the_client_and_forwards_its_output() {
        #[cfg(windows)]
        let command = PtyCommand::new(shell()).args(["/d", "/c", "echo session-ok"]);
        #[cfg(unix)]
        let command = PtyCommand::new(shell()).args(["-c", "printf session-ok"]);

        let mut session = Session::spawn(&command, PtySize::default()).expect("spawn");
        let status = session.wait().expect("wait");
        session.close();

        let mut output = Vec::new();
        while let Ok(chunk) = session.output.chunks.recv() {
            output.extend_from_slice(&chunk);
        }
        let text = String::from_utf8_lossy(&output);

        assert!(status.success());
        assert!(text.contains("session-ok"), "output was {text:?}");
    }
}
