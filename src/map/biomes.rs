//! Stage 3: classify tiles into the five biomes.
//!
//! - Water: oceans and lakes.
//! - Mountains: high altitude.
//! - Hills: moderately high, or rugged (steep relative to neighbors).
//! - Sand: low-lying beaches on drier ocean coasts, and deserts (dry land with
//!   no river running through it).
//! - Earth: everything else.

use super::{Biome, Map, TerrainConfig, Water};

pub fn assign(map: &mut Map, config: &TerrainConfig) {
    let biomes: Vec<Biome> = (0..map.tiles.len())
        .map(|i| classify(map, config, i))
        .collect();
    for (tile, biome) in map.tiles.iter_mut().zip(biomes) {
        tile.biome = biome;
    }
}

fn classify(map: &Map, config: &TerrainConfig, i: usize) -> Biome {
    let tile = &map.tiles[i];
    if matches!(tile.water, Water::Ocean | Water::Lake) {
        return Biome::Water;
    }
    if tile.altitude >= config.mountain_altitude {
        return Biome::Mountains;
    }
    let relief = map
        .neighbors(i)
        .filter(|&nb| map.tiles[nb].water != Water::Ocean)
        .map(|nb| (map.tiles[nb].altitude - tile.altitude).abs())
        .max()
        .unwrap_or(0);
    if tile.altitude >= config.hills_altitude || relief >= config.hills_relief {
        return Biome::Hills;
    }
    let coastal = map
        .neighbors(i)
        .any(|nb| map.tiles[nb].water == Water::Ocean);
    if coastal && tile.altitude <= config.beach_altitude && tile.moisture < config.beach_moisture {
        return Biome::Sand;
    }
    if tile.moisture < config.desert_moisture && tile.water != Water::River {
        return Biome::Sand;
    }
    Biome::Earth
}
