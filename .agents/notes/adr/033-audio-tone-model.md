# ADR 033 — Audio is a portable tone model with a Windows sink

- Status: Accepted
- Date: 2026-11-22
- Related: `crates/audio/`, `crates/winter/src/sound.rs`,
  `crates/winter/src/visual_bell.rs`, `crates/adapter/src/adapt_dispatch.rs`;
  ADRs 001, 006, 016

## Context

Two things make a noise: BEL, and the DECPS note sequence. Upstream answers BEL
with the console host's `Beep(800, 200)` (`interactivity/onecore/ConsoleWindow.cpp`)
and, in Windows Terminal, with the system "critical stop" sound plus an optional
light (`TerminalPaneContent::_controlWarningBellHandler`, default `bellStyle`
Audible). DECPS goes to `MidiAudio`, which repeats a 16-byte triangle wave in a
DirectSound buffer at the note's rate, blocking for each note
(`src/audio/midi/MidiAudio.cpp`).

The workspace ring has no sound crate, and adding one must not make the parser
or the buffer know a device exists. Getting sound out of a Rust process without
a device crate means per-OS FFI; the usual crate, `cpal`, needs ALSA headers on
Linux, and the gate runs on Ubuntu, Windows and macOS with no extra system
packages.

## Decision

- **`crates/audio` is a leaf.** It depends on no workspace crate and opens no
  device. It owns the DECPS arithmetic (note to frequency, wave rate, the
  General MIDI 2 gain law), a `Note`/`Tone` pair, and the bell's tone. Its tests
  run everywhere.
- **The parser keeps its own record.** `adapter`'s `MidiNote` is what `PlaySounds`
  queues; `winter` converts it to `audio::Note` at the seam. `adapter` never
  names `audio`, and `audio` never names `adapter`.
- **`winter::sound` is the sink.** It is the one module besides `utils` and the
  pty backends allowed `unsafe`. On Windows it plays tones with `Beep` on a
  thread of its own, one sequence at a time: the same blocking order as
  `MidiAudio::PlayNote`. A `Beep` that fails still sleeps out the note, so the
  sequence keeps the rhythm the program asked for.
- **The bell is a tone and a flash.** The tone is 800 Hz for 200 ms. The flash
  lives in `winter::visual_bell` as data (ADR 006): one second, fading, and a
  second bell while one runs is ignored, which is what `BellLightOn` does.
- **Every other target is silent for now.** `sound::play` drops the tone; the
  bell still flashes the pane and asks for attention.

## Rationale

- **The gate is the constraint.** A device crate would need a system package on
  one of the three CI platforms, and the build could not be verified here. A
  dependency that cannot be checked on two of three targets is not a good trade
  for one feature.
- **The arithmetic is the port.** Frequency, wave rate and gain are the parts
  with an upstream answer and a test; the device is the part with no upstream
  counterpart. Splitting them means the ported formulas are tested without
  hardware, the same split the renderer makes between its software backend and
  the GPU backend it does not have.
- **Velocity has nowhere to go.** `Beep` takes a frequency and a duration; the
  gain law is kept and tested for a mixer sink, and a velocity of 0 stays a
  silent tone because upstream waits out the duration whether or not the buffer
  sounds.

## Consequences

- Windows hears the bell and DECPS notes. Linux and macOS flash the pane and
  stay silent until a backend exists; the plan's Phase 11 row says so, and no
  code pretends otherwise.
- A DECPS note is at most 255/32 seconds, and the sound thread blocks for it.
  The thread is never joined, so closing the window does not wait for a song.
- Replacing the sink later is a change to `winter::sound::play` only; the crate
  and its tests stay.

## Alternatives considered

- **`cpal` or `rodio`.** Real mixer output and the gain law in use, at the cost
  of a Linux system dependency the three-OS gate does not install, and a build I
  cannot verify.
- **Writing BEL to the host's terminal.** The emulator *is* the terminal; a
  window launched from an icon has none to write to.
- **Shelling out to `paplay`, `aplay` or `afplay`.** A runtime dependency on a
  program that may not be installed, and one process per note.
- **`MessageBeep` for BEL.** Faithful to Windows Terminal's default, but it is
  another Windows-only call for a sound that is not configurable yet; the
  console host's tone needs no second API and no bell-style setting.
