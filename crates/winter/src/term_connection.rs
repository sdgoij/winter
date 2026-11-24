//! A terminal's backend, ported from `src/cascadia/TerminalConnection/`.
//!
//! Upstream's `ITerminalConnection` is all a control knows about its shell: it
//! takes input, reports output and a state, and can be closed. Upstream has
//! several implementations (ConPTY, Azure, a local echo); this port has the pty
//! one, and the trait is the seam a second backend needs (ADR 036).
//!
//! The control never names `Session`: a test can hand it a connection that is a
//! byte buffer, which is how the control's own logic is tested without a pty.

use std::io;

use server::pty::PtySize;

use crate::session::Session;

/// A terminal's backend.
pub trait Connection {
    /// Appends everything the backend produced since the last call.
    ///
    /// Returns whether the stream has ended. Once it has, later calls keep
    /// returning `true` and append nothing.
    fn drain_into(&mut self, out: &mut Vec<u8>) -> bool;

    /// Sends input to the backend, as if the user had typed it.
    fn write(&mut self, bytes: &[u8]) -> io::Result<()>;

    /// Tells the backend how large the terminal it drives is.
    fn resize(&mut self, size: PtySize) -> io::Result<()>;

    /// Whether the shell behind the connection has exited.
    ///
    /// The output stream is not always enough: a ConPTY keeps its pipe open
    /// after the client exits, so the backend asks its process instead.
    fn exited(&mut self) -> bool;

    /// Ends the connection and releases what it holds.
    fn close(&mut self);
}

impl Connection for Session {
    fn drain_into(&mut self, out: &mut Vec<u8>) -> bool {
        Session::drain_into(self, out)
    }

    fn write(&mut self, bytes: &[u8]) -> io::Result<()> {
        Session::write(self, bytes)
    }

    fn resize(&mut self, size: PtySize) -> io::Result<()> {
        Session::resize(self, size)
    }

    fn exited(&mut self) -> bool {
        Session::exited(self)
    }

    fn close(&mut self) {
        Session::close(self);
    }
}

/// A connection with no pty behind it, for the tests of this crate and of the
/// shells built on it.
///
/// It is not gated on `test` because a shell's tests are a different crate's:
/// the double has to be part of the library to be reachable from them.
pub mod tests {
    use std::sync::{Arc, Mutex};

    use super::*;

    /// What a [`Buffer`] connection has seen, shared with the test that built it.
    #[derive(Debug, Default)]
    pub struct Seen {
        /// What the backend produces on the next drain.
        pub output: Vec<u8>,
        /// Whether the stream has ended.
        pub ended: bool,
        /// Everything the control sent.
        pub written: Vec<u8>,
        /// The sizes the control asked for.
        pub resized: Vec<PtySize>,
    }

    /// A connection with no pty behind it, for the control's tests.
    ///
    /// Cloning shares the state, so a test keeps a handle after the control has
    /// taken the box.
    #[derive(Debug, Clone, Default)]
    pub struct Buffer {
        seen: Arc<Mutex<Seen>>,
    }

    impl Buffer {
        /// A connection that will produce `output` on its first drain.
        pub fn with_output(output: &[u8]) -> Self {
            let buffer = Self::default();
            buffer.seen.lock().expect("lock").output = output.to_vec();
            buffer
        }

        /// A connection whose stream has already ended.
        pub fn ended() -> Self {
            let buffer = Self::default();
            buffer.seen.lock().expect("lock").ended = true;
            buffer
        }

        /// Everything the control sent so far.
        pub fn written(&self) -> Vec<u8> {
            self.seen.lock().expect("lock").written.clone()
        }

        /// The sizes the control asked for.
        pub fn resized(&self) -> Vec<PtySize> {
            self.seen.lock().expect("lock").resized.clone()
        }

        /// Queues output the backend will produce on its next drain.
        pub fn send(&self, output: &[u8]) {
            self.seen.lock().expect("lock").output.extend_from_slice(output);
        }
    }

    impl Connection for Buffer {
        fn drain_into(&mut self, out: &mut Vec<u8>) -> bool {
            let mut seen = self.seen.lock().expect("lock");
            out.extend_from_slice(&std::mem::take(&mut seen.output));
            seen.ended
        }

        fn write(&mut self, bytes: &[u8]) -> io::Result<()> {
            self.seen.lock().expect("lock").written.extend_from_slice(bytes);
            Ok(())
        }

        fn resize(&mut self, size: PtySize) -> io::Result<()> {
            self.seen.lock().expect("lock").resized.push(size);
            Ok(())
        }

        fn exited(&mut self) -> bool {
            self.seen.lock().expect("lock").ended
        }

        fn close(&mut self) {}
    }

    #[test]
    fn a_connection_is_object_safe() {
        let buffer = Buffer::with_output(b"hi");
        let mut connection: Box<dyn Connection> = Box::new(buffer.clone());

        connection.write(b"typed").expect("write");
        connection.resize(PtySize::DEFAULT).expect("resize");
        let mut out = Vec::new();
        assert!(!connection.drain_into(&mut out));
        connection.close();

        assert_eq!(out, b"hi");
        assert_eq!(buffer.written(), b"typed");
        assert_eq!(buffer.resized(), [PtySize::DEFAULT]);
    }
}
