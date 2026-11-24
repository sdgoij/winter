# ADR 072 — A Win32 input record carries the character a key types

- Status: Accepted
- Date: 2026-11-24
- Related: `crates/winter/src/input.rs`, `crates/winui/src/app.rs`,
  `crates/term/src/input/terminal_input.rs`, ADR 056

## Context

A console client that reads *characters* rather than virtual keys hung at
`ssh`'s host-key prompt: the prompt was printed, `yes` was echoed, Enter did
nothing. A cooked console — PowerShell's line editor — submitted the same Enter
happily, which is why every command still ran.

The terminal was in Win32 input mode. The console host asks for it with
`DECSET 9001` (`\x1b[?9001h`), and the packed `OpenConsole.exe` this shell
copies beside itself (ADR 055) always asks; the inbox host does not. In that mode
every key is sent as `CSI Vk;Sc;Uc;Kd;Cs;Rc _`, and **`Uc` is the only place a
console client is told what the key typed**.

The shell passed `codepoint: None` for every key that does not arrive through
XAML's `CharacterReceived` — Enter, Tab, Backspace and Escape — so their records
carried `Uc=0`: a key went down and nothing was typed. Upstream does not have
this hole: `Terminal::SendKeyEvent` asks the keyboard layout for the key's
character (`_CharacterFromKeyEvent`) and hands any key that has one to its
character handler, so the record it builds carries `\r`, `\t`, `\b` or `\x1b`.

Two probe runs against a real ConPTY (a PowerShell child printing
`[int][Console]::ReadKey($true).KeyChar`) settled it: `CSI 13;28;0;1;0;1_`
delivers character `0`, and `CSI 13;28;13;1;0;1_` delivers `13`. The console
takes the character from the record and nowhere else.

## Decision

- `input::key_character` names the character of a key that never produces a
  character event: `\r` for Enter, `\n` for Ctrl+Enter, `\t` for Tab, `\x08` for
  Backspace, `\x1b` for Escape, and nothing for a chord with Alt or the command
  key, whose character is the layout's business.
- The WinUI shell passes it as the event's `codepoint` for the keys that take
  the key path, so the record's `Uc` field carries it.
- The legacy encoding is unaffected: it derives Enter, Tab and Backspace from
  the key and the modes, and ignores `codepoint`.

## Rationale

The character belongs on the event, not in the encoder: `translate` is the layer
that pairs a key with the character its toolkit reported, and the encoder's job
is to say what it was given. Putting it anywhere else would leave the same hole
for the next toolkit.

`key_character` mirrors upstream's `_CharacterFromKeyEvent` only for the keys
whose character is layout-independent. A printable key's character comes from
the toolkit's character event, which is more accurate than any layout guess, and
a chord's character is not the terminal's to invent.

## Consequences

- Enter, Tab, Backspace and Escape now reach a character-reading client as
  themselves under the packed console host. Before, only virtual-key readers
  saw them.
- A record for a key *release* also carries the character, which is what
  upstream sends: a console's line editor acts on the press, and a
  character-reading client only ever sees characters.
- Caps Lock and Num Lock are still reported as `0` in the record's modifier
  state, because the toolkit reports no lock state. That is a separate gap and
  unchanged by this one.
- The egui shell sets no virtual key, so `translate` never builds a record for
  it and it is unaffected.

## Alternatives considered

- **Turn Win32 input mode off in the terminal.** It is the mode the host wants,
  and it is what keeps a held key held (ADR 056); the terminal cannot decline a
  mode the console asked for without breaking other clients.
- **Special-case Enter in the encoder.** The encoder cannot know the character a
  toolkit meant; deciding it there would misreport every layout-dependent key.
- **Copy the character into the record from a table inside `term`.** Same
  objection, one layer lower, and it would disagree with `key_character` the day
  one of them changed.
