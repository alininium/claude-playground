//! Land fertility: how well a tile can feed people, 0 (barren) to 1000.
//!
//! `fertility = soil × water × warmth`, each part 0-1000:
//!
//! - **Soil** (fixed at world creation): plains have deep soils, hills
//!   thinner ones, deserts, beaches and mountains poor ones. Rivers deposit
//!   rich alluvium on their floodplains, and lake shores benefit too. Steep
//!   ground loses soil. Plus regional variation (loess, volcanic ash, ...).
//! - **Water**: rainfall, or a river or lake to irrigate from. Too much rain
//!   leaches the soil a little.
//! - **Warmth**: last year's growing degree-days (warmth above 5 °C), so a
//!   cold year really does mean a poor harvest next year.
//!
//! Permafrost cuts fertility to tundra levels; glaciers, open water and
//! frozen-over land are barren. Recomputed at the start of every year.

use bevy_ecs::prelude::{Res, ResMut, Resource};

use crate::climate::cryosphere::GLACIER;
use crate::climate::{self, Climate, interpolate};
use crate::map::noise::{Noise, ONE};
use crate::map::{Biome, Map, Water};
use crate::rng::SimRng;
use crate::time::Turn;

#[derive(Resource, Debug, Clone)]
pub struct Fertility {
    soil: Vec<u16>,
    fertility: Vec<u16>,
}

impl Fertility {
    pub fn new(map: &Map, climate: &Climate, rng: &mut SimRng) -> Self {
        let variation = Noise::new(rng.next_u64());
        let soil = (0..map.tile_count() as usize)
            .map(|i| soil_quality(map, i, &variation))
            .collect();
        let mut fertility = Self {
            soil,
            fertility: vec![0; map.tile_count() as usize],
        };
        fertility.recompute(map, climate);
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

    pub fn recompute(&mut self, map: &Map, climate: &Climate) {
        for (i, f) in self.fertility.iter_mut().enumerate() {
            *f = fertility(map, climate, i, self.soil[i]);
        }
    }
}

/// Recomputes fertility at the start of each year, from the year that ended.
pub fn update_fertility(
    turn: Res<Turn>,
    map: Res<Map>,
    climate: Res<Climate>,
    mut fertility: ResMut<Fertility>,
) {
    if climate::is_new_year(turn.0, climate.days_per_year()) {
        fertility.recompute(&map, &climate);
    }
}

fn soil_quality(map: &Map, i: usize, variation: &Noise) -> u16 {
    let tile = &map.tiles()[i];
    if !matches!(tile.water, Water::Dry | Water::River) {
        return 0;
    }
    let mut soil: i64 = match tile.biome {
        Biome::Earth => 700,
        Biome::Hills => 450,
        Biome::Sand => 150,
        Biome::Mountains => 80,
        Biome::Water => 0,
    };
    let near = |w: Water| map.neighbors(i).any(|nb| map.tiles()[nb].water == w);
    if tile.water == Water::River {
        soil += 250;
    } else if near(Water::River) || near(Water::Lake) {
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

fn fertility(map: &Map, climate: &Climate, i: usize, soil: u16) -> u16 {
    let tile = &map.tiles()[i];
    let frozen = climate.cryosphere().tile(i);
    if soil == 0 || frozen.glacier >= GLACIER {
        return 0;
    }
    let fresh_water = tile.water == Water::River
        || map
            .neighbors(i)
            .any(|nb| matches!(map.tiles()[nb].water, Water::River | Water::Lake));
    let rain = i64::from(tile.moisture);
    let effective = if fresh_water { rain.max(650) } else { rain };
    let water = interpolate(&WATER, effective);
    let warmth = interpolate(&WARMTH, i64::from(climate.growing_degree_days(i)));
    let mut f = i64::from(soil) * water / 1000 * warmth / 1000;
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
