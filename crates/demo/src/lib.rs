//! A host for [`winter::embed::Surface`], for a process or an addon.
//!
//! The editor demo. A host owns the shell and the pixels; what it lacks is the
//! terminal, so it asks `winter` for one. There are two hosts: a process that
//! speaks a line protocol over stdio (ADR 083), and an addon the editor's own
//! process calls directly (ADR 084). Both build the same surface here — the
//! font measurement a shell would do (ADR 029), the pointer policy a shell would
//! apply (ADR 019), and the frames an editor needs.
//!
//! This is not a port, and it is deliberately not a terminal: everything that
//! decides what the screen says is `winter`.

use std::io::{self, BufRead, Write};
use std::path::{Path, PathBuf};
use std::time::Instant;

use renderer::{CellSizeInDIP, FontDueRasterizer, FontInfo, Settings};
use term::input::{Key, MouseButton, MouseButtons, MouseEvent, WHEEL_DELTA, WheelAxis};
use utils::Size;
use winter::embed::{Setup, Surface};
use winter::input;
use winter::term_events::TerminalEvent;

/// The shell this platform starts when a host names none.
///
/// PowerShell 7 first on Windows: it is the current one, and a host that
/// installed it expects it. Then Windows PowerShell, whose directory a
/// locked-down `PATH` may omit. Then whatever `COMSPEC` names. Elsewhere it is
/// `$SHELL`, falling back to `/bin/sh`.
pub fn default_shell() -> PathBuf {
    #[cfg(windows)]
    {
        if let Some(program) = first_found(&["pwsh.exe", "powershell.exe"], on_path) {
            return program;
        }
        if let Some(program) = windows_powershell() {
            return program;
        }
        if let Some(program) = std::env::var_os("COMSPEC") {
            return PathBuf::from(program);
        }
        PathBuf::from("cmd.exe")
    }

    #[cfg(not(windows))]
    {
        std::env::var_os("SHELL")
            .map(PathBuf::from)
            .unwrap_or_else(|| PathBuf::from("/bin/sh"))
    }
}

/// The first of `names` that `find` resolves, in order.
fn first_found(names: &[&str], find: impl Fn(&str) -> Option<PathBuf>) -> Option<PathBuf> {
    names.iter().find_map(|name| find(name))
}

/// A program on `PATH`, if the name resolves to a file there.
#[cfg(windows)]
fn on_path(name: &str) -> Option<PathBuf> {
    let path = std::env::var_os("PATH")?;
    std::env::split_paths(&path)
        .map(|directory| directory.join(name))
        .find(|candidate| candidate.is_file())
}

/// Windows PowerShell's own directory, which a host's `PATH` may not carry.
#[cfg(windows)]
fn windows_powershell() -> Option<PathBuf> {
    let root = std::env::var_os("SystemRoot").or_else(|| std::env::var_os("WINDIR"))?;
    let candidate = PathBuf::from(root)
        .join("System32")
        .join("WindowsPowerShell")
        .join("v1.0")
        .join("powershell.exe");
    candidate.is_file().then_some(candidate)
}

/// Measures the host's monospace face at `size` for a `columns` by `rows` grid.
///
/// Font discovery is the host's (ADR 029), so this mirrors what the GTK shell
/// measures, reduced to the pieces an embedding surface wants: the face, its
/// metrics and the atlas settings for the grid the host is about to draw.
pub fn measure(size: f32, columns: i32, rows: i32) -> Option<Setup> {
    let directories = winter::fonts::font_directories();
    let paths: Vec<&Path> = directories.iter().map(std::path::PathBuf::as_path).collect();
    let collection = renderer::FontCollection::from_directories(paths);
    let face = collection.default_monospace(renderer::PREFERRED_MONOSPACE_FAMILIES)?;

    let shaper = FontDueRasterizer::from_file(face.path(), size).ok()?;
    let rasterizer = FontDueRasterizer::from_file(face.path(), size).ok()?;

    let advance = shaper.advance_width('M');
    let (ascent, descent) = shaper.vertical_metrics();
    let cell_width = (advance.ceil() as u16).max(1);
    // The cell is the face's own box, so a glyph cannot spill into the row
    // above it (the GTK shell draws the same box).
    let baseline = (ascent.ceil() as u16).max(1);
    let descender = (descent.ceil() as u16).max(1);
    let cell_height = baseline.saturating_add(descender);

    let mut font = FontInfo::default();
    font.set_from_engine(
        face.family().to_string(),
        0,
        400,
        65001,
        size * 0.75,
        CellSizeInDIP {
            width: f32::from(cell_width),
            height: f32::from(cell_height),
        },
        Size::new(i32::from(cell_width), i32::from(cell_height)),
    );

    let side = |cells: i32, cell: u16| {
        cell.saturating_mul(u16::try_from(cells.max(1)).unwrap_or(u16::MAX))
            .max(1)
    };
    let cells = |count: i32| u16::try_from(count.max(1)).unwrap_or(u16::MAX);

    let settings = Settings {
        font: renderer::FontSettings {
            font_name: face.family().to_string(),
            font_size: size,
            cell_size: (cell_width, cell_height),
            font_weight: 400,
            advance_width: cell_width,
            baseline,
            descender,
            thin_line_width: 1,
            dpi: 96,
            ..renderer::FontSettings::default()
        },
        misc: renderer::MiscellaneousSettings {
            background_color: 0x000C_0C0C,
            foreground_color: 0x00CC_CCCC,
            ..renderer::MiscellaneousSettings::default()
        },
        target_size: (side(columns, cell_width), side(rows, cell_height)),
        viewport_cell_count: (cells(columns), cells(rows)),
        ..Settings::default()
    };

    Some(Setup {
        font,
        settings,
        shaper: Box::new(shaper),
        rasterizer: Box::new(rasterizer),
        columns,
        rows,
    })
}

