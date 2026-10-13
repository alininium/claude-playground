//! Cities: founding and population growth.

use bevy_ecs::prelude::*;

use crate::climate::Climate;
use crate::climate::cryosphere::GLACIER;
use crate::config::SimConfig;
use crate::events::{EventLog, SimEvent};
use crate::fertility::Fertility;
use crate::map::Map;
use crate::rng::SimRng;

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
    /// Fractional growth carried between turns, in units of
    /// `1 / (1000 · turns_per_year · capacity)` of a person. Keeps growth exact
    /// without floating point.
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

/// Logistic growth in exact integer arithmetic. Per turn:
/// `growth = pop · rate · (capacity − pop) / (capacity · 1000 · turns_per_year)`.
pub fn grow_cities(config: Res<SimConfig>, mut cities: Query<&mut Population, With<City>>) {
    let cap = config.city_carrying_capacity.max(1);
    let denominator = u128::from(cap) * 1000 * u128::from(config.turns_per_year());
    for mut pop in &mut cities {
        if pop.count >= cap {
            continue;
        }
        let headroom = cap - pop.count;
        let numerator = u128::from(pop.count)
            * u128::from(config.city_growth_per_mille_per_year)
            * u128::from(headroom);
        let total = u128::from(pop.growth_remainder) + numerator;
        pop.count = (pop.count + (total / denominator) as u64).min(cap);
        pop.growth_remainder = (total % denominator) as u64;
    }
}

/// Cities can be founded on habitable land that is not under a glacier.
pub fn can_settle(map: &Map, climate: &Climate, index: usize) -> bool {
    map.is_habitable(index) && climate.cryosphere().tile(index).glacier < GLACIER
}

/// Places the starting cities when the world is created; no cities are
/// founded during play. Sites are drawn with probability proportional to
/// fertility (plus a small floor), and no two cities are closer than
/// `min_city_distance` tiles. Fewer cities are placed if the land runs out.
/// `turn` is the turn the world starts at (0 for a new world).
#[allow(clippy::too_many_arguments)]
pub fn found_initial_cities(
    world: &mut World,
    turn: u64,
    config: &SimConfig,
    map: &Map,
    climate: &Climate,
    fertility: &Fertility,
    rng: &mut SimRng,
) {
    let mut weight: Vec<u64> = (0..map.tile_count() as usize)
        .map(|i| {
            if can_settle(map, climate, i) {
                u64::from(fertility.fertility(i)) + 10
            } else {
                0
            }
        })
        .collect();
    let spacing = i64::from(config.min_city_distance);

    for n in 0..config.initial_cities {
        let total: u64 = weight.iter().sum();
        if total == 0 {
            break;
        }
        // Weighted pick over tiles in row-major order.
        let mut r = rng.below(total);
        let index = weight
            .iter()
            .position(|&w| {
                if r < w {
                    true
                } else {
                    r -= w;
                    false
                }
            })
            .expect("r < total") as u32;
        let position = map.position_of(index);

        // Rule out everything within the spacing distance.
        let (cx, cy) = (i64::from(position.x), i64::from(position.y));
        for y in (cy - spacing + 1).max(0)..(cy + spacing).min(i64::from(map.height())) {
            for x in (cx - spacing + 1).max(0)..(cx + spacing).min(i64::from(map.width())) {
                weight[(y * i64::from(map.width()) + x) as usize] = 0;
            }
        }
        weight[index as usize] = 0;

        let id = CityId(n);
        let name = generate_name(rng);
        world
            .resource_mut::<EventLog>()
            .push(SimEvent::CityFounded {
                turn,
                id,
                name: name.clone(),
                position,
            });
        world.spawn((
            id,
            City {
                name,
                founded_on: turn,
            },
            position,
            Population::new(config.initial_city_population),
        ));
    }
}

pub(crate) fn generate_name(rng: &mut SimRng) -> String {
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
