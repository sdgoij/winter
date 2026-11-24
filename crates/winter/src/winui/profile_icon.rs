//! A profile's icon, taken from the program the profile runs.
//!
//! Windows Terminal puts the shell's own icon on its tab and beside its name in
//! the profile menu. The `windows-reactor-pre` 0.108 snapshot has no icon
//! property on a hand-built tab and no `IconSource` setter on a `Button`'s
//! content, so the icon is carried as an `Image` over encoded bytes; this module
//! produces those bytes from the executable (ADR 066).
//!
//! Everything is cached for the life of the process: `view` runs on every frame,
//! and shelling out to the icon extractor per frame would be a frame drop per
//! tab.

use std::cell::RefCell;
use std::collections::HashMap;
use std::os::windows::ffi::OsStrExt;
use std::path::{Path, PathBuf};
use std::sync::Arc;

use windows_sys::Win32::Graphics::Gdi::{
    BI_RGB, BITMAP, BITMAPINFO, BITMAPINFOHEADER, CreateCompatibleDC, DIB_RGB_COLORS, DeleteDC, DeleteObject,
    GetDIBits, GetObjectW,
};
use windows_sys::Win32::UI::WindowsAndMessaging::{DestroyIcon, GetIconInfo, HICON, ICONINFO, PrivateExtractIconsW};

use super::png;

/// The icon's edge in pixels.
///
/// A tab's icon is 16 DIP, so 32 pixels covers a 200% display from the icon the
/// extractor scales for us; asking for 256 would be a megabyte through the PNG
/// writer for a 16-DIP slot.
const SIZE: i32 = 32;

thread_local! {
    /// One entry per program, so a program is extracted once per session.
    static CACHE: RefCell<HashMap<String, Option<Arc<[u8]>>>> = RefCell::new(HashMap::new());
    /// The icon this executable carries, read once.
    static OWN: RefCell<Option<Option<Arc<[u8]>>>> = const { RefCell::new(None) };
}

/// The PNG of the icon `program` carries, or None when there is none.
///
/// `program` is what a profile's `command` holds: a name on `PATH`, or a path.
/// A profile whose program has no icon of its own — `bash` on a machine where
/// the name resolves to nothing, a script, a command that is not a Windows
/// executable — wears the shell's own icon, so no row or tab is left blank
/// (ADR 070).
pub(super) fn png_for(program: &str) -> Option<Arc<[u8]>> {
    CACHE.with_borrow_mut(|cache| {
        if let Some(cached) = cache.get(program) {
            return cached.clone();
        }
        let bytes = lookup(program, own_icon);
        cache.insert(program.to_string(), bytes.clone());
        bytes
    })
}

/// The icon `program` carries, or what `fallback` names when it has none.
///
/// Split out from the cache so the rule can be tested with a fallback of the
/// test's own: the test binary is not the executable the icon is embedded in.
fn lookup(program: &str, fallback: impl FnOnce() -> Option<Arc<[u8]>>) -> Option<Arc<[u8]>> {
    encode(program).or_else(fallback)
}

/// The icon this executable carries.
fn own_icon() -> Option<Arc<[u8]>> {
    OWN.with_borrow_mut(|cache| {
        cache
            .get_or_insert_with(|| std::env::current_exe().ok().and_then(|path| png_of(&path)))
            .clone()
    })
}

fn encode(program: &str) -> Option<Arc<[u8]>> {
    png_of(&resolve(program)?)
}

fn png_of(path: &Path) -> Option<Arc<[u8]>> {
    let (width, height, pixels) = extract(path)?;
    let png = png::encode(u32::try_from(width).ok()?, u32::try_from(height).ok()?, &pixels);
    (!png.is_empty()).then(|| Arc::from(png))
}

/// The executable `program` names, looked up on `PATH` when it is a bare name.
fn resolve(program: &str) -> Option<PathBuf> {
    let direct = PathBuf::from(program);
    if direct.is_absolute() || program.contains(['/', '\\']) {
        return direct.is_file().then_some(direct);
    }

    let name = match direct.extension() {
        Some(_) => program.to_string(),
        None => format!("{program}.exe"),
    };
    std::env::split_paths(&std::env::var_os("PATH")?)
        .map(|directory| directory.join(&name))
        .find(|candidate| candidate.is_file())
}

/// The icon `path` carries, as its edge lengths and `RGBA` pixels.
#[allow(unsafe_code)]
fn extract(path: &Path) -> Option<(i32, i32, Vec<u8>)> {
    let wide: Vec<u16> = path.as_os_str().encode_wide().chain(std::iter::once(0)).collect();
    let mut icon: HICON = std::ptr::null_mut();

    // SAFETY: `wide` is a NUL-terminated path that outlives the call, and `icon`
    // is a local the call writes one handle into; the request is for one icon.
    let found = unsafe { PrivateExtractIconsW(wide.as_ptr(), 0, SIZE, SIZE, &mut icon, std::ptr::null_mut(), 1, 0) };
    if found == 0 || icon.is_null() {
        return None;
    }

    // SAFETY: the handle was just created by the extractor and is not used
    // again, whatever `rgba` made of it.
    let pixels = unsafe { rgba(icon) };
    unsafe { DestroyIcon(icon) };
    pixels
}

