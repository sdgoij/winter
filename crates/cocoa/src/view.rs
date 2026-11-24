//! The terminal's view: AppKit events in, our chrome and pixels out.
//!
//! The view is the window's content view. It is flipped, so the strip is at
//! `y = 0` and the pane below it; AppKit reports pointer positions in the same
//! space once converted from the window. Every event is translated into a
//! [`winter::input`] record or one of [`Shell`]'s pointer methods, which are the
//! same rules the WinUI shell uses.

use std::cell::{Cell, RefCell};
use std::rc::Rc;

use objc2::rc::Retained;
use objc2::runtime::{AnyObject, Sel};
use objc2::{AnyThread, DefinedClass, MainThreadMarker, MainThreadOnly, define_class, msg_send};
use objc2_app_kit::{
    NSAccessibilityRole, NSAccessibilityTextAreaRole, NSEvent, NSEventModifierFlags, NSTextInputClient, NSTrackingArea,
    NSTrackingAreaOptions, NSView,
};
use objc2_foundation::{
    NSArray, NSAttributedString, NSAttributedStringKey, NSNotFound, NSPoint, NSRange, NSRect, NSSize, NSString,
    NSUInteger,
};

use term::input::Key;
use winter::input::{self, Modifiers};

use super::chrome::{self, ChromeState, STRIP_HEIGHT};
use super::keys;
use super::shell::{INITIAL_GRID, Shell};

/// The view's instance state.
pub struct Ivars {
    /// The shell the view paints and feeds.
    pub(crate) shell: Rc<RefCell<Shell>>,
    /// The tracking area that delivers `mouseMoved`, replaced on every resize.
    tracking: RefCell<Option<Retained<NSTrackingArea>>>,
    /// The input method's marked text, as AppKit reported it.
    marked: RefCell<String>,
    /// `insertText:` committed a composition during the current key press.
    committed: Cell<bool>,
    /// The window draws the content and its overlays as layers, so the view
    /// leaves them to them.
    layered: Cell<bool>,
    /// `WINTER_SCREENSHOT` is capturing, so the content is painted here even
    /// when the layers would otherwise have it.
    capturing: Cell<bool>,
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
            let programs = shell.programs();
            let (background, _) = shell.active_colors();
            let (scroll, document, visible) = shell.scroll_facts();
            let layered = self.ivars().layered.get();
            let capturing = self.ivars().capturing.get();
            let state = ChromeState {
                strip: shell.strip(),
                active: shell.active_index(),
                titles: &titles,
                programs: &programs,
                hovered_tab: shell.hovered_tab(),
                hovered_close: shell.hovered_close(),
                hovered_new: shell.hovered_new(),
                hovered_menu: shell.hovered_menu(),
                colors: shell.strip_colors(),
                pane: chrome::rgb(background),
                frame: if layered && shell.layer_presented() && !capturing {
                    None
                } else {
                    Some(shell.frame())
                },
                overlays: !layered || capturing,
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
            // AppKit's text system turns the key into marked text or a commit;
            // anything it leaves alone the shell's own mapping handles below.
            let events = NSArray::from_slice(&[event]);
            self.interpretKeyEvents(&events);
            let committed = self.ivars().committed.replace(false);
            let marked = !self.ivars().marked.borrow().is_empty();

            let mut shell = self.ivars().shell.borrow_mut();
            if committed || marked {
                // The commit already reached the shell through `insertText:`,
                // and marked text is the preview; neither is a key to send.
                drop(shell);
                self.setNeedsDisplay(true);
                return;
            }

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

        #[unsafe(method(isAccessibilityElement))]
        fn is_accessibility_element(&self) -> bool {
            true
        }

        #[unsafe(method_id(accessibilityRole))]
        fn accessibility_role(&self) -> Option<Retained<NSAccessibilityRole>> {
            // SAFETY: AppKit's accessibility role constants live for the
            // process, so retaining one is a no-op increment.
            unsafe {
                let role = NSAccessibilityTextAreaRole;
                Retained::retain(role as *const NSAccessibilityRole as *mut NSAccessibilityRole)
            }
        }

        #[unsafe(method_id(accessibilityRoleDescription))]
        fn accessibility_role_description(&self) -> Option<Retained<NSString>> {
            Some(NSString::from_str("terminal"))
        }

        #[unsafe(method_id(accessibilityLabel))]
        fn accessibility_label(&self) -> Option<Retained<NSString>> {
            let title = self.ivars().shell.borrow().title().to_string();
            Some(NSString::from_str(&title))
        }

        #[unsafe(method_id(accessibilityValue))]
        fn accessibility_value(&self) -> Option<Retained<NSString>> {
            // The visible screen is the value VoiceOver reads; the terminal's
            // own cells are not an `NSTextView`, so there is no per-cell tree.
            let text = self.ivars().shell.borrow().accessible_text();
            Some(NSString::from_str(&text))
        }

        #[unsafe(method(accessibilityFrame))]
        fn accessibility_frame(&self) -> NSRect {
            let bounds = self.bounds();
            self.window()
                .map(|window| window.convertRectToScreen(self.convertRect_toView(bounds, None)))
                .unwrap_or(bounds)
        }

        #[unsafe(method_id(accessibilityChildren))]
        fn accessibility_children(&self) -> Option<Retained<NSArray<AnyObject>>> {
            // The pane draws no subviews, so an empty tree is the truth.
            Some(NSArray::new())
        }
    }

