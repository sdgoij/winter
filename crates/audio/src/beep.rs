//! The terminal's bell.
//!
//! The console host answers BEL with `Beep(800, 200)`
//! (`interactivity/onecore/ConsoleWindow.cpp`), so that is the tone this module
//! describes.

use std::time::Duration;

use crate::midi::Tone;

/// The frequency the console host beeps at, in hertz.
pub const BELL_HZ: u32 = 800;

/// How long the console host beeps for.
pub const BELL_DURATION: Duration = Duration::from_millis(200);

/// The tone BEL plays.
pub fn tone() -> Tone {
    Tone::new(BELL_HZ, BELL_DURATION)
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn the_bell_is_the_console_hosts_beep() {
        let tone = tone();
        assert_eq!(tone.hz, 800);
        assert_eq!(tone.duration, Duration::from_millis(200));
        assert!(!tone.silent);
    }
}
