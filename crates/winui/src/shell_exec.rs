//! Opening a URL in the user's browser, for an extension's `app.openUrl`.
//!
//! The About dialog links to the repository with a `HyperlinkButton`, which
//! navigates the URI itself and needs no Win32 call (ADR 070). A URL that
//! arrives from an extension handler has no control to click, so the shell hands
//! it to the `open` verb instead, which is what Explorer does for a double
//! click. This is the shell's fifth and last permitted `unsafe` module, beside
//! `surface`, `wheel`, `icon` and `profile_icon` (ADR 086).

use windows_sys::Win32::UI::Shell::ShellExecuteW;
use windows_sys::Win32::UI::WindowsAndMessaging::SW_SHOWNORMAL;

/// The verb that gives a file or URL to its registered handler.
const OPEN: &[u16] = &[b'o' as u16, b'p' as u16, b'e' as u16, b'n' as u16, 0];

/// Opens `url` with the user's default handler; `false` when Windows refused.
///
/// `ShellExecuteW` returns a value cast to an integer: anything above 32 is
/// success, and the small values are the documented error codes. An empty
/// string is refused before the call, because Windows treats it as a request
/// with no file rather than an error.
pub(super) fn open(url: &str) -> bool {
    if url.is_empty() {
        return false;
    }
    let wide: Vec<u16> = url.encode_utf16().chain(std::iter::once(0)).collect();

    // SAFETY: `OPEN` and `wide` are NUL-terminated UTF-16 strings that outlive
    // the call; the request has no parent window, parameters or working
    // directory, for which null is the documented "none"; and the call returns
    // a status code rather than anything the shell must release.
    let status = unsafe {
        ShellExecuteW(
            std::ptr::null_mut(),
            OPEN.as_ptr(),
            wide.as_ptr(),
            std::ptr::null(),
            std::ptr::null(),
            SW_SHOWNORMAL,
        )
    };

    status as isize > 32
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn an_empty_url_is_refused_before_windows_sees_it() {
        assert!(!open(""));
    }
}