/// The pixels of `icon`, top row first, as `RGBA`.
#[allow(unsafe_code)]
unsafe fn rgba(icon: HICON) -> Option<(i32, i32, Vec<u8>)> {
    let mut info: ICONINFO = unsafe { std::mem::zeroed() };
    // SAFETY: `icon` is a live icon handle; `info` is a local of the type the
    // call expects.
    if unsafe { GetIconInfo(icon, &mut info) } == 0 {
        return None;
    }

    // SAFETY: both bitmaps belong to the icon and are ours to release once
    // `bits` has copied what it needs; `null` is not a valid handle, which the
    // call tolerates.
    let pixels = unsafe { bits(info.hbmColor) };
    unsafe {
        DeleteObject(info.hbmColor);
        DeleteObject(info.hbmMask);
    }
    pixels
}

/// The pixels of a colour bitmap, top row first, as `RGBA`.
///
/// # Safety
///
/// `bitmap` must be a GDI bitmap the caller owns and may not delete while this
/// runs.
#[allow(unsafe_code)]
unsafe fn bits(bitmap: windows_sys::Win32::Graphics::Gdi::HBITMAP) -> Option<(i32, i32, Vec<u8>)> {
    if bitmap.is_null() {
        return None;
    }

    let mut header: BITMAP = unsafe { std::mem::zeroed() };
    // SAFETY: `header` is a `BITMAP` and the size passed is its own.
    let read = unsafe {
        GetObjectW(
            bitmap,
            i32::try_from(std::mem::size_of::<BITMAP>()).ok()?,
            std::ptr::from_mut(&mut header).cast(),
        )
    };
    if read == 0 {
        return None;
    }
    let (width, height) = (header.bmWidth, header.bmHeight);
    if width <= 0 || height <= 0 || width > 256 || height > 256 {
        return None;
    }

    // SAFETY: the device context is created and released here, and a null one
    // only means the call below fails.
    let device = unsafe { CreateCompatibleDC(std::ptr::null_mut()) };
    if device.is_null() {
        return None;
    }

    let mut request: BITMAPINFO = unsafe { std::mem::zeroed() };
    request.bmiHeader.biSize = u32::try_from(std::mem::size_of::<BITMAPINFOHEADER>()).ok()?;
    request.bmiHeader.biWidth = width;
    // Negative: the caller wants the top row first, as PNG stores it.
    request.bmiHeader.biHeight = -height;
    request.bmiHeader.biPlanes = 1;
    request.bmiHeader.biBitCount = 32;
    request.bmiHeader.biCompression = BI_RGB;

    let bytes = usize::try_from(width).ok()? * usize::try_from(height).ok()? * 4;
    let mut pixels = vec![0u8; bytes];

    // SAFETY: `pixels` is `width * height * 4` bytes, which is what a 32bpp
    // bitmap of that size needs, and `request` describes exactly that.
    let rows = unsafe {
        GetDIBits(
            device,
            bitmap,
            0,
            u32::try_from(height).ok()?,
            pixels.as_mut_ptr().cast(),
            &mut request,
            DIB_RGB_COLORS,
        )
    };
    unsafe { DeleteDC(device) };
    if rows == 0 {
        return None;
    }

    // `GetDIBits` writes BGRA; PNG wants RGBA. An icon from before the alpha
    // channel existed comes back with every alpha at zero, which would paint
    // nothing at all, so such an icon is taken as opaque. Reading its mask to
    // find the transparent corners is the follow-up if that shows.
    let mut transparent = false;
    for pixel in pixels.as_chunks_mut::<4>().0 {
        pixel.swap(0, 2);
        transparent |= pixel[3] != 0;
    }
    if !transparent {
        for pixel in pixels.as_chunks_mut::<4>().0 {
            pixel[3] = 255;
        }
    }
    Some((width, height, pixels))
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn a_program_on_the_path_is_found() {
        // `cmd.exe` ships with every supported Windows release.
        let path = resolve("cmd.exe").expect("cmd.exe on PATH");
        assert!(path.is_file());
    }

    #[test]
    fn a_program_with_no_extension_is_tried_as_an_executable() {
        assert_eq!(resolve("cmd"), resolve("cmd.exe"));
    }

    #[test]
    fn a_program_that_is_not_on_the_machine_has_no_path() {
        assert!(resolve("winter-no-such-program-6b2f").is_none());
    }

    #[test]
    fn a_system_program_has_an_icon_that_encodes_to_a_png() {
        let png = png_for("cmd.exe").expect("cmd.exe has an icon");

        assert_eq!(&png[..8], b"\x89PNG\r\n\x1a\n");
        // IHDR's width and height, which is the size the extractor was asked
        // for: an icon the shell could not scale would be a 1-DIP painting.
        let width = u32::from_be_bytes(png[16..20].try_into().unwrap());
        let height = u32::from_be_bytes(png[20..24].try_into().unwrap());
        assert!((1..=256).contains(&width), "width {width}");
        assert!((1..=256).contains(&height), "height {height}");
    }

    #[test]
    fn a_program_with_no_icon_of_its_own_falls_back() {
        let fallback = || Some(Arc::from(&b"the app's own icon"[..]));
        assert_eq!(lookup("winter-no-such-program-6b2f", fallback), fallback());

        // A program that does have one keeps it; the fallback is not reached.
        assert_eq!(lookup("cmd.exe", || None), png_for("cmd.exe"));
        assert!(lookup("cmd.exe", || None).is_some());
    }
}
