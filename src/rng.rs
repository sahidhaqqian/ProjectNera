//! Tiny deterministic PRNG for Nera's non-cryptographic randomness.
//!
//! Phase 5.75 needs directional jitter for the drifting alarm dismiss box and
//! Phase 5.8 needs randomized idle micro-action scheduling. Neither justifies
//! a randomness dependency, and neither needs cryptographic quality, so this
//! module provides one small generator that can be audited at a glance and
//! seeded explicitly in tests.
//!
//! Deliberately *not* used for anything security-relevant. If Nera ever needs
//! a token, a nonce, or a secret, this is the wrong tool.

use std::time::{
    SystemTime,
    UNIX_EPOCH,
};

/// Golden-ratio fractional bits. Used both as the fallback entropy source and
/// to repair a zero seed, because zero is xorshift's only fixed point.
const GOLDEN_RATIO: u64 = 0x9E37_79B9_7F4A_7C15;

/// xorshift64* — period 2^64 - 1.
#[derive(Clone, Copy, Debug)]
pub(crate) struct NeraRng {
    state: u64,
}

impl NeraRng {
    /// Seed from the wall clock.
    ///
    /// Coarse, and two processes launched in the same instant would generate
    /// identical sequences. That is acceptable here: the outputs only decide
    /// which idle animation plays and which way a dismiss box drifts.
    pub(crate) fn from_entropy() -> Self {
        let mixed = SystemTime::now()
            .duration_since(UNIX_EPOCH)
            .map(|elapsed| {
                elapsed
                    .as_secs()
                    .wrapping_mul(1_000_000_007)
                    ^ elapsed.subsec_nanos() as u64
            })
            .unwrap_or(GOLDEN_RATIO);

        Self::seeded(mixed)
    }

    /// Explicit seed. A zero seed is silently replaced, since xorshift cannot
    /// escape zero on its own.
    pub(crate) const fn seeded(seed: u64) -> Self {
        Self {
            state: if seed == 0 { GOLDEN_RATIO } else { seed },
        }
    }

    pub(crate) fn next_u64(&mut self) -> u64 {
        let mut x = self.state;
        x ^= x >> 12;
        x ^= x << 25;
        x ^= x >> 27;
        self.state = x;

        x.wrapping_mul(0x2545_F491_4F6C_DD1D)
    }

    /// Uniform in `[0.0, 1.0)`. Uses the high 24 bits, which are the best
    /// mixed part of an xorshift output.
    pub(crate) fn next_f32(&mut self) -> f32 {
        const SCALE: f32 = 1.0 / (1_u64 << 24) as f32;
        (self.next_u64() >> 40) as f32 * SCALE
    }

    /// Uniform in `[min, max)`.
    ///
    /// Non-finite or inverted bounds collapse to a finite value instead of
    /// propagating NaN: a NaN position would poison the dismiss box HWND
    /// permanently, and there is no recovery path once it is written.
    pub(crate) fn range_f32(&mut self, min: f32, max: f32) -> f32 {
        if !min.is_finite() {
            return 0.0;
        }

        // Also covers `max <= min` and NaN `max`, since every comparison
        // against NaN is false.
        if !max.is_finite() || !(max > min) {
            return min;
        }

        min + self.next_f32() * (max - min)
    }

    /// Uniform in `[min, max]`, inclusive on both ends.
    pub(crate) fn range_i32(&mut self, min: i32, max: i32) -> i32 {
        if max <= min {
            return min;
        }

        let span = (max - min) as u32 + 1;
        // Rejection-free: a small modulo bias is irrelevant for animation
        // scheduling, and avoiding a rejection loop keeps this branchless.
        min + ((self.next_u64() >> 32) as u32 % span) as i32
    }

    /// Uniform index into a slice of length `len`. `None` when there is nothing
    /// to pick from, so callers must handle the empty case explicitly.
    pub(crate) fn pick_index(&mut self, len: usize) -> Option<usize> {
        if len == 0 {
            return None;
        }

        Some((self.next_u64() % len as u64) as usize)
    }
}

