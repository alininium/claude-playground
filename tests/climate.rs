mod common;

use societies::climate::{Climate, ClimateConfig};
use societies::map::{Landform, Map, Soil, TerrainConfig, Water};
use societies::rng::SimRng;

fn world(seed: u64) -> (Map, Climate) {
    let mut rng = SimRng::from_seed(seed);
    let map = Map::generate(256, 128, &TerrainConfig::default(), &mut rng);
    let climate = Climate::new(&map, &ClimateConfig::default(), &mut rng);
    (map, climate)
}

/// Average of `f` over tiles matching `filter`.
fn avg(map: &Map, filter: impl Fn(usize) -> bool, f: impl Fn(usize) -> i64) -> i64 {
    let tiles: Vec<usize> = (0..map.tile_count() as usize)
        .filter(|&i| filter(i))
        .collect();
    assert!(!tiles.is_empty(), "no tiles match");
    tiles.iter().map(|&i| f(i)).sum::<i64>() / tiles.len() as i64
}

fn abs_lat(map: &Map, i: usize) -> i64 {
    map.latitude_tenths(map.position_of(i as u32).y).abs()
}

#[test]
fn equator_is_warmer_than_poles() {
    let (map, climate) = world(1);
    let mean = |i: usize| i64::from(climate.profile(i).mean);
    let tropics = avg(&map, |i| abs_lat(&map, i) < 150, mean);
    let polar = avg(&map, |i| abs_lat(&map, i) > 700, mean);
    // Polar seas bottom out near −2 °C (seawater freezes at −1.8 °C), so an
    // all-ocean polar region is "only" ~28 °C colder than the tropics.
    assert!(tropics > polar + 250, "{tropics} vs {polar}");
}

#[test]
fn mountains_are_colder_than_lowlands_at_the_same_latitude() {
    let (map, climate) = world(2);
    let band = |i: usize| (200..500).contains(&abs_lat(&map, i));
    let mean = |i: usize| i64::from(climate.profile(i).mean);
    let mountains = avg(
        &map,
        |i| band(i) && map.tiles()[i].landform == Landform::Mountains,
        mean,
    );
    let lowland = avg(
        &map,
        |i| band(i) && map.tiles()[i].landform == Landform::Plains && map.tiles()[i].altitude < 300,
        mean,
    );
    assert!(mountains < lowland - 50, "{mountains} vs {lowland}");
}

#[test]
fn interiors_have_harsher_seasons_than_coasts_and_seas() {
    let (map, climate) = world(3);
    let band = |i: usize| (400..650).contains(&abs_lat(&map, i));
    let swing = |i: usize| i64::from(climate.profile(i).seasonal_amplitude).abs();
    let cont = |i: usize| climate.profile(i).continentality;
    let land = |i: usize| matches!(map.tiles()[i].water, Water::Dry | Water::River);
    let interior = avg(&map, |i| band(i) && land(i) && cont(i) > 700, swing);
    let coast = avg(&map, |i| band(i) && land(i) && cont(i) < 150, swing);
    let sea = avg(
        &map,
        |i| band(i) && map.tiles()[i].water == Water::Ocean,
        swing,
    );
    assert!(
        interior > coast && coast > sea,
        "{interior} / {coast} / {sea}"
    );
}

#[test]
fn deserts_swing_more_between_day_and_night_than_the_sea() {
    let (map, climate) = world(1);
    let range = |i: usize| i64::from(climate.profile(i).diurnal_range);
    let desert = avg(
        &map,
        |i| map.tiles()[i].soil == Soil::Sandy && map.tiles()[i].moisture < 200,
        range,
    );
    let sea = avg(&map, |i| map.tiles()[i].water == Water::Ocean, range);
    assert!(desert > sea * 5, "{desert} vs {sea}");
}

#[test]
fn oscillation_is_felt_most_in_the_tropical_ocean() {
    let (map, climate) = world(4);
    let weight = |i: usize| i64::from(climate.profile(i).oscillation_weight).abs();
    let tropical_sea = avg(
        &map,
        |i| abs_lat(&map, i) < 100 && map.tiles()[i].water == Water::Ocean,
        weight,
    );
    let polar = avg(&map, |i| abs_lat(&map, i) > 600, weight);
    assert!(tropical_sea > polar * 2, "{tropical_sea} vs {polar}");
}

mod frozen {
    use societies::Simulation;
    use societies::climate::cryosphere::{Cover, GLACIER};
    use societies::config::SimConfig;
    use societies::map::Water;

    fn sim(seed: u64) -> Simulation {
        Simulation::new(SimConfig {
            seed,
            map_width: 128,
            map_height: 64,
            ..SimConfig::default()
        })
    }

    fn abs_lat(sim: &Simulation, i: usize) -> i64 {
        let map = sim.map();
        map.latitude_tenths(map.position_of(i as u32).y).abs()
    }

