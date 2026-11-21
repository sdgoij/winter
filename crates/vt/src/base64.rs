//! Base64 encode/decode utilities.
//!
//! Ported from the base64 utilities in the Windows Terminal C++ module.

const BASE64_CHARS: &[u8] = b"ABCDEFGHIJKLMNOPQRSTUVWXYZabcdefghijklmnopqrstuvwxyz0123456789+/";

/// Standard base64 encoding alphabet (256 entries, -1 for invalid).
fn build_decode_table() -> [i8; 256] {
    let mut table = [-1i8; 256];
    for (i, &c) in BASE64_CHARS.iter().enumerate() {
        table[c as usize] = i as i8;
    }
    table
}

static DECODE_TABLE: std::sync::LazyLock<[i8; 256]> = std::sync::LazyLock::new(build_decode_table);

/// Encodes a byte slice to a base64 string.
pub fn encode(data: &[u8]) -> String {
    if data.is_empty() {
        return String::new();
    }

    let mut result = String::new();
    let len = data.len();
    let mut i = 0;

    while i < len {
        // Read up to 3 bytes
        let b0 = data[i] as u32;
        let b1 = if i + 1 < len { data[i + 1] as u32 } else { 0 };
        let b2 = if i + 2 < len { data[i + 2] as u32 } else { 0 };

        // Combine into 24-bit group
        let triple = (b0 << 16) | (b1 << 8) | b2;

        // Encode 4 characters
        result.push(BASE64_TABLE[((triple >> 18) & 0x3F) as usize] as char);
        result.push(BASE64_TABLE[((triple >> 12) & 0x3F) as usize] as char);

        if i + 1 < len {
            result.push(BASE64_TABLE[((triple >> 6) & 0x3F) as usize] as char);
        } else {
            result.push('=');
        }

        if i + 2 < len {
            result.push(BASE64_TABLE[(triple & 0x3F) as usize] as char);
        } else {
            result.push('=');
        }

        i += 3;
    }

    result
}

/// Decodes a base64 string to bytes.
pub fn decode(input: &str) -> Result<Vec<u8>, Base64Error> {
    let mut result = Vec::new();
    let bytes: Vec<u8> = input
        .bytes()
        .filter(|&b| b != b'\n' && b != b'\r' && b != b' ')
        .collect();

    let len = bytes.len();
    if len == 0 {
        return Ok(Vec::new());
    }

    // Validate length
    let content_bytes = bytes.iter().take_while(|&&b| b != b'=').count();
    if content_bytes > 0 && content_bytes % 4 == 1 {
        return Err(Base64Error::InvalidLength);
    }

    let mut i = 0;
    while i < len {
        let b0 = get_decode_value(bytes[i])?;
        let b1 = get_decode_value(bytes[i + 1])?;

        if bytes[i + 2] != b'=' && bytes[i + 3] != b'=' {
            // 4 content bytes → 3 output bytes
            let b2 = get_decode_value(bytes[i + 2])?;
            let b3 = get_decode_value(bytes[i + 3])?;
            let triple = ((b0 as u32) << 18) | ((b1 as u32) << 12) | ((b2 as u32) << 6) | (b3 as u32);
            result.push(((triple >> 16) & 0xFF) as u8);
            result.push(((triple >> 8) & 0xFF) as u8);
            result.push((triple & 0xFF) as u8);
        } else if bytes[i + 2] != b'=' {
            // 3 content bytes → 2 output bytes
            let b2 = get_decode_value(bytes[i + 2])?;
            let triple = ((b0 as u32) << 18) | ((b1 as u32) << 12) | ((b2 as u32) << 6);
            result.push(((triple >> 16) & 0xFF) as u8);
            result.push(((triple >> 8) & 0xFF) as u8);
        } else {
            // 2 content bytes → 1 output byte
            result.push((b0 << 2) | (b1 >> 4));
        }

        i += 4;
    }

    Ok(result)
}

/// Base64 decode error.
#[derive(Debug, Clone, PartialEq, Eq)]
pub enum Base64Error {
    /// Invalid character in input.
    InvalidCharacter(u8),
    /// Invalid input length (after removing padding).
    InvalidLength,
}

