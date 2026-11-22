//! `TerminalInput` — mouse events to VT input sequences.
//!
//! Ported from `src/terminal/input/mouseInput.cpp`. The Win32 `WM_*` message
//! codes become [`MouseEventKind`] (ADR 009); the sequences produced are
//! upstream's.
//!
//! One deliberate divergence: the UTF-8 encoding writes coordinates as UTF-8
//! characters. Upstream writes UTF-16 units because its input stream is UTF-16;
//! this terminal writes UTF-8 to a pty, so a coordinate above 0x7F occupies the
//! bytes an application expects from the xterm mode.

use crate::input::terminal_input::{Key, Modifiers, TerminalInput};
use types::InputModes;

/// The largest coordinate the default encoding can carry: 95 + 32 + 1 is the
/// first value that is no longer ASCII.
const MAX_DEFAULT_COORDINATE: i32 = 94;

/// The wheel delta that makes up one line, as `WHEEL_DELTA` does on Windows.
///
/// A host with a smooth-scrolling wheel converts its distance to this unit, so
/// the accumulation below is unchanged.
pub const WHEEL_DELTA: i32 = 120;

/// A mouse button.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum MouseButton {
    /// The left button.
    Left,
    /// The middle button.
    Middle,
    /// The right button.
    Right,
}

/// Which axis a wheel event turned.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum WheelAxis {
    /// The vertical wheel.
    Vertical,
    /// A horizontal wheel or tilt.
    Horizontal,
}

/// What happened to the mouse.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum MouseEventKind {
    /// The pointer moved.
    Move,
    /// A button went down.
    Press(MouseButton),
    /// A button came up.
    Release(MouseButton),
    /// A button was double-clicked.
    DoubleClick(MouseButton),
    /// The wheel turned.
    Wheel(WheelAxis),
}

impl MouseEventKind {
    /// Returns whether the kind is a button event, wheel included.
    const fn is_button(self) -> bool {
        !matches!(self, Self::Move)
    }

    /// Returns whether the kind is a pointer move.
    const fn is_hover(self) -> bool {
        matches!(self, Self::Move)
    }

    /// Returns whether the kind is a wheel turn.
    const fn is_wheel(self) -> bool {
        matches!(self, Self::Wheel(_))
    }

    /// Returns whether the kind releases a button.
    const fn is_release(self) -> bool {
        matches!(self, Self::Release(_))
    }
}

/// Which buttons are held down during an event.
#[derive(Debug, Clone, Copy, PartialEq, Eq, Default)]
pub struct MouseButtons {
    /// The left button is held.
    pub left: bool,
    /// The middle button is held.
    pub middle: bool,
    /// The right button is held.
    pub right: bool,
}

impl MouseButtons {
    /// Returns the first held button, in the order upstream tests them.
    const fn pressed(self) -> Option<MouseButton> {
        if self.left {
            Some(MouseButton::Left)
        } else if self.middle {
            Some(MouseButton::Middle)
        } else if self.right {
            Some(MouseButton::Right)
        } else {
            None
        }
    }
}

/// A mouse event in zero-based cell coordinates.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub struct MouseEvent {
    /// The cell the pointer is over, `(column, row)`.
    pub position: (i32, i32),
    /// What happened.
    pub kind: MouseEventKind,
    /// Which buttons were held at the time.
    pub buttons: MouseButtons,
    /// The modifier keys held at the time.
    pub modifiers: Modifiers,
    /// The wheel distance, in [`WHEEL_DELTA`] units. Zero for anything else.
    pub delta: i32,
}

impl MouseEvent {
    /// Returns a button press at `position` with no modifiers and no wheel.
    pub fn press(position: (i32, i32), button: MouseButton) -> Self {
        Self {
            position,
            kind: MouseEventKind::Press(button),
            buttons: MouseButtons::default(),
            modifiers: Modifiers::default(),
            delta: 0,
        }
    }

    /// Returns a button release at `position`.
    pub fn release(position: (i32, i32), button: MouseButton) -> Self {
        Self {
            kind: MouseEventKind::Release(button),
            ..Self::press(position, button)
        }
    }

    /// Returns a pointer move at `position`.
    pub fn moved(position: (i32, i32)) -> Self {
        Self {
            kind: MouseEventKind::Move,
            ..Self::press(position, MouseButton::Left)
        }
    }

    /// Returns a wheel turn at `position`.
    pub fn wheel(position: (i32, i32), axis: WheelAxis, delta: i32) -> Self {
        Self {
            kind: MouseEventKind::Wheel(axis),
            delta,
            ..Self::press(position, MouseButton::Left)
        }
    }
}

