mod common;

use societies::Simulation;
use societies::biome::Biome;
use societies::climate::cryosphere::GLACIER;
use societies::config::SimConfig;
use societies::map::{Landform, Water};

fn sim(seed: u64) -> Simulation {
    Simulation::new(SimConfig {
        seed,
        map_width: 256,
        map_height: 128,
        ..SimConfig::default()
    })
}

fn abs_lat(sim: &Simulation, i: usize) -> i64 {
    sim.map()
        .latitude_tenths(sim.map().position_of(i as u32).y)
        .abs()
}

#[test]
fn water_and_ice_biomes_match_the_world() {
    let sim = sim(1);
    let (map, climate, biomes) = (sim.map(), sim.climate(), sim.biomes());
    for i in 0..map.tile_count() as usize {
        let b = biomes.biome(i);
        match map.tiles()[i].water {
            Water::Ocean => assert_eq!(b, Biome::Ocean),
            Water::Lake => assert_eq!(b, Biome::Lake),
            _ => {
                assert!(!matches!(b, Biome::Ocean | Biome::Lake));
                let glacier = climate.cryosphere().tile(i).glacier >= GLACIER;
                assert_eq!(b == Biome::IceSheet, glacier, "tile {i}");
                if b == Biome::Alpine {
                    assert_eq!(map.tiles()[i].landform, Landform::Mountains);
                }
            }
        }
    }
}

#[test]
fn a_rich_variety_of_biomes_appears() {
    for seed in [1, 3, 7] {
        let sim = sim(seed);
        let present = Biome::ALL
            .iter()
            .filter(|&&b| sim.biomes().all().contains(&b))
            .count();
        assert!(present >= 12, "seed {seed}: only {present} biome kinds");
    }
}

#[test]
fn biomes_follow_latitude() {
    let sim = sim(3);
    let mean_lat = |b: Biome| {
        let tiles: Vec<i64> = (0..sim.map().tile_count() as usize)
            .filter(|&i| sim.biomes().biome(i) == b)
            .map(|i| abs_lat(&sim, i))
            .collect();
        assert!(!tiles.is_empty(), "no {b:?}");
        tiles.iter().sum::<i64>() / tiles.len() as i64
    };
    let rainforest = mean_lat(Biome::TropicalRainforest);
    let temperate = mean_lat(Biome::TemperateForest);
    let boreal = mean_lat(Biome::BorealForest);
    let tundra = mean_lat(Biome::Tundra);
    assert!(
        rainforest < temperate && temperate < boreal && boreal < tundra,
        "{rainforest} / {temperate} / {boreal} / {tundra}"
    );
}

#[test]
fn deserts_are_dry_and_forests_are_wet() {
    let sim = sim(7);
    let mean_rain = |pred: &dyn Fn(Biome) -> bool| {
        let v: Vec<i64> = (0..sim.map().tile_count() as usize)
            .filter(|&i| pred(sim.biomes().biome(i)))
            .map(|i| sim.climate().normal(i).annual_precipitation_mm())
            .collect();
        v.iter().sum::<i64>() / v.len().max(1) as i64
    };
    let deserts = mean_rain(&|b| b.is_desert());
    let forests = mean_rain(&|b| b.is_forest());
    assert!(
        deserts * 4 < forests,
        "deserts {deserts} mm vs forests {forests} mm"
    );
}

#[test]
fn biomes_are_reclassified_every_year_and_stay_mostly_stable() {
    let mut sim = common::sim(common::Fixture::Small);
    let start = sim.biomes().all().to_vec();
    sim.run(sim.config().turns_per_year() - 1);
    assert_eq!(
        sim.biomes().all(),
        &start[..],
        "changed before the year ended"
    );
    sim.run(5 * sim.config().turns_per_year() + 1);
    let changed = start
        .iter()
        .zip(sim.biomes().all())
        .filter(|(a, b)| a != b)
        .count();
    // Climate normals move slowly: only a few borderline tiles may switch.
    assert!(
        changed * 20 < start.len(),
        "{changed} of {} tiles changed",
        start.len()
    );
}
