//! Stage 1: continents and altitudes.
//!
//! A low-frequency, domain-warped fractal field decides where the continents
//! are; the sea level is then chosen so that exactly the configured share of
//! the map is land. Land gets rolling lowlands plus ridged mountain ranges in
//! "orogenic" belts; the sea floor gets a shallow continental shelf that
//! drops off into the abyss.

use super::TerrainConfig;
use super::noise::{Noise, ONE, smoothstep};
use crate::rng::SimRng;

pub fn generate(width: u32, height: u32, config: &TerrainConfig, rng: &mut SimRng) -> Vec<i32> {
    let continents = Noise::new(rng.next_u64());
    let warp_x = Noise::new(rng.next_u64());
    let warp_y = Noise::new(rng.next_u64());
    let detail = Noise::new(rng.next_u64());
    let ridges = Noise::new(rng.next_u64());
    let orogeny = Noise::new(rng.next_u64());
    let roughness = Noise::new(rng.next_u64());

    let (w, h) = (i64::from(width), i64::from(height));
    let freq = i64::from(config.continent_frequency.max(1));
    // Tile center -> noise lattice coordinates (same scale on both axes).
    let coord = |v: i64| (2 * v + 1) * ONE * freq / (2 * w);
    // Tile center -> lattice coordinates with a fixed 6-tile cell, for relief
    // that looks the same at any map size (and makes rivers meander).
    let tile_coord = |v: i64| (2 * v + 1) * ONE / 12;

    // Continental base field.
    let mut base = Vec::with_capacity((w * h) as usize);
    for y in 0..h {
        for x in 0..w {
            let (fx, fy) = (coord(x), coord(y));
            let wx = warp_x.fbm(fx * 2, fy * 2, 4) * 3 / 5;
            let wy = warp_y.fbm(fx * 2, fy * 2, 4) * 3 / 5;
            let mut v = continents.fbm(fx + wx, fy + wy, 6);
            v -= edge_falloff(x, w).max(edge_falloff(y, h));
            base.push(v);
        }
    }

    let mut sorted = base.clone();
    sorted.sort_unstable();
    let land = i64::from(config.land_per_mille.min(1000));
    let water_tiles = ((sorted.len() as i64 * (1000 - land)) / 1000) as usize;
    let sea_level = sorted[water_tiles.min(sorted.len() - 1)];
    let (lowest, highest) = (sorted[0], sorted[sorted.len() - 1]);

    let max_alt = i64::from(config.max_altitude);
    let max_depth = i64::from(config.max_depth);
    let mut altitudes = Vec::with_capacity(base.len());
    for (i, &b) in base.iter().enumerate() {
        let (x, y) = (i as i64 % w, i as i64 / w);
        let (fx, fy) = (coord(x), coord(y));
        let alt = if b >= sea_level && land > 0 {
            // 0 at the coast, ONE at the most inland point.
            let r = (b - sea_level) * ONE / (highest - sea_level).max(1);
            let inland = (r * 6).min(ONE);
            // Land rises steadily inland, so most rain finds a way to the sea.
            let lowland = 2 + max_alt / 12 * r / ONE + max_alt / 8 * r / ONE * r / ONE;
            let bumps = detail.fbm(fx * 8, fy * 8, 4) * (max_alt / 40) / ONE * inland / ONE;
            let rough = roughness.fbm(tile_coord(x), tile_coord(y), 3) * (max_alt / 150) / ONE
                * inland
                / ONE;
            let belt = ((orogeny.fbm(fx * 2, fy * 2, 3) + ONE / 5) * 3).clamp(0, ONE);
            let ridge = ridges.ridged(fx * 5, fy * 5, 5);
            let ridge3 = ridge * ridge / ONE * ridge / ONE;
            let peaks = max_alt * ridge3 / ONE * smoothstep(belt) / ONE * inland / ONE;
            (lowland + bumps + rough + peaks).max(1)
        } else {
            // 0 at the coast, ONE at the deepest point.
            let t = (sea_level - b) * ONE / (sea_level - lowest).max(1);
            let shelf = 20 + 150 * (t * 6).min(ONE) / ONE;
            -(shelf + (max_depth - 170) * smoothstep((t * 3).min(ONE)) / ONE)
        };
        altitudes.push(alt as i32);
    }
    altitudes
}

/// `ONE` at the map border, fading to 0 an eighth of the way in, so continents
/// don't get cut off by the edge of the world.
fn edge_falloff(v: i64, size: i64) -> i64 {
    let margin = (size / 8).max(1);
    let d = v.min(size - 1 - v);
    if d >= margin {
        return 0;
    }
    let t = ONE - d * ONE / margin;
    t * t / ONE
}