/// The state hover reporting needs between events.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub(super) struct MouseState {
    last_position: (i32, i32),
    last_kind: Option<MouseEventKind>,
    accumulated_delta: i32,
    /// The tracking modes the state was recorded under.
    tracking: (bool, bool, bool),
}

impl MouseState {
    pub(super) const fn new() -> Self {
        Self {
            last_position: (-1, -1),
            last_kind: None,
            accumulated_delta: 0,
            tracking: (false, false, false),
        }
    }

    fn reset_position(&mut self) {
        self.last_position = (-1, -1);
        self.last_kind = None;
    }
}

impl Default for MouseState {
    fn default() -> Self {
        Self::new()
    }
}

impl TerminalInput {
    /// Returns whether any mouse tracking mode is enabled.
    pub fn is_tracking_mouse_input(&self, modes: &InputModes) -> bool {
        modes.is_tracking_mouse()
    }

    /// Returns whether a wheel event should move the cursor instead.
    pub fn should_send_alternate_scroll(&self, event: &MouseEvent, modes: &InputModes) -> bool {
        let wheel = event.kind.is_wheel() && event.delta != 0;
        modes.in_alternate_buffer && modes.alternate_scroll && wheel
    }

    /// Translates a mouse event, or returns `None` when it is not reported.
    ///
    /// `Some("")` means the event was consumed without producing bytes, which
    /// is how a partly accumulated wheel turn is handled.
    pub fn handle_mouse(&mut self, event: &MouseEvent, modes: &InputModes) -> Option<String> {
        // Upstream clears the saved position whenever a tracking mode is set.
        // The adapter owns the modes, so the equivalent is a change of the
        // tracking configuration between two events.
        let tracking = (modes.default_mouse, modes.button_event_mouse, modes.any_event_mouse);
        if tracking != self.mouse.tracking {
            self.mouse.tracking = tracking;
            self.mouse.reset_position();
        }

        // Only accumulate wheel deltas with the same sign; a non-wheel event
        // has a delta of zero and therefore always resets the total.
        if event.delta.signum() != self.mouse.accumulated_delta.signum() {
            self.mouse.accumulated_delta = 0;
        }

        if event.kind.is_wheel() {
            self.mouse.accumulated_delta += event.delta;
            if self.mouse.accumulated_delta.abs() < WHEEL_DELTA {
                // A fraction of a line: consumed, but nothing to send yet.
                return if modes.is_tracking_mouse() || self.should_send_alternate_scroll(event, modes) {
                    Some(String::new())
                } else {
                    None
                };
            }
            self.mouse.accumulated_delta = 0;
        }

        if modes.is_tracking_mouse() {
            let hover = event.kind.is_hover();
            let same_cell = event.position == self.mouse.last_position && self.mouse.last_kind == Some(event.kind);

            // A move does not say which button is held, so the held set does.
            let real_button = if hover { event.buttons.pressed() } else { None };
            let physical_button_pressed = real_button.is_some();

            let report_motion =
                hover && !same_cell && ((modes.button_event_mouse && physical_button_pressed) || modes.any_event_mouse);

            if event.kind.is_button() || report_motion {
                if modes.button_event_mouse || modes.any_event_mouse {
                    self.mouse.last_position = event.position;
                    self.mouse.last_kind = Some(event.kind);
                }

                let kind = effective_kind(event.kind, real_button);
                if modes.utf8_mouse {
                    return generate_utf8(event, kind, modes);
                }
                if modes.sgr_mouse {
                    return generate_sgr(event, kind, modes);
                }
                return generate_default(event, kind, modes);
            }
        }

        if self.should_send_alternate_scroll(event, modes) {
            return self.alternate_scroll(event, modes);
        }

        None
    }

    /// Turns a wheel turn into the cursor keys the alternate screen uses.
    fn alternate_scroll(&self, event: &MouseEvent, modes: &InputModes) -> Option<String> {
        let key = match event.kind {
            MouseEventKind::Wheel(WheelAxis::Vertical) => {
                if event.delta > 0 {
                    Key::Up
                } else {
                    Key::Down
                }
            }
            MouseEventKind::Wheel(WheelAxis::Horizontal) => {
                if event.delta > 0 {
                    Key::Right
                } else {
                    Key::Left
                }
            }
            _ => return None,
        };
        self.encode_cursor_key(key, modes)
    }
}

