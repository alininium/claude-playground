//! What is frozen: sea ice, lake and river ice, snow cover, frozen ground,
//! permafrost and glaciers (the polar caps).
//!
//! Every tile keeps a small physical state, advanced from its temperature:
//!
//! - **Water ice** grows below the freezing point (0 °C fresh water,
//!   −1.8 °C sea water) following Stefan's law: thin ice thickens quickly,
//!   thick ice slowly, because the ice insulates the water beneath it. It
//!   melts with positive degree-days. Sea ice grows slower than lake ice
//!   (ocean heat, salt), river ice slower too (flowing water).
//! - **Snow** accumulates from the tile's precipitation when it is at or
//!   below about +1 °C and melts with positive degree-days.
//! - **Glaciers** form when snow outlasts the summers and piles up beyond a
//!   firn threshold. Ice spreads from thick to thin areas, calves into the
//!   sea and lakes, and melts (slower than snow) once its snow cover is gone.
//! - **Frozen ground**: frost penetrates the soil in winter (Stefan's law
//!   again, much slower under a snow blanket). In summer, once the snow is
//!   gone, an *active layer* thaws from the top down. If it reaches the
//!   bottom of the frost the ground is thawed; if summer ends first, the
//!   rest stays frozen. Ground that never fully thawed during a year is
//!   **permafrost**.
//!
//! Ice and snow reflect sunlight, so they cool their own tile (albedo
//! feedback), which is what lets polar caps sustain themselves.
//!
//! Lengths are in tenths of a millimeter (water equivalent for snow and
//! glaciers); temperatures in tenths of a degree; time steps in quarter days.

use crate::map::{Map, Water};

/// Snow beyond 1 m water equivalent compacts into glacier ice.
const FIRN_THRESHOLD: u32 = 10_000;
const MAX_WATER_ICE: u32 = 50_000;
const MAX_FROST: u32 = 30_000;
const SEA_FREEZING_POINT: i64 = -18;
/// Glacier ice moved to one neighbor per day, at most.
const MAX_FLOW: i64 = 100;
/// Ice this thick (10 cm) can be walked on and stops boats.
pub const SOLID_ICE: u32 = 1_000;
/// Snow this deep (5 cm water equivalent) covers the ground.
pub const SNOW_COVER: u32 = 500;
/// Glacier ice thicker than this (1 m) counts as a glacier.
pub const GLACIER: u32 = 10_000;
/// Albedo cooling of a glacier, tenths of a degree.
pub const GLACIER_COOLING: i64 = 40;
/// Years per step when settling the ice sheets before turn 0. Two years
/// halve the cost of one-year steps and move the settled ice volume by
/// about 0.1%, as much as the settling tolerance itself.
const SETTLE_STEP_YEARS: u32 = 2;

#[derive(
    Debug, Clone, Copy, Default, PartialEq, Eq, Hash, serde::Serialize, serde::Deserialize,
)]
pub struct Frozen {
    /// Snow on the ground, water equivalent.
    pub snow: u32,
    /// Ice on the water surface (oceans, lakes, rivers).
    pub ice: u32,
    /// Glacier ice, water equivalent.
    pub glacier: u32,
    /// Depth to which the ground is frozen.
    pub frost: u32,
    /// Depth of the thawed top layer above still-frozen ground (summer).
    pub thaw: u32,
    /// The ground stayed frozen through the whole previous year.
    pub permafrost: bool,
    thawed_this_year: bool,
}

/// The single most visible frozen thing on a tile.
#[derive(
    Debug, Clone, Copy, PartialEq, Eq, Hash, PartialOrd, Ord, serde::Serialize, serde::Deserialize,
)]
pub enum Cover {
    None,
    FrozenGround,
    Snow,
    RiverIce,
    LakeIce,
    SeaIce,
    Glacier,
}

#[derive(Debug, Clone, serde::Serialize, serde::Deserialize)]
pub struct Cryosphere {
    tiles: Vec<Frozen>,
    /// Meltwater (snow and glacier) released in the last step, tenths of a
    /// millimeter.
    melt: Vec<u32>,
}

impl Cryosphere {
    pub fn new(tile_count: usize) -> Self {
        Self {
            tiles: vec![Frozen::default(); tile_count],
            melt: vec![0; tile_count],
        }
    }

    pub fn tiles(&self) -> &[Frozen] {
        &self.tiles
    }

    pub fn tile(&self, index: usize) -> &Frozen {
        &self.tiles[index]
    }

    /// Meltwater released by snow and glaciers in the last step, tenths of a
    /// millimeter.
    pub fn melt(&self) -> &[u32] {
        &self.melt
    }