    #[test]
    fn polar_caps_exist_at_turn_zero_and_never_in_the_tropics() {
        for seed in [1, 2, 3] {
            let sim = sim(seed);
            let cryo = sim.climate().cryosphere();
            let n = sim.map().tile_count() as usize;
            let polar_ice = (0..n)
                .filter(|&i| abs_lat(&sim, i) > 750)
                .filter(|&i| matches!(cryo.cover(sim.map(), i), Cover::SeaIce | Cover::Glacier))
                .count();
            assert!(polar_ice > 0, "seed {seed}: no polar cap");
            let tropical_ice = (0..n)
                .filter(|&i| abs_lat(&sim, i) < 200 && sim.map().tiles()[i].altitude < 2000)
                .filter(|&i| cryo.tile(i).glacier >= GLACIER || cryo.tile(i).ice > 0)
                .count();
            assert_eq!(tropical_ice, 0, "seed {seed}: ice in the lowland tropics");
        }
    }

    #[test]
    fn northern_sea_ice_grows_in_winter_and_shrinks_in_summer() {
        let mut sim = super::common::sim(super::common::Fixture::Medium);
        let days = u64::from(sim.config().climate.days_per_year);
        let northern_sea_ice = |sim: &Simulation| {
            (0..sim.map().tile_count() as usize)
                .filter(|&i| {
                    let lat = sim.map().latitude_tenths(sim.map().position_of(i as u32).y);
                    lat > 0 && sim.climate().cryosphere().cover(sim.map(), i) == Cover::SeaIce
                })
                .count()
        };
        // Late northern summer (minimum) vs late northern winter (maximum).
        sim.run((days / 4 + days / 6) * 4);
        let summer = northern_sea_ice(&sim);
        sim.run(days / 2 * 4);
        let winter = northern_sea_ice(&sim);
        assert!(winter > summer, "winter {winter} vs summer {summer}");
    }

    #[test]
    fn lakes_and_rivers_freeze_only_in_cold_places() {
        let mut sim = super::common::sim(super::common::Fixture::Medium);
        let days = u64::from(sim.config().climate.days_per_year);
        sim.run((days * 3 / 4 + 30) * 4); // a month into northern winter
        let map = sim.map();
        let climate = sim.climate();
        for i in 0..map.tile_count() as usize {
            let cover = climate.cryosphere().cover(map, i);
            if matches!(cover, Cover::LakeIce | Cover::RiverIce) {
                assert!(
                    climate.profile(i).mean < 150,
                    "ice on fresh water with an annual mean of {}",
                    climate.profile(i).mean
                );
                assert!(matches!(map.tiles()[i].water, Water::Lake | Water::River));
            }
        }
    }

    #[test]
    fn permafrost_lies_between_glaciers_and_temperate_land() {
        let sim = sim(3);
        let (map, climate) = (sim.map(), sim.climate());
        let land = |i: usize| matches!(map.tiles()[i].water, Water::Dry | Water::River);
        let mean_of = |filter: &dyn Fn(usize) -> bool| {
            let v: Vec<i64> = (0..map.tile_count() as usize)
                .filter(|&i| land(i) && filter(i))
                .map(|i| i64::from(climate.profile(i).mean))
                .collect();
            v.iter().sum::<i64>() / v.len().max(1) as i64
        };
        let cryo = climate.cryosphere();
        let glacier = mean_of(&|i| cryo.tile(i).glacier >= GLACIER);
        let permafrost = mean_of(&|i| cryo.tile(i).permafrost && cryo.tile(i).glacier < GLACIER);
        let thawing = mean_of(&|i| !cryo.tile(i).permafrost);
        assert!(
            glacier < permafrost && permafrost < thawing,
            "{glacier} / {permafrost} / {thawing}"
        );
    }

    #[test]
    fn fertility_follows_soil_water_and_warmth() {
        let sim = sim(2);
        let (map, climate, fertility) = (sim.map(), sim.climate(), sim.fertility());
        for i in 0..map.tile_count() as usize {
            let tile = &map.tiles()[i];
            let f = fertility.fertility(i);
            if matches!(tile.water, Water::Ocean | Water::Lake)
                || climate.cryosphere().tile(i).glacier >= GLACIER
            {
                assert_eq!(f, 0);
            }
            assert!(f <= fertility.soil(i), "fertility above soil quality");
            if climate.growing_degree_days(i) == 0 {
                assert_eq!(f, 0, "fertile without any growing season");
            }
        }
        // River floodplains beat the land average.
        let avg = |filter: &dyn Fn(usize) -> bool| {
            let v: Vec<u64> = (0..map.tile_count() as usize)
                .filter(|&i| filter(i))
                .map(|i| u64::from(fertility.fertility(i)))
                .collect();
            v.iter().sum::<u64>() / v.len().max(1) as u64
        };
        let rivers = avg(&|i| map.tiles()[i].water == Water::River);
        let dry = avg(&|i| map.tiles()[i].water == Water::Dry);
        assert!(rivers > dry, "rivers {rivers} vs dry land {dry}");
    }
}
