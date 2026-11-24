//! The terminal's view: AppKit events in, our chrome and pixels out.
//!
//! The view is the window's content view. It is flipped, so the strip is at
//! `y = 0` and the pane below it; AppKit reports pointer positions in the same
//! space once converted from the window. Every event is translated into a
//! [`crate::input`] record or one of [`Shell`]'s pointer methods, which are the
//! same rules the WinUI shell uses.

use std::cell::RefCell;
use std::rc::Rc;

use objc2::rc::Retained;
use objc2::runtime::AnyObject;
use objc2::{AnyThread, DefinedClass, MainThreadMarker, MainThreadOnly, define_class, msg_send};
use objc2_app_kit::{NSEvent, NSEventModifierFlags, NSTrackingArea, NSTrackingAreaOptions, NSView};
use objc2_foundation::{NSRect, NSSize};

use crate::input::{self, Modifiers};
use term::input::Key;

use super::chrome::{self, ChromeState, STRIP_HEIGHT};
use super::keys;
use super::shell::Shell;

/// The view's instance state.
pub struct Ivars {
    /// The shell the view paints and feeds.
    pub(crate) shell: Rc<RefCell<Shell>>,
    /// The tracking area that delivers `mouseMoved`, replaced on every resize.
    tracking: RefCell<Option<Retained<NSTrackingArea>>>,
}