/// One thing the editor asks the server to do.
#[derive(Debug, Clone, PartialEq, Eq)]
pub enum Command {
    /// Text for the program, as the user typed it.
    Write(String),
    /// A new grid, in cells.
    Resize {
        /// The grid's new width.
        columns: i32,
        /// The grid's new height.
        rows: i32,
    },
    /// A key from the toolkit, with the identity the encoder needs.
    Key {
        /// The toolkit's name for the key: `ArrowUp`, `F5`, `Enter`, `a`.
        name: String,
        /// The text the key produced, when it produced any.
        text: String,
        /// The key's physical name: `KeyD`, `ArrowUp`. This is what a Win32
        /// virtual key describes, so a program that asked for Win32 records
        /// gets a key it can hold down rather than one frame of text.
        code: String,
        /// Whether Shift is held.
        shift: bool,
        /// Whether Ctrl is held.
        ctrl: bool,
        /// Whether Alt is held.
        alt: bool,
        /// Whether this is the press rather than the release.
        pressed: bool,
        /// Whether this is an auto-repeat rather than the first press.
        repeat: bool,
    },
    /// Text to paste through the terminal's bracketed-paste filter.
    Paste(String),
    /// Paint and send a frame, if anything changed.
    Render,
    /// A button press at a cell.
    Press {
        /// The cell's column.
        column: i32,
        /// The cell's row.
        row: i32,
        /// 0 left, 1 middle, 2 right.
        button: i32,
        /// Whether Shift is held, which keeps the pointer for the host.
        shift: bool,
    },
    /// A pointer move, with the button that is held or -1 for none.
    Move {
        /// The cell's column.
        column: i32,
        /// The cell's row.
        row: i32,
        /// 0 left, 1 middle, 2 right, -1 nothing held.
        button: i32,
        /// Whether Shift is held.
        shift: bool,
    },
    /// A button release at a cell.
    Release {
        /// The cell's column.
        column: i32,
        /// The cell's row.
        row: i32,
        /// 0 left, 1 middle, 2 right.
        button: i32,
        /// Whether Shift is held.
        shift: bool,
    },
    /// A wheel turn at a cell; positive `lines` scrolls into the history.
    Wheel {
        /// The cell's column.
        column: i32,
        /// The cell's row.
        row: i32,
        /// How many lines, signed.
        lines: i32,
        /// Whether Shift is held, which keeps the wheel for the host.
        shift: bool,
    },
    /// Answer with the selection, for the clipboard.
    Copy,
    /// Answer with what the terminal believes the program asked for.
    ///
    /// A program's request for Win32 records or for the kitty keyboard protocol
    /// changes what a key becomes, and a host cannot see the request itself — a
    /// console host may consume it before any terminal sees the sequence. This
    /// is how a host asks, and what the answer to "why does this key do that"
    /// starts with.
    Modes,
    /// End the session.
    Quit,
}

/// Reads one command, or `None` when the line names nothing this server knows.
///
/// An unknown line is skipped rather than fatal: a host may send a command this
/// build predates, and dropping the session then would be worse than ignoring
/// the line.
pub fn parse(line: &str) -> Option<Command> {
    let value: serde_json::Value = serde_json::from_str(line).ok()?;
    let shift = || value.get("shift").and_then(serde_json::Value::as_bool).unwrap_or(false);
    let flag = |name: &str| value.get(name).and_then(serde_json::Value::as_bool).unwrap_or(false);
    let cell = || {
        Some((
            i32::try_from(value.get("col")?.as_i64()?).ok()?,
            i32::try_from(value.get("row")?.as_i64()?).ok()?,
        ))
    };
    let button = |default: i32| {
        value
            .get("button")
            .and_then(serde_json::Value::as_i64)
            .and_then(|button| i32::try_from(button).ok())
            .unwrap_or(default)
    };

    Some(match value.get("cmd")?.as_str()? {
        "write" => Command::Write(value.get("text")?.as_str()?.to_string()),
        "key" => Command::Key {
            name: value.get("name")?.as_str()?.to_string(),
            text: value
                .get("text")
                .and_then(serde_json::Value::as_str)
                .unwrap_or_default()
                .to_string(),
            code: value
                .get("code")
                .and_then(serde_json::Value::as_str)
                .unwrap_or_default()
                .to_string(),
            shift: shift(),
            ctrl: flag("ctrl"),
            alt: flag("alt"),
            pressed: value
                .get("pressed")
                .and_then(serde_json::Value::as_bool)
                .unwrap_or(true),
            repeat: flag("repeat"),
        },
        "paste" => Command::Paste(value.get("text")?.as_str()?.to_string()),
        "resize" => Command::Resize {
            columns: i32::try_from(value.get("cols")?.as_i64()?).ok()?,
            rows: i32::try_from(value.get("rows")?.as_i64()?).ok()?,
        },
        "render" => Command::Render,
        "modes" => Command::Modes,
        "press" => {
            let (column, row) = cell()?;
            Command::Press {
                column,
                row,
                button: button(0),
                shift: shift(),
            }
        }
        "move" => {
            let (column, row) = cell()?;
            Command::Move {
                column,
                row,
                button: button(-1),
                shift: shift(),
            }
        }
        "release" => {
            let (column, row) = cell()?;
            Command::Release {
                column,
                row,
                button: button(0),
                shift: shift(),
            }
        }
        "wheel" => {
            let (column, row) = cell()?;
            Command::Wheel {
                column,
                row,
                lines: value
                    .get("lines")
                    .and_then(serde_json::Value::as_i64)
                    .and_then(|lines| i32::try_from(lines).ok())
                    .unwrap_or(3),
                shift: shift(),
            }
        }
        "copy" => Command::Copy,
        "quit" => Command::Quit,
        _ => return None,
    })
}

/// Serves commands from `input`, answering on `output`.
///
/// A frame is `u32 0`, width, height, cell width, cell height, byte length, then
/// that many RGBA bytes. A text answer is `u32 1`, byte length, then UTF-8 —
/// which is how a copy reaches the host's clipboard.
///
/// A frame with a zero width is the answer to a render whose terminal did not
/// change: the host keeps the picture it already has.
pub fn serve(input: impl BufRead, output: &mut impl Write, surface: &mut Surface) -> io::Result<()> {
    let start = Instant::now();
    let mut frames = Frames::new();

    for line in input.lines() {
        let line = line?;
        if line.trim().is_empty() {
            continue;
        }
        let Some(command) = parse(&line) else {
            continue;
        };

        // Every command reads the screen, so the program's output is drained
        // first: a copy after a burst of output must see that output.
        surface.pump();

        match command {
            Command::Write(text) => surface.write(&text),
            Command::Key {
                name,
                text,
                code,
                shift,
                ctrl,
                alt,
                pressed,
                repeat,
            } => {
                send_key(
                    surface,
                    &KeyInput {
                        name: &name,
                        text: &text,
                        code: &code,
                        shift,
                        ctrl,
                        alt,
                        pressed,
                        repeat,
                    },
                );
            }
            Command::Paste(text) => surface.paste(&text),
            Command::Resize { columns, rows } => surface.resize(columns, rows).map_err(io::Error::other)?,
            Command::Render => {
                let now = Instant::now();
                let cursor = cursor_on(surface, start, now);
                frames.write(surface, output, cursor)?;
                // The answer goes to the host every time, changed or not: a
                // zero-width header is the answer to a render that painted
                // nothing, and a host that waits for the reply would wait
                // forever if this only flushed when the pane had changed.
                output.flush()?;
                if surface.take_events(now).contains(&TerminalEvent::Exited) {
                    break;
                }
            }
            Command::Press {
                column,
                row,
                button,
                shift,
            } => pointer_press(surface, (column, row), button, shift),
            Command::Move {
                column,
                row,
                button,
                shift,
            } => pointer_move(surface, (column, row), button, shift),
            Command::Release {
                column,
                row,
                button,
                shift,
            } => pointer_release(surface, (column, row), button, shift),
            Command::Wheel {
                column,
                row,
                lines,
                shift,
            } => pointer_wheel(surface, (column, row), lines, shift),
            Command::Copy => {
                let text = surface.selection_text().unwrap_or_default();
                write_text(output, &text)?;
                output.flush()?;
            }
            Command::Modes => {
                write_diagnostic(output, &format!("{:?}", surface.input_modes()))?;
                output.flush()?;
            }
            Command::Quit => break,
        }
    }
    Ok(())
}

