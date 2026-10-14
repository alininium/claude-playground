//! Winds and rain shadows, seasonal and oscillation-driven rain, weather,
//! soil water and floods, frost-free seasons and day length, and ice
//! sheets that start out settled.

mod common;

use common::Fixture;
use societies::Simulation;
use societies::biome::Biome;
use societies::climate::cryosphere::GLACIER;
use societies::config::SimConfig;
use societies::map::Water;
use societies::map::hydrology::prevailing_wind;

fn sim(seed: u64, width: u32, height: u32) -> Simulation {
    Simulation::new(SimConfig {
        seed,
        map_width: width,
        map_height: height,
        ..SimConfig::default()
    })
}

fn lat(sim: &Simulation, i: usize) -> i64 {
    sim.map().latitude_tenths(sim.map().position_of(i as u32).y)
}

fn is_land(sim: &Simulation, i: usize) -> bool {
    matches!(sim.map().tiles()[i].water, Water::Dry | Water::River)
}

fn mean(values: impl Iterator<Item = i64>) -> i64 {
    let v: Vec<i64> = values.collect();
    assert!(!v.is_empty(), "no samples");
    v.iter().sum::<i64>() / v.len() as i64
}

#[test]
fn prevailing_winds_follow_the_three_cells() {
    assert_eq!(prevailing_wind(100), -1, "trade winds blow westward");
    assert_eq!(prevailing_wind(-150), -1);
    assert_eq!(prevailing_wind(450), 1, "westerlies blow eastward");
    assert_eq!(prevailing_wind(-450), 1);
    assert_eq!(prevailing_wind(750), -1, "polar easterlies");
}

#[test]
fn windward_coasts_are_wetter_than_lee_interiors() {
    let sim = sim(3, 256, 128);
    let band = |i: usize| (300..600).contains(&lat(&sim, i).abs()) && is_land(&sim, i);
    let moisture = |i: usize| i64::from(sim.map().tiles()[i].moisture);
    let n = sim.map().tile_count() as usize;
    let windward = mean(
        (0..n)
            .filter(|&i| band(i) && sim.map().tiles()[i].upwind_land <= 2)
            .map(moisture),
    );
    let lee = mean(
        (0..n)
            .filter(|&i| band(i) && sim.map().tiles()[i].upwind_land >= 15)
            .map(moisture),
    );
    assert!(windward > lee + 100, "windward {windward} vs lee {lee}");
}

#[test]
fn monsoon_rain_falls_in_the_local_summer() {
    let mut sim = common::sim(Fixture::Medium);
    let days = u64::from(sim.config().climate.days_per_year);
    let n = sim.map().tile_count() as usize;
    let outer_tropics: Vec<usize> = (0..n)
        .filter(|&i| (120..220).contains(&lat(&sim, i).abs()) && is_land(&sim, i))
        .collect();
    // Rain relative to the long-term amount, northern minus southern tiles:
    // positive in northern summer, negative in northern winter.
    let contrast = |sim: &Simulation| {
        mean(outer_tropics.iter().map(|&i| {
            let p = sim.climate();
            let relative = i64::from(p.precipitation(i)) * 1000
                / i64::from(p.profile(i).base_precipitation.max(1));
            if lat(sim, i) > 0 { relative } else { -relative }
        }))
    };
    sim.run((days / 4 + 30) * 4 + 1); // a month after the northern solstice
    let northern_summer = contrast(&sim);
    sim.run(days / 2 * 4);
    let northern_winter = contrast(&sim);
    assert!(
        northern_summer > northern_winter + 300,
        "{northern_summer} vs {northern_winter}"
    );
}

#[test]
fn the_oscillation_brings_rain_to_its_warm_pool() {
    let mut sim = common::sim(Fixture::Small);
    let n = sim.map().tile_count() as usize;
    let core = (0..n)
        .max_by_key(|&i| sim.climate().profile(i).rain_oscillation)
        .unwrap();
    assert!(sim.climate().profile(core).rain_oscillation >= 300);
    let base = i64::from(sim.climate().profile(core).base_precipitation.max(1));
    let (mut warm, mut cold) = (Vec::new(), Vec::new());
    for _ in 0..12 * 364 {
        sim.run(4);
        let relative = i64::from(sim.climate().precipitation(core)) * 1000 / base;
        match sim.climate().oscillation().index_tenths() {
            i if i >= 7 => warm.push(relative),
            i if i <= -7 => cold.push(relative),
            _ => {}
        }
    }
    let (warm, cold) = (mean(warm.into_iter()), mean(cold.into_iter()));
    assert!(
        warm > cold + 300,
        "warm phase {warm}‰ vs cold phase {cold}‰ of normal rain"
    );
}