define_class!(
    // SAFETY: NSView has no subclassing requirements this view does not meet,
    // and `Ivars` holds a plain `Rc` for the shell plus the tracking area. The
    // class is main-thread only, which matches AppKit's own contract.
    #[unsafe(super(NSView))]
    #[thread_kind = MainThreadOnly]
    #[name = "WinterTerminalView"]
    #[ivars = Ivars]
    pub struct TerminalView;

    impl TerminalView {
        #[unsafe(method(isFlipped))]
        fn is_flipped(&self) -> bool {
            true
        }

        #[unsafe(method(acceptsFirstResponder))]
        fn accepts_first_responder(&self) -> bool {
            true
        }

        #[unsafe(method(acceptsFirstMouse:))]
        fn accepts_first_mouse(&self, _event: Option<&NSEvent>) -> bool {
            true
        }

        #[unsafe(method(updateTrackingAreas))]
        fn update_tracking_areas(&self) {
            // SAFETY: `self` is an NSView; calling its inherited implementation
            // is what lets AppKit keep its own tracking state.
            unsafe {
                let _: () = msg_send![super(self), updateTrackingAreas];
            }

            if let Some(previous) = self.ivars().tracking.borrow_mut().take() {
                self.removeTrackingArea(&previous);
            }

            let options = NSTrackingAreaOptions::MouseEnteredAndExited
                | NSTrackingAreaOptions::MouseMoved
                | NSTrackingAreaOptions::ActiveInKeyWindow
                | NSTrackingAreaOptions::InVisibleRect;
            // SAFETY: the owner is this view, which outlives the area, and the
            // user info is null, which the initializer allows.
            let area = unsafe {
                NSTrackingArea::initWithRect_options_owner_userInfo(
                    NSTrackingArea::alloc(),
                    self.bounds(),
                    options,
                    Some(self.as_ref()),
                    None,
                )
            };
            self.addTrackingArea(&area);
            *self.ivars().tracking.borrow_mut() = Some(area);
        }

        #[unsafe(method(drawRect:))]
        fn draw_rect(&self, _dirty: NSRect) {
            let Ok(shell) = self.ivars().shell.try_borrow() else {
                return;
            };
            let Some(mtm) = MainThreadMarker::new() else {
                return;
            };
            let bounds = self.bounds();
            let titles = shell.titles();
            let (background, _) = shell.active_colors();
            let (scroll, document, visible) = shell.scroll_facts();
            let state = ChromeState {
                strip: shell.strip(),
                active: shell.active_index(),
                titles: &titles,
                hovered_tab: shell.hovered_tab(),
                hovered_close: shell.hovered_close(),
                hovered_new: shell.hovered_new(),
                hovered_menu: shell.hovered_menu(),
                colors: shell.strip_colors(),
                pane: chrome::rgb(background),
                frame: Some(shell.frame()),
                scale: f64::from(shell.scale()),
                grid: shell.grid(),
                cell: shell.cell(),
                scroll: (scroll, document, visible),
                marks: shell.marks(),
                bell_alpha: shell.bell_alpha(std::time::Instant::now()),
                find_open: shell.find_open(),
            };
            chrome::paint(bounds, &state, mtm);
        }

        #[unsafe(method(keyDown:))]
        fn key_down(&self, event: &NSEvent) {
            let mut shell = self.ivars().shell.borrow_mut();
            let modifiers = modifiers(event.modifierFlags());
            let characters = event.characters().map(|text| text.to_string());
            let ignoring = event.charactersIgnoringModifiers().map(|text| text.to_string());
            let Some(key) = keys::key_of(event.keyCode(), ignoring.as_deref()) else {
                if let Some(text) = characters {
                    shell.text_input(text);
                }
                drop(shell);
                self.setNeedsDisplay(true);
                return;
            };

            shell.key_down(key, modifiers, event.isARepeat(), characters);
            drop(shell);
            self.setNeedsDisplay(true);
        }

        #[unsafe(method(keyUp:))]
        fn key_up(&self, event: &NSEvent) {
            let mut shell = self.ivars().shell.borrow_mut();
            let modifiers = modifiers(event.modifierFlags());
            let ignoring = event.charactersIgnoringModifiers().map(|text| text.to_string());
            if let Some(key) = keys::key_of(event.keyCode(), ignoring.as_deref()) {
                shell.key_up(key, modifiers);
            }
            drop(shell);
        }

        #[unsafe(method(flagsChanged:))]
        fn flags_changed(&self, event: &NSEvent) {
            let Some(key) = keys::modifier_key(event.keyCode()) else {
                return;
            };
            let flags = event.modifierFlags();
            let pressed = match key {
                Key::LeftShift | Key::RightShift => flags.contains(NSEventModifierFlags::Shift),
                Key::LeftControl | Key::RightControl => flags.contains(NSEventModifierFlags::Control),
                Key::LeftAlt | Key::RightAlt => flags.contains(NSEventModifierFlags::Option),
                Key::LeftSuper | Key::RightSuper => flags.contains(NSEventModifierFlags::Command),
                Key::CapsLock => flags.contains(NSEventModifierFlags::CapsLock),
                _ => false,
            };
            let mut shell = self.ivars().shell.borrow_mut();
            shell.flags_changed(key, pressed, modifiers(flags));
        }

        #[unsafe(method(mouseDown:))]
        fn mouse_down(&self, event: &NSEvent) {
            self.press(event, 0);
        }

        #[unsafe(method(rightMouseDown:))]
        fn right_mouse_down(&self, event: &NSEvent) {
            self.press(event, 1);
        }

        #[unsafe(method(otherMouseDown:))]
        fn other_mouse_down(&self, event: &NSEvent) {
            self.press(event, event.buttonNumber());
        }

        #[unsafe(method(mouseDragged:))]
        fn mouse_dragged(&self, event: &NSEvent) {
            self.drag(event, 0);
        }

        #[unsafe(method(rightMouseDragged:))]
        fn right_mouse_dragged(&self, event: &NSEvent) {
            self.drag(event, 1);
        }

        #[unsafe(method(otherMouseDragged:))]
        fn other_mouse_dragged(&self, event: &NSEvent) {
            self.drag(event, event.buttonNumber());
        }

        #[unsafe(method(mouseUp:))]
        fn mouse_up(&self, event: &NSEvent) {
            self.release(event, 0);
        }

        #[unsafe(method(rightMouseUp:))]
        fn right_mouse_up(&self, event: &NSEvent) {
            self.release(event, 1);
        }

        #[unsafe(method(otherMouseUp:))]
        fn other_mouse_up(&self, event: &NSEvent) {
            self.release(event, event.buttonNumber());
        }

        #[unsafe(method(scrollWheel:))]
        fn scroll_wheel(&self, event: &NSEvent) {
            let point = self.point(event);
            let modifiers = modifiers(event.modifierFlags());
            let mut shell = self.ivars().shell.borrow_mut();
            shell.wheel(
                point,
                event.scrollingDeltaY(),
                event.hasPreciseScrollingDeltas(),
                modifiers.shift,
            );
            drop(shell);
            self.setNeedsDisplay(true);
        }

        #[unsafe(method(mouseMoved:))]
        fn mouse_moved(&self, event: &NSEvent) {
            self.move_pointer(event);
        }

        #[unsafe(method(mouseEntered:))]
        fn mouse_entered(&self, event: &NSEvent) {
            self.move_pointer(event);
        }

        #[unsafe(method(copy:))]
        fn copy(&self, _sender: Option<&AnyObject>) {
            self.ivars().shell.borrow_mut().copy_selection();
            self.setNeedsDisplay(true);
        }

        #[unsafe(method(paste:))]
        fn paste(&self, _sender: Option<&AnyObject>) {
            self.ivars().shell.borrow_mut().paste_clipboard();
            self.setNeedsDisplay(true);
        }

        #[unsafe(method(mouseExited:))]
        fn mouse_exited(&self, _event: &NSEvent) {
            self.ivars().shell.borrow_mut().pointer_left();
            self.setNeedsDisplay(true);
        }
    }
);

