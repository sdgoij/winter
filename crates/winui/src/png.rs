//! The smallest PNG writer that can put a pixel grid into an `ImageSource`.
//!
//! A XAML `ImageSource` is built from an encoded image, so the icon a profile's
//! executable carries has to leave this process as a file format. PNG is the
//! one every Windows release decodes. Nothing here is clever: the rows are
//! stored, the deflate blocks are stored, and the only compressed-looking parts
//! are the two checksums, which are a dozen lines each and are specified in RFC
//! 1950 and 1952.

/// A PNG holding `rgba`, four bytes per pixel with the top row first.
///
/// An empty grid has no PNG: PNG has no way to state a zero-width image, so
/// nothing is written and the caller gets no bytes.
pub(super) fn encode(width: u32, height: u32, rgba: &[u8]) -> Vec<u8> {
    let stride = width as usize * 4;
    if stride == 0 || height == 0 {
        return Vec::new();
    }

    let mut rows = Vec::with_capacity((stride + 1) * height as usize);
    for row in rgba.chunks_exact(stride).take(height as usize) {
        // Filter 0, "None": an icon is small, and a filter would be another
        // decoder rule to get right on the other side.
        rows.push(0);
        rows.extend_from_slice(row);
    }

    let mut png = Vec::from(*b"\x89PNG\r\n\x1a\n");
    let mut header = Vec::with_capacity(13);
    header.extend_from_slice(&width.to_be_bytes());
    header.extend_from_slice(&height.to_be_bytes());
    // Eight bits per channel, colour type 6 (RGBA), deflate, adaptive filtering,
    // no interlacing.
    header.extend_from_slice(&[8, 6, 0, 0, 0]);
    chunk(&mut png, b"IHDR", &header);
    chunk(&mut png, b"IDAT", &zlib(&rows));
    chunk(&mut png, b"IEND", &[]);
    png
}

/// One PNG chunk: the length, the kind, the body, and the CRC of kind and body.
fn chunk(png: &mut Vec<u8>, kind: &[u8; 4], body: &[u8]) {
    let length = u32::try_from(body.len()).unwrap_or(u32::MAX);
    png.extend_from_slice(&length.to_be_bytes());
    png.extend_from_slice(kind);
    png.extend_from_slice(body);

    let mut crc = Crc32::new();
    crc.update(kind);
    crc.update(body);
    png.extend_from_slice(&crc.finish().to_be_bytes());
}

/// A zlib stream of stored blocks, and its Adler-32.
///
/// `0x78 0x01` is a 32 KiB window with the fastest compression level, which is
/// the truth: there is no compression. An icon is a few kilobytes and a
/// deflate implementation is not what this module is for.
fn zlib(bytes: &[u8]) -> Vec<u8> {
    let mut stream = vec![0x78, 0x01];
    // Stored blocks carry a 65535-byte maximum, so a larger payload is split.
    let mut blocks = bytes.chunks(0xFFFF).peekable();
    if blocks.peek().is_none() {
        stream.extend_from_slice(&[1, 0, 0, 0xFF, 0xFF]);
    }
    while let Some(block) = blocks.next() {
        let last = blocks.peek().is_none();
        let length = u16::try_from(block.len()).unwrap_or(0xFFFF);
        stream.push(u8::from(last));
        stream.extend_from_slice(&length.to_le_bytes());
        stream.extend_from_slice(&(!length).to_le_bytes());
        stream.extend_from_slice(block);
    }
    stream.extend_from_slice(&adler32(bytes).to_be_bytes());
    stream
}

/// The Adler-32 of `bytes`, as RFC 1950 defines it.
fn adler32(bytes: &[u8]) -> u32 {
    let (mut low, mut high) = (1u32, 0u32);
    for &byte in bytes {
        low = (low + u32::from(byte)) % 65521;
        high = (high + low) % 65521;
    }
    (high << 16) | low
}

/// The CRC-32 PNG chunks carry, as ISO 3309 defines it.
struct Crc32(u32);

impl Crc32 {
    const fn new() -> Self {
        Self(0xFFFF_FFFF)
    }

    fn update(&mut self, bytes: &[u8]) {
        for &byte in bytes {
            let mut value = (self.0 ^ u32::from(byte)) & 0xFF;
            for _ in 0..8 {
                value = if value & 1 == 1 {
                    0xEDB8_8320 ^ (value >> 1)
                } else {
                    value >> 1
                };
            }
            self.0 = (self.0 >> 8) ^ value;
        }
    }

    const fn finish(self) -> u32 {
        self.0 ^ 0xFFFF_FFFF
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn the_checksums_match_their_specifications() {
        // Both vectors are the ones the specifications' own articles print.
        let mut crc = Crc32::new();
        crc.update(b"123456789");
        assert_eq!(crc.finish(), 0xCBF4_3926);
        assert_eq!(adler32(b"Wikipedia"), 0x11E6_0398);
    }

    #[test]
    fn an_encoded_image_inflates_back_to_its_pixels() {
        let pixels: Vec<u8> = (0..=(6 * 4 - 1)).collect();
        let png = encode(3, 2, &pixels);

        assert_eq!(&png[..8], b"\x89PNG\r\n\x1a\n");
        assert_eq!(
            chunk_body(&png, b"IHDR"),
            &[0u8, 0, 0, 3, 0, 0, 0, 2, 8, 6, 0, 0, 0][..]
        );

        let raw = inflate(zlib_body(&png));
        let rows: Vec<&[u8]> = raw.as_chunks::<13>().0.iter().map(|row| &row[..]).collect();
        assert_eq!(rows.len(), 2);
        assert!(rows.iter().all(|row| row[0] == 0), "every row states filter None");
        let decoded: Vec<u8> = rows.iter().flat_map(|row| row[1..].to_vec()).collect();
        assert_eq!(decoded, pixels);
    }

    #[test]
    fn a_payload_over_one_block_is_split_and_still_reads_back() {
        let pixels = vec![7u8; 70_000 * 4];
        let png = encode(70_000, 1, &pixels);

        let mut expected = vec![0u8];
        expected.extend_from_slice(&pixels);
        assert_eq!(inflate(zlib_body(&png)), expected);
    }

    #[test]
    fn an_empty_grid_writes_nothing() {
        assert!(encode(0, 4, &[]).is_empty());
        assert!(encode(4, 0, &[]).is_empty());
    }

    /// The body of the first chunk of `kind`.
    fn chunk_body<'a>(png: &'a [u8], kind: &[u8; 4]) -> &'a [u8] {
        let mut at = 8;
        while at + 8 <= png.len() {
            let length = u32::from_be_bytes(png[at..at + 4].try_into().unwrap()) as usize;
            if &png[at + 4..at + 8] == kind {
                return &png[at + 8..at + 8 + length];
            }
            at += 12 + length;
        }
        panic!("no {:?} chunk", String::from_utf8_lossy(kind));
    }

    fn zlib_body(png: &[u8]) -> &[u8] {
        let body = chunk_body(png, b"IDAT");
        &body[2..body.len() - 4]
    }

    /// The bytes a stream of stored deflate blocks holds.
    fn inflate(blocks: &[u8]) -> Vec<u8> {
        let mut raw = Vec::new();
        let mut at = 0;
        loop {
            let last = blocks[at] & 1 == 1;
            let length = u16::from_le_bytes([blocks[at + 1], blocks[at + 2]]) as usize;
            raw.extend_from_slice(&blocks[at + 5..at + 5 + length]);
            at += 5 + length;
            if last {
                return raw;
            }
        }
    }
}
