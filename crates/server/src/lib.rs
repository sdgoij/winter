//! # winterm-server
//!
//! The console server: the API surface a client process talks to, and the
//! handles and wait queues behind it. Ported from `src/server`.
//!
//! Upstream's server is built around the console driver: `ConDrvDeviceComm`
//! talks to it, `Entrypoints` receives its messages, `WinNTControl` wraps the
//! NT object manager, and the API dispatchers are the routine table the driver
//! indexes. None of that exists for a portable terminal, which speaks to a
//! pty instead (ADR 015). What this crate keeps is the portable policy and
//! bookkeeping the console API needs.
//!
//! ## Modules
//!
//! - [`console_shim_policy`] — `ConsoleShimPolicy`, the cmd and PowerShell
//!   compatibility shims

pub mod console_shim_policy;
