//! Climate: a temperature for every tile, every turn.
//!
//! Each tile gets a static [`TileClimate`] profile at world creation:
//!
//! - **Annual mean** from latitude, lowered by altitude (lapse rate), warmed
//!   by dryness in the subtropics, cooled deep inside high-latitude
//!   continents. Oceans are milder than land at the same latitude, and an
//!   irregular pattern of warm and cold ocean currents (strongest at high
//!   latitudes, also felt on nearby coasts) makes the sea-ice edge ragged.
//! - **Seasonal swing**, small at the equator and large toward the poles,
//!   amplified far from the sea (continentality), damped by oceans, lakes
//!   and rivers, reversed in the southern hemisphere. Water heats up and
//!   cools down slowly, so its seasons lag behind the land's.
//! - **Day/night range**: large in deserts and on high ground, small at sea
//!   and near lakes and rivers.
//! - **Continentality**: how far maritime air has to travel to reach the
//!   tile. Mountain ranges count as barriers, so the lee of a range gets a
//!   harsher, more continental climate.
//! - Sensitivity to the planet-wide [`oscillation`]s, for temperature and
//!   for rain.
//! - **Precipitation**: a long-term amount from the map's rainfall pattern
//!   (prevailing winds, rain shadows) and a seasonal cycle: monsoon summer
//!   rain in the outer tropics, winter rain on subtropical west coasts
//!   (Mediterranean climates), convective summer rain inside continents.
//! - How variable its **weather** is.
//!
//! The temperature at a given turn is then a function of that profile, the
//! date, the time of day (with a daily cycle that shrinks in polar night and
//! midnight sun, see [`daylight`]), the global oscillation state, the day's
//! [`weather`], volcanic aerosols, and the tile's ice and snow (which reflect
//! sunlight; see [`cryosphere`]). Precipitation follows the season, the
//! oscillation and the weather; it feeds snow and the [`soil`] water balance,
//! whose runoff makes the rivers rise and flood.
//!
//! At world creation the climate is spun up for a few years so seasonal
//! snow, sea ice, frozen ground and the polar caps are already in place at
//! turn 0. Glaciers are seeded where snow survives the summer, with the
//! thickness a few centuries of that surplus would have built.

pub mod cryosphere;
pub mod daylight;
pub mod oscillation;
pub mod soil;
pub mod stats;
pub mod trig;
pub mod weather;

use std::cmp::Reverse;
use std::collections::BinaryHeap;

use bevy_ecs::prelude::{Res, ResMut, Resource};

use crate::config::SimConfig;
use crate::map::noise::{Noise, ONE};
use crate::map::{Map, Water};
use crate::rng::SimRng;
use crate::time::{Date, TURNS_PER_DAY, TimeOfDay, Turn};
use cryosphere::Cryosphere;
use oscillation::{Drift, Oscillation, OscillationConfig};
use soil::SoilWater;
use stats::{ClimateNormal, SeasonSummary, YearStats};
use weather::Weather;

#[derive(Debug, Clone, PartialEq, Eq, serde::Serialize, serde::Deserialize)]
pub struct ClimateConfig {
    pub days_per_year: u32,
    /// Cooling with altitude, tenths of a degree per kilometer.
    pub lapse_rate: i32,
    /// The map spans the whole planet east-west; this sets the tile size.
    pub planet_circumference_km: u32,
    /// Distance from the sea (barriers included) at which a climate is fully
    /// continental.
    pub maritime_range_km: u32,
    /// Years of climate simulated before turn 0 (at least 2).
    pub spinup_years: u32,
    /// Glaciers are seeded with this many years of their yearly snow surplus.
    pub glacier_seed_years: u32,
    pub oscillation: OscillationConfig,
}

impl Default for ClimateConfig {
    fn default() -> Self {
        Self {
            days_per_year: 364,
            lapse_rate: 65,
            planet_circumference_km: 40_000,
            maritime_range_km: 1_500,
            spinup_years: 4,
            glacier_seed_years: 300,
            oscillation: OscillationConfig::default(),
        }
    }
}

