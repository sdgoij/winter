//! The icon a tab wears: its program's, or the shell's drawn mark.
//!
//! Windows Terminal extracts the shell's own icon from the executable (ADR
//! 066); macOS has no GDI and no `PrivateExtractIcons`, so the icon is the one
//! Launch Services shows for the program's file. The shell's own mark (ADR 058)
//! is the fallback, so no tab is blank when the program is a script or the name
//! does not resolve (ADR 070's rule, on this platform).
//!
//! The same mark draws the `.app` bundle's icon: `WINTER_APPICON` asks for an
//! `.iconset` at every size macOS reads, and `scripts/package.sh` compiles it
//! with `iconutil` (ADR 074).
//!
//! Both are cached for the life of the process: the strip is painted every
//! frame, and asking Launch Services per paint would cost a frame per tab.

use std::cell::RefCell;
use std::collections::HashMap;
use std::path::{Path, PathBuf};

use objc2::AnyThread;
use objc2::rc::Retained;
use objc2::runtime::AnyObject;
use objc2_app_kit::{
    NSBitmapFormat, NSBitmapImageFileType, NSBitmapImageRep, NSBitmapImageRepPropertyKey, NSDeviceRGBColorSpace,
    NSImage, NSWorkspace,
};
use objc2_foundation::{NSDictionary, NSSize, NSString};

thread_local! {
    /// One entry per program's name, so Launch Services is asked once.
    static CACHE: RefCell<HashMap<String, Option<Retained<NSImage>>>> = RefCell::new(HashMap::new());
    /// The drawn application mark, built once.
    static OWN: RefCell<Option<Retained<NSImage>>> = const { RefCell::new(None) };
}

/// The PNGs `iconutil` turns into an `.icns`.
///
/// The pairs are the same drawing at two scales, which is what an iconset's
/// `@2x` name means; macOS picks the file whose pixel size matches the display.
const ICONSET: [(&str, usize); 10] = [
    ("icon_16x16.png", 16),
    ("icon_16x16@2x.png", 32),
    ("icon_32x32.png", 32),
    ("icon_32x32@2x.png", 64),
    ("icon_128x128.png", 128),
    ("icon_128x128@2x.png", 256),
    ("icon_256x256.png", 256),
    ("icon_256x256@2x.png", 512),
    ("icon_512x512.png", 512),
    ("icon_512x512@2x.png", 1024),
];

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

/// Draws the ten `.iconset` PNGs into `directory`.
///
/// `WINTER_APPICON` asks for this, and `scripts/package.sh` hands the result to
/// `iconutil`. The mark is the window icon's own geometry (ADR 058), so the
/// Dock, the Windows executable and the bundle never disagree.
pub(crate) fn write_iconset(directory: &Path) -> Result<(), String> {
    std::fs::create_dir_all(directory).map_err(|error| format!("{}: {error}", directory.display()))?;
    for (name, size) in ICONSET {
        write_png(&directory.join(name), size)?;
    }
    Ok(())
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

/// Writes the mark at `size` as a straight-RGBA PNG.
fn write_png(path: &Path, size: usize) -> Result<(), String> {
    let rgba = winter::icon::rgba(size);
    let side = isize::try_from(size).map_err(|_| format!("icon size {size} does not fit"))?;
    let bytes_per_row = side.checked_mul(4).ok_or("the icon row overflows")?;
    // SAFETY: null planes ask AppKit to allocate the buffer; the remaining
    // arguments describe the straight RGBA `winter::icon::rgba` produces.
    let rep = unsafe {
        NSBitmapImageRep::initWithBitmapDataPlanes_pixelsWide_pixelsHigh_bitsPerSample_samplesPerPixel_hasAlpha_isPlanar_colorSpaceName_bitmapFormat_bytesPerRow_bitsPerPixel(
            NSBitmapImageRep::alloc(),
            std::ptr::null_mut(),
            side,
            side,
            8,
            4,
            true,
            false,
            NSDeviceRGBColorSpace,
            NSBitmapFormat::AlphaNonpremultiplied,
            bytes_per_row,
            32,
        )
    }
    .ok_or("cannot allocate the icon bitmap")?;

    let destination = rep.bitmapData();
    let needed = size * size * 4;
    if destination.is_null() || rgba.len() < needed {
        return Err("the icon bitmap is too small".to_string());
    }
    // SAFETY: the rep owns a buffer of `size * size * 4` bytes, and `rgba` is
    // at least that long; the copy borrows both for its duration.
    unsafe {
        std::ptr::copy_nonoverlapping(rgba.as_ptr(), destination, needed);
    }

    let properties = NSDictionary::<NSBitmapImageRepPropertyKey, AnyObject>::new();
    // SAFETY: `rep` came from this process, and the dictionary is AppKit's
    // properties dictionary for a PNG representation.
    let data = unsafe { rep.representationUsingType_properties(NSBitmapImageFileType::PNG, &properties) }
        .ok_or("AppKit produced no PNG")?;
    std::fs::write(path, data.to_vec()).map_err(|error| format!("{}: {error}", path.display()))
}

fn build_own() -> Option<Retained<NSImage>> {
    let size = winter::icon::SIZE;
    let rgba = winter::icon::rgba(size);
    let side = isize::try_from(size).ok()?;
    let bytes_per_row = side.checked_mul(4)?;

    // SAFETY: null planes ask AppKit to allocate the buffer; the remaining
    // arguments describe the straight RGBA the icon drawing produces.
    let rep = unsafe {
        NSBitmapImageRep::initWithBitmapDataPlanes_pixelsWide_pixelsHigh_bitsPerSample_samplesPerPixel_hasAlpha_isPlanar_colorSpaceName_bitmapFormat_bytesPerRow_bitsPerPixel(
            NSBitmapImageRep::alloc(),
            std::ptr::null_mut(),
            side,
            side,
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
    let needed = size * size * 4;
    if destination.is_null() || rgba.len() < needed {
        return None;
    }
    // SAFETY: the rep owns a buffer of the size asked for above, and `rgba` is
    // at least that long; the copy borrows both for its duration.
    unsafe {
        std::ptr::copy_nonoverlapping(rgba.as_ptr(), destination, needed);
    }

    let image = NSImage::initWithSize(
        NSImage::alloc(),
        NSSize::new(
            f64::from(u32::try_from(size).ok()?),
            f64::from(u32::try_from(size).ok()?),
        ),
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

    #[test]
    fn the_iconset_answers_every_size_macos_asks_for() {
        assert_eq!(ICONSET.len(), 10);
        let mut sizes: Vec<usize> = ICONSET.iter().map(|&(_, size)| size).collect();
        sizes.sort_unstable();
        sizes.dedup();
        assert_eq!(sizes, vec![16, 32, 64, 128, 256, 512, 1024]);
        assert_eq!(ICONSET.last().map(|&(_, size)| size), Some(1024));
    }
}
