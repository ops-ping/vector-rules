// Content-addressing for the static embedding cache, mirroring the daemon's
// `crates/vrules-shim/src/cache_key.rs`. The REST `{hash}` path segment is
// `blake3(canon_text)` truncated to its first 16 bytes as lowercase hex (32
// chars); the on-the-wire vector body is little-endian f32. Canonicalization is
// currently identity (the daemon's is too, "until rule-driven canon is wired
// in"), so `canon_text` is the raw input string.

import { blake3 } from '@noble/hashes/blake3';

const encoder = new TextEncoder();

function toHex(bytes) {
  let hex = '';
  for (const byte of bytes) hex += byte.toString(16).padStart(2, '0');
  return hex;
}

/** The 32-hex `{hash}` path segment for `text` (blake3 → first 16 bytes). */
export function textHash(text) {
  return toHex(blake3(encoder.encode(text)).subarray(0, 16));
}

/** Decode a little-endian f32 wire body (matches `vector_from_le_bytes`). */
export function vectorFromLeBytes(buffer) {
  if (buffer.byteLength === 0 || buffer.byteLength % 4 !== 0) return null;
  return Array.from(new Float32Array(buffer));
}
