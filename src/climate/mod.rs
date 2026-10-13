//! Climate: a temperature for every tile, every turn.
//!
//! Each tile gets a static [`TileClimate`] profile at world creation:
//!
//! - **Annual mean** from latitude, lowered by altitude (lapse rate), warmed
//!   by dryness in the subtropics, cooled deep inside high-latitude
//!   continents. Oceans are milder than land at the same latitude.
//! - **Seasonal swing**, small at the equator and large toward the poles,
//!   amplified far from the sea (continentality), damped by oceans, lakes
//!   and rivers, reversed in the southern hemisphere. Water heats up and
//!   cools down slowly, so its seasons lag behind the land's.
//! - **Day/night range**: large in deserts and on high ground, small at sea
//!   and near lakes and rivers.
//! - **Continentality**: how far maritime air has to travel to reach the
//!   tile. Mountain ranges count as barriers, so the lee of a range gets a
//!   harsher, more continental climate.
//! - Sensitivity to the planet-wide [`oscillation`]s.
//!
//! The temperature at a given turn is then a pure function of that profile,
//! the date, the time of day and the global oscillation state.

pub mod oscillation;
pub mod trig;

use std::cmp::Reverse;
use std::collections::BinaryHeap;

use bevy_ecs::prelude::{Res, ResMut, Resource};

use crate::config::SimConfig;
use crate::map::noise::{Noise, ONE};
use crate::map::{Map, Water};
use crate::rng::SimRng;
use crate::time::{Date, TURNS_PER_DAY, TimeOfDay, Turn};
use oscillation::{Drift, Oscillation, OscillationConfig};

#[derive(Debug, Clone, PartialEq, Eq)]
pub struct ClimateConfig {
    pub days_per_year: u32,
    /// Cooling with altitude, tenths of a degree per kilometer.
    pub lapse_rate: i32,
    /// The map spans the whole planet east-west; this sets the tile size.
    pub planet_circumference_km: u32,
    /// Distance from the sea (barriers included) at which a climate is fully
    /// continental.
    pub maritime_range_km: u32,
    pub oscillation: OscillationConfig,
}

impl Default for ClimateConfig {
    fn default() -> Self {
        Self {
            days_per_year: 364,
            lapse_rate: 65,
            planet_circumference_km: 40_000,
            maritime_range_km: 1_500,
            oscillation: OscillationConfig::default(),
        }
    }
}

/// Static climate profile of one tile. Temperatures in tenths of a degree C.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub struct TileClimate {
    pub mean: i16,
    /// Half the summer-winter difference. Negative in the southern hemisphere.
    pub seasonal_amplitude: i16,
    /// Days by which the hottest day trails the summer solstice.
    pub seasonal_lag_days: u16,
    /// Difference between the warmest and the coldest part of the day.
    pub diurnal_range: i16,
    /// Response to the oscillation index, per mille (negative = opposite).
    pub oscillation_weight: i16,
    /// Response to global drift, per mille (polar amplification).
    pub drift_weight: i16,
    /// 0 (maritime) to 1000 (fully continental).
    pub continentality: u16,
}

#[derive(Resource, Debug, Clone)]
pub struct Climate {
    days_per_year: u32,
    profiles: Vec<TileClimate>,
    /// Current temperature of every tile, tenths of a degree C.
    temperature: Vec<i16>,
    /// cos(latitude) per row, per mille: area weights for global averages.
    row_weight: Vec<i64>,
    width: usize,
    oscillation: Oscillation,
    drift: Drift,
}

impl Climate {
    pub fn new(map: &Map, config: &ClimateConfig, rng: &mut SimRng) -> Self {
        let oscillation = Oscillation::new(SimRng::from_seed(rng.next_u64()));
        let drift = Drift::new(SimRng::from_seed(rng.next_u64()));
        let teleconnections = Noise::new(rng.next_u64());
        // Longitude of the oscillation's warm pool (its "eastern Pacific").
        let center_x = rng.below(u64::from(map.width())) as i64;

        let continentality = continentality(map, config);
        let profiles = (0..map.tile_count() as usize)
            .map(|i| {
                profile(
                    map,
                    config,
                    i,
                    continentality[i],
                    &teleconnections,
                    center_x,
                )
            })
            .collect();
        let row_weight = (0..map.height())
            .map(|y| trig::cos(map.latitude_tenths(y) * trig::FULL_TURN / 3600) * 1000 / ONE)
            .collect();

        let mut climate = Self {
            days_per_year: config.days_per_year,
            profiles,
            temperature: vec![0; map.tile_count() as usize],
            row_weight,
            width: map.width() as usize,
            oscillation,
            drift,
        };
        climate.update_temperatures(0);
        climate
    }

