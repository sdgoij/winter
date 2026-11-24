//! The host's side of sound: a thread that plays the tones the emulation asked
//! for.
//!
//! `Beep` blocks for the tone's duration, so a sequence cannot run on the frame
//! thread. One thread takes whole sequences and walks each in order, which is
//! what `MidiAudio::PlayNote` does on the terminal's own thread.
//!
//! `unsafe` is confined to this module by the allow on its declaration in
//! `lib.rs` (ADR 033). Only the Windows backend uses it; the macOS backend is
//! safe code around `afplay`.

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
                play_sequence(&sequence);
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

/// Plays a whole sequence, blocking until it ends.
#[cfg(windows)]
fn play_sequence(tones: &[Tone]) {
    for &tone in tones {
        play(tone);
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

/// Plays a whole sequence through `afplay`.
///
/// macOS has no `Beep` equivalent that takes a frequency, so the sequence is
/// rendered to a temporary WAV and handed to the system player. Rendering the
/// whole sequence at once keeps the notes' timing in one file instead of one
/// process per note; the sound thread waits for the player, as `Beep` makes it
/// wait.
#[cfg(target_os = "macos")]
fn play_sequence(tones: &[Tone]) {
    if tones.is_empty() {
        return;
    }

    let path = temp_path();
    let played = std::fs::write(&path, wav(tones))
        .ok()
        .and_then(|()| std::process::Command::new("/usr/bin/afplay").arg(&path).status().ok())
        .is_some_and(|status| status.success());
    let _ = std::fs::remove_file(&path);

    if !played {
        // No player: keep the sequence's rhythm rather than dropping it.
        for tone in tones {
            thread::sleep(tone.duration);
        }
    }
}

/// Drops the tone: Linux has no backend yet (ADR 033).
#[cfg(not(any(windows, target_os = "macos")))]
fn play_sequence(_tones: &[Tone]) {}

/// The sample rate the macOS sink renders at, in hertz.
#[cfg(target_os = "macos")]
const SAMPLE_RATE: u32 = 44_100;

/// A generated tone's amplitude, a quarter of full scale.
#[cfg(target_os = "macos")]
const AMPLITUDE: f64 = 0.25;

/// Renders `tones` as a mono 16-bit PCM WAV, one after another.
#[cfg(target_os = "macos")]
fn wav(tones: &[Tone]) -> Vec<u8> {
    let mut samples: Vec<i16> = Vec::new();
    for tone in tones {
        let count = (tone.duration.as_secs_f64() * f64::from(SAMPLE_RATE)).round() as usize;
        if tone.silent || tone.hz == 0 {
            samples.resize(samples.len() + count, 0);
            continue;
        }

        // Upstream repeats an 8-bit triangle; this renders the same shape at the
        // tone's frequency, then the WAV carries it at 44.1 kHz.
        let step = f64::from(tone.hz) / f64::from(SAMPLE_RATE);
        let mut phase = 0.0f64;
        for _ in 0..count {
            let triangle = 4.0 * (phase - (phase + 0.5).floor()).abs() - 1.0;
            samples.push((triangle * AMPLITUDE * f64::from(i16::MAX)) as i16);
            phase = (phase + step).fract();
        }
    }
    encode_wav(&samples)
}

/// The canonical 44-byte WAV header around 16-bit mono samples.
#[cfg(target_os = "macos")]
fn encode_wav(samples: &[i16]) -> Vec<u8> {
    let data_len = u32::try_from(samples.len() * 2).unwrap_or(u32::MAX);
    let mut out = Vec::with_capacity(44 + samples.len() * 2);
    out.extend_from_slice(b"RIFF");
    out.extend_from_slice(&data_len.saturating_add(36).to_le_bytes());
    out.extend_from_slice(b"WAVEfmt ");
    out.extend_from_slice(&16u32.to_le_bytes());
    out.extend_from_slice(&1u16.to_le_bytes());
    out.extend_from_slice(&1u16.to_le_bytes());
    out.extend_from_slice(&SAMPLE_RATE.to_le_bytes());
    out.extend_from_slice(&(SAMPLE_RATE * 2).to_le_bytes());
    out.extend_from_slice(&2u16.to_le_bytes());
    out.extend_from_slice(&16u16.to_le_bytes());
    out.extend_from_slice(b"data");
    out.extend_from_slice(&data_len.to_le_bytes());
    for sample in samples {
        out.extend_from_slice(&sample.to_le_bytes());
    }
    out
}

#[cfg(target_os = "macos")]
fn temp_path() -> std::path::PathBuf {
    let stamp = std::time::SystemTime::now()
        .duration_since(std::time::UNIX_EPOCH)
        .map_or(0, |elapsed| elapsed.as_nanos());
    std::env::temp_dir().join(format!("winter-{}-{stamp}.wav", std::process::id()))
}

#[cfg(all(test, target_os = "macos"))]
mod tests {
    use super::*;
    use std::time::Duration;

    fn samples(wav: &[u8]) -> &[u8] {
        &wav[44..]
    }

    #[test]
    fn a_sequence_becomes_a_canonical_mono_wav() {
        let tones = [
            Tone::new(440, Duration::from_millis(100)),
            Tone {
                hz: 0,
                duration: Duration::from_millis(50),
                silent: true,
            },
        ];
        let wav = wav(&tones);

        assert_eq!(&wav[..4], b"RIFF");
        assert_eq!(&wav[8..16], b"WAVEfmt ");
        assert_eq!(u16::from_le_bytes([wav[20], wav[21]]), 1, "PCM");
        assert_eq!(u16::from_le_bytes([wav[22], wav[23]]), 1, "mono");
        assert_eq!(u32::from_le_bytes(wav[24..28].try_into().unwrap()), SAMPLE_RATE);
        // 100 ms and 50 ms at 44.1 kHz, two bytes per sample.
        assert_eq!(samples(&wav).len(), (4410 + 2205) * 2);
    }

    #[test]
    fn a_silent_tone_is_zeroes_for_its_duration() {
        let wav = wav(&[Tone {
            hz: 440,
            duration: Duration::from_millis(10),
            silent: true,
        }]);
        assert!(samples(&wav).iter().all(|&byte| byte == 0));
    }

    #[test]
    fn an_audible_tone_alternates_around_silence() {
        let wav = wav(&[Tone::new(440, Duration::from_millis(10))]);
        let samples: Vec<i16> = samples(&wav)
            .as_chunks::<2>()
            .0
            .iter()
            .map(|pair| i16::from_le_bytes([pair[0], pair[1]]))
            .collect();
        assert!(samples.iter().any(|&sample| sample > 0));
        assert!(samples.iter().any(|&sample| sample < 0));
    }
}
