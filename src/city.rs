//! Cities and barbarian tribes: where they are founded and how big they
//! start. Their people live in `people`.

use bevy_ecs::prelude::*;

use crate::biome::{Biome, Biomes};
use crate::climate::Climate;
use crate::climate::cryosphere::GLACIER;
use crate::config::SimConfig;
use crate::events::{EventLog, SimEvent};
use crate::fertility::Fertility;
use crate::map::{Map, Position, Soil};
use crate::rng::SimRng;
use crate::tech::{Site, TechTree};

/// Stable, simulation-assigned identifier. Unlike `Entity`, it is part of the
/// simulated state and is what systems sort by whenever order matters.
#[derive(Component, Debug, Clone, Copy, PartialEq, Eq, Hash, PartialOrd, Ord)]
pub struct CityId(pub u32);

/// The next [`CityId`] to hand out (for cities founded during play).
#[derive(Resource, Debug, Default, Clone, Copy, PartialEq, Eq)]
pub struct NextCityId(pub u32);

#[derive(Component, Debug, Clone, PartialEq, Eq)]
pub struct City {
    pub name: String,
    pub founded_on: u64,
    pub people: People,
}

/// How many people live in a city (counted every turn from its people; see
/// `people::census`).
#[derive(Component, Debug, Clone, Copy, PartialEq, Eq, Default)]
pub struct Population {
    pub count: u64,
    /// Deaths owed to a disaster (such as a volcano's ash) that the census
    /// carries out at the end of the turn.
    pub pending_deaths: u64,
}

impl Population {
    pub fn new(count: u64) -> Self {
        Self {
            count,
            pending_deaths: 0,
        }
    }
}

/// Cities can be founded on habitable land that is not under a glacier.
pub fn can_settle(map: &Map, climate: &Climate, index: usize) -> bool {
    map.is_habitable(index) && climate.cryosphere().tile(index).glacier < GLACIER
}

/// Who lives in a settlement.
#[derive(Debug, Clone, Copy, PartialEq, Eq, Hash, serde::Serialize, serde::Deserialize)]
pub enum People {
    /// One of the first cities, in a cradle of civilization.
    Civilized,
    /// A tribe outside the cradles, knowing few technologies.
    Barbarian,
}

/// Places the starting settlements when the world is created; none are
/// founded during play. `turn` is the turn the world starts at (0 for a new
/// world).
///
/// First come `initial_cities` civilized cities, on sites like those of the
/// first cities around 4500 BCE (see [`cradle_score`]): drawn with
/// probability proportional to the square of the score, so the best sites
/// usually win. If the cradle sites run out, the rest go anywhere fertile.
/// Then `initial_barbarians` tribes, drawn in proportion to fertility (plus a
/// small floor) wherever there is room. No two settlements are closer than
/// `min_city_distance_km` (see [`too_close`]); fewer are placed if the land
/// runs out.
///
/// Each settlement starts with the technologies of the start year that suit
/// its surroundings and its people (see [`TechTree::starting_knowledge`]),
/// drawn from `tech_rng` so that changing the technology file never moves
/// anything. Its people (see `people::founding`) come from `people_rng`.
#[allow(clippy::too_many_arguments)]
pub fn found_initial_cities(
    world: &mut World,
    turn: u64,
    config: &SimConfig,
    map: &Map,
    climate: &Climate,
    biomes: &Biomes,
    fertility: &Fertility,
    tree: &TechTree,
    rng: &mut SimRng,
    tech_rng: &mut SimRng,
    founding: &crate::people::founding::FoundingConfig,
    people_rng: &mut SimRng,
) {
    let n = map.tile_count() as usize;
    let mut cradle: Vec<u64> = (0..n)
        .map(|i| cradle_score(map, climate, biomes, fertility, i).pow(2))
        .collect();
    let mut anywhere: Vec<u64> = (0..n)
        .map(|i| {
            if can_settle(map, climate, i) {
                u64::from(fertility.fertility(i)) + 10
            } else {
                0
            }
        })
        .collect();

    let plan = std::iter::repeat_n(People::Civilized, config.initial_cities as usize).chain(
        std::iter::repeat_n(People::Barbarian, config.initial_barbarians as usize),
    );
    let mut next_id = 0;
    for people in plan {
        let index = match people {
            People::Civilized => pick(&cradle, rng).or_else(|| pick(&anywhere, rng)),
            People::Barbarian => pick(&anywhere, rng),
        };
        let Some(index) = index else {
            continue;
        };
        // Rule out everything too close.
        let here = map.position_of(index as u32);
        let reach = rows_within(map, config, config.min_city_distance_km);
        let rows = here.y.saturating_sub(reach)..(here.y + reach + 1).min(map.height());
        for y in rows {
            for x in 0..map.width() {
                let there = Position { x, y };
                if too_close(map, config, here, there) {
                    let i = map.index(there);
                    cradle[i] = 0;
                    anywhere[i] = 0;
                }
            }
        }

        let id = CityId(next_id);
        next_id += 1;
        let name = generate_name(rng);
        let site = Site::of(map, biomes, index);
        let knowledge = tree.starting_knowledge(&site, people, tech_rng);
        let size = settlement_size(map, fertility, config, people, index);
        world
            .resource_mut::<EventLog>()
            .push(SimEvent::CityFounded {
                turn,
                id,
                name: name.clone(),
                position: here,
                people,
            });
        crate::people::founding::spawn_settlement(
            world,
            &crate::people::founding::Settlement {
                city: id,
                size,
                knowledge: &knowledge,
                turn,
                turns_per_year: config.turns_per_year(),
            },
            tree,
            founding,
            people_rng,
        );
        world.spawn((
            id,
            City {
                name,
                founded_on: turn,
                people,
            },
            here,
            Population::new(u64::from(size)),
            knowledge,
            crate::economy::Stores::default(),
            crate::economy::Outlook::default(),
        ));
    }
    world.insert_resource(NextCityId(next_id));
}

