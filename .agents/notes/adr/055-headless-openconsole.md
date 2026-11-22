# ADR 055 — The Windows pty launches a headless OpenConsole

- Status: Accepted
- Date: 2026-11-22
- Related: `crates/server/src/conpty.rs`, `crates/winterm/src/term_control.rs`, ADR 005, ADR 016, ADR 054

## Context

ADR 054 delivered an image protocol to the parser but found that the inbox
ConPTY (`CreatePseudoConsole` in `kernel32`) consumes DCS and APC strings before
the terminal sees them. Windows Terminal displays sixel anyway: it does not use
the inbox pseudoconsole. It launches the `OpenConsole.exe` bundled in its own
package as a **headless console host**, exactly as `winconpty` does:

- `OpenConsole.exe --headless --width W --height H --signal <handle> --server <handle>`
- the server is `\Device\ConDrv\Server`, opened with `NtOpenFile`; the client's
  console is a `\Reference` handle opened from it;
- the host reads resize messages (`8, width, height`) from the signal pipe;
- the `HPCON` is a three-handle structure (`signal`, `reference`, `host`) that
  the OS reads when it processes `PROC_THREAD_ATTRIBUTE_PSEUDOCONSOLE`, which is
  how the client process inherits its console.

A non-packaged process cannot execute a binary inside `WindowsApps` (verified:
`Access is denied`), but it can read it, so the host has to be copied out or
shipped beside the terminal.

Separately, the shell's render snapshot copied row text but not the row's image
band, so even a sequence that reached the parser painted nothing.

## Decision

- `server::conpty` has two console backends behind one `Console` enum:
  - `Inbox(HPCON)` — the existing `CreatePseudoConsole` path;
  - `Packed(Packed)` — a headless `OpenConsole.exe` this process starts.
- A console host is looked for in this order, and `Packed` is used when one is
  found:
  1. `WINTERM_CONSOLE_HOST`;
  2. `OpenConsole.exe` beside `winterm.exe`;
  3. `x64\OpenConsole.exe` beside it;
  4. `%LOCALAPPDATA%\winterm\OpenConsole.exe`.
- Starting the packed host opens the ConDrv server and its `\Reference` client,
  creates an inheritable signal pipe, duplicates the client's pipe ends as
  inheritable, and launches the host with
  `PROC_THREAD_ATTRIBUTE_HANDLE_LIST` limited to those four handles. Failure at
  any step falls back to the inbox pseudoconsole; the session still works, it
  just cannot receive images.
- The client is launched with the `Packed` structure's address as the
  `PROC_THREAD_ATTRIBUTE_PSEUDOCONSOLE` value, the same ABI `winconpty` uses.
- `Console::resize` writes the host's three-integer resize message; `close`
  closes the signal, the reference and the host handle.
- `Control::copy_visible_into` copies each row's image band alongside its text,
  so the renderer's snapshot carries what the buffer holds.
- `just console-host` copies Windows Terminal's packaged `OpenConsole.exe` to
  `target/debug` and `target/release`, since it cannot be executed in place.
  `scripts/console-host.ps1` takes a `-Destination` folder and a `-Source`
  binary for a host built or unpacked elsewhere.
- `just dist` assembles a runnable release folder: the release binary, the
  host beside it (so the sibling lookup finds it without Windows Terminal
  installed), `OpenConsole.NOTICE.html` from the package, and
  `LICENSE-OpenConsole.txt` carrying Windows Terminal's MIT text.

## Consequences

- With a host available, sixel and kitty graphics reach the buffer and are
  painted: the probe renders a green sixel block, a two-colour kitty image, and
  `reverie-amidst-ruins` runs at 60 fps. Without one, the terminal behaves as it
  did before this ADR.
- The host is Windows Terminal's binary, not ours. It is discovered, never
  downloaded, and copying it is an explicit developer step; a release folder
  carries it beside `winterm.exe` together with its notices. Building our own
  conhost remains the long-term answer, which is why the lookup checks beside
  our own executable first.
- `Packed` is an ABI, not a convenience struct: `signal` must stay at offset 0
  and `reference` at offset 8, because the OS reads the reference out of it.
  Never reorder or add a field before them.
- All of this is `unsafe` code in `conpty.rs`, covered by ADR 016's budget; each
  call carries a `SAFETY:` note.
- The inbox fallback stays: an installation without a host keeps working, and
  the packed path is never required for text.

## Alternatives considered

- **A passthrough flag on `CreatePseudoConsole`.** There is none; `0x8` is the
  SDK's glyph-width flag and had no effect on image forwarding.
- **Executing the packaged host in place.** The OS denies it.
- **Copying the host automatically from a registry-discovered package root.**
  That would execute a binary named by an HKCU-writable key; an explicit
  `just console-host` step keeps the choice visible.
- **Building our own conhost.** The right long-term answer, and the reason the
  lookup first checks beside our own executable; it is a C++ build of its own.
