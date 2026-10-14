//! Initial conditions. Everything that shapes a run lives here so that
//! experiments are a matter of changing a `SimConfig`, nothing else.

use bevy_ecs::prelude::Resource;

use crate::climate::ClimateConfig;
use crate::map::TerrainConfig;
use crate::volcano::VolcanoConfig;

#[derive(Resource, Debug, Clone, PartialEq, Eq, serde::Serialize, serde::Deserialize)]
pub struct SimConfig {
    /// Seed for the single simulation RNG stream.
    pub seed: u64,
    pub map_width: u32,
    pub map_height: u32,
    pub terrain: TerrainConfig,
    pub climate: ClimateConfig,
    pub volcanoes: VolcanoConfig,
    /// Civilized cities placed at world creation, on cradle sites like
    /// those of the first cities (none are founded later).
    pub initial_cities: u32,
    /// Barbarian tribes placed at world creation, anywhere habitable.
    pub initial_barbarians: u32,
    /// Minimum distance between any two settlements, in kilometers.
    /// Settlements are never on neighboring tiles either.
    pub min_city_distance_km: u32,
    /// Population a civilized city starts with.
    pub initial_city_population: u64,
    /// Population a barbarian tribe starts with.
    pub initial_barbarian_population: u64,
    /// Maximum growth rate per year, in 1/1000 of the population.
    pub city_growth_per_mille_per_year: u64,
    /// Population at which a city stops growing (logistic carrying capacity).
    pub city_carrying_capacity: u64,
}

impl SimConfig {
    pub fn turns_per_year(&self) -> u64 {
        u64::from(self.climate.days_per_year) * crate::time::TURNS_PER_DAY
    }
}

impl Default for SimConfig {
    fn default() -> Self {
        Self {
            seed: 0,
            map_width: 128,
            map_height: 64,
            terrain: TerrainConfig::default(),
            climate: ClimateConfig::default(),
            volcanoes: VolcanoConfig::default(),
            initial_cities: 6,
            initial_barbarians: 10,
            min_city_distance_km: 800,
            initial_city_population: 10,
            initial_barbarian_population: 10,
            city_growth_per_mille_per_year: 30,
            city_carrying_capacity: 10_000,
        }
    }
}
