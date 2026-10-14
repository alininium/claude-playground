//! Personal random numbers.
//!
//! Every person owns a seed and a counter. A draw hashes the two and bumps
//! the counter, so a person's numbers depend on nothing but their own
//! history: one person's choices never shift another's, and people can be
//! processed in any order, or in parallel, with identical results.
//!
//! A child's seed comes from its parents' seeds and its mother's birth
//! count alone, so the same parents always have the same children (see
//! [`child_seed`]).

use bevy_ecs::prelude::Component;

/// A person's own random stream: `seed` fixed for life, `counter` the number
/// of draws so far.
#[derive(
    Component, Debug, Clone, Copy, PartialEq, Eq, Hash, serde::Serialize, serde::Deserialize,
)]
pub struct PersonRng {
    pub seed: u64,
    pub counter: u64,
}

impl PersonRng {
    pub fn new(seed: u64) -> Self {
        Self { seed, counter: 0 }
    }

    pub fn next_u64(&mut self) -> u64 {
        let v = mix(self.seed, self.counter);
        self.counter += 1;
        v
    }

    /// Uniform integer in `0..bound` (multiply-shift: unbiased enough for a
    /// simulation, and a single draw, so the counter stays predictable).
    pub fn below(&mut self, bound: u64) -> u64 {
        assert!(bound > 0, "PersonRng::below called with bound 0");
        ((u128::from(self.next_u64()) * u128::from(bound)) >> 64) as u64
    }

    /// Uniform integer in `lo..=hi`.
    pub fn range(&mut self, lo: i64, hi: i64) -> i64 {
        debug_assert!(lo <= hi);
        lo + self.below((hi - lo + 1) as u64) as i64
    }

    /// True with probability `per_mille / 1000`.
    pub fn chance_per_mille(&mut self, per_mille: u32) -> bool {
        self.below(1000) < u64::from(per_mille)
    }
}

/// SplitMix64-style finalizer of a (seed, counter) pair: a counter-based
/// generator, so any draw can be computed without the ones before it.
pub fn mix(seed: u64, counter: u64) -> u64 {
    let mut z = seed ^ counter.wrapping_mul(0x9E37_79B9_7F4A_7C15);
    z = z.wrapping_add(0x9E37_79B9_7F4A_7C15);
    z = (z ^ (z >> 30)).wrapping_mul(0xBF58_476D_1CE4_E5B9);
    z = (z ^ (z >> 27)).wrapping_mul(0x94D0_49BB_1331_11EB);
    z ^ (z >> 31)
}

/// The seed of the `birth`-th child (0-based) of a mother and father. Only
/// the parents and the birth order matter, never when or where the child is
/// born. Not symmetric: the mother's seed and the father's play different
/// parts.
pub fn child_seed(mother: u64, father: u64, birth: u32) -> u64 {
    mix(mix(mother, 0xC41D_0000 ^ u64::from(birth)), father)
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn draws_depend_only_on_seed_and_counter() {
        let mut a = PersonRng::new(42);
        let first: Vec<u64> = (0..5).map(|_| a.next_u64()).collect();
        let mut b = PersonRng::new(42);
        assert_eq!(first, (0..5).map(|_| b.next_u64()).collect::<Vec<_>>());
        // Jumping straight to draw 3 gives the same number.
        assert_eq!(mix(42, 3), first[3]);
        assert_ne!(PersonRng::new(43).next_u64(), first[0]);
    }

    #[test]
    fn below_is_roughly_uniform() {
        let mut r = PersonRng::new(7);
        let mut counts = [0u32; 10];
        for _ in 0..100_000 {
            counts[r.below(10) as usize] += 1;
        }
        for c in counts {
            assert!((9_500..=10_500).contains(&c), "{counts:?}");
        }
        let hits = (0..100_000).filter(|_| r.chance_per_mille(250)).count();
        assert!((24_000..=26_000).contains(&hits), "{hits}");
    }

    #[test]
    fn children_depend_on_parents_and_birth_order() {
        assert_eq!(child_seed(1, 2, 0), child_seed(1, 2, 0));
        assert_ne!(child_seed(1, 2, 0), child_seed(1, 2, 1));
        assert_ne!(child_seed(1, 2, 0), child_seed(2, 1, 0));
        assert_ne!(child_seed(1, 2, 0), child_seed(1, 3, 0));
    }
}
