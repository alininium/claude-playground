//! Decisions: when a person's activity ends (`Activity::until`) or a need
//! becomes urgent, they choose their next action from the catalog in the
//! `actions` section of `data/people.json`, scoring each by how urgent the
//! needs it serves are, their character (`Traits`), what they know
//! (`Skills`), their status, their age and what their city offers.
//! Per-person and parallel. Night is sleep.
//!
//! # The day
//! - **Morning, day, evening** (the waking turns): people whose activity has
//!   ended choose a new one. So does anyone with a need that has become
//!   urgent since they chose. Everyone else carries on, which costs almost
//!   nothing. An action normally lasts the waking day (3 turns); longer ones
//!   are taken up again each morning.
//! - **Night**: everyone sleeps, without deciding. The day's action is kept
//!   in [`Plan::last`] so that a longer action can be resumed, and so that a
//!   farmer keeps working the land they already hold.
//!
//! Children under the founding `adult_age` never decide: they follow their
//! mother when her action is one children can join, and are idle (play)
//! otherwise. People travelling with a caravan (`until == u64::MAX`) are
//! left alone.
//!
//! # Settling away
//! `SettleAway` is only open to adults of a city with no free land and
//! hardly any food in store who have been hungry for days on end (see
//! [`SettleAwayRule`]), and it leans strongly on boldness. The "for days on
//! end" part also keeps a city whose `Outlook` is still all zeros (nothing
//! has filled it in yet) from emptying itself on the first day.

use std::collections::BTreeMap;

use bevy_ecs::prelude::*;
use bevy_ecs::schedule::ScheduleConfigs;
use bevy_ecs::system::ScheduleSystem;

use super::founding::FoundingConfig;
use super::parallel::{self, Threads};
use super::rng::{self, PersonRng};
use super::status::StatusCatalog;
use super::{
    ActionKind, Activity, Family, Need, Needs, Person, PersonId, Skills, Status, StatusId, Traits,
};
use crate::biome::Biomes;
use crate::city::CityId;
use crate::config::SimConfig;
use crate::economy::Outlook;
use crate::map::{Map, Position};
use crate::tech::{Site, TechId, TechTree};
use crate::time::{Date, TURNS_PER_DAY, TimeOfDay, Turn};

/// Every action, in the order of [`ActionKind`]: ties in scoring go to the
/// earlier one.
pub const ACTIONS: [ActionKind; 14] = [
    ActionKind::Idle,
    ActionKind::Sleep,
    ActionKind::Rest,
    ActionKind::Forage,
    ActionKind::Farm,
    ActionKind::Herd,
    ActionKind::Fish,
    ActionKind::Craft,
    ActionKind::Build,
    ActionKind::Socialize,
    ActionKind::Worship,
    ActionKind::Court,
    ActionKind::CareForChildren,
    ActionKind::SettleAway,
];

/// A character trait, as named in the catalog's `traits`.
#[derive(Debug, Clone, Copy, PartialEq, Eq, PartialOrd, Ord, serde::Deserialize)]
#[serde(rename_all = "snake_case")]
pub enum Trait {
    Openness,
    Conscientiousness,
    Extraversion,
    Agreeableness,
    Neuroticism,
    Boldness,
}

/// One catalog entry as written in `data/people.json`.
#[derive(Debug, Clone, serde::Deserialize)]
#[serde(deny_unknown_fields)]
struct ActionSpec {
    #[serde(default)]
    serves: BTreeMap<Need, u16>,
    duration_turns: u64,
    #[serde(default)]
    min_age: u32,
    #[serde(default)]
    techs_any: Vec<String>,
    #[serde(default)]
    land: bool,
    #[serde(default)]
    water: bool,
    #[serde(default)]
    unpartnered: bool,
    #[serde(default)]
    parents: bool,
    #[serde(default)]
    children: bool,
    #[serde(default)]
    settle_away: bool,
    #[serde(default)]
    traits: BTreeMap<Trait, i32>,
}

/// The `actions` section of `data/people.json`.
#[derive(Debug, Clone, serde::Deserialize)]
#[serde(deny_unknown_fields)]
struct ActionsSection {
    #[serde(default)]
    #[allow(dead_code)]
    notes: Vec<String>,
    urgent: u16,
    noise: u32,
    pressing_food: u16,
    settle_away: SettleAwayRule,
    catalog: BTreeMap<ActionKind, ActionSpec>,
}

