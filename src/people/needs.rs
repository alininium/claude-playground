//! Needs: how each person's needs grow over time (hunger, tiredness, cold
//! from the tile's temperature, loneliness, boredom...) and fall when
//! satisfied by what they did.
//!
//! Needs change once a day, at nightfall, which is all a decision made each
//! morning needs to see. For everyone, in this order:
//! 1. Every need grows by its daily rate. Rest grows by a waking day's
//!    tiredness, warmth by how cold the night is on the home tile (and the
//!    season), safety faster after a disaster killed people in the city.
//! 2. The day's action (the one held at nightfall, for the part of the day
//!    it was held) satisfies rest, warmth, safety, company and fun, by the
//!    amounts in the action catalog (`people::decide`).
//! 3. The night's sleep satisfies rest and some warmth.
//!
//! Food is satisfied by eating (`economy`), family by partners and children
//! (`life`) and esteem by status (`status`): this module only grows them.
//!
//! Per-person and parallel (see `people::parallel`). Tuning: the `needs`
//! section of `data/people.json`.

use std::collections::BTreeMap;

use bevy_ecs::prelude::*;
use bevy_ecs::schedule::ScheduleConfigs;
use bevy_ecs::system::ScheduleSystem;

use super::decide::ActionCatalog;
use super::parallel::{self, Threads};
use super::{Activity, Need, Needs, Person};
use crate::city::{CityId, Population};
use crate::climate::Climate;
use crate::config::SimConfig;
use crate::map::{Map, Position};
use crate::time::{Date, TURNS_PER_DAY, TimeOfDay, Turn};

/// The needs a day's action can satisfy here; the others belong to other
/// modules.
const SATISFIED_HERE: [Need; 5] = [
    Need::Rest,
    Need::Warmth,
    Need::Safety,
    Need::Company,
    Need::Fun,
];

/// Waking turns in a day (all but the night).
const WAKING_TURNS: u64 = TURNS_PER_DAY - 1;

#[derive(Debug, Clone, serde::Deserialize)]
#[serde(deny_unknown_fields)]
struct NeedsSection {
    #[serde(default)]
    #[allow(dead_code)]
    notes: Vec<String>,
    growth_per_day: BTreeMap<Need, u16>,
    rest_awake_per_day: u16,
    night_sleep: BTreeMap<Need, u16>,
    warmth: WarmthRule,
    safety: SafetyRule,
}

/// How cold makes people want warmth.
#[derive(Debug, Clone, Copy, PartialEq, Eq, serde::Deserialize)]
#[serde(deny_unknown_fields)]
pub struct WarmthRule {
    /// Below this night temperature (tenths of a degree C) people get cold.
    pub comfort_tenths: i32,
    /// Daily growth of warmth for each degree below comfort.
    pub per_degree_per_day: u32,
    /// Extra daily growth in spring, summer, autumn and winter.
    pub season_extra_per_day: [u16; 4],
}

/// How disasters make people fear for their safety.
#[derive(Debug, Clone, Copy, PartialEq, Eq, serde::Deserialize)]
#[serde(deny_unknown_fields)]
pub struct SafetyRule {
    pub disaster_per_day: u16,
    /// For how many days after people died in a disaster.
    pub disaster_days: u64,
}

/// The `needs` section of `data/people.json`.
#[derive(Resource, Debug, Clone, PartialEq, Eq)]
pub struct NeedsConfig {
    /// Daily growth, indexed by `Need`.
    pub growth_per_day: [u16; Need::COUNT],
    pub rest_awake_per_day: u16,
    /// What a night's sleep satisfies, indexed by `Need`.
    pub night_sleep: [u16; Need::COUNT],
    pub warmth: WarmthRule,
    pub safety: SafetyRule,
}

impl NeedsConfig {
    pub fn builtin() -> Self {
        let section: NeedsSection = super::section("needs");
        let by_need = |map: &BTreeMap<Need, u16>| {
            let mut out = [0; Need::COUNT];
            for (&need, &v) in map {
                out[need as usize] = v.min(1000);
            }
            out
        };
        Self {
            growth_per_day: by_need(&section.growth_per_day),
            rest_awake_per_day: section.rest_awake_per_day,
            night_sleep: by_need(&section.night_sleep),
            warmth: section.warmth,
            safety: section.safety,
        }
    }
}

