// SPDX-License-Identifier: GPL-3.0-or-later
// Copyright (C) 2026 AnnoyingTechnology

//! Small, self-contained SplitMix64 generator used by the Smart
//! Shuffle picker's temperature sampler.
//!
//! The picker is deterministic *by construction* (§14 of the design
//! brief): given identical inputs — seed track, candidate set,
//! schema version, exploration mode, and recent-history state — it
//! must produce the same pick, so the `SUSTAIN_LOG_SMART_SHUFFLE=1`
//! debug trace stays reproducible after the fact. That determinism is
//! what this generator provides; it is seeded from those inputs by
//! the caller, never from the wall clock.
//!
//! This is intentionally a private copy rather than a dependency on
//! the Fisher-Yates generator in `sustain_domain::playback::shuffle`:
//! that one is private to the Pure-shuffle layout and exposes only
//! `next_index`, while the sampler here needs a unit-interval draw.
//! Both are the same well-known 64-bit mixing function.

/// Deterministic 64-bit PRNG (Steele, Lea & Flood, 2014). Cheap, no
/// allocation, good enough for sampling a single track per playback
/// transition.
pub(crate) struct SplitMix64 {
    state: u64,
}

impl SplitMix64 {
    pub(crate) const fn new(seed: u64) -> Self {
        Self { state: seed }
    }

    fn next_u64(&mut self) -> u64 {
        self.state = self.state.wrapping_add(0x9E37_79B9_7F4A_7C15);
        let mut value = self.state;
        value = (value ^ (value >> 30)).wrapping_mul(0xBF58_476D_1CE4_E5B9);
        value = (value ^ (value >> 27)).wrapping_mul(0x94D0_49BB_1331_11EB);
        value ^ (value >> 31)
    }

    /// A draw in `[0.0, 1.0)`. Uses the top 24 bits so the result is
    /// exactly representable as `f32` without rounding bias.
    pub(crate) fn next_unit_interval(&mut self) -> f32 {
        let bits = self.next_u64() >> 40; // keep 24 bits
        bits as f32 / (1u32 << 24) as f32
    }

    /// A uniform index in `[0, upper_bound)`. Rejection-sampled so the
    /// distribution is exactly uniform (no modulo bias). Panics is
    /// impossible for `upper_bound == 0` because callers only reach
    /// here with a non-empty pool, but we guard defensively.
    pub(crate) fn next_bounded(&mut self, upper_bound: usize) -> usize {
        if upper_bound <= 1 {
            return 0;
        }
        let upper = upper_bound as u64;
        let rejection_threshold = u64::MAX - (u64::MAX % upper);
        loop {
            let value = self.next_u64();
            if value < rejection_threshold {
                return (value % upper) as usize;
            }
        }
    }
}

#[cfg(test)]
mod tests {
    use super::SplitMix64;

    #[test]
    fn unit_interval_stays_in_range() {
        let mut rng = SplitMix64::new(42);
        for _ in 0..10_000 {
            let value = rng.next_unit_interval();
            assert!((0.0..1.0).contains(&value), "out of range: {value}");
        }
    }

    #[test]
    fn bounded_stays_in_range_and_is_deterministic() {
        let mut a = SplitMix64::new(7);
        let mut b = SplitMix64::new(7);
        for _ in 0..1_000 {
            let index = a.next_bounded(13);
            assert!(index < 13);
            assert_eq!(index, b.next_bounded(13));
        }
    }

    #[test]
    fn bounded_handles_degenerate_bounds() {
        let mut rng = SplitMix64::new(1);
        assert_eq!(rng.next_bounded(0), 0);
        assert_eq!(rng.next_bounded(1), 0);
    }
}