/// Returns the button the encoders should report for a hover.
///
/// A move with no button held is a bare hover: upstream passes its
/// `WM_*BUTTONUP` sentinel to the X encoder and `WM_MOUSEMOVE` to the SGR one,
/// and both encode as 3.
fn effective_kind(kind: MouseEventKind, real_button: Option<MouseButton>) -> MouseEventKind {
    if kind.is_hover() {
        real_button.map_or(MouseEventKind::Move, MouseEventKind::Press)
    } else {
        kind
    }
}

/// Translates a button into the X10 button byte.
fn x_button(kind: MouseEventKind, event: &MouseEvent) -> u32 {
    let mut value = match kind {
        MouseEventKind::Press(MouseButton::Left) | MouseEventKind::DoubleClick(MouseButton::Left) => 0,
        MouseEventKind::Release(_) | MouseEventKind::Move => 3,
        MouseEventKind::Press(MouseButton::Right) | MouseEventKind::DoubleClick(MouseButton::Right) => 2,
        MouseEventKind::Press(MouseButton::Middle) | MouseEventKind::DoubleClick(MouseButton::Middle) => 1,
        MouseEventKind::Wheel(WheelAxis::Vertical) => {
            if event.delta > 0 {
                0x40
            } else {
                0x41
            }
        }
        MouseEventKind::Wheel(WheelAxis::Horizontal) => {
            if event.delta > 0 {
                0x43
            } else {
                0x42
            }
        }
    };

    if event.kind.is_hover() {
        value += 0x20;
    }
    value |= modifier_bits(event.modifiers);
    value
}

/// Translates a button into the SGR button number.
///
/// Unlike the X encoding, a release keeps the number of the button it released;
/// 3 is reserved for a hover with no button held.
fn sgr_button(kind: MouseEventKind, event: &MouseEvent) -> u32 {
    let mut value = match kind {
        MouseEventKind::Press(MouseButton::Left)
        | MouseEventKind::DoubleClick(MouseButton::Left)
        | MouseEventKind::Release(MouseButton::Left) => 0,
        MouseEventKind::Press(MouseButton::Middle)
        | MouseEventKind::DoubleClick(MouseButton::Middle)
        | MouseEventKind::Release(MouseButton::Middle) => 1,
        MouseEventKind::Press(MouseButton::Right)
        | MouseEventKind::DoubleClick(MouseButton::Right)
        | MouseEventKind::Release(MouseButton::Right) => 2,
        MouseEventKind::Move => 3,
        MouseEventKind::Wheel(WheelAxis::Vertical) => {
            if event.delta > 0 {
                0x40
            } else {
                0x41
            }
        }
        MouseEventKind::Wheel(WheelAxis::Horizontal) => {
            if event.delta > 0 {
                0x43
            } else {
                0x42
            }
        }
    };

    if event.kind.is_hover() {
        value += 0x20;
    }
    value |= modifier_bits(event.modifiers);
    value
}

/// The X11 modifier bits: shift, meta and ctrl.
fn modifier_bits(modifiers: Modifiers) -> u32 {
    (u32::from(modifiers.shift) * 0x04) | (u32::from(modifiers.alt()) * 0x08) | (u32::from(modifiers.ctrl()) * 0x10)
}

/// Generates the default encoding, which cannot express coordinates above 94.
fn generate_default(event: &MouseEvent, kind: MouseEventKind, modes: &InputModes) -> Option<String> {
    let (x, y) = event.position;
    if x > MAX_DEFAULT_COORDINATE || y > MAX_DEFAULT_COORDINATE {
        return None;
    }

    Some(format!(
        "{}M{}{}{}",
        introducer(modes),
        char::from_u32(x_button(kind, event) + 0x20)?,
        char::from_u32((x + 33) as u32)?,
        char::from_u32((y + 33) as u32)?,
    ))
}

/// Generates the UTF-8 encoding, whose coordinates are characters.
fn generate_utf8(event: &MouseEvent, kind: MouseEventKind, modes: &InputModes) -> Option<String> {
    let (x, y) = event.position;
    Some(format!(
        "{}M{}{}{}",
        introducer(modes),
        char::from_u32(x_button(kind, event) + 0x20)?,
        char::from_u32((x + 33) as u32)?,
        char::from_u32((y + 33) as u32)?,
    ))
}

/// Generates the SGR encoding, which names the release explicitly.
fn generate_sgr(event: &MouseEvent, kind: MouseEventKind, modes: &InputModes) -> Option<String> {
    let (x, y) = event.position;
    Some(format!(
        "{}<{};{};{}{}",
        introducer(modes),
        sgr_button(kind, event),
        x + 1,
        y + 1,
        if event.kind.is_release() { 'm' } else { 'M' },
    ))
}