/// Sends a key the toolkit reported, through the terminal's encoder.
///
/// Both hosts use this: the wire protocol's `key` command and the addon's
/// `key` method differ in how the fields arrive, not in what they mean.
pub fn send_key(surface: &mut Surface, input: &KeyInput<'_>) {
    if let Some(event) = key_event(input) {
        surface.key(&event);
    }
}

/// A button press at a cell: to the program if it tracks the mouse, otherwise
/// the start of a selection.
pub fn pointer_press(surface: &mut Surface, cell: (i32, i32), button: i32, shift: bool) {
    if forwards(surface, shift) {
        surface.mouse(&with_shift(MouseEvent::press(cell, mouse_button(button)), shift));
    } else if button == 0 {
        surface.press(cell, Instant::now());
    }
}

/// A pointer move: a drag report to a tracking program, otherwise extending the
/// selection the press started.
pub fn pointer_move(surface: &mut Surface, cell: (i32, i32), button: i32, shift: bool) {
    if forwards(surface, shift) {
        let mut event = MouseEvent::moved(cell);
        event.buttons = mouse_buttons(button);
        event.modifiers.shift = shift;
        surface.mouse(&event);
    } else if button == 0 {
        surface.drag(cell);
    }
}

/// A button release at a cell, which only a tracking program is told about.
pub fn pointer_release(surface: &mut Surface, cell: (i32, i32), button: i32, shift: bool) {
    if forwards(surface, shift) {
        surface.mouse(&with_shift(MouseEvent::release(cell, mouse_button(button)), shift));
    }
}

/// A wheel turn: the program's report if it tracks the mouse, otherwise the
/// pane's own scrolling.
pub fn pointer_wheel(surface: &mut Surface, cell: (i32, i32), lines: i32, shift: bool) {
    if forwards(surface, shift) {
        let delta = WHEEL_DELTA * lines.signum();
        let event = MouseEvent::wheel(cell, WheelAxis::Vertical, delta);
        surface.mouse(&with_shift(event, shift));
    } else {
        surface.scroll_by(lines);
    }
}

/// Whether the pointer belongs to the program rather than to this host.
///
/// The rule is the same one the shells apply (ADR 019): a program that tracks
/// the mouse gets the pointer, and Shift keeps it for selection.
pub fn forwards(surface: &Surface, shift: bool) -> bool {
    input::forwards_to_application(surface.is_tracking_mouse(), shift)
}

/// One key as the toolkit reported it, from either host.
pub struct KeyInput<'a> {
    /// The toolkit's name for the key.
    pub name: &'a str,
    /// The text the key produced.
    pub text: &'a str,
    /// The key's physical name, in the standard PC-101 layout.
    pub code: &'a str,
    /// Whether Shift is held.
    pub shift: bool,
    /// Whether Ctrl is held.
    pub ctrl: bool,
    /// Whether Alt is held.
    pub alt: bool,
    /// Whether this is the press rather than the release.
    pub pressed: bool,
    /// Whether this is an auto-repeat.
    pub repeat: bool,
}

/// The encoder event a toolkit key names, or `None` for a key with no encoded
/// form.
///
/// A printable key arrives twice in a toolkit — as a key and as the text it
/// typed — and only the terminal knows which of the two the program wants: one
/// reading characters gets the text, one that asked for every key (or for Win32
/// records) gets the key and, with it, the release. Both travel in one event and
/// [`Surface::key`] decides (ADR 056).
///
/// The physical `code` is what becomes the Win32 virtual key and scan code, so a
/// host that has only a layout-dependent name should pass an empty one and fall
/// back to the character path.
pub fn key_event(input: &KeyInput<'_>) -> Option<input::Event> {
    let KeyInput {
        name,
        text,
        code,
        shift,
        ctrl,
        alt,
        pressed,
        repeat,
    } = *input;
    let modifiers = input::Modifiers {
        shift,
        ctrl,
        alt,
        command: false,
    };
    let key = named_key(name)?;

    // More than one character is not a key: a composition, or text a toolkit
    // typed on its own, has only the text to carry it.
    if pressed && text.chars().count() > 1 {
        return Some(input::Event::Text(text.to_string()));
    }

    Some(input::Event::Key {
        key,
        pressed,
        repeat,
        modifiers,
        // A Win32 record names the key by this, which is how a program holds it
        // down: a record without one is a lone text byte, and the console answers
        // that with a press and a release together.
        virtual_key: virtual_key(code),
        // The same key in the console's other numbering. A record that leaves it
        // zero is not the record a console host expects.
        scan_code: scan_code(code),
        repeat_count: 0,
        // The character the key typed, for the encodings that carry one: the
        // legacy text path, and Win32's record. A release carries none — the
        // console recovers it from the virtual key (ADR 056).
        codepoint: if pressed && input::produces_text(key, modifiers) {
            text.chars().next()
        } else {
            None
        },
    })
}

/// The PC-101 make code of each letter, in alphabet order.
const LETTER_SCAN: [u16; 26] = [
    0x1E, 0x30, 0x2E, 0x20, 0x12, 0x21, 0x22, 0x23, 0x17, 0x24, 0x25, 0x26, 0x32, 0x31, 0x18, 0x19, 0x10, 0x13, 0x1F,
    0x14, 0x16, 0x2F, 0x11, 0x2D, 0x15, 0x2C,
];

/// The PC-101 make code of each digit key, in the order `1` to `9` then `0`.
const DIGIT_SCAN: [u16; 10] = [0x02, 0x03, 0x04, 0x05, 0x06, 0x07, 0x08, 0x09, 0x0A, 0x0B];

