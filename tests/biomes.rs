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

#[test]
fn rainforest_edges_are_not_straight_lines() {
    for seed in [1, 3, 7] {
        let sim = sim(seed);
        let (map, biomes) = (sim.map(), sim.biomes());
        let (w, h) = (map.width(), map.height());
        // Northernmost rainforest row in each column that has rainforest.
        let edges: Vec<i64> = (0..w)
            .filter_map(|x| {
                (0..h)
                    .find(|&y| biomes.biome((y * w + x) as usize) == Biome::TropicalRainforest)
                    .map(i64::from)
            })
            .collect();
        assert!(edges.len() > 20, "seed {seed}: too little rainforest");
        let mean = edges.iter().sum::<i64>() / edges.len() as i64;
        let spread = edges.iter().map(|e| (e - mean).abs()).sum::<i64>() / edges.len() as i64;
        // At least ~2.5° of average wobble (128 rows over 180°: 1.4° a row).
        assert!(
            spread >= 2,
            "seed {seed}: rainforest edge too straight (±{spread} rows)"
        );
    }
}

#[test]
fn maritime_tropics_are_rainforest_and_continental_ones_savanna() {
    let sim = sim(3);
    let n = sim.map().tile_count() as usize;
    let land_share = |b: Biome| {
        let v: Vec<i64> = (0..n)
            .filter(|&i| sim.biomes().biome(i) == b)
            .map(|i| i64::from(sim.map().tiles()[i].land_share))
            .collect();
        v.iter().sum::<i64>() / v.len().max(1) as i64
    };
    let (rainforest, savanna) = (
        land_share(Biome::TropicalRainforest),
        land_share(Biome::Savanna),
    );
    assert!(
        rainforest < savanna,
        "rainforest in {rainforest}‰ land, savanna in {savanna}‰"
    );
}

#[test]
fn the_thermal_equator_leans_toward_the_land() {
    let sim = sim(7);
    let map = sim.map();
    let (w, h) = (map.width(), map.height());
    // Columns with clearly more tropical land in the north than the south
    // should, on average, have their thermal equator shifted north.
    let (mut lean_north, mut lean_south) = (Vec::new(), Vec::new());
    for x in 0..w {
        let (mut north, mut south) = (0, 0);
        for y in 0..h {
            let i = (y * w + x) as usize;
            let lat = map.latitude_tenths(y);
            if lat.abs() <= 300 && map.tiles()[i].water != Water::Ocean {
                if lat >= 0 { north += 1 } else { south += 1 }
            }
        }
        let shift = i64::from(map.tiles()[(h / 2 * w + x) as usize].tropical_shift);
        if north > south + 5 {
            lean_north.push(shift);
        } else if south > north + 5 {
            lean_south.push(shift);
        }
    }
    let avg = |v: &[i64]| v.iter().sum::<i64>() / v.len().max(1) as i64;
    assert!(!lean_north.is_empty() && !lean_south.is_empty());
    assert!(
        avg(&lean_north) > avg(&lean_south),
        "{} vs {}",
        avg(&lean_north),
        avg(&lean_south)
    );
}

#[test]
fn subtropical_west_coasts_are_dry() {
    let sim = sim(1);
    let map = sim.map();
    let w = map.width() as usize;
    let (mut west, mut east) = (Vec::new(), Vec::new());
    for i in 0..map.tile_count() as usize {
        let lat = map.latitude_tenths(map.position_of(i as u32).y).abs();
        if !(160..=280).contains(&lat) || map.tiles()[i].water == Water::Ocean {
            continue;
        }
        let x = i % w;
        let moisture = i64::from(map.tiles()[i].moisture);
        if x > 0 && map.tiles()[i - 1].water == Water::Ocean {
            west.push(moisture);
        } else if x + 1 < w && map.tiles()[i + 1].water == Water::Ocean {
            east.push(moisture);
        }
    }
    let avg = |v: &[i64]| v.iter().sum::<i64>() / v.len().max(1) as i64;
    assert!(
        avg(&west) < avg(&east),
        "west coasts {} vs east coasts {}",
        avg(&west),
        avg(&east)
    );
}