fn introducer(modes: &InputModes) -> &'static str {
    if modes.send_c1 { "\u{9b}" } else { "\x1b[" }
}

#[cfg(test)]
mod tests {
    use super::*;

    fn tracking() -> InputModes {
        InputModes {
            default_mouse: true,
            ..InputModes::default()
        }
    }

    fn any_event() -> InputModes {
        InputModes {
            any_event_mouse: true,
            ..InputModes::default()
        }
    }

    fn sgr() -> InputModes {
        InputModes {
            default_mouse: true,
            sgr_mouse: true,
            ..InputModes::default()
        }
    }

    fn encode(event: &MouseEvent, modes: &InputModes) -> Option<String> {
        TerminalInput::new().handle_mouse(event, modes)
    }

    #[test]
    fn a_left_press_is_the_default_sequence() {
        let event = MouseEvent::press((2, 3), MouseButton::Left);
        assert_eq!(encode(&event, &tracking()).as_deref(), Some("\x1b[M #$"));
    }

    #[test]
    fn the_middle_and_right_buttons_have_their_own_numbers() {
        let middle = MouseEvent::press((0, 0), MouseButton::Middle);
        assert_eq!(encode(&middle, &tracking()).as_deref(), Some("\x1b[M!!!"));
        let right = MouseEvent::press((0, 0), MouseButton::Right);
        assert_eq!(encode(&right, &tracking()).as_deref(), Some("\x1b[M\"!!"));
    }

    #[test]
    fn a_release_is_three_in_the_default_encoding() {
        let event = MouseEvent::release((0, 0), MouseButton::Left);
        assert_eq!(encode(&event, &tracking()).as_deref(), Some("\x1b[M#!!"));
    }

    #[test]
    fn sgr_names_the_button_and_the_release() {
        let press = MouseEvent::press((2, 3), MouseButton::Left);
        assert_eq!(encode(&press, &sgr()).as_deref(), Some("\x1b[<0;3;4M"));

        let release = MouseEvent::release((2, 3), MouseButton::Left);
        assert_eq!(encode(&release, &sgr()).as_deref(), Some("\x1b[<0;3;4m"));

        let right = MouseEvent::press((0, 0), MouseButton::Right);
        assert_eq!(encode(&right, &sgr()).as_deref(), Some("\x1b[<2;1;1M"));
        let middle = MouseEvent::press((0, 0), MouseButton::Middle);
        assert_eq!(encode(&middle, &sgr()).as_deref(), Some("\x1b[<1;1;1M"));
    }

    #[test]
    fn the_default_tracking_mode_reports_no_motion() {
        assert_eq!(encode(&MouseEvent::moved((1, 1)), &tracking()), None);
    }

    #[test]
    fn the_any_event_mode_reports_motion_once_per_cell() {
        let modes = any_event();
        let mut input = TerminalInput::new();

        assert_eq!(
            input.handle_mouse(&MouseEvent::moved((0, 0)), &modes).as_deref(),
            Some("\x1b[MC!!")
        );
        assert_eq!(input.handle_mouse(&MouseEvent::moved((0, 0)), &modes), None);
        assert_eq!(
            input.handle_mouse(&MouseEvent::moved((1, 0)), &modes).as_deref(),
            Some("\x1b[MC\"!")
        );
    }

    #[test]
    fn the_button_event_mode_reports_only_drags() {
        let modes = InputModes {
            button_event_mouse: true,
            ..InputModes::default()
        };
        assert_eq!(encode(&MouseEvent::moved((1, 1)), &modes), None);

        let drag = MouseEvent {
            buttons: MouseButtons {
                left: true,
                ..MouseButtons::default()
            },
            ..MouseEvent::moved((1, 1))
        };
        assert_eq!(encode(&drag, &modes).as_deref(), Some("\x1b[M@\"\""));
    }

    #[test]
    fn a_sgr_drag_reports_the_held_button_with_the_motion_bit() {
        let modes = InputModes {
            button_event_mouse: true,
            sgr_mouse: true,
            ..InputModes::default()
        };
        let drag = MouseEvent {
            buttons: MouseButtons {
                left: true,
                ..MouseButtons::default()
            },
            ..MouseEvent::moved((2, 3))
        };
        assert_eq!(encode(&drag, &modes).as_deref(), Some("\x1b[<32;3;4M"));
    }