/// The console's scan code for a toolkit's physical key code, or zero.
///
/// Set 1 make codes: the other half of the identity a Win32 record carries, and
/// the number a console host's own records use. The keypad is left out — the
/// virtual key already distinguishes it — and so is anything this table does not
/// name, which the console reads as "no scan code".
fn scan_code(code: &str) -> u16 {
    match code {
        "Enter" | "NumpadEnter" => 0x1C,
        "Escape" => 0x01,
        "Tab" => 0x0F,
        "Backspace" => 0x0E,
        "Space" => 0x39,
        "Minus" => 0x0C,
        "Equal" => 0x0D,
        "BracketLeft" => 0x1A,
        "BracketRight" => 0x1B,
        "Backslash" => 0x2B,
        "Semicolon" => 0x27,
        "Quote" => 0x28,
        "Backquote" => 0x29,
        "Comma" => 0x33,
        "Period" => 0x34,
        "Slash" => 0x35,
        "ArrowLeft" => 0x4B,
        "ArrowUp" => 0x48,
        "ArrowRight" => 0x4D,
        "ArrowDown" => 0x50,
        "Home" => 0x47,
        "End" => 0x4F,
        "PageUp" => 0x49,
        "PageDown" => 0x51,
        "Insert" => 0x52,
        "Delete" => 0x53,
        _ => {
            if let Some(letter) = code
                .strip_prefix("Key")
                .and_then(|rest| rest.as_bytes().first().copied())
                && letter.is_ascii_uppercase()
            {
                return LETTER_SCAN[usize::from(letter - b'A')];
            }
            if let Some(digit) = code
                .strip_prefix("Digit")
                .and_then(|rest| rest.as_bytes().first().copied())
                && digit.is_ascii_digit()
            {
                let index = if digit == b'0' { 9 } else { usize::from(digit - b'1') };
                return DIGIT_SCAN[index];
            }
            if let Some(number) = code.strip_prefix('F').and_then(|rest| rest.parse::<usize>().ok()) {
                return match number {
                    1..=10 => 0x3B + (number as u16 - 1),
                    11 => 0x57,
                    12 => 0x58,
                    _ => 0,
                };
            }
            0
        }
    }
}

/// The Win32 virtual key a toolkit's physical key code names.
///
/// The names are the DOM's `KeyboardEvent.code`, which is the physical key in the
/// standard PC-101 layout — the layout Win32's virtual keys describe. A key this
/// table does not know has no virtual key, and its press falls back to the
/// character it typed (ADR 056).
fn virtual_key(code: &str) -> u16 {
    match code {
        "Enter" | "NumpadEnter" => 0x0D,
        "Escape" => 0x1B,
        "Tab" => 0x09,
        "Backspace" => 0x08,
        "Space" => 0x20,
        "ArrowLeft" => 0x25,
        "ArrowUp" => 0x26,
        "ArrowRight" => 0x27,
        "ArrowDown" => 0x28,
        "Home" => 0x24,
        "End" => 0x23,
        "PageUp" => 0x21,
        "PageDown" => 0x22,
        "Insert" => 0x2D,
        "Delete" => 0x2E,
        "CapsLock" => 0x14,
        "ShiftLeft" => 0xA0,
        "ShiftRight" => 0xA1,
        "ControlLeft" => 0xA2,
        "ControlRight" => 0xA3,
        "AltLeft" => 0xA4,
        "AltRight" => 0xA5,
        "MetaLeft" => 0x5B,
        "MetaRight" => 0x5C,
        "Minus" => 0xBD,
        "Equal" => 0xBB,
        "BracketLeft" => 0xDB,
        "BracketRight" => 0xDD,
        "Backslash" => 0xDC,
        "Semicolon" => 0xBA,
        "Quote" => 0xDE,
        "Backquote" => 0xC0,
        "Comma" => 0xBC,
        "Period" => 0xBE,
        "Slash" => 0xBF,
        "NumpadAdd" => 0x6B,
        "NumpadSubtract" => 0x6D,
        "NumpadMultiply" => 0x6A,
        "NumpadDivide" => 0x6F,
        "NumpadDecimal" => 0x6E,
        _ => ranged_virtual_key(code).unwrap_or(0),
    }
}

/// The virtual key of a code that names a letter, a digit or a function key.
///
/// These are the only families whose code is a prefix and a number, and their
/// virtual keys are contiguous from a base.
fn ranged_virtual_key(code: &str) -> Option<u16> {
    if let Some(letter) = code
        .strip_prefix("Key")
        .and_then(|rest| rest.as_bytes().first().copied())
        && letter.is_ascii_uppercase()
    {
        return Some(0x41 + u16::from(letter - b'A'));
    }
    if let Some(digit) = code
        .strip_prefix("Digit")
        .and_then(|rest| rest.as_bytes().first().copied())
        && digit.is_ascii_digit()
    {
        return Some(0x30 + u16::from(digit - b'0'));
    }
    if let Some(digit) = code
        .strip_prefix("Numpad")
        .and_then(|rest| rest.as_bytes().first().copied())
        && digit.is_ascii_digit()
    {
        return Some(0x60 + u16::from(digit - b'0'));
    }
    if let Some(number) = code.strip_prefix('F').and_then(|rest| rest.parse::<u16>().ok())
        && (1..=24).contains(&number)
    {
        return Some(0x70 + number - 1);
    }
    None
}

/// The terminal key a toolkit name stands for.
fn named_key(name: &str) -> Option<Key> {
    if let Some(number) = name.strip_prefix('F').and_then(|digits| digits.parse().ok()) {
        return Some(Key::Function(number));
    }

    let mut characters = name.chars();
    match (characters.next(), characters.next()) {
        (Some(' '), None) => Some(Key::Space),
        (Some(character), None) => Some(Key::Char(character)),
        _ => Some(match name {
            "Enter" | "NumpadEnter" => Key::Enter,
            "Tab" => Key::Tab,
            "Escape" => Key::Escape,
            "Backspace" => Key::Backspace,
            "Delete" => Key::Delete,
            "Insert" => Key::Insert,
            "Home" => Key::Home,
            "End" => Key::End,
            "PageUp" => Key::PageUp,
            "PageDown" => Key::PageDown,
            "ArrowUp" => Key::Up,
            "ArrowDown" => Key::Down,
            "ArrowLeft" => Key::Left,
            "ArrowRight" => Key::Right,
            _ => return None,
        }),
    }
}

/// The cursor phase at `now`, or `true` while the cursor is not drawn.
///
/// The half-period is the shells' 500 ms, and the phase is constant while the
/// cursor is hidden, so an idle pane stops asking for frames.
/// Whether the cursor is drawn at `now`, from the blink the host started.
///
/// The half-period is the shell's — 500 ms, like every terminal — and a hidden
/// cursor never blinks, so a host passes a constant phase and a full-screen
/// program stops asking for frames.
pub fn cursor_on(surface: &Surface, start: Instant, now: Instant) -> bool {
    if !surface.cursor_visible() {
        return true;
    }
    (now.saturating_duration_since(start).as_millis() / 500).is_multiple_of(2)
}