/// Static climate profile of one tile. Temperatures in tenths of a degree C.
#[derive(Debug, Clone, Copy, PartialEq, Eq, serde::Serialize, serde::Deserialize)]
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
    /// Long-term precipitation, mm per year.
    pub base_precipitation: u16,
    /// Seasonal precipitation cycle, per mille of the mean: positive peaks
    /// in northern summer, negative in northern winter (so a southern
    /// monsoon is negative).
    pub rain_seasonality: i16,
    /// Change in precipitation per °C of oscillation index, per mille.
    pub rain_oscillation: i16,
    /// Typical day-to-day weather temperature anomaly, tenths °C.
    pub weather_amplitude: i16,
}

#[derive(Resource, Debug, Clone, serde::Serialize, serde::Deserialize)]
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
    cryosphere: Cryosphere,
    /// Current precipitation rate of every tile, mm per year.
    precipitation: Vec<u16>,
    stats: YearStats,
    weather: Weather,
    soil: SoilWater,
    /// Today's day length per row, minutes.
    daylight: Vec<u16>,
    /// The same, per tile (for the soil water balance).
    tile_daylight: Vec<u16>,
    /// Latitude of each row, tenths of a degree.
    latitudes: Vec<i64>,
    /// Volcanic aerosol cooling, tenths °C: northern, southern hemisphere.
    volcanic_cooling: [i16; 2],
    /// Absolute day number (days since turn 0).
    day: u64,
    /// The last season that ended, and the turn it was closed on.
    last_season: Option<(u64, SeasonSummary)>,
}

