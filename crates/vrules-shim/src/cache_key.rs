//! Content-addressed cache key for the vrules-rest embedding tier, plus the
//! lossless little-endian f32 wire format its HTTP routes exchange.
//!
//! Layout: `blake3(model_version)[..8] ‖ blake3(canon_ns)[..8] ‖ blake3(canon_text)[..16]`.
//! The 128-bit text segment is collision-safe well past billions of entries; the
//! 64-bit model/canon segments namespace by version, so a model swap or a
//! canonicalizer-rule change can never serve a stale vector. Only the host
//! derives keys — the cache component treats them as opaque 64-hex strings.

/// Canon namespace used until rule-driven canonicalization is wired in.
pub const DEFAULT_CANON_NS: &str = "default/v1";

/// A 32-byte content-addressed key.
#[derive(Debug, Clone, Copy, PartialEq, Eq, Hash)]
pub struct CacheKey([u8; 32]);

impl CacheKey {
    /// Derive the key for `canon_text` under a model version + canon namespace.
    #[must_use]
    pub fn new(model_version: &str, canon_ns: &str, canon_text: &str) -> Self {
        let mut key = [0u8; 32];
        key[0..8].copy_from_slice(&blake3::hash(model_version.as_bytes()).as_bytes()[..8]);
        key[8..16].copy_from_slice(&blake3::hash(canon_ns.as_bytes()).as_bytes()[..8]);
        key[16..32].copy_from_slice(&blake3::hash(canon_text.as_bytes()).as_bytes()[..16]);
        Self(key)
    }

    /// Reconstruct a key from a model version, canon namespace, and the raw
    /// 16-byte text hash (the REST `{hash}` segment, = `blake3(canon_text)[..16]`).
    #[must_use]
    pub fn from_parts(model_version: &str, canon_ns: &str, text_hash: [u8; 16]) -> Self {
        let mut key = [0u8; 32];
        key[0..8].copy_from_slice(&blake3::hash(model_version.as_bytes()).as_bytes()[..8]);
        key[8..16].copy_from_slice(&blake3::hash(canon_ns.as_bytes()).as_bytes()[..8]);
        key[16..32].copy_from_slice(&text_hash);
        Self(key)
    }

    /// Lowercase hex of all 32 key bytes — what the cache component stores by.
    #[must_use]
    pub fn to_hex(&self) -> String {
        hex(&self.0)
    }

    /// Hex of the 16-byte text-hash portion — the REST `{hash}` path segment.
    #[must_use]
    pub fn text_hash_hex(&self) -> String {
        hex(&self.0[16..32])
    }
}

/// Parse a REST `{hash}` path segment: exactly 32 lowercase hex characters.
#[must_use]
pub fn parse_text_hash(hash: &str) -> Option<[u8; 16]> {
    if hash.len() != 32 {
        return None;
    }
    let mut bytes = [0u8; 16];
    for (index, chunk) in hash.as_bytes().chunks_exact(2).enumerate() {
        let high = hex_digit(chunk[0])?;
        let low = hex_digit(chunk[1])?;
        bytes[index] = (high << 4) | low;
    }
    Some(bytes)
}

/// Encode a vector as the lossless little-endian f32 wire body.
#[must_use]
pub fn vector_to_le_bytes(vector: &[f32]) -> Vec<u8> {
    let mut bytes = Vec::with_capacity(vector.len() * 4);
    for value in vector {
        bytes.extend_from_slice(&value.to_le_bytes());
    }
    bytes
}

/// Decode a little-endian f32 wire body; `None` unless a whole number of f32s.
#[must_use]
pub fn vector_from_le_bytes(bytes: &[u8]) -> Option<Vec<f32>> {
    if bytes.is_empty() || !bytes.len().is_multiple_of(4) {
        return None;
    }
    Some(
        bytes
            .chunks_exact(4)
            .map(|chunk| f32::from_le_bytes([chunk[0], chunk[1], chunk[2], chunk[3]]))
            .collect(),
    )
}

