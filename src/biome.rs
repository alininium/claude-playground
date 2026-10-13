//! Vegetation biomes, classified every year from each tile's long-term
//! climate (its [`ClimateNormal`]), its ice, and its landform.
//!
//! The rules follow the Köppen-Geiger climate classification as formulated
//! by Peel, Finlayson & McMahon (2007), mapped to the vegetation that
//! typically grows in each climate:
//!
//! | Köppen | Rule (T in °C, P in mm) | Biome |
//! |---|---|---|
//! | EF / glacier | glacier ice on the tile | ice sheet |
//! | ET | warmest month < 10 | tundra (alpine on mountains) |
//! | BW | P < 10·P<sub>th</sub> / 2 | hot desert (mean ≥ 18) / cold desert |
//! | BS | P < 10·P<sub>th</sub> | savanna (mean ≥ 18) / grassland (steppe) |
//! | Af | coldest ≥ 18, driest month ≥ 60 | tropical rainforest |
//! | Am | coldest ≥ 18, driest ≥ 100 − P/25 | tropical seasonal forest |
//! | Aw | coldest ≥ 18, otherwise | savanna |
//! | Cs | coldest > 0, summer month < 40 mm and < ⅓ of the winter month | Mediterranean shrubland |
//! | C | coldest > 0 | temperate forest; rainforest if P ≥ 2000, grassland if P < 500 |
//! | D | coldest ≤ 0 | boreal forest if warmest < 18; else temperate forest, grassland if P < 500 |
//!
//! where P<sub>th</sub> = 2·mean + 28 if ≥ 70% of the rain falls in the warm
//! half of the year, 2·mean if ≥ 70% in the cold half, else 2·mean + 14.
//! Large, slow, low-lying rivers outside dry or polar climates are wetlands.
//!
//! The thresholds on P for rainforest and grassland inside C and D, and the
//! wetland rule, are this project's own additions: Köppen does not separate
//! forest from grassland within humid climates.

use bevy_ecs::prelude::{Res, ResMut, Resource};

use crate::climate::cryosphere::GLACIER;
use crate::climate::stats::ClimateNormal;
use crate::climate::{self, Climate};
use crate::config::SimConfig;
use crate::map::{Landform, Map, Water};
use crate::time::Turn;

#[derive(Debug, Clone, Copy, PartialEq, Eq, Hash, PartialOrd, Ord)]
pub enum Biome {
    Ocean,
    Lake,
    IceSheet,
    Tundra,
    Alpine,
    BorealForest,
    TemperateForest,
    TemperateRainforest,
    Grassland,
    MediterraneanShrubland,
    HotDesert,
    ColdDesert,
    Savanna,
    TropicalSeasonalForest,
    TropicalRainforest,
    Wetland,
}

impl Biome {
    pub const ALL: [Biome; 16] = [
        Biome::Ocean,
        Biome::Lake,
        Biome::IceSheet,
        Biome::Tundra,
        Biome::Alpine,
        Biome::BorealForest,
        Biome::TemperateForest,
        Biome::TemperateRainforest,
        Biome::Grassland,
        Biome::MediterraneanShrubland,
        Biome::HotDesert,
        Biome::ColdDesert,
        Biome::Savanna,
        Biome::TropicalSeasonalForest,
        Biome::TropicalRainforest,
        Biome::Wetland,
    ];

    pub fn is_forest(self) -> bool {
        matches!(
            self,
            Biome::BorealForest
                | Biome::TemperateForest
                | Biome::TemperateRainforest
                | Biome::TropicalSeasonalForest
                | Biome::TropicalRainforest
        )
    }

    pub fn is_desert(self) -> bool {
        matches!(self, Biome::HotDesert | Biome::ColdDesert)
    }
}

/// The climate of one tile, as the classification sees it.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub struct ClimateSummary {
    /// Tenths of a degree C.
    pub mean: i64,
    pub warmest_month: i64,
    pub coldest_month: i64,
    /// Millimeters per year.
    pub precipitation: i64,
    pub driest_month: i64,
    /// Share of precipitation in the local warm half of the year, per mille.
    pub warm_half_share: i64,
    /// Mean monthly precipitation in the local summer and winter, mm.
    pub summer_month: i64,
    pub winter_month: i64,
}

impl ClimateSummary {
    pub fn of(climate: &Climate, map: &Map, index: usize) -> Self {
        let normal: &ClimateNormal = climate.normal(index);
        let (warmest_month, coldest_month) = climate.warmest_and_coldest_month(index);
        let northern = map.latitude_tenths(map.position_of(index as u32).y) >= 0;
        let (summer_month, winter_month) = normal.summer_and_winter_month_mm(northern);
        Self {
            mean: normal.mean_tenths(),
            warmest_month,
            coldest_month,
            precipitation: normal.annual_precipitation_mm(),
            driest_month: normal.driest_month_mm(),
            warm_half_share: normal.warm_half_share(northern),
            summer_month,
            winter_month,
        }
    }
}