/// The pointer button a toolkit's number names: 1 middle, 2 right, else left.
fn mouse_button(button: i32) -> MouseButton {
    match button {
        1 => MouseButton::Middle,
        2 => MouseButton::Right,
        _ => MouseButton::Left,
    }
}

/// The buttons a drag is holding, from a toolkit's number.
fn mouse_buttons(button: i32) -> MouseButtons {
    let mut buttons = MouseButtons::default();
    match button {
        0 => buttons.left = true,
        1 => buttons.middle = true,
        2 => buttons.right = true,
        _ => {}
    }
    buttons
}

/// The same event with Shift set, which a program reads as a modifier.
fn with_shift(mut event: MouseEvent, shift: bool) -> MouseEvent {
    event.modifiers.shift = shift;
    event
}

/// The last frame the host was sent, so a repaint can say what moved.
///
/// A shell mostly changes one row — a keystroke, a prompt, one line of output —
/// and shipping the whole pane for it is what made the demo feel sluggish. The
/// band is aligned to cell rows, because that is what the engine paints and what
/// a host's `putImageData` can place without arithmetic.
#[derive(Debug)]
pub struct Frames {
    previous: Vec<u32>,
    width: u32,
    height: u32,
}

/// One frame's worth of pixels, and where on the pane they belong.
///
/// A host that reads the wire format (the `winter-frame` binary) serializes
/// this; a host that calls the surface directly (the addon, ADR 084) takes it
/// as the value it is.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct Painted {
    /// Whether this covers the whole pane, so the host replaces the canvas
    /// rather than patching a band of it.
    pub full: bool,
    /// The first pixel row this payload covers.
    pub top: u32,
    /// The payload's width in pixels.
    pub width: u32,
    /// How many pixel rows the payload covers.
    pub rows: u32,
    /// Deflated RGBA, the pane's pixels with alpha forced opaque.
    pub pixels: Vec<u8>,
}

impl Frames {
    /// A painter with no frame behind it.
    pub const fn new() -> Self {
        Self {
            previous: Vec::new(),
            width: 0,
            height: 0,
        }
    }

    /// Paints the surface and returns what changed, or `None` for nothing.
    pub fn paint(&mut self, surface: &mut Surface, cursor: bool) -> io::Result<Option<Painted>> {
        if !surface.render(cursor).map_err(io::Error::other)? {
            return Ok(None);
        }

        let (width, height) = surface.frame_size();
        let (_, cell_height) = surface.cell_size();
        let frame = surface.pixels();

        let comparable = self.width == width && self.height == height && self.previous.len() == frame.len();
        let band = comparable
            .then(|| changed_rows(&self.previous, &frame, width))
            .flatten()
            .map(|(first, last)| {
                let cell = u32::from(cell_height.max(1));
                let top = (first as u32 / cell) * cell;
                let bottom = (((last as u32 / cell) + 1) * cell).min(height);
                (top, bottom - top)
            });

        // More than half the pane is not worth a patch: the host has to keep a
        // full frame anyway, and a scroll changes every row.
        let (top, rows, full) = match band {
            Some((top, rows)) if rows * 2 < height => (top, rows, false),
            _ => (0, height, true),
        };

        let start = top as usize * width as usize;
        let end = start + rows as usize * width as usize;
        // The pixels are deflated: a full-screen program repaints its whole pane
        // every frame, and raw RGBA is more than a webview can decode and paint
        // over a message channel at speed. `rows` says how much pane the payload
        // covers; the host inflates before painting.
        let pixels = deflate(&rgba(&frame[start..end]));

        self.previous.clear();
        self.previous.extend_from_slice(&frame);
        self.width = width;
        self.height = height;

        Ok(Some(Painted {
            full,
            top,
            width,
            rows,
            pixels,
        }))
    }

    /// Paints the surface and writes the frame the protocol describes.
    ///
    /// Returns whether anything was written; a host that reads a zero-width
    /// header keeps the picture it already has.
    pub fn write(&mut self, surface: &mut Surface, output: &mut impl Write, cursor: bool) -> io::Result<bool> {
        let Some(painted) = self.paint(surface, cursor)? else {
            // Nothing changed: a header with a zero width says so.
            output.write_all(&0u32.to_be_bytes())?;
            output.write_all(&[0u8; 20])?;
            return Ok(false);
        };

        if painted.full {
            let (cell_width, cell_height) = surface.cell_size();
            output.write_all(&0u32.to_be_bytes())?;
            output.write_all(&painted.width.to_be_bytes())?;
            output.write_all(&painted.rows.to_be_bytes())?;
            output.write_all(&u32::from(cell_width).to_be_bytes())?;
            output.write_all(&u32::from(cell_height).to_be_bytes())?;
        } else {
            output.write_all(&2u32.to_be_bytes())?;
            output.write_all(&painted.top.to_be_bytes())?;
            output.write_all(&painted.width.to_be_bytes())?;
            output.write_all(&painted.rows.to_be_bytes())?;
            output.write_all(&0u32.to_be_bytes())?;
        }
        output.write_all(&(painted.pixels.len() as u32).to_be_bytes())?;
        output.write_all(&painted.pixels)?;
        Ok(true)
    }
}

impl Default for Frames {
    fn default() -> Self {
        Self::new()
    }
}

/// The first and last pixel row that differ between two frames of `width`.
fn changed_rows(previous: &[u32], current: &[u32], width: u32) -> Option<(usize, usize)> {
    let stride = width as usize;
    if stride == 0 {
        return None;
    }

    let mut first = None;
    let mut last = 0;
    for (row, (before, after)) in previous.chunks(stride).zip(current.chunks(stride)).enumerate() {
        if before != after {
            first.get_or_insert(row);
            last = row;
        }
    }

    first.map(|first| (first, last))
}

/// The pixels, deflated.
///
/// Level 1 because this runs on every frame of a full-screen program: it is the
/// cheapest level that still turns flat colour into a fraction of its size, and
/// the host pays only the inflate (ADR 083).
fn deflate(pixels: &[u8]) -> Vec<u8> {
    miniz_oxide::deflate::compress_to_vec(pixels, 1)
}

