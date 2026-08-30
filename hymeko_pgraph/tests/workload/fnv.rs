//! Test-only stable input fingerprint: FNV-1a-64.
//!
//! `DefaultHasher`'s algorithm is not a stable format across Rust versions, so a
//! benchmark line's fingerprint could change under you. FNV-1a-64 is a fixed,
//! documented algorithm used **only to identify a source byte sequence** — it is
//! not a cryptographic hash and must not be treated as one.

const FNV_OFFSET: u64 = 0xCBF2_9CE4_8422_2325;
const FNV_PRIME: u64 = 0x0000_0100_0000_01B3;

/// FNV-1a-64 over `bytes`.
#[must_use]
pub fn fnv1a_64(bytes: &[u8]) -> u64 {
    let mut h = FNV_OFFSET;
    for &b in bytes {
        h ^= u64::from(b);
        h = h.wrapping_mul(FNV_PRIME);
    }
    h
}

#[cfg(test)]
mod tests {
    use super::fnv1a_64;

    /// Canonical FNV-1a-64 test vectors (Fowler–Noll–Vo reference).
    #[test]
    fn fnv1a_64_known_answers() {
        assert_eq!(fnv1a_64(b""), 0xCBF2_9CE4_8422_2325);
        assert_eq!(fnv1a_64(b"a"), 0xAF63_DC4C_8601_EC8C);
        assert_eq!(fnv1a_64(b"foobar"), 0x85944171F73967E8);
    }

    #[test]
    fn distinct_inputs_distinct_fingerprints() {
        assert_ne!(fnv1a_64(b"steiner"), fnv1a_64(b"sunflower"));
    }
}
