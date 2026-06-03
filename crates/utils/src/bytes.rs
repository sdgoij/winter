//! Byte slice helpers for reading/writing binary data.

/// Reads a u16 from a byte slice in little-endian format.
/// Returns None if there aren't enough bytes.
#[inline]
pub fn read_u16_le(bytes: &[u8], offset: usize) -> Option<u16> {
    if bytes.len() < offset + 2 {
        return None;
    }
    Some(u16::from_le_bytes([bytes[offset], bytes[offset + 1]]))
}

/// Reads a u32 from a byte slice in little-endian format.
#[inline]
pub fn read_u32_le(bytes: &[u8], offset: usize) -> Option<u32> {
    if bytes.len() < offset + 4 {
        return None;
    }
    Some(u32::from_le_bytes([
        bytes[offset],
        bytes[offset + 1],
        bytes[offset + 2],
        bytes[offset + 3],
    ]))
}

/// Reads a u64 from a byte slice in little-endian format.
#[inline]
pub fn read_u64_le(bytes: &[u8], offset: usize) -> Option<u64> {
    if bytes.len() < offset + 8 {
        return None;
    }
    Some(u64::from_le_bytes([
        bytes[offset],
        bytes[offset + 1],
        bytes[offset + 2],
        bytes[offset + 3],
        bytes[offset + 4],
        bytes[offset + 5],
        bytes[offset + 6],
        bytes[offset + 7],
    ]))
}

/// Reads a u16 from a byte slice in big-endian format.
#[inline]
pub fn read_u16_be(bytes: &[u8], offset: usize) -> Option<u16> {
    if bytes.len() < offset + 2 {
        return None;
    }
    Some(u16::from_be_bytes([bytes[offset], bytes[offset + 1]]))
}

/// Reads a u32 from a byte slice in big-endian format.
#[inline]
pub fn read_u32_be(bytes: &[u8], offset: usize) -> Option<u32> {
    if bytes.len() < offset + 4 {
        return None;
    }
    Some(u32::from_be_bytes([
        bytes[offset],
        bytes[offset + 1],
        bytes[offset + 2],
        bytes[offset + 3],
    ]))
}

/// Reads a u64 from a byte slice in big-endian format.
#[inline]
pub fn read_u64_be(bytes: &[u8], offset: usize) -> Option<u64> {
    if bytes.len() < offset + 8 {
        return None;
    }
    Some(u64::from_be_bytes([
        bytes[offset],
        bytes[offset + 1],
        bytes[offset + 2],
        bytes[offset + 3],
        bytes[offset + 4],
        bytes[offset + 5],
        bytes[offset + 6],
        bytes[offset + 7],
    ]))
}

/// Writes a u16 to a byte slice in little-endian format.
#[inline]
pub fn write_u16_le(bytes: &mut [u8], offset: usize, value: u16) {
    let b = value.to_le_bytes();
    bytes[offset..offset + 2].copy_from_slice(&b);
}

/// Writes a u32 to a byte slice in little-endian format.
#[inline]
pub fn write_u32_le(bytes: &mut [u8], offset: usize, value: u32) {
    let b = value.to_le_bytes();
    bytes[offset..offset + 4].copy_from_slice(&b);
}

/// Writes a u64 to a byte slice in little-endian format.
#[inline]
pub fn write_u64_le(bytes: &mut [u8], offset: usize, value: u64) {
    let b = value.to_le_bytes();
    bytes[offset..offset + 8].copy_from_slice(&b);
}

/// Writes a u16 to a byte slice in big-endian format.
#[inline]
pub fn write_u16_be(bytes: &mut [u8], offset: usize, value: u16) {
    let b = value.to_be_bytes();
    bytes[offset..offset + 2].copy_from_slice(&b);
}

/// Writes a u32 to a byte slice in big-endian format.
#[inline]
pub fn write_u32_be(bytes: &mut [u8], offset: usize, value: u32) {
    let b = value.to_be_bytes();
    bytes[offset..offset + 4].copy_from_slice(&b);
}

/// Writes a u64 to a byte slice in big-endian format.
#[inline]
pub fn write_u64_be(bytes: &mut [u8], offset: usize, value: u64) {
    let b = value.to_be_bytes();
    bytes[offset..offset + 8].copy_from_slice(&b);
}

/// Finds the first occurrence of a pattern in a byte slice.
pub fn find_pattern(haystack: &[u8], pattern: &[u8]) -> Option<usize> {
    if pattern.is_empty() {
        return Some(0);
    }
    if pattern.len() > haystack.len() {
        return None;
    }

    for i in 0..=haystack.len() - pattern.len() {
        if haystack[i..i + pattern.len()] == pattern[..] {
            return Some(i);
        }
    }
    None
}