impl Default for NeraRng {
    fn default() -> Self {
        Self::from_entropy()
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn same_seed_produces_the_same_sequence() {
        let mut a = NeraRng::seeded(1234);
        let mut b = NeraRng::seeded(1234);

        for _ in 0..64 {
            assert_eq!(a.next_u64(), b.next_u64());
        }
    }

    #[test]
    fn different_seeds_diverge() {
        let mut a = NeraRng::seeded(1);
        let mut b = NeraRng::seeded(2);

        assert_ne!(a.next_u64(), b.next_u64());
    }

    /// Zero is xorshift's fixed point; a zero seed must be repaired rather than
    /// producing an endless stream of zeros.
    #[test]
    fn zero_seed_is_repaired_and_still_advances() {
        let mut rng = NeraRng::seeded(0);

        assert_ne!(rng.state, 0);

        let first = rng.next_u64();
        let second = rng.next_u64();

        assert_ne!(first, 0);
        assert_ne!(first, second);
    }

    #[test]
    fn sequence_never_collapses_to_a_fixed_point() {
        let mut rng = NeraRng::seeded(GOLDEN_RATIO);
        let mut seen_zero_state = false;

        for _ in 0..10_000 {
            rng.next_u64();
            if rng.state == 0 {
                seen_zero_state = true;
            }
        }

        assert!(!seen_zero_state);
    }

    #[test]
    fn next_f32_stays_inside_unit_interval() {
        let mut rng = NeraRng::seeded(7);

        for _ in 0..10_000 {
            let value = rng.next_f32();
            assert!((0.0..1.0).contains(&value), "got {value}");
        }
    }

    #[test]
    fn range_f32_respects_bounds() {
        let mut rng = NeraRng::seeded(11);

        for _ in 0..10_000 {
            let value = rng.range_f32(-3.5, 2.25);
            assert!(value >= -3.5 && value < 2.25, "got {value}");
        }
    }

    /// An inverted, empty, or non-finite span must not yield NaN or infinity,
    /// which would poison the dismiss box position permanently.
    #[test]
    fn range_f32_degrades_safely_on_degenerate_spans() {
        let mut rng = NeraRng::seeded(13);

        assert_eq!(rng.range_f32(5.0, 5.0), 5.0);
        assert_eq!(rng.range_f32(5.0, 1.0), 5.0);

        for (min, max) in [
            (f32::NAN, 1.0),
            (0.0, f32::NAN),
            (f32::NAN, f32::NAN),
            (0.0, f32::INFINITY),
            (f32::NEG_INFINITY, 1.0),
        ] {
            let value = rng.range_f32(min, max);
            assert!(value.is_finite(), "range_f32({min}, {max}) = {value}");
        }
    }

    #[test]
    fn range_i32_is_inclusive_on_both_ends() {
        let mut rng = NeraRng::seeded(17);

        let mut saw_min = false;
        let mut saw_max = false;

        for _ in 0..20_000 {
            let value = rng.range_i32(-2, 2);
            assert!((-2..=2).contains(&value), "got {value}");

            saw_min |= value == -2;
            saw_max |= value == 2;
        }

        assert!(saw_min && saw_max, "range never reached its endpoints");
    }

    #[test]
    fn pick_index_covers_every_slot() {
        let mut rng = NeraRng::seeded(19);
        let len = 5;
        let mut hits = [0_u32; 5];

        for _ in 0..20_000 {
            let index = rng.pick_index(len).expect("len is non-zero");
            assert!(index < len);
            hits[index] += 1;
        }

        // Every micro-action clip must be reachable, otherwise one animation
        // would silently never play.
        assert!(hits.iter().all(|count| *count > 0), "hits={hits:?}");
    }

    #[test]
    fn pick_index_on_empty_returns_none() {
        let mut rng = NeraRng::seeded(23);
        assert_eq!(rng.pick_index(0), None);
    }
}