/// The last turn each city lost people to a disaster, by `CityId`.
#[derive(Resource, Debug, Clone, Default, PartialEq, Eq)]
pub struct Disasters {
    pub last: Vec<Option<u64>>,
}

/// Inserts this module's resources (its section of `data/people.json`).
pub fn install(world: &mut World) {
    world.insert_resource(NeedsConfig::builtin());
    world.init_resource::<Disasters>();
}

/// This module's systems, in order, for its slot in the turn.
pub fn systems() -> ScheduleConfigs<ScheduleSystem> {
    (note_disasters, grow_needs).chain()
}

/// Remembers when a city last owed deaths to a disaster. Runs every turn,
/// before the census carries the deaths out; there are only a few cities.
fn note_disasters(
    turn: Res<Turn>,
    cities: Query<(&CityId, &Population)>,
    mut disasters: ResMut<Disasters>,
) {
    for (id, population) in &cities {
        if population.pending_deaths == 0 {
            continue;
        }
        let i = id.0 as usize;
        if disasters.last.len() <= i {
            disasters.last.resize(i + 1, None);
        }
        disasters.last[i] = Some(turn.0);
    }
}

/// The daily growth a city's weather and fortune add to everyone in it.
#[derive(Debug, Clone, Copy, Default)]
struct Hardship {
    warmth: u16,
    safety: u16,
}

/// The daily update, at nightfall (see the module documentation).
#[allow(clippy::too_many_arguments)]
fn grow_needs(
    turn: Res<Turn>,
    config: Res<SimConfig>,
    rules: Res<NeedsConfig>,
    catalog: Res<ActionCatalog>,
    climate: Res<Climate>,
    map: Res<Map>,
    disasters: Res<Disasters>,
    threads: Res<Threads>,
    cities: Query<(&CityId, &Position)>,
    mut people: Query<(&Person, &Activity, &mut Needs)>,
) {
    let now = turn.0;
    let date = Date::from_turn(now, config.climate.days_per_year);
    if date.time_of_day != TimeOfDay::Night {
        return;
    }

    let mut hardship = Vec::new();
    for (id, position) in &cities {
        let i = id.0 as usize;
        if hardship.len() <= i {
            hardship.resize(i + 1, Hardship::default());
        }
        let season = if map.latitude_tenths(position.y) < 0 {
            date.season.opposite()
        } else {
            date.season
        };
        let temperature = i32::from(climate.temperature(map.index(*position)));
        let cold = (rules.warmth.comfort_tenths - temperature).max(0) as u32;
        let warmth = cold * rules.warmth.per_degree_per_day / 10
            + u32::from(rules.warmth.season_extra_per_day[season as usize]);
        let shaken =
            disasters.last.get(i).copied().flatten().is_some_and(|t| {
                now.saturating_sub(t) < rules.safety.disaster_days * TURNS_PER_DAY
            });
        hardship[i] = Hardship {
            warmth: warmth.min(1000) as u16,
            safety: if shaken {
                rules.safety.disaster_per_day
            } else {
                0
            },
        };
    }

    // Each person's update reads only them and the shared tables above, so
    // the order of the rows (and the number of threads) does not matter.
    let mut rows: Vec<_> = people.iter_mut().collect();
    let day_start = now + 1 - TURNS_PER_DAY;
    parallel::for_each(&mut rows, *threads, |(person, activity, needs)| {
        let city = hardship
            .get(person.home.0 as usize)
            .copied()
            .unwrap_or_default();
        let needs = &mut **needs;
        for need in Need::ALL {
            needs.grow(need, rules.growth_per_day[need as usize]);
        }
        needs.grow(Need::Rest, rules.rest_awake_per_day);
        needs.grow(Need::Warmth, city.warmth);
        needs.grow(Need::Safety, city.safety);

        let held = now
            .saturating_sub(activity.since.max(day_start))
            .min(WAKING_TURNS);
        if let Some(action) = catalog.get(activity.action) {
            for need in SATISFIED_HERE {
                let full = u64::from(action.serves[need as usize]);
                needs.satisfy(need, (full * held / WAKING_TURNS) as u16);
            }
        }
        for need in SATISFIED_HERE {
            needs.satisfy(need, rules.night_sleep[need as usize]);
        }
    });
}
