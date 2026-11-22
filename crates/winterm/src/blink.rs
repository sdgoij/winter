//! The cursor's blink state.
//!
//! The blink is data, not a timer thread (ADR 006's stance): the frame loop
//! advances it and asks the window when the next repaint is due.

use std::time::{Duration, Instant};

/// Half a blink cycle: the cursor is on, then off, then on.
pub const BLINK_PERIOD: Duration = Duration::from_millis(500);

/// Whether the cursor is drawn at a given moment.
#[derive(Debug)]
pub struct Blink {
    on: bool,
    last: Instant,
}

impl Blink {
    /// Creates a blink that is on and due to turn off one period from `now`.
    pub fn new(now: Instant) -> Self {
        Self { on: true, last: now }
    }

    /// Advances to `now` and returns whether the cursor is drawn.
    ///
    /// A cursor that is not blinking, or a window that lost focus, stays on:
    /// only an active blink alternates.
    pub fn update(&mut self, blinking: bool, focused: bool, now: Instant) -> bool {
        if !blinking || !focused {
            self.on = true;
            self.last = now;
            return true;
        }

        if now.duration_since(self.last) >= BLINK_PERIOD {
            self.on = !self.on;
            self.last = now;
        }

        self.on
    }

    /// Returns how long until the next toggle, when one is due.
    pub fn next_toggle(&self, blinking: bool, focused: bool, now: Instant) -> Option<Duration> {
        if !blinking || !focused {
            return None;
        }
        Some(BLINK_PERIOD.saturating_sub(now.duration_since(self.last)))
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn a_blinking_cursor_alternates_every_period() {
        let start = Instant::now();
        let mut blink = Blink::new(start);

        assert!(blink.update(true, true, start));
        assert!(blink.update(true, true, start + Duration::from_millis(499)));
        assert!(!blink.update(true, true, start + Duration::from_millis(500)));
        assert!(!blink.update(true, true, start + Duration::from_millis(999)));
        assert!(blink.update(true, true, start + Duration::from_millis(1000)));
    }

    #[test]
    fn a_steady_or_unfocused_cursor_stays_on() {
        let start = Instant::now();
        let mut blink = Blink::new(start);

        assert!(blink.update(false, true, start + Duration::from_secs(5)));
        assert!(blink.update(true, false, start + Duration::from_secs(10)));
        assert!(blink.next_toggle(false, true, start).is_none());
        assert!(blink.next_toggle(true, false, start).is_none());
    }

    #[test]
    fn the_next_toggle_is_the_rest_of_the_period() {
        let start = Instant::now();
        let mut blink = Blink::new(start);

        blink.update(true, true, start + Duration::from_millis(200));
        assert_eq!(
            blink.next_toggle(true, true, start + Duration::from_millis(200)),
            Some(Duration::from_millis(300))
        );
    }
}