impl Climate {
    pub fn new(map: &Map, config: &ClimateConfig, rng: &mut SimRng) -> Self {
        let oscillation = Oscillation::new(SimRng::from_seed(rng.next_u64()));
        let drift = Drift::new(SimRng::from_seed(rng.next_u64()));
        let teleconnections = Noise::new(rng.next_u64());
        let currents = Noise::new(rng.next_u64());
        let weather_seed = rng.next_u64();
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
                    &currents,
                    center_x,
                )
            })
            .collect();
        let row_weight = (0..map.height()).map(|y| map.area_weight(y)).collect();

        let n = map.tile_count() as usize;
        let mut climate = Self {
            days_per_year: config.days_per_year,
            profiles,
            temperature: vec![0; n],
            row_weight,
            width: map.width() as usize,
            oscillation,
            drift,
            cryosphere: Cryosphere::new(n),
            precipitation: vec![0; n],
            stats: YearStats::new(n),
            weather: Weather::new(weather_seed, n),
            soil: SoilWater::new(map),
            daylight: vec![720; map.height() as usize],
            tile_daylight: vec![720; n],
            latitudes: (0..map.height()).map(|y| map.latitude_tenths(y)).collect(),
            volcanic_cooling: [0, 0],
            day: 0,
            last_season: None,
        };
        climate.spin_up(map, config);
        climate.start_day(0, false);
        climate.update_temperatures(0);
        climate
    }

    /// Sets up a day: day length, weather (unless `neutral`) and
    /// precipitation rates.
    fn start_day(&mut self, day: u64, neutral: bool) {
        let day_of_year = (day % u64::from(self.days_per_year)) as u32;
        for (d, &lat) in self.daylight.iter_mut().zip(&self.latitudes) {
            *d = daylight::day_length_minutes(lat, day_of_year, self.days_per_year) as u16;
        }
        for (i, d) in self.tile_daylight.iter_mut().enumerate() {
            *d = self.daylight[i / self.width];
        }
        if !neutral {
            let profiles = &self.profiles;
            self.weather
                .update(day, self.width as u32, |i| profiles[i].weather_amplitude);
        }
        // Seasonal rain peaks about a month after the northern summer solstice.
        let year = i64::from(self.days_per_year);
        let phase = (i64::from(day_of_year) - (year / 4 + 30)) * trig::FULL_TURN / year;
        let season = trig::cos(phase);
        let osc = if neutral {
            0
        } else {
            self.oscillation.state().0
        };
        for (i, (rate, p)) in self
            .precipitation
            .iter_mut()
            .zip(&self.profiles)
            .enumerate()
        {
            let seasonal = 1000 + i64::from(p.rain_seasonality) * season / ONE;
            let oscillation = (1000 + osc * i64::from(p.rain_oscillation) / ONE).clamp(200, 3000);
            let weather = if neutral {
                1000
            } else {
                i64::from(self.weather.precipitation_factor(i))
            };
            let r = i64::from(p.base_precipitation) * seasonal / 1000 * oscillation / 1000
                * weather
                / 1000;
            *rate = r.clamp(0, i64::from(u16::MAX)) as u16;
        }
    }

    /// Runs whole years of neutral climate (no oscillation, no drift) at
    /// daily resolution, seeding glaciers after the second year.
    fn spin_up(&mut self, map: &Map, config: &ClimateConfig) {
        let years = config.spinup_years.max(2);
        let land = |i: usize| matches!(map.tiles()[i].water, Water::Dry | Water::River);
        let mut before = Vec::new();
        for year in 0..years {
            if year == 1 {
                before = self.ice_mass();
            }
            let mut night = vec![0i16; self.temperature.len()];
            for day in 0..self.days_per_year {
                self.start_day(u64::from(day), true);
                for (i, t) in self.temperature.iter_mut().enumerate() {
                    let p = &self.profiles[i];
                    let mean = daily_mean_temperature(p, day, self.days_per_year, 0, 0)
                        - self.cryosphere.albedo_cooling(i);
                    *t = clamp16(mean);
                    let scale = diurnal_scale(self.tile_daylight[i]);
                    night[i] =
                        clamp16(mean - i64::from(p.diurnal_range) * scale / 1000 * 400 / 1000);
                }
                self.step_water(map, TURNS_PER_DAY as u32);
                self.soil.route(map);
                let season = (day * 4 / self.days_per_year) as usize;
                let soil = &self.soil;
                self.stats.accumulate(
                    &self.temperature,
                    &self.precipitation,
                    |i| soil.wetness(i),
                    TURNS_PER_DAY as u32,
                    season,
                );
                self.stats
                    .record_day(&night, |i| soil.is_flooding(i), season);
                // No flow before seeding: the surplus must be snowfall alone.
                if year >= 2 {
                    self.cryosphere.flow_glaciers(map);
                }
            }
            self.end_year();
            if year == 1 {
                let after = self.ice_mass();
                let seed_years = i64::from(config.glacier_seed_years);
                for (i, f) in self.cryosphere.tiles_mut().iter_mut().enumerate() {
                    let surplus = after[i] - before[i];
                    if land(i) && surplus > 0 {
                        f.glacier =
                            (i64::from(f.glacier) + surplus * seed_years).min(30_000_000) as u32;
                    }
                }
            }
        }
    }

    fn ice_mass(&self) -> Vec<i64> {
        self.cryosphere
            .tiles()
            .iter()
            .map(|f| i64::from(f.snow) + i64::from(f.glacier))
            .collect()
    }

    /// Snow, ice and soil water for `quarters` quarter-days.
    fn step_water(&mut self, map: &Map, quarters: u32) {
        self.cryosphere.step(
            map,
            &self.temperature,
            &self.precipitation,
            quarters,
            self.days_per_year,
        );
        self.soil.step(
            map,
            &self.temperature,
            &self.precipitation,
            self.cryosphere.melt(),
            &self.tile_daylight,
            quarters,
            self.days_per_year,
        );
    }

    fn end_year(&mut self) {
        self.cryosphere.end_year();
        self.stats.end_year(self.days_per_year);
    }

    /// The most recently finished season and the turn it ended on.
    pub fn last_season(&self) -> Option<&(u64, SeasonSummary)> {
        self.last_season.as_ref()
    }

    pub fn soil(&self) -> &SoilWater {
        &self.soil
    }

    pub fn weather(&self) -> &Weather {
        &self.weather
    }

    /// Today's day length at a row, minutes.
    pub fn day_length_minutes(&self, y: u32) -> u16 {
        self.daylight[y as usize]
    }

    /// Mean soil wetness during last year's growing season, per mille.
    pub fn growing_season_wetness(&self, index: usize) -> u16 {
        self.stats.wetness_last_year[index]
    }

    /// Frost-free nights in a row last year (the growing season length).
    pub fn frost_free_days(&self, index: usize) -> u16 {
        self.stats.frost_free_last_year[index]
    }

    /// Days the tile's river was in flood last year.
    pub fn flood_days(&self, index: usize) -> u16 {
        self.stats.flood_days_last_year[index]
    }

    /// Volcanic aerosol cooling (tenths °C) for the northern and southern
    /// hemispheres; set by the volcano system.
    pub fn set_volcanic_cooling(&mut self, northern: i16, southern: i16) {
        self.volcanic_cooling = [northern, southern];
    }

    pub fn volcanic_cooling(&self) -> [i16; 2] {
        self.volcanic_cooling
    }

    pub fn cryosphere(&self) -> &Cryosphere {
        &self.cryosphere
    }

    /// Growing degree-days (base 5 °C) of the last full year, °C·days.
    pub fn growing_degree_days(&self, index: usize) -> u32 {
        self.stats.growing_last_year[index]
    }

    /// The tile's long-term climate (about a 10-year moving average).
    pub fn normal(&self, index: usize) -> &ClimateNormal {
        &self.stats.normals[index]
    }

    /// Current precipitation rate, mm per year.
    pub fn precipitation(&self, index: usize) -> u16 {
        self.precipitation[index]
    }

    pub fn precipitations(&self) -> &[u16] {
        &self.precipitation
    }

    /// Mean temperature of the warmest and the coldest month, tenths °C,
    /// from the long-term mean and the tile's seasonal swing.
    pub fn warmest_and_coldest_month(&self, index: usize) -> (i64, i64) {
        let mean = self.stats.normals[index].mean_tenths();
        // A month-long average of a cosine peak: amplitude × sinc(π/12) ≈ 0.989.
        let swing = i64::from(self.profiles[index].seasonal_amplitude).abs() * 989 / 1000;
        (mean + swing, mean - swing)
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
        // The seasonal cosine depends on the tile only through its lag:
        // compute it once per distinct lag (same formula as `temperature_at`).
        let now = quarter_day(&date);
        let mut season_by_lag = [None; 128];
        let share = diurnal_share(date.time_of_day);
        for (i, (t, profile)) in self.temperature.iter_mut().zip(&self.profiles).enumerate() {
            let row = i / self.width;
            let scale = diurnal_scale(self.daylight[row]);
            let volcanic = self.volcanic_cooling[usize::from(self.latitudes[row] < 0)];
            let lag = usize::from(profile.seasonal_lag_days).min(127);
            let season = *season_by_lag[lag]
                .get_or_insert_with(|| seasonal_cosine(profile, now, self.days_per_year));
            let temp = mean_with_season(profile, season, osc, drift)
                + i64::from(profile.diurnal_range) * scale / 1000 * share / 1000
                - self.cryosphere.albedo_cooling(i)
                + i64::from(self.weather.temperature(i))
                - i64::from(volcanic);
            *t = clamp16(temp);
        }
    }

    pub(crate) fn digest(&self, write: &mut impl FnMut(u64)) {
        let (index, heat) = self.oscillation.state();
        for v in [index, heat, self.drift.anomaly()] {
            write(v as u64);
        }
        for &t in &self.temperature {
            write(t as u64);
        }
        for f in self.cryosphere.tiles() {
            for v in [
                f.snow,
                f.ice,
                f.glacier,
                f.frost,
                f.thaw,
                u32::from(f.permafrost),
            ] {
                write(u64::from(v));
            }
        }
        for &p in &self.precipitation {
            write(u64::from(p));
        }
        self.stats.digest(write);
        self.weather.digest(write);
        self.soil.digest(write);
        write(self.day);
        write(
            u64::from(self.volcanic_cooling[0] as u16) << 16
                | u64::from(self.volcanic_cooling[1] as u16),
        );
    }
}

