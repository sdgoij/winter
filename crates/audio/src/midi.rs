//! DECPS tones, ported from `src/audio/midi/MidiAudio.cpp`.
//!
//! Upstream keeps the arithmetic and the waveform and hands the bytes to
//! DirectSound; this module keeps the arithmetic and the waveform and hands a
//! list of tones to the host.

use std::time::Duration;

/// The length of upstream's triangle waveform, in samples.
pub const WAVE_SIZE: u32 = 16;

/// Upstream's `WAVE_DATA`: an 8-bit triangle over one cycle.
///
/// Nothing reads it: the host plays a tone at a frequency rather than the
/// buffer upstream repeats at that frequency. It stays here because it is the
/// waveform the notes are defined against, and a future mixer sink needs it.
pub const WAVE_DATA: [u8; 16] = [128, 159, 191, 223, 255, 223, 191, 159, 128, 96, 64, 32, 0, 32, 64, 96];

/// The frequency of a MIDI note, in hertz.
///
/// A4 is note 69 at 440 Hz, so the note's distance from it in semitones is the
/// exponent of a twelfth of an octave.
pub fn frequency(note: u8) -> f64 {
    (2.0f64).powf((f64::from(note) - 69.0) / 12.0) * 440.0
}

/// The rate upstream asks DirectSound to play the waveform at, in samples per
/// second: the note's frequency times the waveform's length.
pub fn wave_rate(note: u8) -> f64 {
    frequency(note) * f64::from(WAVE_SIZE)
}

/// The amplitude of a MIDI velocity, from 0 to 1.
///
/// The General MIDI 2 gain law is `40 * log10(v / 127)` decibels, and an
/// amplitude of `a` relative to full scale is `20 * log10(a)` decibels, so the
/// amplitude is `(v / 127)^2`.
pub fn gain(velocity: u8) -> f32 {
    let ratio = f32::from(velocity) / 127.0;
    ratio * ratio
}

/// A note a program asked for with DECPS.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub struct Note {
    /// The MIDI note number; 72 is C5.
    pub number: u8,
    /// The MIDI velocity; 0 is silent.
    pub velocity: u8,
    /// How long the note lasts.
    pub duration: Duration,
}

impl Note {
    /// The tone to play for this note.
    ///
    /// A silent note still takes its time: upstream waits out the duration
    /// whether or not the buffer plays.
    pub fn tone(&self) -> Tone {
        Tone {
            hz: frequency(self.number).round() as u32,
            duration: self.duration,
            silent: self.velocity == 0,
        }
    }
}

/// A tone the host can play.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub struct Tone {
    /// The frequency in hertz.
    pub hz: u32,
    /// How long it lasts.
    pub duration: Duration,
    /// Whether to stay quiet for the duration.
    pub silent: bool,
}

impl Tone {
    /// A tone of `hz` for `duration`.
    pub fn new(hz: u32, duration: Duration) -> Self {
        Self {
            hz,
            duration,
            silent: false,
        }
    }
}

/// The tones for a DECPS sequence, in order.
///
/// The notes play one after another, as upstream blocks on each before it
/// starts the next.
pub fn tones(notes: &[Note]) -> Vec<Tone> {
    notes.iter().map(Note::tone).collect()
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn a4_is_440_hertz() {
        assert!((frequency(69) - 440.0).abs() < 1e-9);
    }

    #[test]
    fn c5_is_the_octave_above_middle_c() {
        // 72 is C5, the first note DECPS can name.
        assert!((frequency(72) - 523.251_130_601).abs() < 1e-6);
        // An octave doubles the frequency.
        assert!((frequency(81) - 880.0).abs() < 1e-9);
    }

    #[test]
    fn the_wave_rate_is_the_frequency_times_the_table() {
        assert!((wave_rate(69) - 440.0 * 16.0).abs() < 1e-9);
    }

    #[test]
    fn velocity_follows_the_general_midi_gain_law() {
        assert!((gain(127) - 1.0).abs() < 1e-6);
        assert!(gain(0).abs() < 1e-6);
        let expected = (64.0f32 / 127.0) * (64.0f32 / 127.0);
        assert!((gain(64) - expected).abs() < 1e-6);
        // Half velocity is a quarter of the amplitude, not half.
        assert!(gain(64) < 0.3);
    }

    #[test]
    fn a_velocity_of_zero_is_a_silent_tone() {
        let note = Note {
            number: 72,
            velocity: 0,
            duration: Duration::from_millis(100),
        };
        assert!(note.tone().silent);
        assert!(!Note { velocity: 1, ..note }.tone().silent);
    }

    #[test]
    fn a_sequence_keeps_its_order_and_durations() {
        let notes = [
            Note {
                number: 72,
                velocity: 127,
                duration: Duration::from_millis(31),
            },
            Note {
                number: 74,
                velocity: 0,
                duration: Duration::from_millis(62),
            },
        ];
        let tones = tones(&notes);

        assert_eq!(tones.len(), 2);
        assert_eq!(tones[0].hz, 523);
        assert_eq!(tones[0].duration, Duration::from_millis(31));
        assert!(!tones[0].silent);
        assert_eq!(tones[1].hz, 587);
        assert_eq!(tones[1].duration, Duration::from_millis(62));
        assert!(tones[1].silent);
    }
}
