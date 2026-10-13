//! The world map: a grid of tiles, each with an altitude, a biome and water.
//!
//! Generation runs in three stages, each in its own module:
//! 1. [`terrain`]: continents, ocean floor and mountain ranges (altitudes).
//! 2. [`hydrology`]: oceans, lakes, rainfall and rivers.
//! 3. [`biomes`]: classify every tile as one of the five [`Biome`]s.

pub mod biomes;
pub mod hydrology;
pub mod noise;
pub mod terrain;

use bevy_ecs::prelude::{Component, Resource};

use crate::rng::SimRng;

#[derive(Debug, Clone, Copy, PartialEq, Eq, Hash, PartialOrd, Ord)]
pub enum Biome {
    Water,
    Earth,
    Sand,
    Hills,
    Mountains,
}

#[derive(Debug, Clone, Copy, PartialEq, Eq, Hash, PartialOrd, Ord)]
pub enum Water {
    Dry,
    River,
    Lake,
    Ocean,
}

#[derive(Debug, Clone, Copy, PartialEq, Eq, Hash)]
pub struct Tile {
    /// Meters relative to sea level. For oceans and lakes this is the bed.
    pub altitude: i32,
    pub biome: Biome,
    pub water: Water,
    /// Yearly rainfall, 0 (bone dry) to 1000 (rainforest).
    pub moisture: u16,
    /// Rain collected from this tile and everything upstream of it.
    pub flow: u32,
    /// Tile index this tile drains into; `None` for oceans and map-edge outlets.
    pub downstream: Option<u32>,
}

/// Tuning knobs for world generation.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct TerrainConfig {
    /// Share of the map that is land, in 1/1000 (Earth is about 290).
    pub land_per_mille: u32,
    /// Roughly how many continent-sized features fit across the map width.
    pub continent_frequency: u32,
    /// Highest peaks, in meters.
    pub max_altitude: i32,
    /// Deepest ocean, in meters (positive number).
    pub max_depth: i32,
    /// A depression needs at least this many tiles to hold a lake; smaller
    /// ones are filled in.
    pub min_lake_tiles: u32,
    /// Water must stand at least this deep (meters) to count as lake; the
    /// shallower rims of basins are filled in to flat land.
    pub min_lake_depth: i32,
    /// Upstream area (in tiles of full rainfall) needed to form a river, on a
    /// 256x128 map; scaled with map area.
    pub river_catchment: u32,
    /// Altitude from which land counts as hills.
    pub hills_altitude: i32,
    /// Local relief (max height difference to a neighbor) that makes hills.
    pub hills_relief: i32,
    /// Altitude from which land counts as mountains.
    pub mountain_altitude: i32,
    /// Coastal land up to this altitude is beach (sand).
    pub beach_altitude: i32,
    /// Coasts drier than this get beaches; wetter ones stay earth.
    pub beach_moisture: u16,
    /// Land drier than this (and without a river) is desert (sand).
    pub desert_moisture: u16,
}

impl Default for TerrainConfig {
    fn default() -> Self {
        Self {
            land_per_mille: 330,
            continent_frequency: 3,
            max_altitude: 6000,
            max_depth: 5500,
            min_lake_tiles: 3,
            min_lake_depth: 100,
            river_catchment: 25,
            hills_altitude: 800,
            hills_relief: 450,
            mountain_altitude: 1700,
            beach_altitude: 15,
            beach_moisture: 500,
            desert_moisture: 220,
        }
    }
}

#[derive(Component, Debug, Clone, Copy, PartialEq, Eq, Hash, PartialOrd, Ord)]
pub struct Position {
    pub x: u32,
    pub y: u32,
}

#[derive(Resource, Debug, Clone)]
pub struct Map {
    width: u32,
    height: u32,
    tiles: Vec<Tile>,
}

impl Map {
    /// Generates a new world. All randomness is drawn from `rng`.
    pub fn generate(width: u32, height: u32, config: &TerrainConfig, rng: &mut SimRng) -> Self {
        assert!(width >= 2 && height >= 2, "map must be at least 2x2");
        let altitudes = terrain::generate(width, height, config, rng);
        let mut map = Self {
            width,
            height,
            tiles: altitudes
                .into_iter()
                .map(|altitude| Tile {
                    altitude,
                    biome: Biome::Water,
                    water: Water::Dry,
                    moisture: 0,
                    flow: 0,
                    downstream: None,
                })
                .collect(),
        };
        hydrology::generate(&mut map, config, rng);
        biomes::assign(&mut map, config);
        map
    }

    pub fn width(&self) -> u32 {
        self.width
    }

    pub fn height(&self) -> u32 {
        self.height
    }

    pub fn tile_count(&self) -> u32 {
        self.width * self.height
    }

    pub fn tiles(&self) -> &[Tile] {
        &self.tiles
    }

    pub fn tile(&self, pos: Position) -> &Tile {
        &self.tiles[self.index(pos)]
    }

    /// Maps a linear tile index (row-major) back to a position.
    pub fn position_of(&self, index: u32) -> Position {
        Position {
            x: index % self.width,
            y: index / self.width,
        }
    }

    pub fn index(&self, pos: Position) -> usize {
        assert!(
            pos.x < self.width && pos.y < self.height,
            "position out of bounds"
        );
        (pos.y * self.width + pos.x) as usize
    }

    /// The up-to-8 neighbors of a tile, always in the same order.
    pub fn neighbors(&self, index: usize) -> impl Iterator<Item = usize> + use<> {
        let (w, h) = (self.width as i64, self.height as i64);
        let (x, y) = (index as i64 % w, index as i64 / w);
        const OFFSETS: [(i64, i64); 8] = [
            (0, -1),
            (1, 0),
            (0, 1),
            (-1, 0),
            (1, -1),
            (1, 1),
            (-1, 1),
            (-1, -1),
        ];
        OFFSETS.into_iter().filter_map(move |(dx, dy)| {
            let (nx, ny) = (x + dx, y + dy);
            (nx >= 0 && ny >= 0 && nx < w && ny < h).then_some((ny * w + nx) as usize)
        })
    }

    pub fn is_edge(&self, index: usize) -> bool {
        let pos = self.position_of(index as u32);
        pos.x == 0 || pos.y == 0 || pos.x == self.width - 1 || pos.y == self.height - 1
    }

    /// Cities can be founded on dry, non-mountain land.
    pub fn is_habitable(&self, index: usize) -> bool {
        let tile = &self.tiles[index];
        matches!(tile.biome, Biome::Earth | Biome::Sand | Biome::Hills)
            && matches!(tile.water, Water::Dry | Water::River)
    }
}
