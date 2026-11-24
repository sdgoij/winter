//! The icon's pixels, drawn rather than shipped.
//!
//! Upstream embeds an `.ico` through its packaging; this port has no binary
//! asset in the tree, so the icon is computed from the same geometry at every
//! size. The macOS bundle draws its iconset from [`rgba`], and the Windows build
//! script encodes [`ico`] and embeds it in the executable (ADR 058).
//!
//! This file is included by `build.rs` as well as by `icon.rs`, so it must stay
//! free of crate imports.

/// The size of the window icon, in pixels.
pub const SIZE: usize = 64;

/// The square the geometry is expressed in; drawing scales into it.
const DESIGN: f32 = SIZE as f32;

/// The icon's background.
const BACKGROUND: [u8; 3] = [0x0C, 0x0C, 0x0C];
/// The prompt's colour.
const FOREGROUND: [u8; 3] = [0x3B, 0x8E, 0xEA];
/// The rounded corners' radius, in design units.
const RADIUS: f32 = 12.0;
/// The prompt's stroke width, in design units.
const STROKE: f32 = 3.5;

/// The icon's pixels at `size`, as straight RGBA.
pub fn rgba(size: usize) -> Vec<u8> {
    let mut rgba = vec![0u8; size * size * 4];
    let scale = DESIGN / size as f32;

    for y in 0..size {
        for x in 0..size {
            let fx = (x as f32 + 0.5) * scale;
            let fy = (y as f32 + 0.5) * scale;
            let alpha = if inside_rounded_square(fx, fy) { 255 } else { 0 };

            // A `>` and an underscore: the shape every terminal's title bar uses.
            let chevron = distance_to_segment(fx, fy, 18.0, 18.0, 34.0, 32.0)
                .min(distance_to_segment(fx, fy, 18.0, 46.0, 34.0, 32.0));
            let underscore = if (40.0..52.0).contains(&fx) {
                (43.0 - fy).abs()
            } else {
                f32::MAX
            };
            let ink = chevron.min(underscore) <= STROKE;

            let colour = if ink { FOREGROUND } else { BACKGROUND };
            let offset = (y * size + x) * 4;
            rgba[offset] = colour[0];
            rgba[offset + 1] = colour[1];
            rgba[offset + 2] = colour[2];
            rgba[offset + 3] = alpha;
        }
    }

    rgba
}

/// Encodes the icon as a Windows ICO at each requested pixel size.
///
/// Each image is an uncompressed 32-bit bottom-up DIB, which every Windows
/// version reads; the alpha channel carries the rounded corners.
///
/// `build.rs` includes this file and calls the encoder; in the library only a
/// test needs it, so it is allowed to be dead here.
#[cfg_attr(not(test), allow(dead_code))]
pub fn ico(sizes: &[usize]) -> Vec<u8> {
    let images: Vec<(usize, Vec<u8>)> = sizes
        .iter()
        .map(|&size| {
            assert!(size <= 256, "an ICO dimension is one byte, up to 256");
            (size, dib(size, &rgba(size)))
        })
        .collect();

    let mut out = Vec::new();
    out.extend_from_slice(&0u16.to_le_bytes()); // reserved
    out.extend_from_slice(&1u16.to_le_bytes()); // type: icon
    out.extend_from_slice(&(images.len() as u16).to_le_bytes());

    // The entries name their payloads by absolute offset, so it is computed
    // from the header size before any image is appended.
    let mut offset = 6 + 16 * images.len();
    for (size, image) in &images {
        let dimension = if *size == 256 { 0 } else { *size as u8 };
        out.push(dimension);
        out.push(dimension);
        out.push(0); // palette entries
        out.push(0); // reserved
        out.extend_from_slice(&1u16.to_le_bytes()); // planes
        out.extend_from_slice(&32u16.to_le_bytes()); // bits per pixel
        out.extend_from_slice(&(image.len() as u32).to_le_bytes());
        out.extend_from_slice(&(offset as u32).to_le_bytes());
        offset += image.len();
    }

    for (_, image) in images {
        out.extend_from_slice(&image);
    }
    out
}

/// One uncompressed 32-bit image, header first.
#[cfg_attr(not(test), allow(dead_code))]
fn dib(size: usize, rgba: &[u8]) -> Vec<u8> {
    let mut out = Vec::new();
    out.extend_from_slice(&40u32.to_le_bytes()); // BITMAPINFOHEADER
    out.extend_from_slice(&(size as i32).to_le_bytes());
    out.extend_from_slice(&((size * 2) as i32).to_le_bytes()); // colour rows and mask
    out.extend_from_slice(&1u16.to_le_bytes()); // planes
    out.extend_from_slice(&32u16.to_le_bytes()); // bits per pixel
    out.extend_from_slice(&0u32.to_le_bytes()); // BI_RGB
    out.extend_from_slice(&((size * size * 4) as u32).to_le_bytes());
    out.extend_from_slice(&0i32.to_le_bytes()); // horizontal resolution
    out.extend_from_slice(&0i32.to_le_bytes()); // vertical resolution
    out.extend_from_slice(&0u32.to_le_bytes()); // colours in the palette
    out.extend_from_slice(&0u32.to_le_bytes()); // colours that matter

    // The colour rows run bottom-up, and their channels are BGRA.
    for y in (0..size).rev() {
        for x in 0..size {
            let offset = (y * size + x) * 4;
            out.push(rgba[offset + 2]);
            out.push(rgba[offset + 1]);
            out.push(rgba[offset]);
            out.push(rgba[offset + 3]);
        }
    }

    // The AND mask is one bit per pixel padded to four bytes; all zero leaves
    // the decision to the alpha channel.
    let mask_row = size.div_ceil(32) * 4;
    out.resize(out.len() + mask_row * size, 0);
    out
}

