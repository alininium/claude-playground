//! Initial conditions. Everything that shapes a run lives here so that
//! experiments are a matter of changing a `SimConfig`, nothing else.

use bevy_ecs::prelude::Resource;

#[derive(Resource, Debug, Clone, PartialEq, Eq)]
pub struct SimConfig {
    /// Seed for the single simulation RNG stream.
    pub seed: u64,
    pub map_width: u32,
    pub map_height: u32,
    /// Chance (in 1/1000) that a new city is founded on any given turn.
    pub city_spawn_chance_per_mille: u32,
    /// Population a freshly founded city starts with.
    pub initial_city_population: u64,
    /// Maximum growth rate per turn, in 1/1000 of the population.
    pub city_growth_per_mille: u64,
    /// Population at which a city stops growing (logistic carrying capacity).
    pub city_carrying_capacity: u64,
}

impl Default for SimConfig {
    fn default() -> Self {
        Self {
            seed: 0,
            map_width: 64,
            map_height: 32,
            city_spawn_chance_per_mille: 100,
            initial_city_population: 10,
            city_growth_per_mille: 20,
            city_carrying_capacity: 10_000,
        }
    }
}