/// The RGBA bytes of a run of pixels, with alpha forced opaque.
///
/// The pane is opaque; a canvas pixel with `alpha = 0` would show through.
fn rgba(pixels: &[u32]) -> Vec<u8> {
    let mut bytes = vec![255u8; pixels.len() * 4];
    for (pixel, chunk) in pixels.iter().zip(bytes.as_chunks_mut::<4>().0) {
        let [_, red, green, blue] = pixel.to_be_bytes();
        chunk[0] = red;
        chunk[1] = green;
        chunk[2] = blue;
    }
    bytes
}

/// Writes a text answer, which is how a copy reaches the host.
pub fn write_text(output: &mut impl Write, text: &str) -> io::Result<()> {
    output.write_all(&1u32.to_be_bytes())?;
    output.write_all(&(text.len() as u32).to_be_bytes())?;
    output.write_all(text.as_bytes())
}

/// Writes a diagnostic answer: `u32 3` and a length.
///
/// A host logs it. It is neither a frame nor clipboard text, so it cannot be
/// pasted into the program by a host that confuses the two.
pub fn write_diagnostic(output: &mut impl Write, text: &str) -> io::Result<()> {
    output.write_all(&3u32.to_be_bytes())?;
    output.write_all(&(text.len() as u32).to_be_bytes())?;
    output.write_all(text.as_bytes())
}

#[cfg(test)]
mod tests {
    use renderer::{CellShaping, FontId, GlyphBitmap, GlyphRasterizer};
    use winter::embed::Surface;
    use winter::term_connection::Queue;

    use super::*;

    #[test]
    fn the_first_name_that_resolves_wins() {
        let found = first_found(&["pwsh.exe", "powershell.exe"], |name| {
            (name == "powershell.exe").then(|| PathBuf::from(name))
        });
        assert_eq!(found.as_deref(), Some(Path::new("powershell.exe")));
    }

    #[test]
    fn no_candidate_resolving_is_no_shell() {
        assert_eq!(first_found(&["pwsh.exe", "powershell.exe"], |_| None), None);
    }

    /// A rasteriser that paints the whole cell, so any glyph is visible.
    struct SolidRasterizer;

    impl GlyphRasterizer for SolidRasterizer {
        fn rasterize(&mut self, _font: FontId, _glyph: u16) -> Option<GlyphBitmap> {
            Some(GlyphBitmap {
                width: 8,
                height: 16,
                left: 0,
                top: 0,
                coverage: vec![255; 8 * 16],
            })
        }
    }

    fn fixture(columns: i32, rows: i32) -> (Surface, Queue) {
        let mut font = FontInfo::default();
        font.set_from_engine(
            "Test".to_string(),
            0,
            400,
            65001,
            16.0,
            CellSizeInDIP {
                width: 8.0,
                height: 16.0,
            },
            Size::new(8, 16),
        );

        let settings = Settings {
            font: renderer::FontSettings {
                cell_size: (8, 16),
                font_size: 16.0,
                advance_width: 8,
                baseline: 12,
                descender: 4,
                thin_line_width: 1,
                ..renderer::FontSettings::default()
            },
            target_size: (columns as u16 * 8, rows as u16 * 16),
            viewport_cell_count: (columns as u16, rows as u16),
            ..Settings::default()
        };

        let queue = Queue::default();
        let surface = Surface::new(
            Setup {
                font,
                settings,
                shaper: Box::new(CellShaping),
                rasterizer: Box::new(SolidRasterizer),
                columns,
                rows,
            },
            Box::new(queue.clone()),
        );
        (surface, queue)
    }

    /// The frame header is `u32 0` then five more `u32`s.
    const FRAME_HEADER: usize = 24;
    /// A text answer is `u32 1` and a length.
    const TEXT_HEADER: usize = 8;

    fn kind(message: &[u8]) -> u32 {
        u32::from_be_bytes(message[0..4].try_into().expect("four bytes"))
    }

    fn width(frame: &[u8]) -> u32 {
        u32::from_be_bytes(frame[4..8].try_into().expect("four bytes"))
    }

    fn height(frame: &[u8]) -> u32 {
        u32::from_be_bytes(frame[8..12].try_into().expect("four bytes"))
    }

    fn text_of(message: &[u8]) -> String {
        assert_eq!(kind(message), 1, "not a text answer");
        let length = u32::from_be_bytes(message[4..8].try_into().expect("four bytes")) as usize;
        String::from_utf8(message[TEXT_HEADER..TEXT_HEADER + length].to_vec()).expect("utf-8")
    }

    /// The pixels a frame carries, after the deflate the host undoes.
    fn pixels_of(frame: &[u8]) -> Vec<u8> {
        miniz_oxide::inflate::decompress_to_vec(&frame[FRAME_HEADER..]).expect("deflated pixels")
    }

    fn payload_length(frame: &[u8]) -> usize {
        u32::from_be_bytes(frame[20..24].try_into().expect("four bytes")) as usize
    }