/// Whether a point in design space is inside the rounded square.
fn inside_rounded_square(x: f32, y: f32) -> bool {
    if !(0.0..DESIGN).contains(&x) || !(0.0..DESIGN).contains(&y) {
        return false;
    }

    // The corner circle's centre, clamped so the straight edges are unaffected.
    let cx = x.clamp(RADIUS, DESIGN - RADIUS);
    let cy = y.clamp(RADIUS, DESIGN - RADIUS);
    let (dx, dy) = (x - cx, y - cy);

    dx * dx + dy * dy <= RADIUS * RADIUS
}

/// The distance from a point to a line segment.
fn distance_to_segment(px: f32, py: f32, x1: f32, y1: f32, x2: f32, y2: f32) -> f32 {
    let (vx, vy) = (x2 - x1, y2 - y1);
    let (wx, wy) = (px - x1, py - y1);
    let length = vx * vx + vy * vy;
    let t = if length == 0.0 {
        0.0
    } else {
        ((wx * vx + wy * vy) / length).clamp(0.0, 1.0)
    };
    let (dx, dy) = (wx - t * vx, wy - t * vy);
    (dx * dx + dy * dy).sqrt()
}

#[cfg(test)]
mod tests {
    use super::*;

    fn pixel(icon: &[u8], x: usize, y: usize) -> [u8; 4] {
        let offset = (y * SIZE + x) * 4;
        [icon[offset], icon[offset + 1], icon[offset + 2], icon[offset + 3]]
    }

    #[test]
    fn the_icon_is_a_square_of_rgba_pixels() {
        let icon = rgba(SIZE);
        assert_eq!(icon.len(), SIZE * SIZE * 4);
    }

    #[test]
    fn the_corners_are_rounded_and_the_middle_is_painted() {
        let icon = rgba(SIZE);

        assert_eq!(pixel(&icon, 0, 0)[3], 0, "a corner is transparent");
        assert_eq!(pixel(&icon, SIZE - 1, 0)[3], 0, "a corner is transparent");
        assert_eq!(pixel(&icon, SIZE / 2, SIZE / 2)[3], 255, "the middle is opaque");
    }

    #[test]
    fn the_prompt_is_drawn_in_the_foreground_colour() {
        let icon = rgba(SIZE);
        let mut ink = 0;
        for y in 0..SIZE {
            for x in 0..SIZE {
                let [r, g, b, a] = pixel(&icon, x, y);
                if a == 255 && [r, g, b] == FOREGROUND {
                    ink += 1;
                }
            }
        }
        assert!(ink > 80, "the chevron and the underscore cover {ink} pixels");
    }

    #[test]
    fn the_background_fills_the_square() {
        let icon = rgba(SIZE);
        let [r, g, b, a] = pixel(&icon, 4, SIZE / 2);
        assert_eq!(a, 255);
        assert_eq!([r, g, b], BACKGROUND);
    }

    #[test]
    fn the_icon_can_be_drawn_at_another_size() {
        let icon = rgba(16);
        assert_eq!(icon.len(), 16 * 16 * 4);
        assert_eq!(icon[3], 0, "the top-left corner stays transparent");
    }

    #[test]
    fn the_ico_has_one_entry_per_size() {
        let sizes = [16usize, 32, 48, 64, 256];
        let bytes = ico(&sizes);

        assert_eq!(&bytes[..4], &[0, 0, 1, 0], "ICO header");
        assert_eq!(u16::from_le_bytes([bytes[4], bytes[5]]) as usize, sizes.len());

        let mut end = 6 + 16 * sizes.len();
        for (index, size) in sizes.iter().enumerate() {
            let entry = 6 + index * 16;
            let dimension = if *size == 256 { 0 } else { *size as u8 };
            assert_eq!((bytes[entry], bytes[entry + 1]), (dimension, dimension));
            assert_eq!(u16::from_le_bytes([bytes[entry + 4], bytes[entry + 5]]), 1);
            assert_eq!(u16::from_le_bytes([bytes[entry + 6], bytes[entry + 7]]), 32);

            let length = u32::from_le_bytes(bytes[entry + 8..entry + 12].try_into().expect("length"));
            let offset = u32::from_le_bytes(bytes[entry + 12..entry + 16].try_into().expect("offset")) as usize;
            assert_eq!(offset, end, "the payloads follow the entries in order");
            assert_eq!(
                u32::from_le_bytes(bytes[offset..offset + 4].try_into().expect("header")),
                40,
                "a BITMAPINFOHEADER"
            );
            assert_eq!(
                i32::from_le_bytes(bytes[offset + 4..offset + 8].try_into().expect("width")),
                *size as i32
            );

            end += length as usize;
        }
        assert_eq!(end, bytes.len(), "no trailing bytes");
    }
}
