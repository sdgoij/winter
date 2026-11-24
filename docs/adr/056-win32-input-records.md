# ADR 056 — The WinUI shell sends Win32 input records

- Status: Accepted
- Date: 2026-11-22
- Related: `crates/winui/src/app.rs`, `crates/winter/src/input.rs`, `crates/term/src/input/terminal_input.rs`, ADR 016, ADR 055

## Context

A ConPTY asks the hosting terminal for Win32 input mode as its first act: the
pty's output begins `ESC[?9001h` (DECSET 9001). In that mode the terminal sends
`CSI Vk;Sc;Uc;Kd;Cs;Rc _`, and the console host turns each record into one
`KEY_EVENT_RECORD`. That record is the only way a console client can see a key
go down and later come up: the legacy encoding sends text, and conhost answers
a text byte with a key-down and a key-up together.

The WinUI shell was sending printable keys only through `CharacterReceived`,
which reaches the terminal as text, and it never sent a key-up at all:
`PreviewKeyDown` was the only key event it subscribed to.

The symptom in a platformer: a held jump is cut to a few pixels, movement does
not happen, and a key that is not bound appears to act by itself because the
press and its release land in the same frame.

## Decision

- The pane subscribes to `KeyUp` as well; `Message::KeyUp` carries the release,
  and `key()` takes the press/release flag from the event rather than from a
  status bit.
- A printable key's `KeyDown` is held until its `CharacterReceived` arrives, so
  one record carries the virtual key, scan code and translated character.
  Releases are sent with `Uc = 0`; crossterm recovers the character from the
  virtual key for a release.
- Space is the exception. It is marked handled on both key-down and key-up,
  because XAML treats an unhandled space as an invocation and moves focus off
  the pane. A handled key raises no `CharacterReceived`, so the key event
  carries the space itself, which is layout-independent. The pane asks for
  focus back if XAML still takes it.
- Keys that are not printable stay handled as before.
- The kitty keyboard protocol cannot help here: conhost's input parser has no
  `CSI u` case, so a kitty sequence would be dropped. ConPTY keeps the protocol
  disabled in `VtIo` and consumes the client's push.

## Consequences

- `reverie-amidst-ruins` receives held movement and a full jump; its `s` key
  does nothing, as its own key map says.
- A host with no Win32 key identity (the egui shell) keeps the text path:
  `virtual_key == 0` falls back to the legacy encoding.
- The pane must stay focused for the records to be delivered. The focus restore
  is scoped to a space the terminal consumed, so clicking a tab or a button is
  not fought.

## Alternatives considered

- **Sending kitty keyboard sequences.** The window's conhost cannot parse them,
  and the hosting terminal never learns that kitty was requested.
- **Sending the character and the key separately.** conhost turns the character
  into its own down/up pair, which cuts a held key.
- **Marking every printable key handled.** `CharacterReceived` is suppressed
  for a handled key, so the translated character would be lost.
