//! Winter's terminal as a Node addon, for an editor whose own process hosts it.
//!
//! ADR 083 put the terminal behind a process boundary and a JSON line protocol,
//! because that is what an editor extension has without shipping native code.
//! ADR 084 takes the other trade: the addon is built per platform, and in
//! exchange the editor calls the surface directly — no second process, no
//! protocol, no framing.
//!
//! The API is the surface's own: input methods return nothing, and `render`
//! returns the pixels that changed, or `null` when nothing did. Banding, the
//! font, the pty and the encoder are all in [`winter`]; what lives here is the
//! translation to JavaScript's types.

// The NAPI glue is Windows-style FFI: `napi-derive` generates `extern "C"`
// entry points, the class table and the value conversions, and none of it can
// be written without `unsafe`. This crate exists so that is the only place it
// appears in a host (ADR 084); every type it touches is safe.
#![allow(unsafe_code)]

use std::path::PathBuf;
use std::sync::{Mutex, MutexGuard};
use std::time::Instant;

use napi::bindgen_prelude::Buffer;
use napi::{Error, Result, Status};
use napi_derive::napi;
use server::pty::{PtyCommand, PtySize};
use winter::embed::Surface;
use winter::session::Session;
use winter::term_events::TerminalEvent;
use winter_demo::{Frames, KeyInput};

/// The face size a pane measures the system font at unless told otherwise.
const FONT_SIZE: f32 = 14.0;

/// The shell this platform starts when a host names none, for a host whose own
/// process is the editor's (ADR 084).
#[napi]
pub fn default_shell() -> String {
    winter_demo::default_shell().to_string_lossy().into_owned()
}

/// One terminal: a shell on a pty, and the surface that draws it.
///
/// A JS class, because the editor holds one per pane and drops it when the pane
/// closes. Dropping it kills the shell.
#[napi]
pub struct Terminal {
    // `Option` so `close` can take the surface and drop it, which is what ends
    // the shell; the mutex so JS can call from any callback without a borrow
    // panic.
    inner: Mutex<Option<Inner>>,
}

struct Inner {
    surface: Surface,
    frames: Frames,
    /// When the cursor's blink started, so the phase is the pane's own.
    started: Instant,
}

/// The pixels that changed, and where they belong.
#[napi(object)]
pub struct Frame {
    /// `true` for a band of the pane, `false` for the whole of it.
    pub patch: bool,
    /// The first pixel row this payload covers.
    pub top: u32,
    /// The payload's width in pixels.
    pub width: u32,
    /// How many pixel rows the payload covers.
    pub height: u32,
    /// Deflated RGBA, to inflate before painting.
    pub pixels: Buffer,
}

/// A grid size in cells.
#[napi(object)]
pub struct Grid {
    /// Columns.
    pub columns: i32,
    /// Rows.
    pub rows: i32,
}

/// A cell size in pixels.
#[napi(object)]
pub struct Cell {
    /// Pixels across.
    pub width: u32,
    /// Pixels down.
    pub height: u32,
}

/// Something the program asked the terminal to report.
#[napi(object)]
pub struct EventReport {
    /// `bell`, `notification` or `exited`.
    pub kind: String,
    /// A notification's title.
    pub title: Option<String>,
    /// A notification's body.
    pub body: Option<String>,
}

#[napi]
impl Terminal {
    /// Starts `shell` on a pty, measured at `fontSize` points.
    ///
    /// The font comes from the system's font directories, as the shell's own
    /// panes do; a machine without a monospace face is an error rather than a
    /// pane that draws nothing.
    #[napi(factory)]
    pub fn spawn(shell: String, columns: i32, rows: i32, font_size: Option<f64>) -> Result<Self> {
        let points = font_size.map_or(FONT_SIZE, |value| value as f32);
        let setup = winter_demo::measure(points, columns, rows).ok_or_else(|| {
            Error::new(
                Status::GenericFailure,
                "no monospace face was found in the font directories".to_string(),
            )
        })?;

        let size = PtySize::new(cells(columns), cells(rows));
        let session = Session::spawn(&PtyCommand::new(PathBuf::from(&shell)), size).map_err(failed)?;
        let surface = Surface::new(setup, Box::new(session));

        Ok(Self {
            inner: Mutex::new(Some(Inner {
                surface,
                frames: Frames::new(),
                started: Instant::now(),
            })),
        })
    }

    /// Sends text to the program, as if the user had typed it.
    #[napi]
    pub fn write(&self, text: String) {
        self.with_surface(|surface| surface.write(&text));
    }

    /// Pastes text through the terminal's bracketed-paste filter.
    #[napi]
    pub fn paste(&self, text: String) {
        self.with_surface(|surface| surface.paste(&text));
    }

    /// Translates one toolkit key and sends what it encodes.
    ///
    /// `name` is the toolkit's name for the key, `code` its physical name (the
    /// DOM's `KeyboardEvent.code`), and `text` what it typed. The terminal
    /// decides which of the three the program wants (ADR 056, 084).
    #[napi]
    #[allow(clippy::too_many_arguments)]
    pub fn key(
        &self,
        name: String,
        text: String,
        code: String,
        shift: bool,
        ctrl: bool,
        alt: bool,
        pressed: bool,
        repeat: bool,
    ) {
        let input = KeyInput {
            name: &name,
            text: &text,
            code: &code,
            shift,
            ctrl,
            alt,
            pressed,
            repeat,
        };
        if let Some(event) = winter_demo::key_event(&input) {
            self.with_surface(|surface| surface.key(&event));
        }
    }

