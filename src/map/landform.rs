//! Stage 3: the fixed physical character of every tile: its landform and
//! its soil. (Vegetation biomes depend on the climate and live in
//! [`crate::biome`].)
//!
//! Landform:
//! - Water: oceans and lakes.
//! - Mountains: high altitude.
//! - Hills: moderately high, or rugged (steep relative to neighbors).
//! - Plains: everything else.
//!
//! Soil:
//! - Alluvial: river tiles and low floodplains next to rivers.
//! - Rocky: mountains and very rugged ground.
//! - Sandy: low-lying beaches on drier ocean coasts, and dry land without a
//!   river (dunes, desert sands).
//! - Loam: everything else.

use super::{Landform, Map, Soil, TerrainConfig, Water};

pub fn assign(map: &mut Map, config: &TerrainConfig) {
    let classified: Vec<(Landform, Soil)> = (0..map.tiles.len())
        .map(|i| (landform(map, config, i), soil(map, config, i)))
        .collect();
    for (tile, (landform, soil)) in map.tiles.iter_mut().zip(classified) {
        tile.landform = landform;
        tile.soil = soil;
    }
}

/// Largest height difference to a non-ocean neighbor, in meters.
fn relief(map: &Map, i: usize) -> i32 {
    let tile = &map.tiles[i];
    map.neighbors(i)
        .filter(|&nb| map.tiles[nb].water != Water::Ocean)
        .map(|nb| (map.tiles[nb].altitude - tile.altitude).abs())
        .max()
        .unwrap_or(0)
}

fn landform(map: &Map, config: &TerrainConfig, i: usize) -> Landform {
    let tile = &map.tiles[i];
    if matches!(tile.water, Water::Ocean | Water::Lake) {
        Landform::Water
    } else if tile.altitude >= config.mountain_altitude {
        Landform::Mountains
    } else if tile.altitude >= config.hills_altitude || relief(map, i) >= config.hills_relief {
        Landform::Hills
    } else {
        Landform::Plains
    }
}

fn soil(map: &Map, config: &TerrainConfig, i: usize) -> Soil {
    let tile = &map.tiles[i];
    if matches!(tile.water, Water::Ocean | Water::Lake) {
        return Soil::None;
    }
    let near_river = map
        .neighbors(i)
        .any(|nb| map.tiles[nb].water == Water::River);
    if tile.water == Water::River || (near_river && tile.altitude < 500) {
        return Soil::Alluvial;
    }
    if tile.altitude >= config.mountain_altitude || relief(map, i) >= 2 * config.hills_relief {
        return Soil::Rocky;
    }
    let coastal = map
        .neighbors(i)
        .any(|nb| map.tiles[nb].water == Water::Ocean);
    let beach =
        coastal && tile.altitude <= config.beach_altitude && tile.moisture < config.beach_moisture;
    if beach || tile.moisture < config.desert_moisture {
        return Soil::Sandy;
    }
    Soil::Loam
}