    pub fn cover(&self, map: &Map, index: usize) -> Cover {
        let f = &self.tiles[index];
        let water = map.tiles()[index].water;
        if f.glacier >= GLACIER {
            Cover::Glacier
        } else if f.ice >= SOLID_ICE {
            match water {
                Water::Ocean => Cover::SeaIce,
                Water::Lake => Cover::LakeIce,
                _ => Cover::RiverIce,
            }
        } else if f.snow >= SNOW_COVER {
            Cover::Snow
        } else if f.frost > f.thaw && f.thaw == 0 && matches!(water, Water::Dry | Water::River) {
            Cover::FrozenGround
        } else {
            Cover::None
        }
    }

    /// How much the tile's ice and snow cool it, in tenths of a degree.
    pub fn albedo_cooling(&self, index: usize) -> i64 {
        let f = &self.tiles[index];
        if f.glacier >= GLACIER {
            GLACIER_COOLING
        } else if f.ice >= 3 * SOLID_ICE {
            20
        } else if f.snow >= SNOW_COVER {
            15
        } else if f.ice >= SOLID_ICE / 2 {
            10
        } else {
            0
        }
    }

    /// Advances every tile by `quarters` quarter-days at the given
    /// temperatures (tenths °C) and precipitation rates (mm per year).
    pub fn step(
        &mut self,
        map: &Map,
        temperatures: &[i16],
        precipitation: &[u16],
        quarters: u32,
        days_per_year: u32,
    ) {
        let q = i64::from(quarters);
        let per_year = i64::from(days_per_year.max(1)) * 4;
        for ((((f, melt), tile), &temp), &rate) in self
            .tiles
            .iter_mut()
            .zip(self.melt.iter_mut())
            .zip(map.tiles())
            .zip(temperatures)
            .zip(precipitation)
        {
            *melt = 0;
            let t = i64::from(temp);
            match tile.water {
                Water::Ocean => water_ice(&mut f.ice, t, SEA_FREEZING_POINT, q, 700),
                Water::Lake => water_ice(&mut f.ice, t, 0, q, 1000),
                Water::River | Water::Dry => {
                    if tile.water == Water::River {
                        water_ice(&mut f.ice, t, 0, q, 800);
                    }
                    *melt = land(f, t, q, precipitation_step(rate, q, per_year));
                }
            }
        }
    }

    /// Daily glacier flow: ice creeps toward thinner neighbors (at most
    /// 10 mm of ice per day, a few meters a year, like a real glacier front)
    /// and calves into the sea and lakes. Computed from a snapshot, so the
    /// result does not depend on iteration order.
    pub fn flow_glaciers(&mut self, map: &Map) {
        let mut delta = vec![0i64; self.tiles.len()];
        for (i, f) in self.tiles.iter().enumerate() {
            let g = i64::from(f.glacier);
            if g == 0 {
                continue;
            }
            for nb in map.neighbors(i) {
                match map.tiles()[nb].water {
                    Water::Ocean | Water::Lake => delta[i] -= calving(g),
                    _ => {
                        let flux = creep(g, i64::from(self.tiles[nb].glacier));
                        delta[i] -= flux;
                        delta[nb] += flux;
                    }
                }
            }
        }
        for (f, d) in self.tiles.iter_mut().zip(delta) {
            f.glacier = (i64::from(f.glacier) + d).max(0) as u32;
        }
    }

