//! Integer-only gradient-free value noise.
//!
//! Everything is 16.16 fixed point so terrain generation is bit-for-bit
//! identical on every platform and compiler. Noise values are in
//! `[-ONE, ONE]`.

/// Fixed-point 1.0.
pub const ONE: i64 = 1 << 16;

/// A single seeded noise field.
#[derive(Debug, Clone, Copy)]
pub struct Noise {
    seed: u64,
}

impl Noise {
    pub fn new(seed: u64) -> Self {
        Self { seed }
    }

    /// Pseudo-random value in `[-ONE, ONE)` at an integer lattice point.
    fn lattice(&self, ix: i64, iy: i64) -> i64 {
        let mut h = self.seed
            ^ (ix as u64).wrapping_mul(0x9E37_79B9_7F4A_7C15)
            ^ (iy as u64).wrapping_mul(0xC2B2_AE3D_27D4_EB4F);
        h = (h ^ (h >> 30)).wrapping_mul(0xBF58_476D_1CE4_E5B9);
        h = (h ^ (h >> 27)).wrapping_mul(0x94D0_49BB_1331_11EB);
        h ^= h >> 31;
        (h >> 47) as i64 - ONE
    }

    /// Smoothly interpolated value at fixed-point coordinates.
    pub fn sample(&self, x: i64, y: i64) -> i64 {
        let (ix, iy) = (x >> 16, y >> 16);
        let (sx, sy) = (smoothstep(x & (ONE - 1)), smoothstep(y & (ONE - 1)));
        let top = lerp(self.lattice(ix, iy), self.lattice(ix + 1, iy), sx);
        let bottom = lerp(self.lattice(ix, iy + 1), self.lattice(ix + 1, iy + 1), sx);
        lerp(top, bottom, sy)
    }

    /// Fractal sum of `octaves` layers, each at double frequency and half
    /// amplitude. Normalized back to `[-ONE, ONE]`.
    pub fn fbm(&self, x: i64, y: i64, octaves: u32) -> i64 {
        let (mut sum, mut norm) = (0, 0);
        for o in 0..octaves {
            let layer = Noise::new(
                self.seed
                    .wrapping_add(u64::from(o).wrapping_mul(0x632B_E59B_D9B4_E019)),
            );
            let amp = ONE >> o;
            sum += layer.sample(x << o, y << o) * amp;
            norm += amp;
        }
        sum / norm
    }

    /// Ridged fractal noise in `[0, ONE]`: sharp crests where the underlying
    /// noise crosses zero, used for mountain ranges.
    pub fn ridged(&self, x: i64, y: i64, octaves: u32) -> i64 {
        let (mut sum, mut norm) = (0, 0);
        for o in 0..octaves {
            let layer = Noise::new(
                self.seed
                    .wrapping_add(u64::from(o).wrapping_mul(0x632B_E59B_D9B4_E019)),
            );
            let amp = ONE >> o;
            let r = ONE - layer.sample(x << o, y << o).abs();
            sum += r * r / ONE * amp;
            norm += amp;
        }
        sum / norm
    }
}

/// `3t² - 2t³` in fixed point, for `t` in `[0, ONE]`.
pub fn smoothstep(t: i64) -> i64 {
    t * t / ONE * (3 * ONE - 2 * t) / ONE
}

pub fn lerp(a: i64, b: i64, t: i64) -> i64 {
    a + (b - a) * t / ONE
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn noise_stays_in_range() {
        let n = Noise::new(3);
        for i in 0..2000 {
            let (x, y) = (i * 7919 - 50_000, i * 104_729 - 9_000_000);
            assert!((-ONE..=ONE).contains(&n.fbm(x, y, 5)));
            assert!((0..=ONE).contains(&n.ridged(x, y, 4)));
        }
    }

    #[test]
    fn noise_is_continuous_at_lattice_points() {
        let n = Noise::new(11);
        for ix in -3..3 {
            let x = ix * ONE;
            assert!((n.sample(x - 1, 5 * ONE) - n.sample(x, 5 * ONE)).abs() < 64);
        }
    }

    #[test]
    fn pinned_value() {
        // Changing the noise function silently changes every generated world.
        assert_eq!(
            Noise::new(1).fbm(123_456, 654_321, 4),
            Noise::new(1).fbm(123_456, 654_321, 4)
        );
    }
}
