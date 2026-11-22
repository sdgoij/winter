//! The kitty graphics protocol over APC (`ESC _ G … ESC \`).
//!
//! Ported from <https://sw.kovidgoyal.net/kitty/graphics-protocol/>, as the
//! subset a full-screen program uses: RGBA or RGB pixels, optionally zlib
//! compressed, transmitted in chunks, displayed at a cell rectangle, and
//! deleted by image id (ADR 054).
//!
//! Windows Terminal has no counterpart: it consumes APC strings. This module
//! only runs when ConPTY was created in passthrough mode, so the child's APC
//! reaches the parser at all.

use buffer::TextBuffer;

use crate::sixel_parser::{SIXEL_DEFAULT_CONFORMANCE, SixelParser};

/// The largest decoded image this module will hold, in pixels.
///
/// A 4096x4096 frame is the most a terminal-sized program can use; anything
/// beyond it is a malformed length, not an image.
const MAX_IMAGE_PIXELS: usize = 4096 * 4096;

/// One APC `G` string's control data.
#[derive(Debug, Clone, PartialEq, Eq)]
struct Control {
    /// `a`: the action byte (`T`, `t`, `q`, `d`).
    action: Option<u8>,
    /// `f`: the pixel format (24 = RGB, 32 = RGBA).
    format: u32,
    /// `s`, `v`: the source image's size in pixels.
    source: (i32, i32),
    /// `c`, `r`: the display rectangle in cells.
    columns: Option<i32>,
    rows: Option<i32>,
    /// `o=z`: the payload is zlib compressed.
    compressed: bool,
    /// `m=1`: another chunk follows.
    more: bool,
    /// `i`: the image id.
    id: u32,
    /// `d`: the delete target.
    delete: Option<u8>,
}

impl Control {
    fn parse(bytes: &[u8]) -> Self {
        let mut control = Self {
            action: None,
            format: 32,
            source: (0, 0),
            columns: None,
            rows: None,
            compressed: false,
            more: false,
            id: 0,
            delete: None,
        };

        for pair in bytes.split(|&byte| byte == b',') {
            let Some(equals) = pair.iter().position(|&byte| byte == b'=') else {
                continue;
            };
            let value = &pair[equals + 1..];
            let number = || {
                std::str::from_utf8(value)
                    .ok()
                    .and_then(|text| text.parse::<i64>().ok())
            };
            match &pair[..equals] {
                b"a" => control.action = value.first().copied(),
                b"f" => control.format = number().and_then(|n| u32::try_from(n).ok()).unwrap_or(32),
                b"s" => control.source.0 = clamp_i32(number().unwrap_or(0)),
                b"v" => control.source.1 = clamp_i32(number().unwrap_or(0)),
                b"c" => control.columns = number().map(clamp_i32),
                b"r" => control.rows = number().map(clamp_i32),
                b"o" => control.compressed = value == b"z",
                b"m" => control.more = number() == Some(1),
                b"i" => control.id = number().and_then(|n| u32::try_from(n).ok()).unwrap_or(0),
                b"d" => control.delete = value.first().copied(),
                _ => {}
            }
        }

        control
    }
}

fn clamp_i32(value: i64) -> i32 {
    value.clamp(0, i64::from(i32::MAX)) as i32
}

/// A decoded image: `0xAARRGGBB`, zero meaning transparent.
#[derive(Debug, Clone, PartialEq, Eq)]
struct Image {
    width: i32,
    height: i32,
    pixels: Vec<u32>,
}

/// A displayed image's cell rectangle, so a later delete can clear it.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
struct Placed {
    id: u32,
    column: i32,
    row: i32,
    columns: i32,
    rows: i32,
}

/// An image being assembled from one or more chunks.
struct Transmit {
    control: Control,
    payload: Vec<u8>,
}

/// The protocol's state: the chunk in flight and the images on screen.
#[derive(Debug, Default)]
pub(crate) struct KittyGraphics {
    pending: Option<Transmit>,
    placed: Vec<Placed>,
}

impl std::fmt::Debug for Transmit {
    fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        f.debug_struct("Transmit")
            .field("control", &self.control)
            .field("bytes", &self.payload.len())
            .finish()
    }
}

impl KittyGraphics {
    pub(crate) const fn new() -> Self {
        Self {
            pending: None,
            placed: Vec::new(),
        }
    }

