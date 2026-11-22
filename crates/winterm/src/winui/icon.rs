//! The window icon the executable carries.
//!
//! The build script embeds the drawing the egui shell also uses (ADR 058).
//! WinUI 3 gives its window a framework default rather than the executable's
//! icon, so the embedded resource is loaded and handed to the window once the
//! handle arrives.

use std::ptr::null;

use windows_sys::Win32::Foundation::HWND;
use windows_sys::Win32::System::LibraryLoader::GetModuleHandleW;
use windows_sys::Win32::UI::WindowsAndMessaging::{
    GetSystemMetrics, ICON_BIG, ICON_SMALL, IMAGE_ICON, LR_DEFAULTSIZE, LoadImageW, SM_CXSMICON, SM_CYSMICON,
    SendMessageW, WM_SETICON,
};

/// The icon group the build script writes; resource ids start at one.
const ICON_ID: usize = 1;

/// Gives `window` the icon embedded in this executable.
#[allow(unsafe_code)]
pub fn apply(window: isize) {
    // SAFETY: `window` is the handle `run_window` reported and outlives the
    // process; `GetModuleHandleW(null)` names the running executable, which
    // always has a module. Every loaded handle is the icon the window keeps
    // for its lifetime, so none is destroyed while it is in use.
    unsafe {
        let module = GetModuleHandleW(null());
        if module.is_null() {
            return;
        }

        let name = ICON_ID as *const u16;
        let big = LoadImageW(module, name, IMAGE_ICON, 0, 0, LR_DEFAULTSIZE);
        let small = LoadImageW(
            module,
            name,
            IMAGE_ICON,
            GetSystemMetrics(SM_CXSMICON),
            GetSystemMetrics(SM_CYSMICON),
            0,
        );

        let window = window as HWND;
        if !big.is_null() {
            SendMessageW(window, WM_SETICON, ICON_BIG as usize, big as isize);
        }
        if !small.is_null() {
            SendMessageW(window, WM_SETICON, ICON_SMALL as usize, small as isize);
        }
    }
}