impl TerminalView {
    /// Creates the view the window shows.
    pub(crate) fn new(frame: NSRect, shell: Rc<RefCell<Shell>>, mtm: MainThreadMarker) -> Retained<Self> {
        let this = Self::alloc(mtm).set_ivars(Ivars {
            shell,
            tracking: RefCell::new(None),
        });
        // SAFETY: NSView's designated initializer, called on the main thread
        // with an allocated instance of this subclass.
        unsafe { msg_send![super(this), initWithFrame: frame] }
    }

    /// The point an event names, in this view's flipped coordinates.
    fn point(&self, event: &NSEvent) -> (f64, f64) {
        let window_point = event.locationInWindow();
        let point = self.convertPoint_fromView(window_point, None);
        (point.x, point.y)
    }

    fn press(&self, event: &NSEvent, button: isize) {
        self.window().map(|window| window.makeFirstResponder(Some(self)));
        let point = self.point(event);
        let modifiers = modifiers(event.modifierFlags());
        let button = mouse_button(button);
        let buttons = event_buttons();
        let handled = self
            .ivars()
            .shell
            .borrow_mut()
            .pointer_pressed(point, button, buttons, modifiers);
        if !handled {
            // Empty strip: the title bar's own drag, so the window still moves
            // where there is no tab.
            if let Some(window) = self.window() {
                window.performWindowDragWithEvent(event);
            }
        }
        self.setNeedsDisplay(true);
    }

    fn drag(&self, event: &NSEvent, _button: isize) {
        let point = self.point(event);
        let modifiers = modifiers(event.modifierFlags());
        let buttons = event_buttons();
        let mut shell = self.ivars().shell.borrow_mut();
        shell.pointer_moved(point, buttons, modifiers);
        drop(shell);
        self.setNeedsDisplay(true);
    }

    fn move_pointer(&self, event: &NSEvent) {
        let point = self.point(event);
        let modifiers = modifiers(event.modifierFlags());
        let buttons = event_buttons();
        let mut shell = self.ivars().shell.borrow_mut();
        shell.pointer_moved(point, buttons, modifiers);
        drop(shell);
        self.setNeedsDisplay(true);
    }

    fn release(&self, event: &NSEvent, button: isize) {
        let point = self.point(event);
        let modifiers = modifiers(event.modifierFlags());
        // The button that raised this event is no longer held.
        let buttons = event_buttons();
        let mut shell = self.ivars().shell.borrow_mut();
        shell.pointer_released(point, mouse_button(button), buttons, modifiers);
        drop(shell);
        self.setNeedsDisplay(true);
    }
}

fn mouse_button(number: isize) -> term::input::MouseButton {
    match number {
        1 => term::input::MouseButton::Right,
        2 => term::input::MouseButton::Middle,
        _ => term::input::MouseButton::Left,
    }
}

fn event_buttons() -> term::input::MouseButtons {
    let pressed = NSEvent::pressedMouseButtons();
    let left = pressed & 0b001 != 0;
    let right = pressed & 0b010 != 0;
    let middle = pressed & 0b100 != 0;
    input::mouse_buttons(left, middle, right)
}

fn modifiers(flags: NSEventModifierFlags) -> Modifiers {
    Modifiers {
        shift: flags.contains(NSEventModifierFlags::Shift),
        ctrl: flags.contains(NSEventModifierFlags::Control),
        alt: flags.contains(NSEventModifierFlags::Option),
        command: flags.contains(NSEventModifierFlags::Command),
    }
}

/// The size the view reports for an unscaled terminal.
pub(crate) fn default_size(cell: (u16, u16)) -> NSSize {
    NSSize::new(f64::from(cell.0) * 100.0, f64::from(cell.1) * 28.0 + STRIP_HEIGHT)
}
