//! The bell's flash.
//!
//! Upstream lights the control and fades it out over
//! `TerminalWarningBellInterval` (`TermControl::BellLightOn`). The light is
//! data, not a timer thread (ADR 006's stance): the frame loop advances it and
//! asks the window when the next repaint is due.

use std::time::{Duration, Instant};

/// How long a flash lasts: upstream's `TerminalWarningBellInterval`.
pub const FLASH: Duration = Duration::from_millis(1000);

/// The opacity of the flash when it starts.
pub const FLASH_ALPHA: f32 = 0.45;

/// How often a running flash asks to be repainted.
pub const FLASH_FRAME: Duration = Duration::from_millis(16);

/// The pane's flash, from the bell that started it.
#[derive(Debug, Default)]
pub struct VisualBell {
    started: Option<Instant>,
}

impl VisualBell {
    /// Starts a flash at `now`, unless one is already running.
    ///
    /// Upstream does nothing while its bell timer runs, so a program that
    /// repeats BEL gets one flash rather than a steady light. Returns whether
    /// this call started the flash.
    pub fn flash(&mut self, now: Instant) -> bool {
        if self.alpha(now).is_some() {
            return false;
        }
        self.started = Some(now);
        true
    }

    /// The flash's opacity at `now`, or `None` when none is running.
    ///
    /// It starts at [`FLASH_ALPHA`] and falls to nothing over [`FLASH`].
    pub fn alpha(&self, now: Instant) -> Option<f32> {
        let started = self.started?;
        let elapsed = now.saturating_duration_since(started);
        if elapsed >= FLASH {
            return None;
        }
        let remaining = 1.0 - elapsed.as_secs_f32() / FLASH.as_secs_f32();
        Some(FLASH_ALPHA * remaining)
    }

    /// When a running flash needs its next frame.
    pub fn next_repaint(&self, now: Instant) -> Option<Duration> {
        let started = self.started?;
        let elapsed = now.saturating_duration_since(started);
        if elapsed >= FLASH {
            return None;
        }
        Some(FLASH_FRAME.min(FLASH - elapsed))
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn a_flash_fades_from_its_start_to_nothing() {
        let start = Instant::now();
        let mut bell = VisualBell::default();
        assert!(bell.flash(start));

        let first = bell.alpha(start).expect("a flash");
        assert!((first - FLASH_ALPHA).abs() < 1e-6);

        let middle = bell.alpha(start + FLASH / 2).expect("still flashing");
        assert!(middle < first, "{middle} should be dimmer than {first}");
        assert!(middle > 0.0);

        assert!(bell.alpha(start + FLASH).is_none());
    }

    #[test]
    fn a_second_bell_does_not_restart_a_running_flash() {
        let start = Instant::now();
        let mut bell = VisualBell::default();
        assert!(bell.flash(start));
        assert!(!bell.flash(start + FLASH / 2));

        // Once it has finished, the next bell starts a new one.
        assert!(bell.flash(start + FLASH));
    }

    #[test]
    fn a_flash_asks_for_frames_until_it_ends() {
        let start = Instant::now();
        let mut bell = VisualBell::default();
        assert!(bell.next_repaint(start).is_none());

        bell.flash(start);
        assert_eq!(bell.next_repaint(start), Some(FLASH_FRAME));
        // The last frame is the time left, not a whole frame past the end.
        assert_eq!(
            bell.next_repaint(start + FLASH - Duration::from_millis(4)),
            Some(Duration::from_millis(4))
        );
        assert!(bell.next_repaint(start + FLASH).is_none());
    }

    #[test]
    fn a_bell_after_the_flash_ended_is_not_ignored() {
        let start = Instant::now();
        let mut bell = VisualBell::default();
        bell.flash(start);
        assert!(bell.flash(start + FLASH + Duration::from_millis(1)));
    }
}
