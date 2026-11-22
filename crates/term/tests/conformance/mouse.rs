//! Mouse tracking modes (xterm's control sequences, DECELR family).

use crate::support::Screen;
use term::input::Modifiers;
use term::input::{MouseButton, MouseButtons, MouseEvent, WheelAxis};

fn screen() -> Screen {
    Screen::new(20, 5)
}

#[test]
fn no_tracking_mode_means_no_reporting() {
    let mut screen = screen();
    let press = MouseEvent::press((2, 3), MouseButton::Left);
    assert_eq!(screen.terminal().handle_mouse(&press), None);
}

#[test]
fn mode_1000_reports_presses_and_releases() {
    let mut screen = screen();
    screen.feed_str("\x1b[?1000h");

    let press = MouseEvent::press((2, 3), MouseButton::Left);
    assert_eq!(screen.terminal().handle_mouse(&press).as_deref(), Some("\x1b[M #$"));

    let release = MouseEvent::release((2, 3), MouseButton::Left);
    assert_eq!(screen.terminal().handle_mouse(&release).as_deref(), Some("\x1b[M##$"));

    // Motion is not reported by this mode.
    assert_eq!(screen.terminal().handle_mouse(&MouseEvent::moved((4, 4))), None);
}

#[test]
fn mode_1002_reports_drags_but_not_bare_motion() {
    let mut screen = screen();
    screen.feed_str("\x1b[?1002h");

    assert_eq!(screen.terminal().handle_mouse(&MouseEvent::moved((1, 1))), None);

    let drag = MouseEvent {
        buttons: MouseButtons {
            left: true,
            ..MouseButtons::default()
        },
        ..MouseEvent::moved((1, 1))
    };
    assert_eq!(screen.terminal().handle_mouse(&drag).as_deref(), Some("\x1b[M@\"\""));
}

#[test]
fn mode_1006_selects_the_sgr_encoding() {
    let mut screen = screen();
    screen.feed_str("\x1b[?1000h\x1b[?1006h");

    let press = MouseEvent::press((2, 3), MouseButton::Right);
    assert_eq!(screen.terminal().handle_mouse(&press).as_deref(), Some("\x1b[<2;3;4M"));

    let release = MouseEvent::release((2, 3), MouseButton::Right);
    assert_eq!(
        screen.terminal().handle_mouse(&release).as_deref(),
        Some("\x1b[<2;3;4m")
    );
}

#[test]
fn the_wheel_uses_the_alternate_scroll_on_the_alternate_screen() {
    let mut screen = screen();
    screen.feed_str("\x1b[?1049h");

    let up = MouseEvent::wheel((0, 0), WheelAxis::Vertical, 120);
    assert_eq!(screen.terminal().handle_mouse(&up).as_deref(), Some("\x1b[A"));

    let down = MouseEvent::wheel((0, 0), WheelAxis::Vertical, -120);
    assert_eq!(screen.terminal().handle_mouse(&down).as_deref(), Some("\x1b[B"));
}

#[test]
fn mode_1007_off_reports_the_wheel_as_a_mouse_event() {
    let mut screen = screen();
    screen.feed_str("\x1b[?1000h\x1b[?1049h\x1b[?1007l");

    let up = MouseEvent::wheel((0, 0), WheelAxis::Vertical, 120);
    assert_eq!(screen.terminal().handle_mouse(&up).as_deref(), Some("\x1b[M`!!"));
}

#[test]
fn the_modifier_keys_set_the_x11_bits() {
    let mut screen = screen();
    screen.feed_str("\x1b[?1000h");

    let press = MouseEvent {
        modifiers: Modifiers {
            right_ctrl: true,
            ..Modifiers::default()
        },
        ..MouseEvent::press((0, 0), MouseButton::Left)
    };
    assert_eq!(screen.terminal().handle_mouse(&press).as_deref(), Some("\x1b[M0!!"));
}
