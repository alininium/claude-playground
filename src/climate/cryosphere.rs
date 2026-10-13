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

#[derive(Debug, Clone, Copy, Default, PartialEq, Eq, Hash)]
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
#[derive(Debug, Clone, Copy, PartialEq, Eq, Hash, PartialOrd, Ord)]
pub enum Cover {
    None,
    FrozenGround,
    Snow,
    RiverIce,
    LakeIce,
    SeaIce,
    Glacier,
}

#[derive(Debug, Clone)]
pub struct Cryosphere {
    tiles: Vec<Frozen>,
}

impl Cryosphere {
    pub fn new(tile_count: usize) -> Self {
        Self {
            tiles: vec![Frozen::default(); tile_count],
        }
    }

    pub fn tiles(&self) -> &[Frozen] {
        &self.tiles
    }

    pub fn tile(&self, index: usize) -> &Frozen {
        &self.tiles[index]
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
            40
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
        for (((f, tile), &temp), &rate) in self
            .tiles
            .iter_mut()
            .zip(map.tiles())
            .zip(temperatures)
            .zip(precipitation)
        {
            let t = i64::from(temp);
            match tile.water {
                Water::Ocean => water_ice(&mut f.ice, t, SEA_FREEZING_POINT, q, 700),
                Water::Lake => water_ice(&mut f.ice, t, 0, q, 1000),
                Water::River | Water::Dry => {
                    if tile.water == Water::River {
                        water_ice(&mut f.ice, t, 0, q, 800);
                    }
                    // mm per year → tenths of a millimeter in this step.
                    let precipitation = i64::from(rate) * 10 * q / per_year;
                    land(f, t, q, precipitation);
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
                    Water::Ocean | Water::Lake => delta[i] -= (g / 20_000).min(MAX_FLOW),
                    _ => {
                        let other = i64::from(self.tiles[nb].glacier);
                        if g > other {
                            let flux = ((g - other) / 20_000).min(MAX_FLOW);
                            delta[i] -= flux;
                            delta[nb] += flux;
                        }
                    }
                }
            }
        }
        for (f, d) in self.tiles.iter_mut().zip(delta) {
            f.glacier = (i64::from(f.glacier) + d).max(0) as u32;
        }
    }

    /// Called once a year: ground that never thawed is permafrost.
    pub fn end_year(&mut self) {
        for f in &mut self.tiles {
            f.permafrost = f.glacier >= GLACIER || (!f.thawed_this_year && f.frost > 0);
            f.thawed_this_year = f.frost == 0;
        }
    }

    pub(crate) fn tiles_mut(&mut self) -> &mut [Frozen] {
        &mut self.tiles
    }
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

fn land(f: &mut Frozen, t: i64, q: i64, precipitation: i64) {
    if t < 10 {
        f.snow = f.snow.saturating_add(precipitation as u32);
    }
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
}