    #[test]
    fn a_wheel_turn_accumulates_until_one_line_is_reached() {
        let modes = tracking();
        let mut input = TerminalInput::new();

        assert_eq!(
            input
                .handle_mouse(&MouseEvent::wheel((0, 0), WheelAxis::Vertical, 60), &modes)
                .as_deref(),
            Some("")
        );
        assert_eq!(
            input
                .handle_mouse(&MouseEvent::wheel((0, 0), WheelAxis::Vertical, 60), &modes)
                .as_deref(),
            Some("\x1b[M`!!")
        );
        // The opposite sign starts a new total.
        assert_eq!(
            input
                .handle_mouse(&MouseEvent::wheel((0, 0), WheelAxis::Vertical, -120), &modes)
                .as_deref(),
            Some("\x1b[Ma!!")
        );
    }

    #[test]
    fn horizontal_wheels_use_their_own_numbers() {
        assert_eq!(
            encode(&MouseEvent::wheel((0, 0), WheelAxis::Horizontal, 120), &tracking()).as_deref(),
            Some("\x1b[Mc!!")
        );
        assert_eq!(
            encode(&MouseEvent::wheel((0, 0), WheelAxis::Horizontal, -120), &tracking()).as_deref(),
            Some("\x1b[Mb!!")
        );
    }

    #[test]
    fn alternate_scroll_turns_the_wheel_into_cursor_keys() {
        let modes = InputModes {
            in_alternate_buffer: true,
            ..InputModes::default()
        };
        assert_eq!(
            encode(&MouseEvent::wheel((0, 0), WheelAxis::Vertical, 120), &modes).as_deref(),
            Some("\x1b[A")
        );
        assert_eq!(
            encode(&MouseEvent::wheel((0, 0), WheelAxis::Vertical, -120), &modes).as_deref(),
            Some("\x1b[B")
        );
        assert_eq!(
            encode(&MouseEvent::wheel((0, 0), WheelAxis::Horizontal, 120), &modes).as_deref(),
            Some("\x1b[C")
        );

        let application = InputModes {
            in_alternate_buffer: true,
            cursor_key: true,
            ..InputModes::default()
        };
        assert_eq!(
            encode(&MouseEvent::wheel((0, 0), WheelAxis::Vertical, 120), &application).as_deref(),
            Some("\x1bOA")
        );
    }

    #[test]
    fn the_default_encoding_stops_at_column_94() {
        assert!(encode(&MouseEvent::press((94, 0), MouseButton::Left), &tracking()).is_some());
        assert_eq!(
            encode(&MouseEvent::press((95, 0), MouseButton::Left), &tracking()),
            None
        );
    }

    #[test]
    fn the_utf8_encoding_carries_larger_coordinates() {
        let modes = InputModes {
            default_mouse: true,
            utf8_mouse: true,
            ..InputModes::default()
        };
        assert_eq!(
            encode(&MouseEvent::press((200, 10), MouseButton::Left), &modes).as_deref(),
            Some("\x1b[M é+")
        );
    }

    #[test]
    fn modifiers_set_the_x11_bits() {
        let shift = Modifiers {
            shift: true,
            ..Modifiers::default()
        };
        let event = MouseEvent {
            modifiers: shift,
            ..MouseEvent::press((0, 0), MouseButton::Left)
        };
        assert_eq!(encode(&event, &tracking()).as_deref(), Some("\x1b[M$!!"));

        let alt = Modifiers {
            left_alt: true,
            ..Modifiers::default()
        };
        let event = MouseEvent {
            modifiers: alt,
            ..MouseEvent::press((0, 0), MouseButton::Left)
        };
        assert_eq!(encode(&event, &tracking()).as_deref(), Some("\x1b[M(!!"));

        let ctrl = Modifiers {
            right_ctrl: true,
            ..Modifiers::default()
        };
        let event = MouseEvent {
            modifiers: ctrl,
            ..MouseEvent::press((0, 0), MouseButton::Left)
        };
        assert_eq!(encode(&event, &tracking()).as_deref(), Some("\x1b[M0!!"));
    }

    #[test]
    fn the_tracking_modes_decide_between_alternate_scroll_and_reporting() {
        let input = TerminalInput::new();
        assert!(!input.is_tracking_mouse_input(&InputModes::default()));
        assert!(input.is_tracking_mouse_input(&tracking()));
        assert!(input.is_tracking_mouse_input(&any_event()));
    }

    #[test]
    fn a_wheel_turn_without_any_mode_is_not_consumed() {
        assert_eq!(
            encode(
                &MouseEvent::wheel((0, 0), WheelAxis::Vertical, 120),
                &InputModes::default()
            ),
            None
        );
    }
}