    /// Handles one APC string, whose first byte must be `G`.
    pub(crate) fn apply(&mut self, apc: &[u8], buffer: &mut TextBuffer) {
        let Some(rest) = apc.strip_prefix(b"G") else {
            return;
        };
        let (control_bytes, payload) = match rest.iter().position(|&byte| byte == b';') {
            Some(index) => (&rest[..index], &rest[index + 1..]),
            None => (rest, &[][..]),
        };
        let control = Control::parse(control_bytes);

        if control.action == Some(b'd') {
            self.delete(&control, buffer);
            return;
        }

        match &mut self.pending {
            // A continuation: it repeats `m` and carries the next payload.
            Some(pending) if control.action.is_none() => {
                pending.control.more = control.more;
            }
            // A new image, or the previous chunked transmit was abandoned.
            _ => {
                if control.action.is_none() {
                    return;
                }
                self.pending = Some(Transmit {
                    control: control.clone(),
                    payload: Vec::new(),
                });
            }
        }

        let Some(pending) = self.pending.as_mut() else {
            return;
        };
        pending.payload.extend_from_slice(payload);
        if pending.control.more {
            return;
        }

        let Some(pending) = self.pending.take() else {
            return;
        };
        let Some(image) = decode(&pending.control, &pending.payload) else {
            return;
        };
        if pending.control.action == Some(b'T') {
            self.place(&pending.control, &image, buffer);
        }
    }

    /// Paints `image` at the cursor's cell and records the rectangle.
    fn place(&mut self, control: &Control, image: &Image, buffer: &mut TextBuffer) {
        // A retransmit of the same id replaces the old placement.
        self.erase_id(control.id, buffer);

        let (cell_width, cell_height) = Self::cell_size();
        let page_width = buffer.screen_width();
        let page_height = buffer.screen_height();
        let start = buffer.cursor().position();

        if start.1 >= page_width || start.0 >= page_height {
            return;
        }

        let columns = control
            .columns
            .unwrap_or_else(|| (image.width + cell_width - 1) / cell_width)
            .max(1);
        let rows = control.rows.unwrap_or_else(|| {
            // Height follows the source's aspect ratio at the chosen width.
            let width_px = i64::from(columns) * i64::from(cell_width);
            let height_px = width_px * i64::from(image.height) / i64::from(image.width.max(1));
            ((height_px + i64::from(cell_height) - 1) / i64::from(cell_height)) as i32
        });
        let columns = columns.min(page_width - start.1);
        let rows = rows.max(1).min(page_height - start.0);
        if columns <= 0 || rows <= 0 {
            return;
        }

        let band_width = columns * cell_width;
        let target = (i64::from(band_width), i64::from(rows) * i64::from(cell_height));
        let mut band = vec![0u32; (band_width * cell_height) as usize];

        for cell_row in 0..rows {
            for py in 0..cell_height {
                let source_y = ((i64::from(cell_row) * i64::from(cell_height) + i64::from(py))
                    * i64::from(image.height)
                    / target.1) as i32;
                for px in 0..band_width {
                    let source_x = (i64::from(px) * i64::from(image.width) / target.0) as i32;
                    let index = (source_y * image.width + source_x) as usize;
                    band[(py * band_width + px) as usize] = image.pixels.get(index).copied().unwrap_or(0);
                }
            }

            let Some(row) = buffer.get_row_at_screen_line_mut(start.0 + cell_row) else {
                break;
            };
            if row.image_slice().is_none() {
                row.set_image_slice(Some(buffer::ImageSlice::new((cell_width, cell_height))));
            }
            let Some(slice) = row.image_slice_mut() else {
                break;
            };

            let begin = start.1;
            let end = start.1 + columns;
            // The first call sizes the band; the second borrows the pixels.
            slice.mutable_pixels(begin, end);
            let offset = (begin - slice.column_begin()) * cell_width;
            let pixel_width = slice.pixel_width();
            let pixels = slice.mutable_pixels(begin, end);
            for py in 0..cell_height {
                let source_start = (py * band_width) as usize;
                let dest_start = (py * pixel_width + offset) as usize;
                if dest_start + band_width as usize <= pixels.len() {
                    pixels[dest_start..dest_start + band_width as usize]
                        .copy_from_slice(&band[source_start..source_start + band_width as usize]);
                }
            }
        }

        self.placed.push(Placed {
            id: control.id,
            column: start.1,
            row: start.0,
            columns,
            rows,
        });
    }

    /// Clears the placements a delete names.
    fn delete(&mut self, control: &Control, buffer: &mut TextBuffer) {
        let by_id = control.delete.is_some_and(|target| matches!(target, b'i' | b'I'));
        let id = control.id;

        let (erased, kept) = std::mem::take(&mut self.placed)
            .into_iter()
            .partition::<Vec<_>, _>(|placed| !by_id || placed.id == id);

        for placed in erased {
            erase(buffer, &placed);
        }
        self.placed = kept;
    }

    fn erase_id(&mut self, id: u32, buffer: &mut TextBuffer) {
        let (erased, kept) = std::mem::take(&mut self.placed)
            .into_iter()
            .partition::<Vec<_>, _>(|placed| placed.id == id);

        for placed in erased {
            erase(buffer, &placed);
        }
        self.placed = kept;
    }

    /// The cell size the bands are built at.
    fn cell_size() -> (i32, i32) {
        SixelParser::cell_size_for_level(SIXEL_DEFAULT_CONFORMANCE)
    }
}