    pub fn temperature(&self, index: usize) -> i16 {
        self.temperature[index]
    }

    pub fn temperatures(&self) -> &[i16] {
        &self.temperature
    }

    pub fn profile(&self, index: usize) -> &TileClimate {
        &self.profiles[index]
    }

    pub fn oscillation(&self) -> &Oscillation {
        &self.oscillation
    }

    pub fn drift(&self) -> &Drift {
        &self.drift
    }

    pub fn days_per_year(&self) -> u32 {
        self.days_per_year
    }

    /// Area-weighted (cos latitude) planetary mean temperature, tenths °C.
    pub fn global_mean(&self) -> i64 {
        let (mut sum, mut weight) = (0i64, 0i64);
        for (row, temps) in self.temperature.chunks(self.width).enumerate() {
            let w = self.row_weight[row].max(1);
            sum += temps.iter().map(|&t| i64::from(t)).sum::<i64>() * w;
            weight += w * temps.len() as i64;
        }
        sum / weight.max(1)
    }

    fn advance_day(&mut self, config: &OscillationConfig) {
        self.oscillation.step_day(config);
        self.drift.step_day(config);
    }

    fn update_temperatures(&mut self, turn: u64) {
        let date = Date::from_turn(turn, self.days_per_year);
        let osc = self.oscillation.state().0;
        let drift = self.drift.anomaly();
        for (t, profile) in self.temperature.iter_mut().zip(&self.profiles) {
            *t = temperature_at(profile, &date, self.days_per_year, osc, drift)
                .clamp(i16::MIN as i64, i16::MAX as i64) as i16;
        }
    }

    pub(crate) fn digest_state(&self) -> (i64, i64, i64) {
        let (index, heat) = self.oscillation.state();
        (index, heat, self.drift.anomaly())
    }
}

/// Runs every turn: advances the global oscillations at the start of each
/// day, then recomputes every tile's temperature.
pub fn update_climate(turn: Res<Turn>, config: Res<SimConfig>, mut climate: ResMut<Climate>) {
    if turn.0.is_multiple_of(TURNS_PER_DAY) {
        climate.advance_day(&config.climate.oscillation);
    }
    climate.update_temperatures(turn.0);
}

/// Temperature in tenths of a degree for a tile profile at a point in time.
/// `oscillation_index` and `drift` are in °C · ONE.
pub fn temperature_at(
    profile: &TileClimate,
    date: &Date,
    days_per_year: u32,
    oscillation_index: i64,
    drift: i64,
) -> i64 {
    // Seasons: a cosine peaking `lag` days after the northern summer solstice
    // (a quarter year after turn 0), at quarter-day resolution.
    let quarter_days_per_year = i64::from(days_per_year) * 4;
    let now = i64::from(date.day_of_year) * 4 + date.time_of_day as i64;
    let peak = (i64::from(days_per_year) / 4 + i64::from(profile.seasonal_lag_days)) * 4;
    let phase = (now - peak) * trig::FULL_TURN / quarter_days_per_year;
    let seasonal = i64::from(profile.seasonal_amplitude) * trig::cos(phase) / ONE;

    // Daily cycle, per mille of the diurnal range; averages to zero.
    let diurnal_share = match date.time_of_day {
        TimeOfDay::Morning => -150,
        TimeOfDay::Day => 450,
        TimeOfDay::Evening => 100,
        TimeOfDay::Night => -400,
    };
    let diurnal = i64::from(profile.diurnal_range) * diurnal_share / 1000;

    let oscillation = oscillation_index * i64::from(profile.oscillation_weight) * 10 / (1000 * ONE);
    let drift = drift * i64::from(profile.drift_weight) * 10 / (1000 * ONE);

    i64::from(profile.mean) + seasonal + diurnal + oscillation + drift
}

