//! Sound for the terminal.
//!
//! Two things make a noise: the bell a program rings with BEL, and the notes a
//! program plays with DECPS. Upstream answers BEL with the console host's
//! `Beep(800, 200)` (`interactivity/onecore/ConsoleWindow.cpp`) and DECPS with
//! `MidiAudio`, which fills a DirectSound buffer with a triangle wave
//! (`src/audio/midi/MidiAudio.cpp`).
//!
//! This crate keeps the portable half of both — which tone, how loud, for how
//! long — and says nothing about a device. Playing a tone belongs to the host
//! (ADR 033), so the arithmetic is tested without an audio card.

pub mod beep;
pub mod midi;

pub use midi::{Note, Tone};
