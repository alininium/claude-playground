//! Land fertility: how well a tile can feed people, 0 (barren) to 1000.
//!
//! `fertility = soil × water × warmth × vegetation`, each part 0-1000:
//!
//! - **Soil** (fixed at world creation): plains have deep soils, hills
//!   thinner ones, mountains poor ones. Alluvial floodplain soil is richest,
//!   sandy and rocky soils are poor, lake shores get a bonus. Steep ground
//!   loses soil. Plus regional variation (loess, volcanic ash, ...).
//! - **Water**: long-term rainfall, or a river or lake to irrigate from. Too
//!   much rain leaches the soil a little.
//! - **Warmth**: last year's growing degree-days (warmth above 5 °C), so a
//!   cold year really does mean a poor harvest next year.
//!
//! - **Vegetation**: the biome's typical soil: grassland chernozems are the
//!   best, leached rainforest and boreal podzol soils poorer, wetlands need
//!   draining.
//!
//! Permafrost cuts fertility to tundra levels; glaciers, open water and
//! frozen-over land are barren. Recomputed at the start of every year.

use bevy_ecs::prelude::{Res, ResMut, Resource};

use crate::biome::{Biome, Biomes};
use crate::climate::cryosphere::GLACIER;
use crate::climate::{self, Climate, interpolate};
use crate::map::noise::{Noise, ONE};
use crate::map::{Landform, Map, Soil, Water};
use crate::rng::SimRng;
use crate::time::Turn;

#[derive(Resource, Debug, Clone)]
pub struct Fertility {
    soil: Vec<u16>,
    fertility: Vec<u16>,
}

impl Fertility {
    pub fn new(map: &Map, climate: &Climate, biomes: &Biomes, rng: &mut SimRng) -> Self {
        let variation = Noise::new(rng.next_u64());
        let soil = (0..map.tile_count() as usize)
            .map(|i| soil_quality(map, i, &variation))
            .collect();
        let mut fertility = Self {
            soil,
            fertility: vec![0; map.tile_count() as usize],
        };
        fertility.recompute(map, climate, biomes);
        fertility
    }

    /// Fertility of a tile, 0-1000.
    pub fn fertility(&self, index: usize) -> u16 {
        self.fertility[index]
    }

    pub fn values(&self) -> &[u16] {
        &self.fertility
    }

    /// The fixed soil-quality part, 0-1000.
    pub fn soil(&self, index: usize) -> u16 {
        self.soil[index]
    }

    pub fn recompute(&mut self, map: &Map, climate: &Climate, biomes: &Biomes) {
        for (i, f) in self.fertility.iter_mut().enumerate() {
            *f = fertility(map, climate, biomes.biome(i), i, self.soil[i]);
        }
    }
}

/// Recomputes fertility at the start of each year, from the year that ended.
pub fn update_fertility(
    turn: Res<Turn>,
    map: Res<Map>,
    climate: Res<Climate>,
    biomes: Res<Biomes>,
    mut fertility: ResMut<Fertility>,
) {
    if climate::is_new_year(turn.0, climate.days_per_year()) {
        fertility.recompute(&map, &climate, &biomes);
    }
}

fn soil_quality(map: &Map, i: usize, variation: &Noise) -> u16 {
    let tile = &map.tiles()[i];
    if !matches!(tile.water, Water::Dry | Water::River) {
        return 0;
    }
    let mut soil: i64 = match tile.landform {
        Landform::Plains => 700,
        Landform::Hills => 450,
        Landform::Mountains => 80,
        Landform::Water => 0,
    };
    soil += match tile.soil {
        Soil::Alluvial => 250,
        Soil::Sandy => -450,
        Soil::Rocky => -200,
        Soil::Loam | Soil::None => 0,
    };
    let near_lake = map
        .neighbors(i)
        .any(|nb| map.tiles()[nb].water == Water::Lake);
    if near_lake && tile.soil != Soil::Alluvial {
        soil += 120;
    }
    let relief = map
        .neighbors(i)
        .filter(|&nb| map.tiles()[nb].water != Water::Ocean)
        .map(|nb| i64::from((map.tiles()[nb].altitude - tile.altitude).abs()))
        .max()
        .unwrap_or(0);
    soil -= relief / 5;
    let pos = map.position_of(i as u32);
    let (x, y) = (i64::from(pos.x), i64::from(pos.y));
    soil += variation.fbm((2 * x + 1) * ONE / 16, (2 * y + 1) * ONE / 16, 3) * 150 / ONE;
    soil.clamp(0, 1000) as u16
}

fn fertility(map: &Map, climate: &Climate, biome: Biome, i: usize, soil: u16) -> u16 {
    let tile = &map.tiles()[i];
    let frozen = climate.cryosphere().tile(i);
    if soil == 0 || frozen.glacier >= GLACIER {
        return 0;
    }
    let fresh_water = tile.water == Water::River
        || map
            .neighbors(i)
            .any(|nb| matches!(map.tiles()[nb].water, Water::River | Water::Lake));
    // Moisture scale: 1000 ≈ 2000 mm of rain a year.
    let rain = climate.normal(i).annual_precipitation_mm() / 2;
    let effective = if fresh_water { rain.max(650) } else { rain };
    let water = interpolate(&WATER, effective);
    let warmth = interpolate(&WARMTH, i64::from(climate.growing_degree_days(i)));
    let mut f = i64::from(soil) * water / 1000 * warmth / 1000 * vegetation(biome) / 1000;
    if frozen.permafrost {
        f = f * 300 / 1000;
    }
    f.clamp(0, 1000) as u16
}

/// Water factor by effective moisture (0-1000).
const WATER: [(i64, i64); 7] = [
    (0, 0),
    (100, 80),
    (250, 350),
    (450, 750),
    (650, 1000),
    (900, 1000),
    (1000, 900),
];

/// Warmth factor by yearly growing degree-days (°C·days above 5 °C).
const WARMTH: [(i64, i64); 7] = [
    (0, 0),
    (300, 100),
    (1000, 500),
    (2000, 900),
    (2800, 1000),
    (5000, 1000),
    (7000, 850),
];

/// Typical soil quality under each vegetation biome, per mille, relative to
/// the best (grassland chernozems).
fn vegetation(biome: Biome) -> i64 {
    match biome {
        Biome::Grassland => 1000,
        Biome::TemperateForest | Biome::HotDesert | Biome::ColdDesert => 900,
        Biome::Savanna | Biome::TropicalSeasonalForest | Biome::MediterraneanShrubland => 820,
        Biome::TemperateRainforest => 770,
        Biome::TropicalRainforest => 680,
        Biome::BorealForest => 640,
        Biome::Wetland => 550,
        Biome::Tundra | Biome::Alpine => 360,
        Biome::IceSheet | Biome::Ocean | Biome::Lake => 0,
    }
}