    /// Runs the glacier model forward in steps of [`SETTLE_STEP_YEARS`] until
    /// the ice stops changing (or for `max_years`), with a fixed yearly
    /// surface mass balance per tile (tenths of a millimeter of ice; see
    /// `Climate::glacier_mass_balance`). Flow and calving are the daily ones
    /// times the days of a step, so the ice ends up where the daily model
    /// keeps it (the equilibrium does not depend on the step length). Returns
    /// the years it took.
    ///
    /// Ice sheets take thousands of years to settle, far too long to
    /// simulate day by day before turn 0; a two-year step is stable here
    /// because a tile sheds at most 8 · 728 / 20000 ≈ 29% of its excess over
    /// its neighbors per step.
    pub(crate) fn settle_glaciers(
        &mut self,
        map: &Map,
        balance: &[i64],
        days_per_year: u32,
        max_years: u32,
    ) -> u32 {
        let n = self.tiles.len();
        let days = i64::from(days_per_year);
        let land = |i: usize| matches!(map.tiles()[i].water, Water::Dry | Water::River);
        let mut g: Vec<i64> = self.tiles.iter().map(|f| i64::from(f.glacier)).collect();
        let mut delta = vec![0i64; n];
        // Each tile's land neighbors (a bit per offset) and number of water
        // neighbors, looked up once instead of every year.
        let w = map.width() as isize;
        let offsets = [-w - 1, -w, -w + 1, -1, 1, w - 1, w, w + 1];
        let mut land_mask = vec![0u8; n];
        let mut water_neighbors = vec![0i64; n];
        for i in 0..n {
            for nb in map.neighbors(i) {
                if land(nb) {
                    let offset = nb as isize - i as isize;
                    let bit = offsets.iter().position(|&o| o == offset).unwrap();
                    land_mask[i] |= 1 << bit;
                } else {
                    water_neighbors[i] += 1;
                }
            }
        }
        // Tiles that may change: those with ice or a positive balance, and
        // their neighbors. `stamp` dedupes them.
        let mut stamp = vec![u32::MAX; n];
        let mut active: Vec<usize> = (0..n)
            .filter(|&i| land(i) && (g[i] > 0 || balance[i] > 0))
            .collect();
        let mut touched = Vec::with_capacity(active.len() * 2);
        let mut years = 0;
        let mut last_volume = -1;
        let step = i64::from(SETTLE_STEP_YEARS);
        while years < max_years {
            years += SETTLE_STEP_YEARS;
            touched.clear();
            let mut touch = |i: usize, touched: &mut Vec<usize>| {
                if stamp[i] != years {
                    stamp[i] = years;
                    touched.push(i);
                }
            };
            for &i in &active {
                touch(i, &mut touched);
                if g[i] == 0 {
                    continue;
                }
                delta[i] -= calving(g[i]) * days * step * water_neighbors[i];
                let mut mask = land_mask[i];
                while mask != 0 {
                    let nb = (i as isize + offsets[mask.trailing_zeros() as usize]) as usize;
                    mask &= mask - 1;
                    let flux = creep(g[i], g[nb]) * days * step;
                    if flux > 0 {
                        delta[i] -= flux;
                        delta[nb] += flux;
                        touch(nb, &mut touched);
                    }
                }
            }
            for &i in &touched {
                g[i] = (g[i] + delta[i] + balance[i] * step).max(0);
                delta[i] = 0;
            }
            touched.retain(|&i| g[i] > 0 || balance[i] > 0);
            std::mem::swap(&mut active, &mut touched);
            // Settled once the total changes by under 0.1% (plus 10 cm per
            // tile) in a century. Single tiles at the edges never quite
            // stop: they flicker by a few centimeters, because the daily
            // creep is rounded to whole units.
            if years.is_multiple_of(100) {
                let volume: i64 = active.iter().map(|&i| g[i]).sum();
                let tolerance = volume / 1000 + active.len() as i64 * 1_000;
                if (volume - last_volume).abs() <= tolerance {
                    break;
                }
                last_volume = volume;
            }
        }
        for (f, &ice) in self.tiles.iter_mut().zip(&g) {
            f.glacier = ice.min(i64::from(u32::MAX)) as u32;
            if f.glacier >= GLACIER {
                f.frost = MAX_FROST;
                f.thaw = 0;
                f.permafrost = true;
            }
        }
        years
    }

    /// Called once a year: ground that never thawed is permafrost.
    pub fn end_year(&mut self) {
        for f in &mut self.tiles {
            f.permafrost = f.glacier >= GLACIER || (!f.thawed_this_year && f.frost > 0);
            f.thawed_this_year = f.frost == 0;
        }
    }
}

/// Precipitation of `quarters` quarter-days at `rate` mm per year, in
/// tenths of a millimeter.
fn precipitation_step(rate: u16, quarters: i64, quarters_per_year: i64) -> i64 {
    i64::from(rate) * 10 * quarters / quarters_per_year
}

/// Advances the snow and glacier of one land tile by one quarter-day, as
/// [`Cryosphere::step`] does; for probes run outside the real cryosphere.
pub(crate) fn step_land_quarter(f: &mut Frozen, temperature: i64, precipitation: i64) {
    land(f, temperature, 1, precipitation);
}

/// Precipitation of one quarter-day at `rate` mm per year, in tenths of a
/// millimeter: the `precipitation` [`step_land_quarter`] takes.
pub(crate) fn quarter_precipitation(rate: u16, days_per_year: u32) -> i64 {
    precipitation_step(rate, 1, i64::from(days_per_year.max(1)) * 4)
}

/// Daily creep of ice `g` thick toward a neighbor with `other` (tenths of a
/// millimeter): proportional to the difference, capped.
fn creep(g: i64, other: i64) -> i64 {
    if g > other {
        ((g - other) / 20_000).min(MAX_FLOW)
    } else {
        0
    }
}