/// Percent-encode a URL path segment (RFC 3986 unreserved characters pass
/// through), so canon namespaces like `default/v1` survive as one segment.
#[must_use]
pub fn encode_path_segment(segment: &str) -> String {
    let mut encoded = String::with_capacity(segment.len());
    for byte in segment.bytes() {
        if byte.is_ascii_alphanumeric() || matches!(byte, b'-' | b'_' | b'.' | b'~') {
            encoded.push(byte as char);
        } else {
            encoded.push('%');
            encoded.push(
                char::from_digit((byte >> 4) as u32, 16)
                    .unwrap()
                    .to_ascii_uppercase(),
            );
            encoded.push(
                char::from_digit((byte & 0xf) as u32, 16)
                    .unwrap()
                    .to_ascii_uppercase(),
            );
        }
    }
    encoded
}

fn hex(bytes: &[u8]) -> String {
    let mut hex = String::with_capacity(bytes.len() * 2);
    for byte in bytes {
        hex.push(char::from_digit((byte >> 4) as u32, 16).unwrap());
        hex.push(char::from_digit((byte & 0xf) as u32, 16).unwrap());
    }
    hex
}

fn hex_digit(byte: u8) -> Option<u8> {
    match byte {
        b'0'..=b'9' => Some(byte - b'0'),
        b'a'..=b'f' => Some(byte - b'a' + 10),
        _ => None,
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn deterministic() {
        let a = CacheKey::new("model-v1", "default/v1", "User <*> login");
        let b = CacheKey::new("model-v1", "default/v1", "User <*> login");
        assert_eq!(a, b);
        assert_eq!(a.to_hex(), b.to_hex());
        assert_eq!(a.to_hex().len(), 64);
        assert_eq!(a.text_hash_hex().len(), 32);
    }

    #[test]
    fn namespaced_by_every_part() {
        let base = CacheKey::new("model-v1", "default/v1", "text");
        assert_ne!(base, CacheKey::new("model-v2", "default/v1", "text"));
        assert_ne!(base, CacheKey::new("model-v1", "other/v1", "text"));
        assert_ne!(base, CacheKey::new("model-v1", "default/v1", "text "));
    }

    #[test]
    fn from_parts_rebuilds_the_key() {
        let key = CacheKey::new("model-v1", "default/v1", "some text");
        let hash = parse_text_hash(&key.text_hash_hex()).expect("parse hash");
        assert_eq!(CacheKey::from_parts("model-v1", "default/v1", hash), key);
        assert_ne!(
            CacheKey::from_parts("model-v2", "default/v1", hash),
            key,
            "model segment must renamespace"
        );
    }

    #[test]
    fn hash_parsing_rejects_bad_input() {
        assert!(parse_text_hash("").is_none());
        assert!(parse_text_hash("abc").is_none());
        assert!(parse_text_hash(&"a".repeat(31)).is_none());
        assert!(parse_text_hash(&"g".repeat(32)).is_none());
        assert!(parse_text_hash(&"A".repeat(32)).is_none());
        assert!(parse_text_hash(&"a".repeat(32)).is_some());
    }

    #[test]
    fn wire_format_round_trips_bit_exactly() {
        let vector = vec![0.0f32, -0.0, 1.5, -1.0e-40, f32::MIN_POSITIVE];
        let bytes = vector_to_le_bytes(&vector);
        assert_eq!(bytes.len(), vector.len() * 4);
        let decoded = vector_from_le_bytes(&bytes).expect("decode");
        let bits = |values: &[f32]| values.iter().map(|v| v.to_bits()).collect::<Vec<_>>();
        assert_eq!(bits(&decoded), bits(&vector));
        assert!(vector_from_le_bytes(&bytes[..3]).is_none());
        assert!(vector_from_le_bytes(&[]).is_none());
    }

    #[test]
    fn path_segments_encode_reserved_characters() {
        assert_eq!(encode_path_segment("default/v1"), "default%2Fv1");
        assert_eq!(encode_path_segment("plain-1.0_~x"), "plain-1.0_~x");
        assert_eq!(encode_path_segment("a b"), "a%20b");
    }
}
