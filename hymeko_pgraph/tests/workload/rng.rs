//! Test-only deterministic PRNG: SplitMix64.
//!
//! No dependency, stable algorithm, documented constants — so a refactor or a
//! platform change cannot silently alter a generated workload's source. A frozen
//! known-answer vector (below) pins the sequence.

/// SplitMix64 (Steele, Lea & Flood 2014). Not cryptographic; used only to make
/// the random workload families deterministic in the seed.
#[derive(Clone, Debug)]
pub struct SplitMix64(u64);

impl SplitMix64 {
    /// Seed the generator. Any seed, including zero, is valid.
    #[must_use]
    pub fn new(seed: u64) -> Self {
        Self(seed)
    }

    /// Next raw 64-bit value.
    pub fn next_u64(&mut self) -> u64 {
        self.0 = self.0.wrapping_add(0x9E37_79B9_7F4A_7C15);
        let mut z = self.0;
        z = (z ^ (z >> 30)).wrapping_mul(0xBF58_476D_1CE4_E5B9);
        z = (z ^ (z >> 27)).wrapping_mul(0x94D0_49BB_1331_11EB);
        z ^ (z >> 31)
    }

    /// Uniform index in `0..bound`. `bound` of 0 is treated as 1 (never panics).
    pub fn below(&mut self, bound: usize) -> usize {
        let b = bound.max(1) as u64;
        (self.next_u64() % b) as usize
    }

    /// True with probability `num/den` (den > 0), deterministically.
    pub fn chance(&mut self, num: u64, den: u64) -> bool {
        let d = den.max(1);
        self.next_u64() % d < num
    }
}

#[cfg(test)]
mod tests {
    use super::SplitMix64;

    /// Frozen known-answer vector for seed 0 — generated once from this impl and
    /// pinned so any accidental change to the algorithm is caught.
    #[test]
    fn splitmix64_seed_0_is_frozen() {
        let mut r = SplitMix64::new(0);
        let got: Vec<u64> = (0..4).map(|_| r.next_u64()).collect();
        assert_eq!(
            got,
            [
                0xE220_A839_7B1D_CDAF,
                0x6E78_9E6A_A1B9_65F4,
                0x06C4_5D18_8009_454F,
                0xF88B_B8A8_724C_81EC,
            ]
        );
    }

    #[test]
    fn different_seeds_diverge() {
        let a = SplitMix64::new(1).next_u64();
        let b = SplitMix64::new(2).next_u64();
        assert_ne!(a, b);
    }

    #[test]
    fn below_is_in_range_and_bound_zero_is_safe() {
        let mut r = SplitMix64::new(42);
        for _ in 0..100 {
            assert!(r.below(7) < 7);
        }
        assert_eq!(r.below(0), 0);
    }
}
