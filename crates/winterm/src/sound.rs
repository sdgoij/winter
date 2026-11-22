//! The host's side of sound: a thread that plays the tones the emulation asked
//! for.
//!
//! `Beep` blocks for the tone's duration, so a sequence cannot run on the frame
//! thread. One thread takes whole sequences and walks each in order, which is
//! what `MidiAudio::PlayNote` does on the terminal's own thread.
//!
//! `unsafe` is confined to this module by the allow on its declaration in
//! `lib.rs` (ADR 033).

use std::sync::mpsc::{self, Sender};
use std::thread;

use audio::Tone;

/// The terminal's tones, played on a thread of their own.
pub struct Sound {
    tones: Sender<Vec<Tone>>,
}

impl Sound {
    /// Starts the player.
    ///
    /// The thread ends with the process: a long song must not hold the window
    /// open, and closing the window never waits for it.
    pub fn new() -> Self {
        let (tones, queue) = mpsc::channel::<Vec<Tone>>();
        thread::spawn(move || {
            for sequence in queue {
                for tone in sequence {
                    play(tone);
                }
            }
        });
        Self { tones }
    }

    /// Queues tones to play after the ones already queued.
    ///
    /// A closed queue is dropped: a bell is not worth a stutter.
    pub fn play(&self, tones: Vec<Tone>) {
        let _ = self.tones.send(tones);
    }
}

impl Default for Sound {
    /// Same as [`Sound::new`]: the player thread has nothing to configure.
    fn default() -> Self {
        Self::new()
    }
}

/// Plays one tone, blocking until it ends.
#[cfg(windows)]
fn play(tone: Tone) {
    // A note is at most 255/32 seconds, and rounding up keeps the sequence's
    // rhythm rather than drifting ahead of it.
    let millis = u32::try_from(tone.duration.as_micros().div_ceil(1000)).unwrap_or(u32::MAX);
    if tone.silent || millis == 0 {
        thread::sleep(tone.duration);
        return;
    }

    // The documented range is 37 to 32767 Hz; a note outside it is inaudible
    // either way, so the clamp only keeps the call inside that contract.
    let hz = tone.hz.clamp(37, 32_767);
    // SAFETY: `Beep` takes two integers and no pointers, and blocks for
    // `millis`. Its result is ignored when it succeeds; a machine with no
    // speaker cannot say so anywhere the window could act on.
    if unsafe { windows_sys::Win32::System::Diagnostics::Debug::Beep(hz, millis) } == 0 {
        // The tone did not sound, so keep the sequence's timing anyway.
        thread::sleep(tone.duration);
    }
}

/// Drops the tone: no platform backend exists yet (ADR 033).
#[cfg(not(windows))]
fn play(_tone: Tone) {}