/// Daily calving of ice `g` thick into a neighboring sea or lake.
fn calving(g: i64) -> i64 {
    (g / 20_000).min(MAX_FLOW)
}

/// Stefan's law growth below `freezing_point`, degree-day melt above it.
/// `rate` scales growth, per mille.
fn water_ice(ice: &mut u32, t: i64, freezing_point: i64, q: i64, rate: i64) {
    if t < freezing_point {
        // h² = α²·FDD with α ≈ 2.4 cm/√(°C·day): dh = α²·dFDD / 2h.
        let h = i64::from(*ice).max(100);
        let growth = 720 * (freezing_point - t) * q * rate / 1000 / h;
        *ice = (i64::from(*ice) + growth).min(i64::from(MAX_WATER_ICE)) as u32;
    } else if t > freezing_point {
        // 15 mm per °C·day.
        let melt = (t - freezing_point) * 15 * q / 4;
        *ice = (i64::from(*ice) - melt).max(0) as u32;
    }
}

/// Advances one land tile; returns the meltwater released.
fn land(f: &mut Frozen, t: i64, q: i64, precipitation: i64) -> u32 {
    let mass_before = i64::from(f.snow) + i64::from(f.glacier);
    let snowfall = if t < 10 { precipitation } else { 0 };
    f.snow = f.snow.saturating_add(snowfall as u32);
    if t > 0 {
        // Snow melts at 4 mm per °C·day, glacier ice at 7.
        let melt = t * q;
        let snow = i64::from(f.snow);
        if snow >= melt {
            f.snow = (snow - melt) as u32;
        } else {
            f.snow = 0;
            let glacier_melt = (melt - snow) * 7 / 4;
            f.glacier = (i64::from(f.glacier) - glacier_melt).max(0) as u32;
        }
    }
    if f.snow > FIRN_THRESHOLD {
        f.glacier = f.glacier.saturating_add(f.snow - FIRN_THRESHOLD);
        f.snow = FIRN_THRESHOLD;
    }

    if f.glacier >= GLACIER {
        f.frost = MAX_FROST;
        f.thaw = 0;
    } else if t < 0 {
        // Soil α ≈ 5 cm/√(°C·day), halved under a snow blanket.
        let coefficient = if f.snow >= 1_000 { 780 } else { 3_125 };
        if f.thaw > 0 {
            // The thin active layer refreezes first, from the top.
            let refreeze = coefficient * -t * q / 1_000;
            f.thaw = (i64::from(f.thaw) - refreeze).max(0) as u32;
        } else {
            let h = i64::from(f.frost).max(100);
            let growth = coefficient * -t * q / h;
            f.frost = (i64::from(f.frost) + growth).min(i64::from(MAX_FROST)) as u32;
        }
    } else if t > 0 && f.snow == 0 && f.frost > 0 {
        // Active layer: α ≈ 3 cm/√(°C·day).
        let h = i64::from(f.thaw).max(100);
        let thaw = i64::from(f.thaw) + 1_125 * t * q / h;
        if thaw >= i64::from(f.frost) {
            f.frost = 0;
            f.thaw = 0;
        } else {
            f.thaw = thaw as u32;
        }
    }
    if f.frost == 0 {
        f.thawed_this_year = true;
    }
    let mass_after = i64::from(f.snow) + i64::from(f.glacier);
    (mass_before + snowfall - mass_after).max(0) as u32
}

#[cfg(test)]
mod tests {
    use super::*;

    /// Runs `days` days at a constant temperature.
    fn run_water(t: i64, freezing_point: i64, days: i64, start: u32) -> u32 {
        let mut ice = start;
        for _ in 0..days {
            water_ice(&mut ice, t, freezing_point, 4, 1000);
        }
        ice
    }

    #[test]
    fn fresh_water_freezes_below_zero_and_thaws_above() {
        assert_eq!(run_water(5, 0, 30, 0), 0);
        let ice = run_water(-100, 0, 30, 0);
        // Stefan: α·√FDD = 2.4 cm · √300 ≈ 42 cm.
        assert!((3_000..=5_500).contains(&ice), "{ice}");
        assert_eq!(run_water(100, 0, 30, ice), 0);
    }

    #[test]
    fn sea_water_needs_colder_temperatures() {
        assert_eq!(run_water(-10, SEA_FREEZING_POINT, 30, 0), 0);
        assert!(run_water(-30, SEA_FREEZING_POINT, 30, 0) > 0);
    }

    #[test]
    fn thick_ice_grows_slower() {
        let thin = run_water(-150, 0, 1, 1_000) - 1_000;
        let thick = run_water(-150, 0, 1, 10_000) - 10_000;
        assert!(thin > thick * 5, "{thin} vs {thick}");
    }

