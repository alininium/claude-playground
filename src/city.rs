//! Cities: founding and population growth.

use bevy_ecs::prelude::*;

use crate::config::SimConfig;
use crate::events::{EventLog, SimEvent};
use crate::map::{Map, Position};
use crate::rng::SimRng;
use crate::turn::Turn;

/// Stable, simulation-assigned identifier. Unlike `Entity`, it is part of the
/// simulated state and is what systems sort by whenever order matters.
#[derive(Component, Debug, Clone, Copy, PartialEq, Eq, Hash, PartialOrd, Ord)]
pub struct CityId(pub u32);

#[derive(Component, Debug, Clone, PartialEq, Eq)]
pub struct City {
    pub name: String,
    pub founded_on: u64,
}

#[derive(Component, Debug, Clone, Copy, PartialEq, Eq)]
pub struct Population {
    pub count: u64,
    /// Fractional growth carried between turns, in thousandths of a person.
    /// Keeps growth exact without floating point.
    pub growth_remainder: u64,
}

impl Population {
    pub fn new(count: u64) -> Self {
        Self {
            count,
            growth_remainder: 0,
        }
    }
}

#[derive(Resource, Debug, Default)]
pub struct NextCityId(pub u32);

/// Logistic growth in integer arithmetic:
/// `growth = pop * rate * (capacity - pop) / capacity`, accumulated in 1/1000ths.
pub fn grow_cities(config: Res<SimConfig>, mut cities: Query<&mut Population, With<City>>) {
    let cap = config.city_carrying_capacity;
    for mut pop in &mut cities {
        if pop.count >= cap {
            continue;
        }
        let headroom = cap - pop.count;
        let growth_milli =
            pop.count as u128 * config.city_growth_per_mille as u128 * headroom as u128
                / cap as u128;
        let total = pop.growth_remainder as u128 + growth_milli;
        pop.count = (pop.count + (total / 1000) as u64).min(cap);
        pop.growth_remainder = (total % 1000) as u64;
    }
}

/// Occasionally founds a new city on a random unoccupied tile.
#[allow(clippy::too_many_arguments)] // Bevy systems declare their data access as parameters.
pub fn spawn_cities(
    mut commands: Commands,
    config: Res<SimConfig>,
    turn: Res<Turn>,
    map: Res<Map>,
    mut rng: ResMut<SimRng>,
    mut next_id: ResMut<NextCityId>,
    mut log: ResMut<EventLog>,
    existing: Query<&Position, With<City>>,
) {
    if !rng.chance_per_mille(config.city_spawn_chance_per_mille) {
        return;
    }

    let mut available: Vec<bool> = (0..map.tile_count() as usize)
        .map(|i| map.is_habitable(i))
        .collect();
    for pos in &existing {
        available[map.index(*pos)] = false;
    }
    let free = available.iter().filter(|a| **a).count() as u64;
    if free == 0 {
        return;
    }

    // Pick the n-th free tile in row-major order: independent of query order.
    let n = rng.below(free) as usize;
    let index = available
        .iter()
        .enumerate()
        .filter(|(_, a)| **a)
        .nth(n)
        .map(|(i, _)| i as u32)
        .expect("n < free");
    let position = map.position_of(index);

    let id = CityId(next_id.0);
    next_id.0 += 1;
    let name = generate_name(&mut rng);

    log.push(SimEvent::CityFounded {
        turn: turn.0,
        id,
        name: name.clone(),
        position,
    });
    commands.spawn((
        id,
        City {
            name,
            founded_on: turn.0,
        },
        position,
        Population::new(config.initial_city_population),
    ));
}

fn generate_name(rng: &mut SimRng) -> String {
    const SYLLABLES: &[&str] = &[
        "ka", "ri", "to", "mel", "an", "dor", "su", "vel", "ith", "ra", "gon", "ben", "ul", "sha",
        "tor", "es", "lin", "mar", "or", "zan",
    ];
    let count = 2 + rng.below(2);
    let mut name = String::new();
    for _ in 0..count {
        name.push_str(SYLLABLES[rng.below(SYLLABLES.len() as u64) as usize]);
    }
    let mut chars = name.chars();
    match chars.next() {
        Some(first) => first.to_uppercase().chain(chars).collect(),
        None => name,
    }
}
