//! Extensions: the manifest, discovery, and — behind the `host` feature — the
//! JavaScript host that runs them.
//!
//! `docs/extensions-scope.md` is the working record of the surface. In short:
//! an extension is a directory with an `extension.toml` and an ES module entry,
//! evaluated in its own `slag::Context` with a `winter` global built from host
//! functions. This crate is the loader half (`manifest`, `discover`, `watch`) and
//! it sees no workspace crate and no window, so it can be unit-tested with
//! neither.
//!
//! The shell implements [`Host`]. That trait is the *internal* half of the
//! contract — never exposed to extensions — and it is what keeps the core
//! `winter` crate free of both this crate and a JavaScript engine.

pub mod discover;
#[cfg(feature = "host")]
pub mod host;
pub mod manifest;
pub mod watch;

pub use discover::{Loaded, Options, Report, Skipped, Source, discover};
#[cfg(feature = "host")]
pub use host::{Command, Engine, Event};
pub use manifest::Manifest;
pub use watch::Stamps;

/// A tab as an extension sees it.
#[derive(Debug, Clone, PartialEq, Eq, serde::Serialize)]
#[serde(rename_all = "camelCase")]
pub struct TabInfo {
    /// The tab's position in the strip.
    pub index: usize,
    /// The title the shell shows for it.
    pub title: String,
    /// Whether it is the selected tab.
    pub active: bool,
    /// How many panes it holds.
    pub panes: usize,
}

/// The focused pane as an extension sees it.
#[derive(Debug, Clone, PartialEq, Eq, serde::Serialize)]
#[serde(rename_all = "camelCase")]
pub struct PaneInfo {
    /// The pane's own identifier, stable for the pane's life.
    pub id: String,
    /// The title the shell shows for the pane.
    pub title: String,
    /// The program running in it, as the session reports it.
    pub program: String,
    /// The grid's width in cells.
    pub columns: usize,
    /// The grid's height in cells.
    pub rows: usize,
}

/// What the shell lends an extension. Implemented by each shell, never by an
/// extension.
///
/// The reads are taken once per dispatch by the host (`tabs`, `focused`,
/// `visible_text`), so an extension's event handler observes one coherent
/// picture of the shell; the writes are applied after the handler returns, so a
/// handler can never re-enter the shell mid-frame. A shell that cannot do
/// something answers `false`, and the extension sees a normal failure rather
/// than a crash.
pub trait Host {
    /// The tabs in the strip, in order.
    fn tabs(&self) -> Vec<TabInfo>;
    /// The focused pane, if a tab has one.
    fn focused(&self) -> Option<PaneInfo>;
    /// The focused pane's visible screen, row by row.
    fn visible_text(&self) -> String;
    /// Up to `lines` document rows directly above the visible screen, joined by
    /// newlines — the scrollback the extension can read.
    fn history(&mut self, lines: usize) -> String;
    /// Select the tab at `index`, if it exists.
    fn select_tab(&mut self, index: usize);
    /// Open a new tab on the shell's default profile, made active. `false` when
    /// the shell could not start one.
    fn open_tab(&mut self) -> bool;
    /// Close the tab at `index`, if it exists and is not the last one. `false`
    /// when nothing was closed; an extension cannot end the window.
    fn close_tab(&mut self, index: usize) -> bool;
    /// Write `text` into the focused pane's pty.
    fn send_text(&mut self, text: &str);
    /// Put `text` on the clipboard.
    fn copy(&mut self, text: &str);
    /// Show a desktop notification.
    fn notify(&mut self, title: &str, body: &str);
    /// Open `url` in the user's browser. `false` when the shell has no opener.
    fn open_url(&mut self, url: &str) -> bool;
}