#[test]
fn weather_changes_from_day_to_day() {
    let mut sim = common::sim(Fixture::Small);
    let n = sim.map().tile_count() as usize;
    let midlat_land = (0..n)
        .find(|&i| (400..550).contains(&lat(&sim, i).abs()) && is_land(&sim, i))
        .expect("mid-latitude land");
    let mut anomalies = Vec::new();
    for _ in 0..120 {
        sim.run(4);
        anomalies.push(i64::from(sim.climate().weather().temperature(midlat_land)));
    }
    let avg = mean(anomalies.iter().copied());
    let spread = mean(anomalies.iter().map(|a| (a - avg).abs()));
    assert!(spread >= 10, "weather barely varies: {spread} tenths");
    assert!(
        anomalies.windows(2).all(|w| (w[0] - w[1]).abs() <= 80),
        "weather jumps too fast"
    );
}

#[test]
fn rivers_grow_downstream_and_flood_sometimes() {
    let mut sim = common::sim(Fixture::Medium);
    sim.run(sim.config().turns_per_year() + 1);
    let map = sim.map();
    let soil = sim.climate().soil();
    let n = map.tile_count() as usize;
    for i in (0..n).filter(|&i| map.tiles()[i].water == Water::River) {
        if let Some(down) = map.tiles()[i].downstream {
            let down = down as usize;
            if map.tiles()[down].water != Water::Ocean {
                assert!(soil.discharge_m3s(down, 156) >= soil.discharge_m3s(i, 156));
            }
        }
    }
    let flooded = (0..n).filter(|&i| sim.climate().flood_days(i) > 0).count();
    assert!(flooded > 0, "no river flooded in a whole year");
}

#[test]
fn deserts_have_dry_soil_and_rainforests_wet_soil() {
    let sim = sim(3, 256, 128);
    let n = sim.map().tile_count() as usize;
    let wetness = |b: Biome| {
        mean(
            (0..n)
                .filter(|&i| sim.biomes().biome(i) == b && sim.map().tiles()[i].water == Water::Dry)
                .map(|i| i64::from(sim.climate().growing_season_wetness(i))),
        )
    };
    let (desert, rainforest) = (
        wetness(Biome::HotDesert),
        wetness(Biome::TropicalRainforest),
    );
    assert!(
        desert * 2 < rainforest,
        "desert {desert}‰ vs rainforest {rainforest}‰"
    );
}

#[test]
fn frost_free_seasons_shrink_toward_the_poles() {
    let sim = sim(1, 256, 128);
    let n = sim.map().tile_count() as usize;
    let days = |b: Biome| {
        mean(
            (0..n)
                .filter(|&i| sim.biomes().biome(i) == b)
                .map(|i| i64::from(sim.climate().frost_free_days(i))),
        )
    };
    let (tropics, temperate, tundra) = (
        days(Biome::TropicalRainforest),
        days(Biome::TemperateForest),
        days(Biome::Tundra),
    );
    assert!(tropics >= 360, "tropics {tropics}");
    assert!(
        tropics > temperate && temperate > tundra,
        "{tropics} / {temperate} / {tundra}"
    );
    assert!(tundra < 120, "tundra {tundra}");
}

#[test]
fn midnight_sun_and_polar_night() {
    let mut sim = common::sim(Fixture::Small);
    let days = u64::from(sim.config().climate.days_per_year);
    sim.run(days / 4 * 4); // northern summer solstice
    assert_eq!(sim.climate().day_length_minutes(0), 1440);
    let last_row = sim.map().height() - 1;
    assert_eq!(sim.climate().day_length_minutes(last_row), 0);
}

#[test]
fn ice_sheets_start_settled() {
    // Without the oscillation, drift and eruptions, the climate only has
    // weather left: the ice should hold steady from the first turn.
    let mut sim = common::sim_with(Fixture::Medium, |c| {
        c.initial_cities = 0;
        c.climate.oscillation.amplitude_tenths = 0;
        c.climate.oscillation.drift_amplitude_tenths = 0;
        c.volcanoes.eruptions_per_millennium = [0; 5];
    });
    let measure = |sim: &Simulation| {
        let c = sim.climate().cryosphere();
        let n = sim.map().tile_count() as usize;
        let tiles = (0..n)
            .filter(|&i| is_land(sim, i) && c.tile(i).glacier >= GLACIER)
            .count() as i64;
        let volume: i64 = (0..n).map(|i| i64::from(c.tile(i).glacier)).sum();
        (tiles, volume)
    };
    let (tiles, volume) = measure(&sim);
    assert!(tiles > 50, "only {tiles} glaciated tiles");
    let turns = sim.config().turns_per_year() * 10;
    sim.run(turns);
    let (tiles_after, volume_after) = measure(&sim);
    assert!(
        (tiles_after - tiles).abs() * 100 <= tiles * 3,
        "glaciated tiles {tiles} -> {tiles_after} in 10 years"
    );
    assert!(
        (volume_after - volume).abs() * 100 <= volume * 2,
        "ice volume {volume} -> {volume_after} in 10 years"
    );
}