/// Long-term mean precipitation for a moisture index (0-1000), mm per year:
/// `m + m²/1000`, skewed like Earth's rainfall distribution. Moisture 1000
/// gives 2000 mm (rainforest), 500 gives 750 mm, 150 gives 170 mm (desert).
pub fn annual_precipitation_mm(moisture: u16) -> u16 {
    let m = u32::from(moisture.min(1000));
    (m + m * m / 1000) as u16
}

fn clamp16(v: i64) -> i16 {
    v.clamp(i64::from(i16::MIN), i64::from(i16::MAX)) as i16
}

/// How much of a tile's normal day/night range applies at a day length,
/// per mille: full at 12 hours, shrinking toward polar night and midnight
/// sun (when the sun neither rises nor sets, there is no daily cycle).
pub fn diurnal_scale(daylight_minutes: u16) -> i64 {
    let l = i64::from(daylight_minutes.min(1440));
    300 + 700 * 4 * l * (1440 - l) / (1440 * 1440)
}

/// Runs every turn: at the start of each day advances the global
/// oscillations, closes the books at the start of each year, and sets up
/// the day's daylight, weather and precipitation; recomputes every tile's
/// temperature; then lets ice, snow and soil water respond. Rivers are
/// routed and frost and floods recorded once a day, at night.
pub fn update_climate(
    turn: Res<Turn>,
    config: Res<SimConfig>,
    map: Res<Map>,
    mut climate: ResMut<Climate>,
) {
    let date = Date::from_turn(turn.0, climate.days_per_year);
    if turn.0.is_multiple_of(TURNS_PER_DAY) {
        climate.advance_day(&config.climate.oscillation);
        if date.day_of_season == 0 && turn.0 > 0 {
            // The previous season just ended: summarize it before the books
            // are closed for the year.
            let ended = (date.season as usize + 3) % 4;
            let summary = climate.stats.season_summary(ended, climate.days_per_year);
            climate.last_season = Some((turn.0, summary));
        }
        if date.day_of_year == 0 {
            climate.end_year();
        }
        climate.day = turn.0 / TURNS_PER_DAY;
        let day = climate.day;
        climate.start_day(day, false);
    }
    climate.update_temperatures(turn.0);
    let climate = &mut *climate;
    climate.step_water(&map, 1);
    let soil = &climate.soil;
    climate.stats.accumulate(
        &climate.temperature,
        &climate.precipitation,
        |i| soil.wetness(i),
        1,
        date.season as usize,
    );
    if date.time_of_day == TimeOfDay::Night {
        climate.cryosphere.flow_glaciers(&map);
        climate.soil.route(&map);
        let soil = &climate.soil;
        climate.stats.record_day(
            &climate.temperature,
            |i| soil.is_flooding(i),
            date.season as usize,
        );
    }
}