    #[test]
    fn a_render_command_asks_for_a_frame() {
        assert_eq!(parse(r#"{"cmd":"render"}"#), Some(Command::Render));
        // A host may still send the phase it used to own; it is ignored now.
        assert_eq!(parse(r#"{"cmd":"render","cursor":false}"#), Some(Command::Render));
    }

    #[test]
    fn a_line_that_names_nothing_this_server_knows_is_ignored() {
        assert_eq!(parse("hello"), None);
        assert_eq!(parse(r#"{"cmd":"nope"}"#), None);
        // A resize missing a side cannot be honoured.
        assert_eq!(parse(r#"{"cmd":"resize","cols":1}"#), None);
        assert_eq!(parse(r#"{"cmd":"press","col":1}"#), None);
    }

    #[test]
    fn a_pointer_command_names_its_cell_button_and_shift() {
        assert_eq!(
            parse(r#"{"cmd":"press","col":2,"row":3,"button":2,"shift":true}"#),
            Some(Command::Press {
                column: 2,
                row: 3,
                button: 2,
                shift: true
            })
        );
        // A move with no button named is a move with nothing held.
        assert_eq!(
            parse(r#"{"cmd":"move","col":0,"row":0}"#),
            Some(Command::Move {
                column: 0,
                row: 0,
                button: -1,
                shift: false
            })
        );
        assert_eq!(
            parse(r#"{"cmd":"wheel","col":0,"row":0}"#),
            Some(Command::Wheel {
                column: 0,
                row: 0,
                lines: 3,
                shift: false
            })
        );
    }

    #[test]
    fn a_write_command_reaches_the_program() {
        let (mut surface, queue) = fixture(20, 2);
        let mut out = Vec::new();

        serve(br#"{"cmd":"write","text":"ls\r"}"#.as_slice(), &mut out, &mut surface).expect("serve");

        assert_eq!(queue.take_written(), b"ls\r");
        assert!(out.is_empty(), "a write is not a frame");
    }

    #[test]
    fn a_render_command_writes_a_tagged_header_and_rgba_pixels() {
        let (mut surface, queue) = fixture(4, 2);
        // Ink on the second row, with the cursor hidden so it can only be text.
        queue.feed(b"\x1b[?25l\r\nhi");
        let mut out = Vec::new();

        serve(br#"{"cmd":"render","cursor":true}"#.as_slice(), &mut out, &mut surface).expect("serve");

        assert_eq!(kind(&out), 0);
        assert_eq!((width(&out), height(&out)), (32, 32));
        assert_eq!(u32::from_be_bytes(out[12..16].try_into().expect("four bytes")), 8);
        assert_eq!(u32::from_be_bytes(out[16..20].try_into().expect("four bytes")), 16);
        assert_eq!(out.len(), FRAME_HEADER + payload_length(&out));

        let rgba = pixels_of(&out);
        assert_eq!(rgba.len(), 32 * 32 * 4, "the frame is the pane's pixels");
        let inked = rgba[16 * 32 * 4..]
            .as_chunks::<4>()
            .0
            .iter()
            .any(|pixel| pixel[0] > 0x80 && pixel[3] == 255);
        assert!(inked, "the fed text did not reach the frame");
    }

    #[test]
    fn a_frame_travels_deflated() {
        let (mut surface, _queue) = fixture(8, 4);
        let mut out = Vec::new();

        serve(br#"{"cmd":"render"}"#.as_slice(), &mut out, &mut surface).expect("serve");

        let raw = 64 * 64 * 4;
        assert_eq!(pixels_of(&out).len(), raw);
        assert!(
            payload_length(&out) < raw / 2,
            "an empty pane deflated to {} of {raw} bytes",
            payload_length(&out)
        );
    }

    #[test]
    fn a_resize_command_moves_the_frame() {
        let (mut surface, _queue) = fixture(4, 2);
        let mut out = Vec::new();

        serve(
            br#"{"cmd":"resize","cols":8,"rows":3}
{"cmd":"render"}"#
                .as_slice(),
            &mut out,
            &mut surface,
        )
        .expect("serve");

        assert_eq!((width(&out), height(&out)), (64, 48));
    }

    #[test]
    fn an_idle_render_answers_with_a_zero_width_header() {
        let (mut surface, _queue) = fixture(4, 2);
        let mut painted = Vec::new();
        serve(br#"{"cmd":"render"}"#.as_slice(), &mut painted, &mut surface).expect("serve");
        assert_eq!(width(&painted), 32);

        let mut idle = Vec::new();
        serve(br#"{"cmd":"render"}"#.as_slice(), &mut idle, &mut surface).expect("serve");

        assert_eq!(idle.len(), FRAME_HEADER, "an idle answer is a header and nothing else");
        assert_eq!(kind(&idle), 0);
        assert_eq!(width(&idle), 0);
    }

    /// Writes nothing until it is flushed, like the buffered stdout a server
    /// runs on. What a host can see is the published half.
    #[derive(Default)]
    struct Buffered {
        pending: Vec<u8>,
        published: Vec<u8>,
    }

    impl std::io::Write for Buffered {
        fn write(&mut self, bytes: &[u8]) -> std::io::Result<usize> {
            self.pending.extend_from_slice(bytes);
            Ok(bytes.len())
        }

        fn flush(&mut self) -> std::io::Result<()> {
            self.published.append(&mut self.pending);
            Ok(())
        }
    }

    #[test]
    fn an_idle_answer_is_flushed_to_the_host() {
        let (mut surface, _queue) = fixture(4, 2);
        let mut out = Buffered::default();

        // The first render paints; the second has nothing to do, and its answer
        // is a zero-width header. The first must not hide the second.
        serve(br#"{"cmd":"render"}"#.as_slice(), &mut out, &mut surface).expect("serve");
        out.published.clear();
        out.flush().expect("flush");

        serve(br#"{"cmd":"render"}"#.as_slice(), &mut out, &mut surface).expect("serve");

        assert_eq!(out.published.len(), FRAME_HEADER, "an idle answer left the writer");
        assert_eq!(out.published.len() + out.pending.len(), FRAME_HEADER);
        assert_eq!(width(&out.published), 0);
    }

    #[test]
    fn a_change_after_an_idle_answer_sends_only_its_band() {
        let (mut surface, queue) = fixture(4, 8);
        let mut frames = Frames::new();
        let mut out = Vec::new();

        frames.write(&mut surface, &mut out, true).expect("a frame");
        assert_eq!(kind(&out), 0);

        out.clear();
        frames.write(&mut surface, &mut out, true).expect("no frame");
        assert_eq!(out.len(), FRAME_HEADER);

        // One row of text: the patch names its top, its width and its height.
        out.clear();
        queue.feed(b"hi");
        frames.write(&mut surface, &mut out, true).expect("a patch");

        assert_eq!(kind(&out), 2);
        assert_eq!(u32::from_be_bytes(out[4..8].try_into().expect("four bytes")), 0);
        assert_eq!(u32::from_be_bytes(out[8..12].try_into().expect("four bytes")), 32);
        // The fixture's glyph spills into the next cell row, so the band is a
        // couple of rows — far less than the eight-row pane.
        let rows = u32::from_be_bytes(out[12..16].try_into().expect("four bytes"));
        assert!((16..128).contains(&rows), "band of {rows} rows");
        assert_eq!(out.len(), FRAME_HEADER + payload_length(&out));
        assert_eq!(pixels_of(&out).len(), 32 * rows as usize * 4);
    }

    #[test]
    fn a_copy_command_answers_with_the_word_that_was_double_clicked() {
        let (mut surface, queue) = fixture(20, 2);
        queue.feed(b"hello world");
        let mut out = Vec::new();

        serve(
            br#"{"cmd":"press","col":7,"row":0}
{"cmd":"press","col":7,"row":0}
{"cmd":"copy"}"#
                .as_slice(),
            &mut out,
            &mut surface,
        )
        .expect("serve");

        assert_eq!(text_of(&out), "world");
    }

    #[test]
    fn a_drag_extends_the_selection_a_press_started() {
        let (mut surface, queue) = fixture(20, 2);
        queue.feed(b"hello world");
        let mut out = Vec::new();

        serve(
            br#"{"cmd":"press","col":1,"row":0}
{"cmd":"move","col":4,"row":0,"button":0}
{"cmd":"copy"}"#
                .as_slice(),
            &mut out,
            &mut surface,
        )
        .expect("serve");

        assert_eq!(text_of(&out), "ello");
    }

    #[test]
    fn a_press_reaches_a_program_that_tracks_the_mouse() {
        let (mut surface, queue) = fixture(20, 2);
        // DECSET 1000 turns the pointer into X10 reports at the cell.
        queue.feed(b"\x1b[?1000h");
        let mut out = Vec::new();

        serve(br#"{"cmd":"press","col":1,"row":0}"#.as_slice(), &mut out, &mut surface).expect("serve");

        let written = queue.take_written();
        assert!(written.starts_with(b"\x1b[M"), "{written:?}");
        assert!(out.is_empty(), "a pointer event is not a frame");
    }

    #[test]
    fn shift_keeps_the_pointer_for_the_host_even_while_the_program_tracks_it() {
        let (mut surface, queue) = fixture(20, 2);
        queue.feed(b"\x1b[?1000h\x1b[?25lhello");
        let mut out = Vec::new();

        serve(
            br#"{"cmd":"press","col":1,"row":0,"shift":true}
{"cmd":"copy"}"#
                .as_slice(),
            &mut out,
            &mut surface,
        )
        .expect("serve");

        // The program was told nothing; the host selected the cell instead.
        assert!(queue.take_written().is_empty(), "the program heard the press");
        assert_eq!(text_of(&out), "e");
    }

    #[test]
    fn a_quit_command_ends_the_loop() {
        let (mut surface, _queue) = fixture(4, 2);
        let mut out = Vec::new();

        serve(
            br#"{"cmd":"quit"}
{"cmd":"render"}"#
                .as_slice(),
            &mut out,
            &mut surface,
        )
        .expect("serve");

        assert!(out.is_empty(), "nothing after a quit may be served");
    }

    #[test]
    fn a_typed_key_goes_down_the_text_path() {
        let (mut surface, queue) = fixture(20, 2);

        serve(
            br#"{"cmd":"key","name":"a","text":"a","pressed":true}"#.as_slice(),
            &mut Vec::new(),
            &mut surface,
        )
        .expect("serve");

        assert_eq!(queue.take_written(), b"a");
    }

    #[test]
    fn a_control_key_and_an_arrow_are_encoded_by_the_terminal() {
        let (mut surface, queue) = fixture(20, 2);

        serve(
            br#"{"cmd":"key","name":"c","text":"c","ctrl":true,"pressed":true}
{"cmd":"key","name":"ArrowUp","text":"","pressed":true}"#
                .as_slice(),
            &mut Vec::new(),
            &mut surface,
        )
        .expect("serve");

        assert_eq!(queue.take_written(), b"\x03\x1b[A");
    }

    #[test]
    fn a_function_key_is_encoded() {
        let (mut surface, queue) = fixture(20, 2);

        serve(
            br#"{"cmd":"key","name":"F5","text":"","pressed":true}"#.as_slice(),
            &mut Vec::new(),
            &mut surface,
        )
        .expect("serve");

        assert!(!queue.take_written().is_empty(), "F5 encoded nothing");
    }

    #[test]
    fn a_typing_keys_release_is_not_sent_to_a_program_reading_characters() {
        let (mut surface, queue) = fixture(20, 2);

        serve(
            br#"{"cmd":"key","name":"a","text":"a","pressed":false}"#.as_slice(),
            &mut Vec::new(),
            &mut surface,
        )
        .expect("serve");

        assert!(queue.take_written().is_empty());
    }

    #[test]
    fn a_release_reaches_a_program_that_asked_for_every_key() {
        let (mut surface, queue) = fixture(20, 2);
        // What a game pushes: disambiguate, report event types, report all keys.
        queue.feed(b"\x1b[>11u");

        serve(
            br#"{"cmd":"key","name":"d","text":"d","pressed":true}
{"cmd":"key","name":"d","text":"d","pressed":false}"#
                .as_slice(),
            &mut Vec::new(),
            &mut surface,
        )
        .expect("serve");

        // The press and its release: a program that tracks a held key has no
        // other way to learn it was let go.
        assert_eq!(queue.take_written(), b"\x1b[100u\x1b[100;1:3u");
    }

    #[test]
    fn a_physical_key_becomes_a_win32_record_the_program_can_hold() {
        let (mut surface, queue) = fixture(20, 2);
        // A ConPTY asks for Win32 input records before its client runs.
        queue.feed(b"\x1b[?9001h");

        serve(
            br#"{"cmd":"key","name":"d","code":"KeyD","text":"d","pressed":true}
{"cmd":"key","name":"d","code":"KeyD","text":"d","pressed":false}"#
                .as_slice(),
            &mut Vec::new(),
            &mut surface,
        )
        .expect("serve");

        // Vk 0x44 and scan 0x20 with the character going down, then the release
        // carrying no character: a text byte instead would be one frame of
        // movement.
        assert_eq!(queue.take_written(), b"\x1b[68;32;100;1;0;1_\x1b[68;32;0;0;0;1_");
    }

    #[test]
    fn a_modes_command_answers_with_what_the_program_asked_for() {
        let (mut surface, queue) = fixture(20, 2);
        // The game's push: disambiguate, report event types, report all keys.
        queue.feed(b"\x1b[>11u");
        let mut out = Vec::new();

        serve(br#"{"cmd":"modes"}"#.as_slice(), &mut out, &mut surface).expect("serve");

        assert_eq!(kind(&out), 3, "a modes answer is a diagnostic, not a frame");
        let length = u32::from_be_bytes(out[4..8].try_into().expect("four bytes")) as usize;
        let text = String::from_utf8(out[8..8 + length].to_vec()).expect("utf-8");
        assert!(text.contains("kitty_flags: 11"), "{text}");
    }

    #[test]
    fn a_paste_goes_through_the_bracketed_filter() {
        let (mut surface, queue) = fixture(20, 2);
        // DECSET 2004 asks the terminal to bracket a paste.
        queue.feed(b"\x1b[?2004h");

        serve(
            br#"{"cmd":"paste","text":"two words"}"#.as_slice(),
            &mut Vec::new(),
            &mut surface,
        )
        .expect("serve");

        assert_eq!(queue.take_written(), b"\x1b[200~two words\x1b[201~");
    }

    #[test]
    fn the_encoding_follows_the_programs_keyboard_flags() {
        let (mut surface, queue) = fixture(20, 2);
        // The kitty protocol's push with "disambiguate" set.
        queue.feed(b"\x1b[>1u");

        serve(
            br#"{"cmd":"key","name":"ArrowUp","text":"","pressed":true}"#.as_slice(),
            &mut Vec::new(),
            &mut surface,
        )
        .expect("serve");

        let written = queue.take_written();
        assert!(written.starts_with(b"\x1b["), "{written:?}");
        assert_ne!(written, b"\x1b[A", "the legacy encoding was used: {written:?}");
    }
}