    unsafe impl NSTextInputClient for TerminalView {
        #[unsafe(method(hasMarkedText))]
        fn has_marked_text(&self) -> bool {
            !self.ivars().marked.borrow().is_empty()
        }

        #[unsafe(method(markedRange))]
        fn marked_range(&self) -> NSRange {
            let length = self.ivars().marked.borrow().encode_utf16().count();
            if length > 0 {
                NSRange::new(0, length)
            } else {
                // Documented to return `{NSNotFound, 0}` when there is none.
                NSRange::new(NSNotFound as NSUInteger, 0)
            }
        }

        #[unsafe(method(selectedRange))]
        fn selected_range(&self) -> NSRange {
            // The terminal has no text selection the input method can address.
            NSRange::new(NSNotFound as NSUInteger, 0)
        }

        #[unsafe(method(setMarkedText:selectedRange:replacementRange:))]
        fn set_marked_text(&self, string: &AnyObject, selected_range: NSRange, _replacement_range: NSRange) {
            let text = ime_text(string);
            self.ivars().marked.replace(text.clone());
            self.ivars()
                .shell
                .borrow_mut()
                .set_composition(&text, selected_range.location);
            self.setNeedsDisplay(true);
        }

        #[unsafe(method(unmarkText))]
        fn unmark_text(&self) {
            self.ivars().marked.borrow_mut().clear();
            self.ivars().shell.borrow_mut().set_composition("", 0);
            self.setNeedsDisplay(true);
        }

        #[unsafe(method(insertText:replacementRange:))]
        fn insert_text(&self, string: &AnyObject, _replacement_range: NSRange) {
            // Without marked text this is plain typing, which the shell's key
            // path sends; committing here too would type every character twice.
            if self.ivars().marked.borrow().is_empty() {
                return;
            }

            let text = ime_text(string);
            self.ivars().marked.borrow_mut().clear();
            self.ivars().committed.set(true);
            let mut shell = self.ivars().shell.borrow_mut();
            shell.set_composition("", 0);
            if !text.chars().any(char::is_control) {
                shell.text_input(text);
            }
            drop(shell);
            self.setNeedsDisplay(true);
        }

        #[unsafe(method(doCommandBySelector:))]
        fn do_command_by_selector(&self, _selector: Sel) {
            // Required by the protocol; the shell's key mapping owns commands,
            // so an unhandled selector falls through to `keyDown:`'s tail.
        }

        #[unsafe(method_id(validAttributesForMarkedText))]
        fn valid_attributes_for_marked_text(&self) -> Retained<NSArray<NSAttributedStringKey>> {
            NSArray::new()
        }