/// True on the first turn of a new simulated year (when last year's
/// statistics have just been finalized).
pub fn is_new_year(turn: u64, days_per_year: u32) -> bool {
    let date = Date::from_turn(turn, days_per_year);
    turn > 0 && date.day_of_year == 0 && date.time_of_day == TimeOfDay::Morning
}

/// Temperature in tenths of a degree for a tile profile at a point in time,
/// before ice/snow albedo, weather and volcanoes. `oscillation_index` and
/// `drift` are in °C · ONE; `diurnal_scale` (per mille) scales the daily
/// cycle (see [`diurnal_scale`]).
pub fn temperature_at(
    profile: &TileClimate,
    date: &Date,
    days_per_year: u32,
    oscillation_index: i64,
    drift: i64,
    diurnal_scale: i64,
) -> i64 {
    let mean = mean_at(
        profile,
        quarter_day(date),
        days_per_year,
        oscillation_index,
        drift,
    );
    let share = diurnal_share(date.time_of_day);
    let diurnal = i64::from(profile.diurnal_range) * diurnal_scale / 1000 * share / 1000;
    mean + diurnal
}

/// Quarter-days since the start of the year.
fn quarter_day(date: &Date) -> i64 {
    i64::from(date.day_of_year) * 4 + date.time_of_day as i64
}

/// Daily cycle, per mille of the diurnal range; averages to zero.
fn diurnal_share(time: TimeOfDay) -> i64 {
    match time {
        TimeOfDay::Morning => -150,
        TimeOfDay::Day => 450,
        TimeOfDay::Evening => 100,
        TimeOfDay::Night => -400,
    }
}

/// Average temperature over a whole day (the daily cycle averages out).
pub fn daily_mean_temperature(
    profile: &TileClimate,
    day_of_year: u32,
    days_per_year: u32,
    oscillation_index: i64,
    drift: i64,
) -> i64 {
    let midday = i64::from(day_of_year) * 4 + 1;
    mean_at(profile, midday, days_per_year, oscillation_index, drift)
}

/// Everything but the daily cycle, at `now` quarter-days into the year.
fn mean_at(
    profile: &TileClimate,
    now: i64,
    days_per_year: u32,
    oscillation_index: i64,
    drift: i64,
) -> i64 {
    let season = seasonal_cosine(profile, now, days_per_year);
    mean_with_season(profile, season, oscillation_index, drift)
}