/// When settling away becomes a choice at all.
#[derive(Debug, Clone, Copy, PartialEq, Eq, serde::Deserialize)]
#[serde(deny_unknown_fields)]
pub struct SettleAwayRule {
    /// Days in a row of pressing hunger (see [`ActionCatalog::pressing_food`])
    /// before someone considers leaving.
    pub hungry_days: u16,
    /// Only if the city has at most this many days of food in store per head.
    pub max_food_days: u32,
}

/// One action, ready for scoring.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct Action {
    pub kind: ActionKind,
    /// How much a day of it satisfies each need, indexed by `Need`.
    pub serves: [u16; Need::COUNT],
    pub duration_turns: u64,
    pub min_age: u32,
    /// Technology ids, any one of which is enough (none: no requirement).
    pub techs_any: Vec<String>,
    /// Needs free land in the city, unless the person already works it.
    pub land: bool,
    /// Needs a river, lake or coast at the city.
    pub water: bool,
    pub unpartnered: bool,
    pub parents: bool,
    /// Children may join their mother in it.
    pub children: bool,
    /// Subject to [`SettleAwayRule`].
    pub settle_away: bool,
    /// Leaning per trait, in [`Traits::as_array`] order.
    pub traits: [i32; 6],
}

/// The action catalog and the decision rules (the `actions` section of
/// `data/people.json`). Also read by `needs`, which satisfies needs by the
/// action done.
#[derive(Resource, Debug, Clone, PartialEq, Eq)]
pub struct ActionCatalog {
    /// The actions present in the data, in [`ACTIONS`] order.
    pub actions: Vec<Action>,
    /// A need at or above this is urgent.
    pub urgent: u16,
    /// The random part of every score is drawn from `0..=noise`.
    pub noise: u32,
    /// Hunger at which a day counts toward [`SettleAwayRule::hungry_days`].
    pub pressing_food: u16,
    pub settle_away: SettleAwayRule,
    index: [Option<u8>; ACTIONS.len()],
}

impl ActionCatalog {
    pub fn builtin() -> Self {
        Self::from_section(super::section("actions"))
    }

    fn from_section(section: ActionsSection) -> Self {
        let mut actions = Vec::new();
        let mut index = [None; ACTIONS.len()];
        for kind in ACTIONS {
            let Some(spec) = section.catalog.get(&kind) else {
                continue;
            };
            let mut serves = [0; Need::COUNT];
            for (&need, &amount) in &spec.serves {
                serves[need as usize] = amount.min(1000);
            }
            let mut traits = [0; 6];
            for (&t, &w) in &spec.traits {
                traits[t as usize] = w;
            }
            index[kind as usize] = Some(actions.len() as u8);
            actions.push(Action {
                kind,
                serves,
                duration_turns: spec.duration_turns.max(1),
                min_age: spec.min_age,
                techs_any: spec.techs_any.clone(),
                land: spec.land,
                water: spec.water,
                unpartnered: spec.unpartnered,
                parents: spec.parents,
                children: spec.children,
                settle_away: spec.settle_away,
                traits,
            });
        }
        Self {
            actions,
            urgent: section.urgent,
            noise: section.noise,
            pressing_food: section.pressing_food,
            settle_away: section.settle_away,
            index,
        }
    }

    pub fn get(&self, kind: ActionKind) -> Option<&Action> {
        self.index[kind as usize].map(|i| &self.actions[usize::from(i)])
    }

    /// How much a day of `kind` satisfies `need` (0 for actions not in the
    /// catalog).
    pub fn serves(&self, kind: ActionKind, need: Need) -> u16 {
        self.get(kind).map_or(0, |a| a.serves[need as usize])
    }

    /// The needs at or above the urgent threshold, one bit per `Need`.
    pub fn urgent_needs(&self, needs: &Needs) -> u8 {
        needs
            .urgency
            .iter()
            .enumerate()
            .filter(|&(_, &u)| u >= self.urgent)
            .fold(0, |mask, (n, _)| mask | 1 << n)
    }
}

/// What a person's decisions remember between turns. Attached to everyone
/// by this module on their first turn.
#[derive(Component, Debug, Clone, Copy, PartialEq, Eq, Default)]
pub struct Plan {
    /// The action of the last waking day, kept over the night.
    pub last: ActionKind,
    /// The needs that were urgent when the current action was chosen (one
    /// bit per `Need`): only a need urgent beyond these makes them decide
    /// again before the action ends.
    pub urgent: u8,
    /// Nights in a row they went to sleep with pressing hunger.
    pub hungry_days: u16,
}