/// How many people a settlement starts with: from the low to the high end of
/// its kind's range (`city_population` or `barbarian_population`) with the
/// mean fertility of its tile and neighbors.
pub fn settlement_size(
    map: &Map,
    fertility: &Fertility,
    config: &SimConfig,
    people: People,
    index: usize,
) -> u32 {
    let [lo, hi] = match people {
        People::Civilized => config.city_population,
        People::Barbarian => config.barbarian_population,
    };
    let tiles: Vec<u32> = std::iter::once(index)
        .chain(map.neighbors(index))
        .map(|i| u32::from(fertility.fertility(i)))
        .collect();
    let mean = tiles.iter().sum::<u32>() / tiles.len() as u32;
    lo + hi.saturating_sub(lo) * mean.min(1000) / 1000
}

/// How much a site looks like those of the first cities around 4500 BCE
/// (Eridu and Uruk, the Nile, Mehrgarh, the Yellow River, Çatalhöyük,
/// Varna): 0 if it does not qualify at all.
///
/// A site qualifies if it can be settled, has fresh water or the sea next
/// to it, has a mean temperature between 10 and 25 °C, and is not
/// rainforest, boreal forest, tundra, cold desert, alpine or ice. Its score
/// is its fertility (0–1000) plus 500 for a river, 200 for a lake, 100 for a
/// coast, 300 for alluvial soil, and 300 more for a river through dry or
/// open country (desert, savanna, grassland or Mediterranean shrubland),
/// where irrigation made the first surpluses.
pub fn cradle_score(
    map: &Map,
    climate: &Climate,
    biomes: &Biomes,
    fertility: &Fertility,
    index: usize,
) -> u64 {
    if !can_settle(map, climate, index) {
        return 0;
    }
    let biome = biomes.biome(index);
    if matches!(
        biome,
        Biome::TropicalRainforest
            | Biome::BorealForest
            | Biome::Tundra
            | Biome::ColdDesert
            | Biome::Alpine
            | Biome::IceSheet
    ) {
        return 0;
    }
    if !(100..=250).contains(&climate.normal(index).mean_tenths()) {
        return 0;
    }
    let site = Site::of(map, biomes, index);
    if !(site.river || site.lake || site.coast) {
        return 0;
    }
    let mut score = u64::from(fertility.fertility(index));
    if site.river {
        score += 500;
    }
    if site.lake {
        score += 200;
    }
    if site.coast {
        score += 100;
    }
    if site.soil == Soil::Alluvial {
        score += 300;
    }
    if site.river
        && matches!(
            biome,
            Biome::HotDesert | Biome::Savanna | Biome::Grassland | Biome::MediterraneanShrubland
        )
    {
        score += 300;
    }
    score
}

/// Whether two settlements at `a` and `b` would be too close: nearer than
/// `min_city_distance_km`, or on the same or neighboring tiles. Distances
/// are measured on the globe: a tile spans `circumference / width`
/// kilometers east-west at the equator (less toward the poles) and
/// `circumference / 2 / height` north-south.
pub fn too_close(map: &Map, config: &SimConfig, a: Position, b: Position) -> bool {
    let (dx, dy) = (a.x.abs_diff(b.x), a.y.abs_diff(b.y));
    if dx.max(dy) < 2 {
        return true;
    }
    let circumference = i64::from(config.climate.planet_circumference_km);
    let row = (a.y + b.y) / 2;
    let ns = i64::from(dy) * circumference / 2 / i64::from(map.height());
    let ew = i64::from(dx) * circumference / i64::from(map.width()) * map.area_weight(row) / 1000;
    let limit = i64::from(config.min_city_distance_km);
    ns * ns + ew * ew < limit * limit
}

/// How many rows `km` spans north-south (rounded up).
fn rows_within(map: &Map, config: &SimConfig, km: u32) -> u32 {
    let per_row =
        (u64::from(config.climate.planet_circumference_km) / 2 / u64::from(map.height())).max(1);
    (u64::from(km).div_ceil(per_row) as u32).max(2)
}

/// Weighted pick over tiles in row-major order; None if all weights are 0.
fn pick(weight: &[u64], rng: &mut SimRng) -> Option<usize> {
    let total: u64 = weight.iter().sum();
    if total == 0 {
        return None;
    }
    let mut r = rng.below(total);
    weight.iter().position(|&w| {
        if r < w {
            true
        } else {
            r -= w;
            false
        }
    })
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