/// Köppen-based classification of a land tile's climate (see module docs).
/// `mountains` picks alpine over tundra.
pub fn classify(c: &ClimateSummary, mountains: bool) -> Biome {
    if c.warmest_month < 100 {
        return if mountains {
            Biome::Alpine
        } else {
            Biome::Tundra
        };
    }
    // Aridity threshold P_th (cm-ish units of Peel et al.), times 10 for mm.
    let seasonal = if c.warm_half_share >= 700 {
        280
    } else if c.warm_half_share <= 300 {
        0
    } else {
        140
    };
    let threshold = (2 * c.mean + seasonal).max(0); // 10 · (2·T°C + 28|14|0)
    let hot = c.mean >= 180;
    if c.precipitation * 2 < threshold {
        return if hot {
            Biome::HotDesert
        } else {
            Biome::ColdDesert
        };
    }
    if c.precipitation < threshold {
        return if hot {
            Biome::Savanna
        } else {
            Biome::Grassland
        };
    }
    if c.coldest_month >= 180 {
        return if c.driest_month >= 60 {
            Biome::TropicalRainforest
        } else if c.driest_month >= 100 - c.precipitation / 25 {
            Biome::TropicalSeasonalForest
        } else {
            Biome::Savanna
        };
    }
    if c.coldest_month > 0 {
        if c.summer_month < 40 && c.summer_month * 3 < c.winter_month {
            Biome::MediterraneanShrubland
        } else if c.precipitation >= 2000 {
            Biome::TemperateRainforest
        } else if c.precipitation < 500 {
            Biome::Grassland
        } else {
            Biome::TemperateForest
        }
    } else if c.warmest_month < 180 {
        Biome::BorealForest
    } else if c.precipitation < 500 {
        Biome::Grassland
    } else {
        Biome::TemperateForest
    }
}

#[derive(Resource, Debug, Clone)]
pub struct Biomes {
    biome: Vec<Biome>,
}

impl Biomes {
    pub fn new(map: &Map, climate: &Climate, config: &SimConfig) -> Self {
        let mut biomes = Self {
            biome: vec![Biome::Ocean; map.tile_count() as usize],
        };
        biomes.recompute(map, climate, config);
        biomes
    }

    pub fn biome(&self, index: usize) -> Biome {
        self.biome[index]
    }

    pub fn all(&self) -> &[Biome] {
        &self.biome
    }

    pub fn recompute(&mut self, map: &Map, climate: &Climate, config: &SimConfig) {
        let river = crate::map::hydrology::river_threshold(map, &config.terrain);
        for (i, b) in self.biome.iter_mut().enumerate() {
            *b = tile_biome(map, climate, i, river);
        }
    }
}

fn tile_biome(map: &Map, climate: &Climate, i: usize, river_threshold: u32) -> Biome {
    let tile = &map.tiles()[i];
    match tile.water {
        Water::Ocean => return Biome::Ocean,
        Water::Lake => return Biome::Lake,
        _ => {}
    }
    if climate.cryosphere().tile(i).glacier >= GLACIER {
        return Biome::IceSheet;
    }
    let biome = classify(
        &ClimateSummary::of(climate, map, i),
        tile.landform == Landform::Mountains,
    );
    let big_slow_river = tile.water == Water::River
        && tile.altitude <= 60
        && tile.flow >= river_threshold.saturating_mul(4);
    let wet_enough = !biome.is_desert() && !matches!(biome, Biome::Tundra | Biome::Alpine);
    if big_slow_river && wet_enough {
        Biome::Wetland
    } else {
        biome
    }
}

/// Re-classifies every tile at the start of each year, after the climate
/// normals have been updated.
pub fn update_biomes(
    turn: Res<Turn>,
    config: Res<SimConfig>,
    map: Res<Map>,
    climate: Res<Climate>,
    mut biomes: ResMut<Biomes>,
) {
    if climate::is_new_year(turn.0, climate.days_per_year()) {
        biomes.recompute(&map, &climate, &config);
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    fn climate(mean: i64, swing: i64, precipitation: i64, warm_share: i64) -> ClimateSummary {
        // Seasonal months from the warm-half share: summer gets 2·share of
        // the average month, winter 2·(1 − share).
        let month = precipitation / 12;
        ClimateSummary {
            mean,
            warmest_month: mean + swing,
            coldest_month: mean - swing,
            precipitation,
            driest_month: month * (1000 - warm_share.abs_diff(500) as i64) / 1000,
            warm_half_share: warm_share,
            summer_month: month * 2 * warm_share / 1000,
            winter_month: month * 2 * (1000 - warm_share) / 1000,
        }
    }

    #[test]
    fn koppen_examples() {
        // Manaus-like: hot, wet all year.
        assert_eq!(
            classify(&climate(270, 10, 2300, 500), false),
            Biome::TropicalRainforest
        );
        // Sahara-like.
        assert_eq!(
            classify(&climate(270, 80, 30, 600), false),
            Biome::HotDesert
        );
        // Sahel-like: hot, summer rain, semi-arid.
        assert_eq!(classify(&climate(280, 40, 500, 900), false), Biome::Savanna);
        // Gobi-like: cold winters, dry.
        assert_eq!(
            classify(&climate(20, 200, 150, 800), false),
            Biome::ColdDesert
        );
        // Kazakh steppe.
        assert_eq!(
            classify(&climate(50, 180, 300, 650), false),
            Biome::Grassland
        );
        // Rome-like: mild, dry summers.
        assert_eq!(
            classify(&climate(160, 80, 800, 200), false),
            Biome::MediterraneanShrubland
        );
        // Paris-like.
        assert_eq!(
            classify(&climate(120, 80, 650, 500), false),
            Biome::TemperateForest
        );
        // Siberian taiga.
        assert_eq!(
            classify(&climate(-50, 220, 450, 650), false),
            Biome::BorealForest
        );
        // Arctic coast.
        assert_eq!(
            classify(&climate(-120, 150, 250, 600), false),
            Biome::Tundra
        );
        assert_eq!(classify(&climate(-120, 150, 250, 600), true), Biome::Alpine);
        // Vancouver Island-like.
        assert_eq!(
            classify(&climate(100, 60, 2500, 350), false),
            Biome::TemperateRainforest
        );
    }
}
