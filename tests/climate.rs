use societies::climate::{Climate, ClimateConfig};
use societies::map::{Biome, Map, TerrainConfig, Water};
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
    assert!(tropics > polar + 300, "{tropics} vs {polar}");
}

#[test]
fn mountains_are_colder_than_lowlands_at_the_same_latitude() {
    let (map, climate) = world(2);
    let band = |i: usize| (200..500).contains(&abs_lat(&map, i));
    let mean = |i: usize| i64::from(climate.profile(i).mean);
    let mountains = avg(
        &map,
        |i| band(i) && map.tiles()[i].biome == Biome::Mountains,
        mean,
    );
    let lowland = avg(
        &map,
        |i| band(i) && map.tiles()[i].biome == Biome::Earth && map.tiles()[i].altitude < 300,
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
        |i| map.tiles()[i].biome == Biome::Sand && map.tiles()[i].moisture < 200,
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