/// Seasons: a cosine (fixed point) peaking `lag` days after the northern
/// summer solstice (a quarter year after turn 0), at quarter-day resolution.
fn seasonal_cosine(profile: &TileClimate, now: i64, days_per_year: u32) -> i64 {
    let quarter_days_per_year = i64::from(days_per_year) * 4;
    let peak = (i64::from(days_per_year) / 4 + i64::from(profile.seasonal_lag_days)) * 4;
    trig::cos((now - peak) * trig::FULL_TURN / quarter_days_per_year)
}

fn mean_with_season(profile: &TileClimate, season: i64, oscillation_index: i64, drift: i64) -> i64 {
    let seasonal = i64::from(profile.seasonal_amplitude) * season / ONE;
    let oscillation = oscillation_index * i64::from(profile.oscillation_weight) * 10 / (1000 * ONE);
    let drift = drift * i64::from(profile.drift_weight) * 10 / (1000 * ONE);

    i64::from(profile.mean) + seasonal + oscillation + drift
}

fn profile(
    map: &Map,
    config: &ClimateConfig,
    i: usize,
    continentality: u16,
    teleconnections: &Noise,
    currents: &Noise,
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

    // Ocean currents: warm and cold anomalies of a few degrees, in an
    // irregular pattern. Fully felt at sea, half on the coast, fading inland.
    let (fx, fy) = (
        (2 * pos.x as i64 + 1) * ONE * 5 / (2 * i64::from(map.width())),
        (2 * pos.y as i64 + 1) * ONE * 5 / (2 * i64::from(map.width())),
    );
    let current = currents.fbm(fx, fy, 5) * 2 * interpolate(&CURRENT_STRENGTH, abs_lat) / ONE;

    // Annual mean.
    // Tropical climates follow the thermal equator, not the geographic one.
    let tropical_lat = crate::map::hydrology::tropical_latitude(lat, tile.tropical_shift).abs();
    let zonal = interpolate(&ZONAL_MEAN, tropical_lat);
    let mean = match tile.water {
        Water::Ocean => (zonal + interpolate(&OCEAN_WARMTH, abs_lat) + current).max(-20),
        Water::Lake => zonal - lapse + current * (1000 - cont) / 2000,
        _ => {
            let desert_heat = dryness * 20 / 1000 * i64::from(abs_lat < 400);
            let cold_interior = cont * (abs_lat - 300).max(0) * 40 / (600 * 1000);
            zonal - lapse + desert_heat - cold_interior + current * (1000 - cont) / 2000
        }
    };

    // Seasonal swing.
    let zonal_swing = interpolate(&ZONAL_SWING, tropical_lat);
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
    // Smooth bell, 1000 at the equator to 0 at 30°, with no kink at the edge.
    let x = (abs_lat * 1000 / 300).min(1000);
    let bell = 1000 - x * x / 1000;
    let tropics = bell * bell / 1000;
    let lon_phase = (pos.x as i64 - center_x) * trig::FULL_TURN / i64::from(map.width());
    let east_west = trig::cos(lon_phase) * 1000 / ONE;
    let fx = (2 * pos.x as i64 + 1) * ONE * 4 / (2 * i64::from(map.width()));
    let fy = (2 * pos.y as i64 + 1) * ONE * 4 / (2 * i64::from(map.width()));
    let remote = teleconnections.fbm(fx, fy, 3) * 400 / ONE * (1000 - tropics) / 1000;
    let oscillation_weight = match tile.water {
        Water::Ocean => tropics * (400 + 600 * east_west / 1000) / 1000 + remote / 2,
        _ => tropics * (200 + 300 * east_west / 1000) / 1000 + remote,
    };

    // Seasonal rain, in the local hemisphere's sense (+ = summer peak):
    // monsoon in the outer tropics, winter rain where the westerlies come in
    // straight off the sea at 30-45° (Mediterranean climates), convective
    // summer rain inside continents.
    // Big landmasses drive strong monsoons (wet summer, dry winter: savanna);
    // maritime tropics stay wet all year (rainforest).
    let monsoon = interpolate(&MONSOON, tropical_lat)
        * interpolate(&MONSOON_LAND, i64::from(tile.land_share))
        / 1000;
    let windward = 1000 - (i64::from(tile.upwind_land) * 1000 / 8).min(1000);
    let mediterranean = -800 * windward / 1000 * interpolate(&MEDITERRANEAN, abs_lat) / 1000;
    let convective = 450 * cont / 1000 * interpolate(&CONVECTIVE, abs_lat) / 1000;
    let mut rain_seasonality = (monsoon + mediterranean + convective).clamp(-900, 900);
    if lat < 0 {
        rain_seasonality = -rain_seasonality;
    }

    let weather_amplitude = interpolate(&WEATHER, abs_lat)
        * match tile.water {
            Water::Ocean => 400,
            _ => 600 + 400 * cont / 1000,
        }
        / 1000;

    TileClimate {
        mean: mean as i16,
        seasonal_amplitude: swing as i16,
        seasonal_lag_days: lag as u16,
        diurnal_range: diurnal as i16,
        oscillation_weight: oscillation_weight as i16,
        drift_weight: (1000 + abs_lat * 1000 / 900) as i16,
        continentality,
        base_precipitation: annual_precipitation_mm(tile.moisture),
        rain_seasonality: rain_seasonality as i16,
        rain_oscillation: (oscillation_weight * 4 / 10) as i16,
        weather_amplitude: weather_amplitude as i16,
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

/// How much warmer the sea surface air is than the zonal mean, tenths °C:
/// ocean currents carry heat poleward, keeping mid- and high-latitude seas
/// ice-free until about 65°.
const OCEAN_WARMTH: [(i64, i64); 7] = [
    (0, 0),
    (300, 0),
    (450, 30),
    (600, 75),
    (700, 75),
    (800, 40),
    (900, 20),
];

/// Strength of ocean-current temperature anomalies by latitude, tenths °C
/// (scaled by noise of typically ±0.3-0.5 and doubled).
const CURRENT_STRENGTH: [(i64, i64); 6] = [
    (0, 10),
    (300, 30),
    (500, 80),
    (650, 120),
    (800, 100),
    (900, 60),
];

/// Strength of the monsoon (summer-peaked rain), per mille, by |latitude|:
/// the tropical rain belt follows the sun, so the outer tropics get a wet
/// summer and a dry winter; at the equator it passes twice a year.
const MONSOON: [(i64, i64); 4] = [(0, 0), (100, 500), (220, 650), (300, 0)];

/// Monsoon strength by the share of land around a tile (per mille).
const MONSOON_LAND: [(i64, i64); 4] = [(0, 150), (300, 400), (600, 1000), (850, 1400)];

/// Where winter (Mediterranean) rain is possible, per mille, by |latitude|.
const MEDITERRANEAN: [(i64, i64); 4] = [(250, 0), (330, 1000), (420, 1000), (480, 0)];

/// Where continental interiors get summer convective rain, per mille.
const CONVECTIVE: [(i64, i64); 5] = [(300, 0), (400, 1000), (650, 1000), (800, 500), (900, 300)];

/// Typical weather temperature anomaly by |latitude|, tenths °C.
const WEATHER: [(i64, i64); 5] = [(0, 10), (200, 15), (350, 35), (500, 50), (900, 50)];

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

/// Piecewise-linear lookup in a table sorted by `x`, clamped at the ends.
pub(crate) fn interpolate(table: &[(i64, i64)], x: i64) -> i64 {
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
            base_precipitation: 800,
            rain_seasonality: 0,
            rain_oscillation: 0,
            weather_amplitude: 0,
        }
    }

    fn temp(profile: &TileClimate, turn: u64) -> i64 {
        temperature_at(profile, &Date::from_turn(turn, 364), 364, 0, 0, 1000)
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
        let base = temperature_at(&p, &date, 364, 0, 0, 1000);
        assert_eq!(temperature_at(&p, &date, 364, 2 * ONE, 0, 1000), base + 10);
        assert_eq!(temperature_at(&p, &date, 364, 0, ONE, 1000), base + 10);
    }

    #[test]
    fn annual_average_is_the_mean() {
        let p = land(1);
        let sum: i64 = (0..364 * TURNS_PER_DAY).map(|t| temp(&p, t)).sum();
        let avg = sum / (364 * TURNS_PER_DAY) as i64;
        assert!((avg - i64::from(p.mean)).abs() <= 2, "avg {avg}");
    }
}