    /// Resizes the terminal, the grid and the frame.
    #[napi]
    pub fn resize(&self, columns: i32, rows: i32) -> Result<()> {
        self.with_surface_result(|surface| surface.resize(columns, rows).map_err(failed))
    }

    /// A button press at a cell: to the program if it tracks the mouse, else the
    /// start of a selection.
    #[napi]
    pub fn press(&self, column: i32, row: i32, button: i32, shift: bool) {
        self.with_surface(|surface| winter_demo::pointer_press(surface, (column, row), button, shift));
    }

    /// A pointer move, with the button held or -1 for none.
    #[napi(js_name = "move")]
    pub fn move_pointer(&self, column: i32, row: i32, button: i32, shift: bool) {
        self.with_surface(|surface| winter_demo::pointer_move(surface, (column, row), button, shift));
    }

    /// A button release at a cell.
    #[napi]
    pub fn release(&self, column: i32, row: i32, button: i32, shift: bool) {
        self.with_surface(|surface| winter_demo::pointer_release(surface, (column, row), button, shift));
    }

    /// A wheel turn at a cell; positive `lines` scrolls into the history.
    #[napi]
    pub fn wheel(&self, column: i32, row: i32, lines: i32, shift: bool) {
        self.with_surface(|surface| winter_demo::pointer_wheel(surface, (column, row), lines, shift));
    }

    /// The selected text, for the clipboard.
    #[napi]
    pub fn copy(&self) -> Option<String> {
        self.with_surface(|surface| surface.selection_text()).flatten()
    }

    /// What the program asked of the input path, for a bug report.
    #[napi]
    pub fn modes(&self) -> Option<String> {
        self.with_surface(|surface| format!("{:?}", surface.input_modes()))
    }

    /// Paints and returns what changed, or `null` when nothing did.
    #[napi]
    pub fn render(&self) -> Result<Option<Frame>> {
        let mut guard = self.lock();
        let Some(inner) = guard.as_mut() else {
            return Ok(None);
        };
        let cursor = winter_demo::cursor_on(&inner.surface, inner.started, Instant::now());
        let painted = inner.frames.paint(&mut inner.surface, cursor).map_err(failed)?;
        Ok(painted.map(|painted| Frame {
            patch: !painted.full,
            top: painted.top,
            width: painted.width,
            height: painted.rows,
            pixels: painted.pixels.into(),
        }))
    }

    /// The grid size in cells.
    #[napi]
    pub fn size(&self) -> Grid {
        let (columns, rows) = self.with_surface(|surface| surface.size()).unwrap_or((0, 0));
        Grid { columns, rows }
    }

    /// The cell size in pixels, which is what a host turns a pane into cells.
    #[napi]
    pub fn cell_size(&self) -> Cell {
        let (width, height) = self.with_surface(|surface| surface.cell_size()).unwrap_or((0, 0));
        Cell {
            width: u32::from(width),
            height: u32::from(height),
        }
    }

    /// Whether the cursor is drawn at all.
    #[napi]
    pub fn cursor_visible(&self) -> bool {
        self.with_surface(|surface| surface.cursor_visible()).unwrap_or(false)
    }

    /// The title the program set, if it set one.
    #[napi]
    pub fn title(&self) -> Option<String> {
        self.with_surface(|surface| surface.title().map(str::to_string))
            .flatten()
    }

    /// Takes what the program asked the terminal to report since the last call.
    #[napi]
    pub fn take_events(&self) -> Vec<EventReport> {
        self.with_surface(|surface| surface.take_events(Instant::now()))
            .unwrap_or_default()
            .into_iter()
            .map(|event| match event {
                TerminalEvent::Bell => EventReport {
                    kind: "bell".to_string(),
                    title: None,
                    body: None,
                },
                TerminalEvent::Notification { title, body } => EventReport {
                    kind: "notification".to_string(),
                    title: Some(title),
                    body: Some(body),
                },
                TerminalEvent::Exited => EventReport {
                    kind: "exited".to_string(),
                    title: None,
                    body: None,
                },
            })
            .collect()
    }

    /// Ends the shell and releases the pty.
    ///
    /// Dropping the surface is what kills the client, so this is the same thing
    /// a closed pane does, only sooner.
    #[napi]
    pub fn close(&self) {
        if let Some(mut inner) = self.lock().take() {
            inner.surface.close();
        }
    }
}

impl Terminal {
    fn lock(&self) -> MutexGuard<'_, Option<Inner>> {
        // A panic in a JS call must not turn every later call into one, so a
        // poisoned lock is taken rather than propagated.
        self.inner.lock().unwrap_or_else(|poisoned| poisoned.into_inner())
    }

    /// Runs `act` on the surface, or returns `None` once the terminal is closed.
    fn with_surface<R>(&self, act: impl FnOnce(&mut Surface) -> R) -> Option<R> {
        let mut guard = self.lock();
        let inner = guard.as_mut()?;
        Some(act(&mut inner.surface))
    }

    /// Runs `act`, or fails when the terminal has already been closed.
    fn with_surface_result<R>(&self, act: impl FnOnce(&mut Surface) -> Result<R>) -> Result<R> {
        let mut guard = self.lock();
        let inner = guard
            .as_mut()
            .ok_or_else(|| Error::new(Status::GenericFailure, "the terminal is closed".to_string()))?;
        act(&mut inner.surface)
    }
}

/// A cell count as the pty layer wants it, never zero.
fn cells(count: i32) -> u16 {
    u16::try_from(count.max(1)).unwrap_or(u16::MAX)
}

/// A failure as the error a JS caller sees.
fn failed(error: impl std::fmt::Display) -> Error {
    Error::new(Status::GenericFailure, error.to_string())
}
