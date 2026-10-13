//! Deterministic pseudo-random number generation.
//!
//! The simulation owns exactly one RNG stream, seeded from the initial
//! conditions. We implement xoshiro256** ourselves (seeded via SplitMix64)
//! instead of depending on an external crate so the sequence of numbers can
//! never change underneath us due to a dependency upgrade. Same seed, same
//! world — forever.

use bevy_ecs::prelude::Resource;

#[derive(Resource, Debug, Clone, PartialEq, Eq, serde::Serialize, serde::Deserialize)]
pub struct SimRng {
    state: [u64; 4],
}

impl SimRng {
    pub fn from_seed(seed: u64) -> Self {
        let mut sm = seed;
        let mut state = [0u64; 4];
        for s in &mut state {
            *s = splitmix64(&mut sm);
        }
        Self { state }
    }

    pub fn next_u64(&mut self) -> u64 {
        let [s0, s1, s2, s3] = &mut self.state;
        let result = s1.wrapping_mul(5).rotate_left(7).wrapping_mul(9);
        let t = *s1 << 17;
        *s2 ^= *s0;
        *s3 ^= *s1;
        *s1 ^= *s2;
        *s0 ^= *s3;
        *s2 ^= t;
        *s3 = s3.rotate_left(45);
        result
    }

    /// Uniform integer in `0..bound`. Unbiased (rejection sampling).
    pub fn below(&mut self, bound: u64) -> u64 {
        assert!(bound > 0, "SimRng::below called with bound 0");
        let zone = u64::MAX - (u64::MAX % bound);
        loop {
            let v = self.next_u64();
            if v < zone {
                return v % bound;
            }
        }
    }

    /// Returns true with probability `per_mille / 1000`.
    pub fn chance_per_mille(&mut self, per_mille: u32) -> bool {
        self.below(1000) < u64::from(per_mille)
    }
}

fn splitmix64(state: &mut u64) -> u64 {
    *state = state.wrapping_add(0x9E37_79B9_7F4A_7C15);
    let mut z = *state;
    z = (z ^ (z >> 30)).wrapping_mul(0xBF58_476D_1CE4_E5B9);
    z = (z ^ (z >> 27)).wrapping_mul(0x94D0_49BB_1331_11EB);
    z ^ (z >> 31)
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn same_seed_same_sequence() {
        let mut a = SimRng::from_seed(42);
        let mut b = SimRng::from_seed(42);
        for _ in 0..1000 {
            assert_eq!(a.next_u64(), b.next_u64());
        }
    }

    #[test]
    fn sequence_is_stable() {
        // Pinned reference values: if these change, every saved seed changes meaning.
        let mut rng = SimRng::from_seed(0);
        assert_eq!(rng.next_u64(), 0x99ec_5f36_cb75_f2b4);
        assert_eq!(rng.next_u64(), 0xbf6e_1f78_4956_452a);
        assert_eq!(rng.next_u64(), 0x1a5f_849d_4933_e6e0);
    }

    #[test]
    fn below_stays_in_range() {
        let mut rng = SimRng::from_seed(7);
        for bound in 1..100 {
            for _ in 0..100 {
                assert!(rng.below(bound) < bound);
            }
        }
    }
}
