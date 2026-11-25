//! A terminal's backend, ported from `src/cascadia/TerminalConnection/`.
//!
//! Upstream's `ITerminalConnection` is all a control knows about its shell: it
//! takes input, reports output and a state, and can be closed. Upstream has
//! several implementations (ConPTY, Azure, a local echo); this port has the pty
//! one in [`Session`] and [`Queue`] for a host that owns the bytes itself
//! (ADR 082), and the trait is the seam a second backend needs (ADR 036).
//!
//! The control never names `Session`: a test can hand it a connection that is a
//! byte buffer, which is how the control's own logic is tested without a pty.

use std::io;
use std::sync::{Arc, Mutex};

#[cfg(feature = "pty")]
use server::pty::PtySize;

#[cfg(feature = "pty")]
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
    ///
    /// The size is in cells, not a `server::PtySize`: a host that owns no pty —
    /// an embedding, a test — has no window geometry to speak of, and naming
    /// the pty layer here would put it in every such host's build (ADR 082).
    fn resize(&mut self, columns: i32, rows: i32) -> io::Result<()>;

    /// Whether the shell behind the connection has exited.
    ///
    /// The output stream is not always enough: a ConPTY keeps its pipe open
    /// after the client exits, so the backend asks its process instead.
    fn exited(&mut self) -> bool;

    /// Ends the connection and releases what it holds.
    fn close(&mut self);
}

#[cfg(feature = "pty")]
impl Connection for Session {
    fn drain_into(&mut self, out: &mut Vec<u8>) -> bool {
        Session::drain_into(self, out)
    }

    fn write(&mut self, bytes: &[u8]) -> io::Result<()> {
        Session::write(self, bytes)
    }

    fn resize(&mut self, columns: i32, rows: i32) -> io::Result<()> {
        Session::resize(self, PtySize::new(cell_count(columns), cell_count(rows)))
    }

    fn exited(&mut self) -> bool {
        Session::exited(self)
    }

    fn close(&mut self) {
        Session::close(self);
    }
}

/// What a [`Queue`] has seen, shared with the host that owns it.
#[derive(Debug, Default)]
struct Seen {
    /// What the backend produces on the next drain.
    output: Vec<u8>,
    /// Whether the stream has ended.
    ended: bool,
    /// Everything the terminal sent.
    written: Vec<u8>,
    /// The sizes the terminal asked for, in cells.
    resized: Vec<(i32, i32)>,
}

/// A connection with no pty behind it.
///
/// The host feeds what the program would have written and takes back what the
/// terminal sent in answer; nothing spawns, nothing blocks, and the bytes never
/// leave the host's control. That is what a shell's tests need, and it is also
/// the seam an embedding uses when the host owns the process — a wasm canvas, an
/// editor panel, an agent's tool (ADR 082).
///
/// Cloning shares the state, so the host keeps a handle after the control has
/// taken the box.
#[derive(Debug, Clone, Default)]
pub struct Queue {
    seen: Arc<Mutex<Seen>>,
}

impl Queue {
    /// A connection that will produce `output` on its first drain.
    pub fn with_output(output: &[u8]) -> Self {
        let queue = Self::default();
        queue.seen.lock().expect("lock").output = output.to_vec();
        queue
    }

    /// A connection whose stream has already ended.
    pub fn ended() -> Self {
        let queue = Self::default();
        queue.seen.lock().expect("lock").ended = true;
        queue
    }

    /// Queues `output` for the next drain, as the program writing it would.
    pub fn feed(&self, output: &[u8]) {
        self.seen.lock().expect("lock").output.extend_from_slice(output);
    }

    /// Ends the stream, as closing the program's side would.
    pub fn end(&self) {
        self.seen.lock().expect("lock").ended = true;
    }

    /// Everything the terminal sent so far, without taking it.
    pub fn written(&self) -> Vec<u8> {
        self.seen.lock().expect("lock").written.clone()
    }

    /// Everything the terminal sent since the last call, taking it.
    ///
    /// A host forwards this to the program; leaving it in the queue would send
    /// the same answer to the program twice.
    pub fn take_written(&self) -> Vec<u8> {
        std::mem::take(&mut self.seen.lock().expect("lock").written)
    }

    /// The sizes the terminal asked for so far, without taking them.
    pub fn resized(&self) -> Vec<(i32, i32)> {
        self.seen.lock().expect("lock").resized.clone()
    }
}

impl Connection for Queue {
    fn drain_into(&mut self, out: &mut Vec<u8>) -> bool {
        let mut seen = self.seen.lock().expect("lock");
        out.extend_from_slice(&std::mem::take(&mut seen.output));
        seen.ended
    }

    fn write(&mut self, bytes: &[u8]) -> io::Result<()> {
        self.seen.lock().expect("lock").written.extend_from_slice(bytes);
        Ok(())
    }

    fn resize(&mut self, columns: i32, rows: i32) -> io::Result<()> {
        self.seen.lock().expect("lock").resized.push((columns, rows));
        Ok(())
    }

    fn exited(&mut self) -> bool {
        self.seen.lock().expect("lock").ended
    }

    fn close(&mut self) {
        self.end();
    }
}

#[cfg(feature = "pty")]
fn cell_count(value: i32) -> u16 {
    value.clamp(1, i32::from(u16::MAX)) as u16
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn a_connection_is_object_safe() {
        let queue = Queue::with_output(b"hi");
        let mut connection: Box<dyn Connection> = Box::new(queue.clone());

        connection.write(b"typed").expect("write");
        connection.resize(80, 24).expect("resize");
        let mut out = Vec::new();
        assert!(!connection.drain_into(&mut out));
        connection.close();

        assert_eq!(out, b"hi");
        assert_eq!(queue.written(), b"typed");
        assert_eq!(queue.resized(), [(80, 24)]);
    }

    #[test]
    fn taking_what_was_written_leaves_nothing_for_the_next_call() {
        let queue = Queue::default();
        let mut connection: Box<dyn Connection> = Box::new(queue.clone());
        connection.write(b"\\x1b[?1;2c").expect("write");

        assert_eq!(queue.take_written(), b"\\x1b[?1;2c");
        assert!(queue.take_written().is_empty());
    }

    #[test]
    fn a_feeding_host_is_drained_once_and_then_reports_the_end() {
        let queue = Queue::default();
        let mut connection: Box<dyn Connection> = Box::new(queue.clone());

        queue.feed(b"one");
        queue.feed(b"two");
        let mut out = Vec::new();
        assert!(!connection.drain_into(&mut out));
        assert_eq!(out, b"onetwo");

        queue.end();
        let mut out = Vec::new();
        assert!(connection.drain_into(&mut out));
        assert!(out.is_empty());
        assert!(connection.exited());
    }

    #[test]
    fn closing_the_connection_ends_the_stream() {
        let queue = Queue::default();
        let mut connection: Box<dyn Connection> = Box::new(queue.clone());

        connection.close();
        assert!(connection.exited());
    }

    #[cfg(feature = "pty")]
    #[test]
    fn a_cell_count_is_clamped_for_the_pty() {
        assert_eq!(cell_count(0), 1);
        assert_eq!(cell_count(120), 120);
        assert_eq!(cell_count(i32::MAX), u16::MAX);
    }
}
