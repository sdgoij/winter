//! What a terminal reports to the window.
//!
//! Upstream raises these from `ControlCore` (`src/cascadia/TerminalControl/
//! ControlCore.idl`) — `WarningBell`, `DesktopNotification`, the connection's
//! state — and their handlers update XAML. Here they are data the window drains
//! once a frame, so nothing crosses a thread and the window decides what a bell
//! or an exit means (ADR 036).
//!
//! The title is deliberately not an event. Upstream raises `TitleChanged`
//! because a XAML control has to be told; this shell reads the tab's title every
//! frame, and an event with no consumer is dead weight.

/// Something the terminal did that the window reacts to.
#[derive(Debug, Clone, PartialEq, Eq)]
pub enum TerminalEvent {
    /// The program rang BEL.
    ///
    /// One per flash, not per bell: the pane's bell light ignores a second bell
    /// while it is fading, so a program that repeats BEL reports one event and
    /// gets one beep.
    Bell,
    /// The program asked for a desktop notification (OSC 777).
    Notification {
        /// The notification's title.
        title: String,
        /// Its body.
        body: String,
    },
    /// The output stream ended, so the shell is gone.
    Exited,
}