        #[unsafe(method_id(attributedSubstringForProposedRange:actualRange:))]
        fn attributed_substring_for_proposed_range(
            &self,
            _range: NSRange,
            _actual_range: *mut NSRange,
        ) -> Option<Retained<NSAttributedString>> {
            None
        }

        #[unsafe(method(characterIndexForPoint:))]
        fn character_index_for_point(&self, _point: NSPoint) -> NSUInteger {
            0
        }

        #[unsafe(method(firstRectForCharacterRange:actualRange:))]
        fn first_rect_for_character_range(&self, _range: NSRange, _actual_range: *mut NSRange) -> NSRect {
            let (x, y, width, height) = self.ivars().shell.borrow().caret_rect();
            let local = NSRect::new(NSPoint::new(x, y), NSSize::new(width, height));
            // AppKit wants the candidate window's anchor in screen coordinates.
            self.window()
                .map(|window| window.convertRectToScreen(self.convertRect_toView(local, None)))
                .unwrap_or(local)
        }
    }
);

impl TerminalView {
    /// Creates the view the window shows.
    pub(crate) fn new(frame: NSRect, shell: Rc<RefCell<Shell>>, mtm: MainThreadMarker) -> Retained<Self> {
        let this = Self::alloc(mtm).set_ivars(Ivars {
            shell,
            tracking: RefCell::new(None),
            marked: RefCell::new(String::new()),
            committed: Cell::new(false),
            layered: Cell::new(false),
            capturing: Cell::new(false),
        });
        // SAFETY: NSView's designated initializer, called on the main thread
        // with an allocated instance of this subclass.
        unsafe { msg_send![super(this), initWithFrame: frame] }
    }

    /// Tells the view that the content and its overlays are layers above it.
    pub(crate) fn set_layered(&self, layered: bool) {
        self.ivars().layered.set(layered);
        self.setNeedsDisplay(true);
    }

    /// Makes the next `drawRect:` paint the content the layers normally show,
    /// which is what the screenshot path needs once it hides them.
    pub(crate) fn set_capturing(&self, capturing: bool) {
        self.ivars().capturing.set(capturing);
        self.setNeedsDisplay(true);
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

/// The text of an `insertText:`/`setMarkedText:` argument.
///
/// AppKit documents the argument as either an `NSString` or an
/// `NSAttributedString`; anything else contributes no text.
fn ime_text(string: &AnyObject) -> String {
    if let Some(attributed) = string.downcast_ref::<NSAttributedString>() {
        return attributed.string().to_string();
    }
    string
        .downcast_ref::<NSString>()
        .map(ToString::to_string)
        .unwrap_or_default()
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
///
/// `cell` is in physical pixels at `scale`, so the opening window is the
/// initial grid plus the pane's left inset and the scrollbar, in points. The
/// half-point slack keeps the first grid computation from rounding a whole cell
/// away and resizing the pty after the shell has started.
pub(crate) fn default_size(cell: (u16, u16), scale: f64) -> NSSize {
    let scale = if scale.is_finite() && scale > 0.0 { scale } else { 1.0 };
    let insets = chrome::PANE_LEFT + f64::from(winter::scrollbar::WIDTH);
    NSSize::new(
        f64::from(cell.0) * f64::from(INITIAL_GRID.0) / scale + insets + 0.5,
        f64::from(cell.1) * f64::from(INITIAL_GRID.1) / scale + STRIP_HEIGHT + 0.5,
    )
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn the_default_window_is_exactly_the_initial_grid() {
        let cases = [
            ((8u16, 16u16), 1.0f64),
            ((16, 32), 2.0),
            ((17, 33), 2.0),
            ((9, 18), 1.5),
        ];
        for (cell, scale) in cases {
            let size = default_size(cell, scale);
            let bounds = NSRect::new(NSPoint::new(0.0, 0.0), size);
            assert_eq!(
                chrome::grid_for(bounds, scale, cell),
                INITIAL_GRID,
                "{cell:?} at {scale}"
            );
        }
    }
}