/// Inserts this module's resources (its section of `data/people.json`).
pub fn install(world: &mut World) {
    world.insert_resource(ActionCatalog::builtin());
}

/// This module's systems, in order, for its slot in the turn.
pub fn systems() -> ScheduleConfigs<ScheduleSystem> {
    (attach_plans, decide).chain()
}

/// Gives a [`Plan`] to everyone who has none yet (the founders, newborns).
fn attach_plans(mut commands: Commands, new: Query<Entity, (With<PersonId>, Without<Plan>)>) {
    for entity in &new {
        commands.entity(entity).insert(Plan::default());
    }
}

/// What a city offers, as decisions see it.
#[derive(Debug, Clone, Copy, Default)]
struct Place {
    free_land: u32,
    food_days: u32,
    water: bool,
}

/// Everything a decision reads besides the person.
struct Context<'a> {
    now: u64,
    morning: bool,
    turns_per_year: u64,
    catalog: &'a ActionCatalog,
    statuses: &'a StatusCatalog,
    /// Per catalog action: the technologies that allow it.
    techs: &'a [Vec<TechId>],
    /// By `CityId`.
    places: &'a [Place],
}

/// One person, as the decision step works on them.
struct Row<'a> {
    id: PersonId,
    person: &'a Person,
    traits: &'a Traits,
    skills: &'a Skills,
    status: StatusId,
    family: &'a Family,
    needs: &'a Needs,
    activity: Mut<'a, Activity>,
    rng: Mut<'a, PersonRng>,
    plan: Mut<'a, Plan>,
}

type PersonQuery<'w, 's> = Query<
    'w,
    's,
    (
        &'static PersonId,
        &'static Person,
        &'static Traits,
        &'static Skills,
        &'static Status,
        &'static Family,
        &'static Needs,
        &'static mut Activity,
        &'static mut PersonRng,
        &'static mut Plan,
    ),
>;

/// The decision step: night sleep, then by day the decisions of everyone
/// whose activity has ended or who has a new urgent need, then children.
#[allow(clippy::too_many_arguments)]
fn decide(
    turn: Res<Turn>,
    config: Res<SimConfig>,
    catalog: Res<ActionCatalog>,
    statuses: Res<StatusCatalog>,
    founding: Res<FoundingConfig>,
    tree: Res<TechTree>,
    map: Res<Map>,
    biomes: Res<Biomes>,
    threads: Res<Threads>,
    cities: Query<(&CityId, &Position, &Outlook)>,
    mut people: PersonQuery,
    mut techs: Local<Option<Vec<Vec<TechId>>>>,
) {
    let now = turn.0;
    let date = Date::from_turn(now, config.climate.days_per_year);
    if date.time_of_day == TimeOfDay::Night {
        for (_, _, _, _, _, _, needs, mut activity, _, mut plan) in &mut people {
            if activity.until == u64::MAX {
                continue;
            }
            let hungry = needs.get(Need::Food) >= catalog.pressing_food;
            *plan = Plan {
                last: activity.action,
                hungry_days: if hungry {
                    plan.hungry_days.saturating_add(1)
                } else {
                    0
                },
                ..*plan
            };
            *activity = Activity {
                action: ActionKind::Sleep,
                since: now,
                until: activity.until.max(now + 1),
            };
        }
        return;
    }

    let techs = techs.get_or_insert_with(|| {
        catalog
            .actions
            .iter()
            .map(|a| a.techs_any.iter().filter_map(|t| tree.id(t)).collect())
            .collect()
    });
    let mut places = Vec::new();
    for (id, position, outlook) in &cities {
        let i = id.0 as usize;
        if places.len() <= i {
            places.resize(i + 1, Place::default());
        }
        let site = Site::of(&map, &biomes, map.index(*position));
        places[i] = Place {
            free_land: outlook.free_land,
            food_days: outlook.food_days,
            water: site.river || site.lake || site.coast,
        };
    }
    let context = Context {
        now,
        morning: date.time_of_day == TimeOfDay::Morning,
        turns_per_year: config.turns_per_year(),
        catalog: &catalog,
        statuses: &statuses,
        techs,
        places: &places,
    };

    let mut rows: Vec<Row> = people
        .iter_mut()
        .map(
            |(id, person, traits, skills, status, family, needs, activity, rng, plan)| Row {
                id: *id,
                person,
                traits,
                skills,
                status: status.0,
                family,
                needs,
                activity,
                rng,
                plan,
            },
        )
        .collect();
    rows.sort_unstable_by_key(|r| r.id);

    let adult_age = founding.adult_age;
    let is_child = |r: &Row| r.person.age_years(now, context.turns_per_year) < adult_age;
    let mut adults: Vec<&mut Row> = rows
        .iter_mut()
        .filter(|r| r.activity.until != u64::MAX && !is_child(r))
        .collect();
    parallel::for_each(&mut adults, *threads, |row| step(row, &context));

    // Children follow the decisions just made.
    let actions: Vec<(PersonId, ActionKind)> =
        rows.iter().map(|r| (r.id, r.activity.action)).collect();
    let day_end = now - now % TURNS_PER_DAY + TURNS_PER_DAY - 1;
    for row in rows.iter_mut() {
        if row.activity.until > now || !is_child(row) {
            continue;
        }
        let mother = row
            .family
            .mother
            .and_then(|m| actions.binary_search_by_key(&m, |&(id, _)| id).ok())
            .map(|i| actions[i].1);
        let action = match mother {
            Some(a) if catalog.get(a).is_some_and(|a| a.children) => a,
            _ => ActionKind::Idle,
        };
        *row.activity = Activity {
            action,
            since: now,
            until: day_end,
        };
    }
}

