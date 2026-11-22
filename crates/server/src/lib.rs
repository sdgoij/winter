//! # winterm-server
//!
//! The console server: the API surface a client process talks to, and the
//! handles and wait queues behind it. Ported from `src/server`.
//!
//! Upstream's server is built around the console driver: `ConDrvDeviceComm`
//! talks to it, `Entrypoints` receives its messages, `WinNTControl` wraps the
//! NT object manager, and the API dispatchers are the routine table the driver
//! indexes. None of that exists for a portable terminal, which speaks to a
//! pty instead (ADRs 015 and 016). What this crate keeps is the portable
//! policy, bookkeeping, and the pty layer the application runs a shell on.
//!
//! ## Modules
//!
//! - [`console_shim_policy`] — `ConsoleShimPolicy`, the cmd and PowerShell
//!   compatibility shims
//! - [`pty`] — the pty façade: `PtySize`, `PtyCommand` and the platform `Pty`
//! - [`device_comm`] — `DeviceComm`, the byte stream to the client
//!
//! The two pty backends are the only modules allowed to use `unsafe`
//! (ADR 016); everywhere else the workspace deny applies.

#[cfg(windows)]
#[allow(unsafe_code)]
mod conpty;
#[cfg(unix)]
#[allow(unsafe_code)]
mod posix_pty;

pub mod console_shim_policy;
pub mod device_comm;
pub mod pty;