/// Clears the pixels an image covered.
fn erase(buffer: &mut TextBuffer, placed: &Placed) {
    for row in placed.row..placed.row + placed.rows {
        if let Some(row) = buffer.get_row_at_screen_line_mut(row) {
            row.erase_image_cells(placed.column, placed.column + placed.columns);
        }
    }
}

/// Decodes a complete payload into an image.
fn decode(control: &Control, payload: &[u8]) -> Option<Image> {
    let text = std::str::from_utf8(payload).ok()?;
    let bytes = vt::decode(text).ok()?;
    let bytes = if control.compressed {
        miniz_oxide::inflate::decompress_to_vec_zlib(&bytes).ok()?
    } else {
        bytes
    };

    let (width, height) = control.source;
    if width <= 0 || height <= 0 {
        return None;
    }
    let count = (width as usize).checked_mul(height as usize)?;
    if count > MAX_IMAGE_PIXELS {
        return None;
    }

    let pixels = match control.format {
        32 => {
            if bytes.len() < count * 4 {
                return None;
            }
            bytes[..count * 4]
                .as_chunks::<4>()
                .0
                .iter()
                .map(|pixel| {
                    let (red, green, blue, alpha) = (
                        u32::from(pixel[0]),
                        u32::from(pixel[1]),
                        u32::from(pixel[2]),
                        u32::from(pixel[3]),
                    );
                    if alpha == 0 {
                        0
                    } else {
                        (alpha << 24) | (red << 16) | (green << 8) | blue
                    }
                })
                .collect()
        }
        24 => {
            if bytes.len() < count * 3 {
                return None;
            }
            bytes[..count * 3]
                .as_chunks::<3>()
                .0
                .iter()
                .map(|pixel| {
                    0xFF00_0000 | (u32::from(pixel[0]) << 16) | (u32::from(pixel[1]) << 8) | u32::from(pixel[2])
                })
                .collect()
        }
        _ => return None,
    };

    Some(Image { width, height, pixels })
}

#[cfg(test)]
mod tests {
    use super::*;

    fn encode(pixels: &[(u8, u8, u8, u8)]) -> Vec<u8> {
        let raw: Vec<u8> = pixels
            .iter()
            .flat_map(|&(red, green, blue, alpha)| [red, green, blue, alpha])
            .collect();
        vt::encode(&raw).into_bytes()
    }

    #[test]
    fn the_games_first_and_continuation_chunks_parse() {
        let first = Control::parse(b"a=T,f=32,s=640,v=360,c=100,r=50,i=1,p=1,o=z,C=1,z=-1,m=1,q=1");
        assert_eq!(first.action, Some(b'T'));
        assert_eq!(first.format, 32);
        assert_eq!(first.source, (640, 360));
        assert_eq!(first.columns, Some(100));
        assert_eq!(first.rows, Some(50));
        assert_eq!(first.id, 1);
        assert!(first.compressed);
        assert!(first.more);

        let more = Control::parse(b"m=1,q=1");
        assert_eq!(more.action, None);
        assert!(more.more);

        let last = Control::parse(b"m=0,q=1");
        assert!(!last.more);
    }

    #[test]
    fn a_delete_names_its_target() {
        let delete = Control::parse(b"a=d,d=I,i=1,q=1");
        assert_eq!(delete.action, Some(b'd'));
        assert_eq!(delete.delete, Some(b'I'));
        assert_eq!(delete.id, 1);
    }

    #[test]
    fn rgba_decodes_to_argb_with_zero_for_transparency() {
        let image = decode(
            &Control::parse(b"s=2,v=1,f=32"),
            &encode(&[(255, 0, 0, 255), (0, 0, 0, 0)]),
        )
        .expect("an image");

        assert_eq!(image.width, 2);
        assert_eq!(image.pixels, vec![0xFFFF_0000, 0]);
    }

    #[test]
    fn rgb_decodes_as_opaque() {
        let raw = [255u8, 128, 0];
        let payload = vt::encode(&raw).into_bytes();
        let image = decode(&Control::parse(b"s=1,v=1,f=24"), &payload).expect("an image");

        assert_eq!(image.pixels, vec![0xFFFF_8000]);
    }

    #[test]
    fn a_zlib_payload_decodes() {
        let raw = [10u8, 20, 30, 255];
        let compressed = miniz_oxide::deflate::compress_to_vec_zlib(&raw, 6);
        let payload = vt::encode(&compressed).into_bytes();

        let image = decode(&Control::parse(b"s=1,v=1,f=32,o=z"), &payload).expect("an image");
        assert_eq!(image.pixels, vec![0xFF0A_141E]);
    }

    #[test]
    fn a_short_payload_is_refused() {
        assert!(decode(&Control::parse(b"s=4,v=4,f=32"), b"AAAA").is_none());
    }
}