fn profile(
    map: &Map,
    config: &ClimateConfig,
    i: usize,
    continentality: u16,
    teleconnections: &Noise,
    center_x: i64,
) -> TileClimate {
    let tile = &map.tiles()[i];
    let pos = map.position_of(i as u32);
    let lat = map.latitude_tenths(pos.y);
    let abs_lat = lat.abs();
    let cont = i64::from(continentality);
    let dryness = 1000 - i64::from(tile.moisture.min(1000));
    let surface = i64::from(tile.surface.max(0));
    let lapse = surface * i64::from(config.lapse_rate) / 1000;
    let near_fresh_water = tile.water == Water::River
        || map
            .neighbors(i)
            .any(|nb| matches!(map.tiles()[nb].water, Water::Lake | Water::River));

    // Annual mean.
    let zonal = interpolate(&ZONAL_MEAN, abs_lat);
    let mean = match tile.water {
        Water::Ocean => zonal.max(-20),
        Water::Lake => zonal - lapse,
        _ => {
            let desert_heat = dryness * 20 / 1000 * i64::from(abs_lat < 400);
            let cold_interior = cont * (abs_lat - 300).max(0) * 40 / (600 * 1000);
            zonal - lapse + desert_heat - cold_interior
        }
    };

    // Seasonal swing.
    let zonal_swing = interpolate(&ZONAL_SWING, abs_lat);
    let mut swing = match tile.water {
        Water::Ocean => zonal_swing * 300 / 1000,
        Water::Lake => zonal_swing * 600 / 1000,
        _ => zonal_swing * (450 + 550 * cont / 1000) / 1000 * (1000 + dryness / 5) / 1000,
    };
    if near_fresh_water && tile.water != Water::Lake {
        swing = swing * 900 / 1000;
    }
    if lat < 0 {
        swing = -swing;
    }
    let lag = match tile.water {
        Water::Ocean => 50,
        Water::Lake => 40,
        _ => 25 + 15 * (1000 - cont) / 1000,
    };

    // Day/night range.
    let diurnal = match tile.water {
        Water::Ocean => 10,
        Water::Lake => 30,
        _ => {
            let range = (60 + dryness * 120 / 1000 + surface * 25 / 1000)
                * (600 + 400 * cont / 1000)
                / 1000;
            if near_fresh_water {
                range * 750 / 1000
            } else {
                range
            }
        }
    };

    // Oscillation teleconnections: strongest over the tropical ocean near the
    // warm pool, opposite sign on the far side of the planet, and a weaker
    // noisy pattern outside the tropics.
    let tropics = (1000 - (abs_lat * 1000 / 250).pow(2) / 1000).max(0);
    let lon_phase = (pos.x as i64 - center_x) * trig::FULL_TURN / i64::from(map.width());
    let east_west = trig::cos(lon_phase) * 1000 / ONE;
    let fx = (2 * pos.x as i64 + 1) * ONE * 4 / (2 * i64::from(map.width()));
    let fy = (2 * pos.y as i64 + 1) * ONE * 4 / (2 * i64::from(map.width()));
    let remote = teleconnections.fbm(fx, fy, 3) * 400 / ONE * (1000 - tropics) / 1000;
    let oscillation_weight = match tile.water {
        Water::Ocean => tropics * (400 + 600 * east_west / 1000) / 1000 + remote / 2,
        _ => tropics * (200 + 300 * east_west / 1000) / 1000 + remote,
    };

    TileClimate {
        mean: mean as i16,
        seasonal_amplitude: swing as i16,
        seasonal_lag_days: lag as u16,
        diurnal_range: diurnal as i16,
        oscillation_weight: oscillation_weight as i16,
        drift_weight: (1000 + abs_lat * 1000 / 900) as i16,
        continentality,
    }
}

/// Zonal annual mean at sea level, tenths °C, by |latitude| in tenths of a degree.
const ZONAL_MEAN: [(i64, i64); 10] = [
    (0, 270),
    (100, 270),
    (200, 250),
    (300, 195),
    (400, 130),
    (500, 60),
    (600, -10),
    (700, -100),
    (800, -180),
    (900, -250),
];