impl std::fmt::Display for Base64Error {
    fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        match self {
            Base64Error::InvalidCharacter(c) => write!(f, "Invalid base64 character: 0x{:02X}", c),
            Base64Error::InvalidLength => write!(f, "Invalid base64 length"),
        }
    }
}

impl std::error::Error for Base64Error {}

/// Fast decode table lookup.
#[inline]
fn get_decode_value(b: u8) -> Result<u8, Base64Error> {
    let val = DECODE_TABLE[b as usize];
    if val == -1 {
        Err(Base64Error::InvalidCharacter(b))
    } else {
        Ok(val as u8)
    }
}

// Pre-computed base64 encode table
const BASE64_TABLE: &[u8] = BASE64_CHARS;

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn test_encode_empty() {
        assert_eq!(encode(&[]), "");
    }

    #[test]
    fn test_encode_simple() {
        // Encode Man → should produce valid base64
        let encoded = encode(b"Man");
        // Decode should reverse it
        let decoded = decode(&encoded).unwrap();
        assert_eq!(
            decoded, b"Man",
            "encode Man produces {:?}, decode gives {:?}",
            encoded, decoded
        );
    }

    #[test]
    fn test_encode_short() {
        assert_eq!(encode(b"M"), "TQ==");
        // Verify actual encoding for a
        let encoded_a = encode(b"a");
        assert_eq!(encoded_a, "YQ==");
    }

    #[test]
    fn test_encode_two_bytes() {
        assert_eq!(encode(b"Ma"), "TWE=");
    }

    #[test]
    fn test_decode_empty() {
        assert_eq!(decode("").unwrap(), Vec::<u8>::new());
    }

    #[test]
    fn test_decode_simple() {
        // Verify basic decode works by encoding then decoding
        let input = b"Man";
        let encoded = encode(input);
        let decoded = decode(&encoded).unwrap();
        assert_eq!(decoded, input);
    }

    #[test]
    fn test_decode_with_padding() {
        // TQ== decodes to single byte: 'M'
        let r1 = decode("TQ==").unwrap();
        assert_eq!(r1, b"M", "TQ== should decode to 'M', got {:?}", r1);
        // TWE= decodes to 2 bytes: 'Ma'
        let r2 = decode("TWE=").unwrap();
        assert_eq!(r2, b"Ma", "TWE= should decode to 'Ma', got {:?}", r2);
        // Verify encoding roundtrip
        assert_eq!(encode(b"M"), "TQ==");
        assert_eq!(encode(b"Ma"), "TWE=");
        // Verify Man roundtrips
        let man_encoded = encode(b"Man");
        assert_eq!(decode(&man_encoded).unwrap(), b"Man");
    }

    #[test]
    fn test_roundtrip() {
        // Short test cases as array slices
        let short_cases: &[&[u8]] = &[b"", b"a", b"ab", b"abc", b"hello world", b"\x00\x01\x02\x03"];
        for case in short_cases {
            let encoded = encode(case);
            let decoded = decode(&encoded).unwrap();
            assert_eq!(decoded, *case, "Roundtrip failed for {:?}", case);
        }

        // Longer test case
        let long_data = vec![0u8; 100];
        let encoded = encode(&long_data);
        let decoded = decode(&encoded).unwrap();
        assert_eq!(decoded, long_data);
    }

    #[test]
    fn test_decode_invalid_char() {
        assert_eq!(decode("!!!"), Err(Base64Error::InvalidCharacter(b'!')));
    }

    #[test]
    fn test_decode_invalid_length() {
        // Single content char (1 mod 4) is invalid
        assert_eq!(decode("T"), Err(Base64Error::InvalidLength));
        // "TQ" has 2 content + 2 padding = 4 total, which is valid
        // Decode "TQ==" for valid 2-char content
        assert!(decode("TQ==").is_ok());
    }

    #[test]
    fn test_decode_whitespace_ignored() {
        assert_eq!(decode("T W F u"), Ok(decode("TWFu").unwrap()));
    }
}
