//! The mouse wheel, which the WinUI binding does not report.
//!
//! `windows-reactor-pre` 0.108 exposes pointer press, move and release but not
//! `PointerWheelChanged`, and a wheel aimed at the window never appears in this
//! thread's message queue either. So the shell reads it from a low-level mouse
//! hook: every wheel over the window is accumulated and the paint loop takes the
//! notches (ADR 047).

use std::cell::Cell;

use windows::Win32::Foundation::{HWND, LPARAM, LRESULT, POINT, RECT, WPARAM};
use windows::Win32::UI::Input::KeyboardAndMouse::{GetAsyncKeyState, VK_SHIFT};
use windows::Win32::UI::WindowsAndMessaging::{
    CallNextHookEx, GetWindowRect, HC_ACTION, HHOOK, MSLLHOOKSTRUCT, SetWindowsHookExW, UnhookWindowsHookEx,
    WH_MOUSE_LL, WM_MOUSEWHEEL,
};

/// One notch of a wheel, as Windows reports it.
const WHEEL_DELTA: i32 = 120;

thread_local! {
    /// The window the shell owns, as a raw handle.
    static TARGET: Cell<isize> = const { Cell::new(0) };
    /// Wheel movement accumulated since the last frame, in `WHEEL_DELTA` units
    /// plus a remainder smaller than one notch.
    static NOTCHES: Cell<i32> = const { Cell::new(0) };
}

/// An installed mouse hook; dropping it uninstalls.
pub struct Wheel(HHOOK);

/// Hooks the mouse for wheel messages over `hwnd`.
pub fn install(hwnd: isize) -> Option<Wheel> {
    TARGET.with(|target| target.set(hwnd));
    // SAFETY: the procedure matches `HOOKPROC`; a low-level hook is global, so
    // it takes no module handle and no thread id, and the returned handle is
    // owned by the `Wheel` that drops it.
    let hook = unsafe { SetWindowsHookExW(WH_MOUSE_LL, Some(procedure), None, 0) }.ok()?;
    Some(Wheel(hook))
}

/// Takes the whole notches accumulated since the last call.
///
/// A high-resolution wheel reports a fraction of a notch per event; the
/// remainder stays for the next frame rather than being dropped.
pub fn take() -> i32 {
    NOTCHES.with(|cell| {
        let total = cell.get();
        let notches = total / WHEEL_DELTA;
        cell.set(total - notches * WHEEL_DELTA);
        notches
    })
}

/// Whether Shift is held, which keeps the wheel for the viewport.
pub fn shift_held() -> bool {
    // SAFETY: no pointers are involved; the call reads the input state of the
    // calling thread's desktop.
    unsafe { GetAsyncKeyState(VK_SHIFT.0 as i32) < 0 }
}

impl Drop for Wheel {
    fn drop(&mut self) {
        // SAFETY: the handle came from `SetWindowsHookExW` and is not used again.
        unsafe {
            let _ = UnhookWindowsHookEx(self.0);
        }
        TARGET.with(|target| target.set(0));
    }
}

/// Accumulates a wheel over the shell's window.
///
/// # Safety
/// Windows calls this with a `MSLLHOOKSTRUCT` pointer in `lparam` when `code`
/// is `HC_ACTION`; that pointer is valid for the duration of the call.
unsafe extern "system" fn procedure(code: i32, wparam: WPARAM, lparam: LPARAM) -> LRESULT {
    if code == HC_ACTION as i32 && wparam.0 as u32 == WM_MOUSEWHEEL {
        // SAFETY: with `HC_ACTION` and `WM_MOUSEWHEEL` the system passes a
        // pointer to a `MSLLHOOKSTRUCT`.
        let mouse = unsafe { *(lparam.0 as *const MSLLHOOKSTRUCT) };
        let target = TARGET.with(Cell::get);
        if target != 0 {
            let mut rect = RECT::default();
            // SAFETY: the rectangle is a local and the handle is the shell's
            // window while the hook is installed.
            if unsafe { GetWindowRect(HWND(target as *mut _), &mut rect) }.is_ok() && inside(&rect, mouse.pt) {
                let delta = ((mouse.mouseData >> 16) & 0xFFFF) as i16 as i32;
                NOTCHES.with(|notches| notches.set(notches.get() + delta));
            }
        }
    }

    // The shell scrolled its own copy; the event still belongs to Windows, so it
    // is passed on rather than swallowed.
    // SAFETY: the arguments are the ones Windows passed to this hook.
    unsafe { CallNextHookEx(None, code, wparam, lparam) }
}

/// Whether `point` is inside `rect`.
fn inside(rect: &RECT, point: POINT) -> bool {
    point.x >= rect.left && point.x < rect.right && point.y >= rect.top && point.y < rect.bottom
}
