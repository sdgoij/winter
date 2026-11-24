//! The icon a tab wears: its program's, or the shell's drawn mark.
//!
//! Windows Terminal extracts the shell's own icon from the executable (ADR
//! 066); macOS has no GDI and no `PrivateExtractIcons`, so the icon is the one
//! Launch Services shows for the program's file. The shell's own mark (ADR 058)
//! is the fallback, so no tab is blank when the program is a script or the name
//! does not resolve (ADR 070's rule, on this platform).
//!
//! Both are cached for the life of the process: the strip is painted every
//! frame, and asking Launch Services per paint would cost a frame per tab.

use std::cell::RefCell;
use std::collections::HashMap;
use std::path::{Path, PathBuf};

use objc2::AnyThread;
use objc2::rc::Retained;
use objc2_app_kit::{NSBitmapFormat, NSBitmapImageRep, NSDeviceRGBColorSpace, NSImage, NSWorkspace};
use objc2_foundation::{NSSize, NSString};

thread_local! {
    /// One entry per program's name, so Launch Services is asked once.
    static CACHE: RefCell<HashMap<String, Option<Retained<NSImage>>>> = RefCell::new(HashMap::new());
    /// The drawn application mark, built once.
    static OWN: RefCell<Option<Retained<NSImage>>> = const { RefCell::new(None) };
}

/// The icon `program` carries, or the shell's own mark when it has none.
///
/// `program` is what a profile's `command` holds: a name on `PATH`, or a path.
pub(crate) fn for_program(program: &str) -> Option<Retained<NSImage>> {
    CACHE
        .with_borrow_mut(|cache| {
            cache
                .entry(program.to_string())
                .or_insert_with(|| resolve(program).map(|path| icon_for(&path)))
                .clone()
        })
        .or_else(own)
}

/// The shell's drawn mark (ADR 058), as an image the Dock or a tab can draw.
pub(crate) fn own() -> Option<Retained<NSImage>> {
    OWN.with_borrow_mut(|cache| {
        if cache.is_none() {
            *cache = build_own();
        }
        cache.clone()
    })
}

fn icon_for(path: &Path) -> Retained<NSImage> {
    NSWorkspace::sharedWorkspace().iconForFile(&NSString::from_str(&path.to_string_lossy()))
}

/// The file `program` names, with a bare name looked up on `PATH`.
fn resolve(program: &str) -> Option<PathBuf> {
    let direct = PathBuf::from(program);
    if direct.is_absolute() || program.contains('/') {
        return direct.is_file().then_some(direct);
    }

    let path = std::env::var_os("PATH")?;
    std::env::split_paths(&path)
        .map(|directory| directory.join(program))
        .find(|candidate| candidate.is_file())
}

fn build_own() -> Option<Retained<NSImage>> {
    let data = crate::icon::data();
    let width = data.width as isize;
    let height = data.height as isize;
    let bytes_per_row = width.checked_mul(4)?;

    // SAFETY: null planes ask AppKit to allocate the buffer; the remaining
    // arguments describe the straight RGBA the icon drawing produces.
    let rep = unsafe {
        NSBitmapImageRep::initWithBitmapDataPlanes_pixelsWide_pixelsHigh_bitsPerSample_samplesPerPixel_hasAlpha_isPlanar_colorSpaceName_bitmapFormat_bytesPerRow_bitsPerPixel(
            NSBitmapImageRep::alloc(),
            std::ptr::null_mut(),
            width,
            height,
            8,
            4,
            true,
            false,
            NSDeviceRGBColorSpace,
            NSBitmapFormat::AlphaNonpremultiplied,
            bytes_per_row,
            32,
        )
    }?;

    let destination = rep.bitmapData();
    let needed = data.width as usize * data.height as usize * 4;
    if destination.is_null() || data.rgba.len() < needed {
        return None;
    }
    // SAFETY: the rep owns a buffer of the size asked for above, and the icon
    // data is at least that long; the copy borrows both for its duration.
    unsafe {
        std::ptr::copy_nonoverlapping(data.rgba.as_ptr(), destination, needed);
    }

    let image = NSImage::initWithSize(
        NSImage::alloc(),
        NSSize::new(f64::from(data.width), f64::from(data.height)),
    );
    image.addRepresentation(&rep);
    Some(image)
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn a_program_on_the_path_is_found() {
        assert!(resolve("sh").is_some_and(|path| path.is_file()));
        assert!(resolve("/bin/sh").is_some_and(|path| path.is_file()));
    }

    #[test]
    fn a_program_that_is_not_on_the_machine_has_no_path() {
        assert!(resolve("winter-no-such-program-6b2f").is_none());
        assert!(resolve("/nonexistent/winter-no-such-program-6b2f").is_none());
    }
}