/// Returns whether a byte slice contains a specific sequence.
pub fn contains_sequence(haystack: &[u8], needle: &[u8]) -> bool {
    find_pattern(haystack, needle).is_some()
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn test_read_write_u16_le() {
        let mut buf = [0u8; 2];
        write_u16_le(&mut buf, 0, 0x1234);
        assert_eq!(buf, [0x34, 0x12]);
        assert_eq!(read_u16_le(&buf, 0), Some(0x1234));
    }

    #[test]
    fn test_read_write_u32_le() {
        let mut buf = [0u8; 4];
        write_u32_le(&mut buf, 0, 0x12345678);
        assert_eq!(buf, [0x78, 0x56, 0x34, 0x12]);
        assert_eq!(read_u32_le(&buf, 0), Some(0x12345678));
    }

    #[test]
    fn test_read_write_u64_le() {
        let mut buf = [0u8; 8];
        write_u64_le(&mut buf, 0, 0x0102030405060708);
        assert_eq!(buf, [0x08, 0x07, 0x06, 0x05, 0x04, 0x03, 0x02, 0x01]);
        assert_eq!(read_u64_le(&buf, 0), Some(0x0102030405060708));
    }

    #[test]
    fn test_read_write_u16_be() {
        let mut buf = [0u8; 2];
        write_u16_be(&mut buf, 0, 0x1234);
        assert_eq!(buf, [0x12, 0x34]);
        assert_eq!(read_u16_be(&buf, 0), Some(0x1234));
    }

    #[test]
    fn test_read_write_u32_be() {
        let mut buf = [0u8; 4];
        write_u32_be(&mut buf, 0, 0x12345678);
        assert_eq!(buf, [0x12, 0x34, 0x56, 0x78]);
        assert_eq!(read_u32_be(&buf, 0), Some(0x12345678));
    }

    #[test]
    fn test_read_out_of_bounds() {
        let buf = [0x12, 0x34];
        assert_eq!(read_u32_le(&buf, 0), None);
        assert_eq!(read_u16_le(&buf, 1), None);
    }

    #[test]
    fn test_find_pattern() {
        let data = b"hello world hello";
        assert_eq!(find_pattern(data, b"world"), Some(6));
        assert_eq!(find_pattern(data, b"hello"), Some(0));
        assert_eq!(find_pattern(data, b"xyz"), None);
        assert_eq!(find_pattern(data, b""), Some(0));
    }

    #[test]
    fn test_contains_sequence() {
        assert!(contains_sequence(b"hello world", b"world"));
        assert!(!contains_sequence(b"hello world", b"xyz"));
    }

    #[test]
    fn test_roundtrip_all_sizes() {
        let test_u16 = |v: u16| {
            let mut buf = [0u8; 2];
            write_u16_le(&mut buf, 0, v);
            assert_eq!(read_u16_le(&buf, 0), Some(v));
            write_u16_be(&mut buf, 0, v);
            assert_eq!(read_u16_be(&buf, 0), Some(v));
        };

        let test_u32 = |v: u32| {
            let mut buf = [0u8; 4];
            write_u32_le(&mut buf, 0, v);
            assert_eq!(read_u32_le(&buf, 0), Some(v));
            write_u32_be(&mut buf, 0, v);
            assert_eq!(read_u32_be(&buf, 0), Some(v));
        };

        let test_u64 = |v: u64| {
            let mut buf = [0u8; 8];
            write_u64_le(&mut buf, 0, v);
            assert_eq!(read_u64_le(&buf, 0), Some(v));
            write_u64_be(&mut buf, 0, v);
            assert_eq!(read_u64_be(&buf, 0), Some(v));
        };

        test_u16(0);
        test_u16(0xFF);
        test_u16(0xFFFF);

        test_u32(0);
        test_u32(0xFF);
        test_u32(0xFFFFFFFF);

        test_u64(0);
        test_u64(0xFF);
        test_u64(0xFFFFFFFFFFFFFFFF);
    }
}

#[cfg(test)]
mod proptests {
    use proptest::prelude::*;

    use super::*;

    proptest! {
        #[test]
        fn test_read_write_roundtrip(data in any::<Vec<u8>>()) {
            let len = data.len();
            let mut buf = data.clone();

            // Test u16 LE roundtrip at offset 0
            if len >= 2 {
                let val = u16::from_le_bytes([buf[0], buf[1]]);
                write_u16_le(&mut buf, 0, val);
                assert_eq!(read_u16_le(&buf, 0), Some(val));
            }

            // Test u16 BE roundtrip at offset 0
            if len >= 2 {
                let val = u16::from_be_bytes([buf[0], buf[1]]);
                write_u16_be(&mut buf, 0, val);
                assert_eq!(read_u16_be(&buf, 0), Some(val));
            }

            // Test u32 LE roundtrip at offset 0
            if len >= 4 {
                let val = u32::from_le_bytes([buf[0], buf[1], buf[2], buf[3]]);
                write_u32_le(&mut buf, 0, val);
                assert_eq!(read_u32_le(&buf, 0), Some(val));
            }

            // Test u32 BE roundtrip at offset 0
            if len >= 4 {
                let val = u32::from_be_bytes([buf[0], buf[1], buf[2], buf[3]]);
                write_u32_be(&mut buf, 0, val);
                assert_eq!(read_u32_be(&buf, 0), Some(val));
            }

            // Test u64 LE roundtrip at offset 0
            if len >= 8 {
                let val = u64::from_le_bytes([
                    buf[0], buf[1], buf[2], buf[3],
                    buf[4], buf[5], buf[6], buf[7],
                ]);
                write_u64_le(&mut buf, 0, val);
                assert_eq!(read_u64_le(&buf, 0), Some(val));
            }

            // Test u64 BE roundtrip at offset 0
            if len >= 8 {
                let val = u64::from_be_bytes([
                    buf[0], buf[1], buf[2], buf[3],
                    buf[4], buf[5], buf[6], buf[7],
                ]);
                write_u64_be(&mut buf, 0, val);
                assert_eq!(read_u64_be(&buf, 0), Some(val));
            }
        }
    }
}
