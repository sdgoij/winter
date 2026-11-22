//! `DeviceComm` — the byte stream between the terminal and the client.
//!
//! Upstream's `server/DeviceComm.h` abstracts the console driver's
//! `ReadFile`/`WriteFile`/`CompleteIo` calls. A portable terminal has a pty
//! instead, so the trait keeps the shape a terminal needs: read, write and
//! resize.

use std::io::{Read, Write};

use crate::pty::PtySize;

/// A bidirectional byte stream to the client process.
pub trait DeviceComm: Read + Write {
    /// Resizes the pseudo terminal the client is attached to.
    fn resize(&mut self, size: PtySize) -> std::io::Result<()>;
}