    #[test]
    fn snow_accumulates_when_cold_and_melts_when_warm() {
        let mut f = Frozen::default();
        for _ in 0..100 {
            land(&mut f, -50, 4, 20);
        }
        assert_eq!(f.snow, 2_000);
        assert!(f.frost > 0);
        for _ in 0..30 {
            land(&mut f, 80, 4, 20);
        }
        assert_eq!(f.snow, 0);
    }

    #[test]
    fn persistent_snow_becomes_glacier() {
        let mut f = Frozen::default();
        for _ in 0..1_000 {
            land(&mut f, -100, 4, 30);
        }
        assert!(f.glacier >= GLACIER, "{f:?}");
        assert_eq!(f.snow, FIRN_THRESHOLD);
    }

    /// Runs years of a sinusoidal climate, returns the end state.
    fn run_years(mean: i64, amplitude: i64, years: usize) -> Frozen {
        let mut f = Frozen::default();
        for _ in 0..years {
            for day in 0..364 {
                let phase = day * crate::climate::trig::FULL_TURN / 364;
                let t =
                    mean + amplitude * crate::climate::trig::sin(phase) / crate::map::noise::ONE;
                land(&mut f, t, 4, 0);
            }
            f.permafrost = !f.thawed_this_year && f.frost > 0;
            f.thawed_this_year = f.frost == 0;
        }
        f
    }

    #[test]
    fn cold_climates_have_permafrost_mild_ones_do_not() {
        // Mean -8 °C, ±15 °C: summers thaw only an active layer.
        let tundra = run_years(-80, 150, 5);
        assert!(tundra.permafrost, "{tundra:?}");
        // Mean +6 °C, ±12 °C: winter frost, but it all thaws.
        let temperate = run_years(60, 120, 5);
        assert!(!temperate.permafrost, "{temperate:?}");
    }

    #[test]
    fn snow_insulates_the_ground() {
        let mut bare = Frozen::default();
        let mut snowy = Frozen {
            snow: 5_000,
            ..Frozen::default()
        };
        for _ in 0..60 {
            land(&mut bare, -100, 4, 0);
            land(&mut snowy, -100, 4, 0);
        }
        assert!(
            bare.frost > snowy.frost * 3 / 2,
            "{} vs {}",
            bare.frost,
            snowy.frost
        );
    }

    #[test]
    fn glaciers_settle_to_a_steady_state() {
        use crate::map::TerrainConfig;
        use crate::rng::SimRng;
        let map = Map::generate(64, 32, &TerrainConfig::default(), &mut SimRng::from_seed(5));
        let land = |i: usize| matches!(map.tiles()[i].water, Water::Dry | Water::River);
        // Snow piles up 30 cm a year in the polar rows and 2 m melts elsewhere.
        let balance: Vec<i64> = (0..map.tile_count() as usize)
            .map(|i| {
                let lat = map.latitude_tenths(map.position_of(i as u32).y).abs();
                match (land(i), lat >= 600) {
                    (false, _) => 0,
                    (true, true) => 3_000,
                    (true, false) => -20_000,
                }
            })
            .collect();
        assert!(balance.iter().any(|&b| b > 0), "no polar land on this map");
        let mut cryo = Cryosphere::new(map.tile_count() as usize);
        let years = cryo.settle_glaciers(&map, &balance, 364, 20_000);
        assert!(years < 20_000, "did not settle");
        let volume = |c: &Cryosphere| c.tiles().iter().map(|f| i64::from(f.glacier)).sum::<i64>();
        let settled = volume(&cryo);
        assert!(settled > 0);
        // Another century barely changes it (these sheets are only meters
        // thick, so the edge flicker is a few percent of them).
        let mut again = cryo.clone();
        again.settle_glaciers(&map, &balance, 364, 100);
        let change = (volume(&again) - settled).abs();
        assert!(change * 30 <= settled, "{settled} -> {}", volume(&again));
        for (i, f) in cryo.tiles().iter().enumerate() {
            if !land(i) {
                assert_eq!(f.glacier, 0, "ice on water");
            } else if f.glacier >= GLACIER {
                assert!(f.permafrost);
            }
        }
        // Ice flows out of the accumulation zone a little, but not far.
        let spread = (0..balance.len())
            .filter(|&i| balance[i] < 0 && cryo.tile(i).glacier >= GLACIER)
            .count();
        let core = (0..balance.len()).filter(|&i| balance[i] > 0).count();
        assert!(
            spread < core,
            "{spread} glaciated ablation tiles for {core} accumulation tiles"
        );
    }
}