/// One adult's waking turn: carry on, take a longer action up again in the
/// morning, or decide.
fn step(row: &mut Row, context: &Context) {
    let catalog = context.catalog;
    let due = row.activity.until <= context.now;
    let urgent = catalog.urgent_needs(row.needs);
    let fresh = urgent & !row.plan.urgent != 0;
    if !due && !fresh {
        if context.morning && row.activity.action == ActionKind::Sleep {
            let last = row.plan.last;
            row.activity.action = last;
            row.activity.since = context.now;
        }
        return;
    }
    let action = choose(row, context);
    let duration = catalog.get(action).map_or(1, |a| a.duration_turns);
    *row.activity = Activity {
        action,
        since: context.now,
        until: context.now + duration,
    };
    row.plan.urgent = urgent;
}

/// The best-scoring action open to this person. One draw from their stream
/// gives every action its random part.
fn choose(row: &mut Row, context: &Context) -> ActionKind {
    let catalog = context.catalog;
    let place = context
        .places
        .get(row.person.home.0 as usize)
        .copied()
        .unwrap_or_default();
    let age = row.person.age_years(context.now, context.turns_per_year);
    let current = row.activity.action;
    let last = row.plan.last;
    let food = row.needs.get(Need::Food);
    let traits = row.traits.as_array();
    let draw = row.rng.next_u64();
    let noise = u64::from(catalog.noise) + 1;

    let mut best = (i64::MIN, ActionKind::Idle);
    for (k, action) in catalog.actions.iter().enumerate() {
        let kind = action.kind;
        let open = age >= action.min_age
            && context.statuses.allows(row.status, kind)
            && (action.techs_any.is_empty()
                || context.techs[k].iter().any(|&t| row.skills.0.knows(t)))
            && (!action.land || place.free_land > 0 || current == kind || last == kind)
            && (!action.water || place.water)
            && (!action.unpartnered || row.family.partner.is_none())
            && (!action.parents || row.family.births > 0)
            && (!action.settle_away
                || (place.free_land == 0
                    && place.food_days <= catalog.settle_away.max_food_days
                    && food >= catalog.pressing_food
                    && row.plan.hungry_days >= catalog.settle_away.hungry_days));
        if !open {
            continue;
        }
        let needs: i64 = row
            .needs
            .urgency
            .iter()
            .zip(action.serves)
            .map(|(&u, s)| i64::from(u) * i64::from(s))
            .sum::<i64>()
            / 1000;
        let character: i64 = traits
            .iter()
            .zip(action.traits)
            .map(|(&t, w)| i64::from(w) * (i64::from(t) - 128) / 128)
            .sum();
        let score = needs
            + character
            + i64::from(context.statuses.leaning(row.status, kind))
            + (rng::mix(draw, k as u64) % noise) as i64;
        if score > best.0 {
            best = (score, kind);
        }
    }
    best.1
}