/// Typical half summer-winter difference over land, tenths °C.
const ZONAL_SWING: [(i64, i64); 7] = [
    (0, 10),
    (150, 25),
    (300, 70),
    (450, 120),
    (600, 170),
    (750, 200),
    (900, 220),
];

fn interpolate(table: &[(i64, i64)], x: i64) -> i64 {
    for pair in table.windows(2) {
        let ((x0, y0), (x1, y1)) = (pair[0], pair[1]);
        if x <= x1 {
            return y0 + (y1 - y0) * (x - x0).max(0) / (x1 - x0);
        }
    }
    table[table.len() - 1].1
}

/// Distance maritime air must travel from the ocean, with mountains as
/// barriers, mapped to 0 (coast) ..= 1000 (fully continental).
fn continentality(map: &Map, config: &ClimateConfig) -> Vec<u16> {
    let n = map.tile_count() as usize;
    let w = map.width() as usize;
    let tiles = map.tiles();
    // Cost units: 10 per tile.
    let range = (i64::from(config.maritime_range_km) * i64::from(map.width()) * 10
        / i64::from(config.planet_circumference_km.max(1)))
    .max(10);

    let mut cost = vec![i64::MAX; n];
    let mut heap = BinaryHeap::new();
    for (i, tile) in tiles.iter().enumerate() {
        if tile.water == Water::Ocean {
            cost[i] = 0;
            heap.push(Reverse((0i64, i)));
        }
    }
    while let Some(Reverse((c, i))) = heap.pop() {
        if c > cost[i] {
            continue;
        }
        for nb in map.neighbors(i) {
            let diagonal = nb % w != i % w && nb / w != i / w;
            let barrier = i64::from((tiles[nb].surface - 1000).max(0)) / 50;
            let next = c + if diagonal { 14 } else { 10 } + barrier;
            if next < cost[nb] {
                cost[nb] = next;
                heap.push(Reverse((next, nb)));
            }
        }
    }
    cost.into_iter()
        .map(|c| (c.min(range) * 1000 / range) as u16)
        .collect()
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::time::Season;

    fn land(lat_sign: i16) -> TileClimate {
        TileClimate {
            mean: 100,
            seasonal_amplitude: 150 * lat_sign,
            seasonal_lag_days: 30,
            diurnal_range: 120,
            oscillation_weight: 500,
            drift_weight: 1000,
            continentality: 800,
        }
    }

    fn temp(profile: &TileClimate, turn: u64) -> i64 {
        temperature_at(profile, &Date::from_turn(turn, 364), 364, 0, 0)
    }

    fn mid_season(season: Season) -> u64 {
        (season as u64 * 91 + 60) * TURNS_PER_DAY + 1
    }

    #[test]
    fn hemispheres_have_opposite_seasons() {
        let (north, south) = (land(1), land(-1));
        let summer = mid_season(Season::Summer);
        let winter = mid_season(Season::Winter);
        assert!(temp(&north, summer) > temp(&north, winter) + 200);
        assert!(temp(&south, summer) + 200 < temp(&south, winter));
    }

    #[test]
    fn day_is_warmer_than_night() {
        let p = land(1);
        let day = 100 * TURNS_PER_DAY;
        let temps: Vec<_> = (0..4).map(|t| temp(&p, day + t)).collect();
        assert!(temps[1] > temps[0] && temps[1] > temps[2] && temps[2] > temps[3]);
        assert!(temps[1] - temps[3] >= 90);
    }

    #[test]
    fn oscillation_and_drift_shift_temperature() {
        let p = land(1);
        let date = Date::from_turn(500, 364);
        let base = temperature_at(&p, &date, 364, 0, 0);
        assert_eq!(temperature_at(&p, &date, 364, 2 * ONE, 0), base + 10);
        assert_eq!(temperature_at(&p, &date, 364, 0, ONE), base + 10);
    }

    #[test]
    fn annual_average_is_the_mean() {
        let p = land(1);
        let sum: i64 = (0..364 * TURNS_PER_DAY).map(|t| temp(&p, t)).sum();
        let avg = sum / (364 * TURNS_PER_DAY) as i64;
        assert!((avg - i64::from(p.mean)).abs() <= 2, "avg {avg}");
    }
}
