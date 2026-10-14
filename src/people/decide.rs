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
//!
//! A city can also outgrow its social organisation: how many people it can
//! hold together depends on what it knows (kin leadership, ranks, temples,
//! kings, see [`OrganisationRule`]). Above that size, some of each day's
//! decisions consider `SettleAway` whatever the land and the stores, more
//! and more strongly the fuller the city: the boldest leave first, a few
//! at a time, until the city is back within its limit.
//!
//! # Providing
//! People work for the city's stores as well as for their own hunger: when
//! the stores hold less than [`ProvisionRule::reserve_days`] of food, every
//! action that serves food scores as if they were that hungry (see
//! [`ProvisionRule`]). Nobody farms in a season when no field grows.

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
use crate::economy::{Larder, Outlook};
use crate::map::{Map, Position};
use crate::tech::{Knowledge, Site, TechId, TechTree};
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
    provision: ProvisionRule,
    organisation: OrganisationRule,
    catalog: BTreeMap<ActionKind, ActionSpec>,
}

/// Working for the stores: with no food in store, food work is as urgent
/// as `urgency`; with `reserve_days` per head or more, not at all; linear
/// in between.
#[derive(Debug, Clone, Copy, PartialEq, Eq, serde::Deserialize)]
#[serde(deny_unknown_fields)]
pub struct ProvisionRule {
    pub reserve_days: u32,
    pub urgency: u16,
}

impl ProvisionRule {
    /// The food urgency the stores of a city with `food_days` per head add.
    pub fn urgency(&self, food_days: u32) -> u16 {
        let reserve = self.reserve_days.max(1);
        let short = reserve.saturating_sub(food_days);
        (u32::from(self.urgency) * short / reserve) as u16
    }
}

/// How many people a city's social organisation holds together, and how
/// hard the overflow pushes people out.
#[derive(Debug, Clone, PartialEq, Eq, serde::Deserialize)]
#[serde(deny_unknown_fields)]
pub struct OrganisationRule {
    /// The city's limit is the largest `people` of the levels it qualifies
    /// for (it knows any of their `techs_any`; none: every city).
    pub levels: Vec<OrganisationLevel>,
    /// The chance, per mille, that a decision of an adult in an overfull
    /// city considers settling away, per 1000 per mille the city is above
    /// its limit (twice its limit: `urge` in full)...
    pub urge: u32,
    /// ...at most this.
    pub max_urge: u32,
    /// Added to the score of settling away for those who consider it, per
    /// 1000 per mille above the limit...
    pub pull: u32,
    /// ...at most this.
    pub max_pull: u32,
}

#[derive(Debug, Clone, PartialEq, Eq, serde::Deserialize)]
#[serde(deny_unknown_fields)]
pub struct OrganisationLevel {
    pub name: String,
    #[serde(default)]
    pub techs_any: Vec<String>,
    pub people: u32,
}

impl OrganisationRule {
    /// The most people a city knowing `knowledge` holds together.
    pub fn limit(&self, tree: &TechTree, knowledge: &Knowledge) -> u32 {
        self.levels
            .iter()
            .filter(|level| {
                level.techs_any.is_empty()
                    || level
                        .techs_any
                        .iter()
                        .filter_map(|t| tree.id(t))
                        .any(|t| knowledge.knows(t))
            })
            .map(|level| level.people)
            .max()
            .unwrap_or(u32::MAX)
    }

    /// How far `residents` are above `limit`, per mille of the limit.
    pub fn crowding(residents: u32, limit: u32) -> u32 {
        let over = u64::from(residents.saturating_sub(limit));
        (over * 1000 / u64::from(limit.max(1))).min(u64::from(u32::MAX)) as u32
    }

    /// The chance per mille that a decision at a crowding of `crowding`
    /// per mille considers settling away.
    pub fn urge(&self, crowding: u32) -> u32 {
        (u64::from(crowding) * u64::from(self.urge) / 1000).min(u64::from(self.max_urge)) as u32
    }

    /// What a crowding of `crowding` per mille adds to settling away.
    pub fn pull(&self, crowding: u32) -> u32 {
        (u64::from(crowding) * u64::from(self.pull) / 1000).min(u64::from(self.max_pull)) as u32
    }
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
    pub provision: ProvisionRule,
    pub organisation: OrganisationRule,
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
            provision: section.provision,
            organisation: section.organisation,
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
    free_farm: u32,
    free_pasture: u32,
    food_days: u32,
    water: bool,
    off_season: bool,
    /// How hungry the stores make food work (see [`ProvisionRule`]).
    provision: u16,
    /// Per mille above the organisation limit.
    crowding: u32,
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
    cities: Query<(&CityId, &Position, &Outlook, Option<&Larder>, &Knowledge)>,
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
    for (id, position, outlook, larder, knowledge) in &cities {
        let i = id.0 as usize;
        if places.len() <= i {
            places.resize(i + 1, Place::default());
        }
        let site = Site::of(&map, &biomes, map.index(*position));
        places[i] = Place {
            free_land: outlook.free_land,
            free_farm: outlook.free_farm,
            free_pasture: outlook.free_pasture,
            food_days: outlook.food_days,
            water: site.river || site.lake || site.coast,
            off_season: outlook.off_season,
            provision: catalog.provision.urgency(outlook.food_days),
            crowding: OrganisationRule::crowding(
                larder.map_or(0, |l| l.residents),
                catalog.organisation.limit(&tree, knowledge),
            ),
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

    // Only adults who might act this turn: everyone in the morning (to
    // decide, or to take a longer action up again); later in the day only
    // those whose action has ended or who have a new urgent need. On most
    // turns that is nearly nobody.
    let adult_age = founding.adult_age;
    let tpy = context.turns_per_year;
    let morning = context.morning;
    let mut rows: Vec<Row> = people
        .iter_mut()
        .filter(|(_, person, _, _, _, _, needs, activity, _, plan)| {
            activity.until != u64::MAX
                && person.age_years(now, tpy) >= adult_age
                && (morning
                    || activity.until <= now
                    || catalog.urgent_needs(needs) & !plan.urgent != 0)
        })
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
    parallel::for_each(&mut rows, *threads, |row| step(row, &context));
    drop(rows);

    // Children follow the decisions just made. They are due once a day.
    let child_due = |person: &Person, activity: &Activity| {
        activity.until <= now && person.age_years(now, tpy) < adult_age
    };
    if !people
        .iter()
        .any(|(_, person, .., activity, _, _)| child_due(person, activity))
    {
        return;
    }
    let mut actions: Vec<(PersonId, ActionKind)> = people
        .iter()
        .map(|(id, .., activity, _, _)| (*id, activity.action))
        .collect();
    actions.sort_unstable_by_key(|a| a.0);
    let day_end = now - now % TURNS_PER_DAY + TURNS_PER_DAY - 1;
    for (_, person, _, _, _, family, _, mut activity, _, _) in &mut people {
        if !child_due(person, &activity) {
            continue;
        }
        let mother = family
            .mother
            .and_then(|m| actions.binary_search_by_key(&m, |&(id, _)| id).ok())
            .map(|i| actions[i].1);
        let action = match mother {
            Some(a) if catalog.get(a).is_some_and(|a| a.children) => a,
            _ => ActionKind::Idle,
        };
        *activity = Activity {
            action,
            since: now,
            until: day_end,
        };
    }
}

/// The room a city has for a land action: fields for farming, pasture for
/// herding (so free pasture never sends people to full fields), any land
/// for the rest.
fn room_for(kind: ActionKind, place: &Place) -> u32 {
    match kind {
        ActionKind::Farm => place.free_farm,
        ActionKind::Herd => place.free_pasture,
        _ => place.free_land,
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
    // In an overfull city some decisions consider leaving, more the fuller
    // it is (the same draw, mixed with a key no action uses).
    let crowded = place.crowding > 0
        && rng::mix(draw, ACTIONS.len() as u64) % 1000
            < u64::from(catalog.organisation.urge(place.crowding));

    let mut best = (i64::MIN, ActionKind::Idle);
    for (k, action) in catalog.actions.iter().enumerate() {
        let kind = action.kind;
        let open = age >= action.min_age
            && context.statuses.allows(row.status, kind)
            && (action.techs_any.is_empty()
                || context.techs[k].iter().any(|&t| row.skills.0.knows(t)))
            && (!action.land || room_for(kind, &place) > 0 || current == kind || last == kind)
            && (!action.water || place.water)
            && (kind != ActionKind::Farm || !place.off_season)
            && (!action.unpartnered || row.family.partner.is_none())
            && (!action.parents || row.family.births > 0)
            && (!action.settle_away
                || crowded
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
            .enumerate()
            .map(|(n, (&u, s))| {
                let u = if n == Need::Food as usize {
                    u.max(place.provision)
                } else {
                    u
                };
                i64::from(u) * i64::from(s)
            })
            .sum::<i64>()
            / 1000;
        let pull = if action.settle_away && crowded {
            i64::from(catalog.organisation.pull(place.crowding))
        } else {
            0
        };
        let character: i64 = traits
            .iter()
            .zip(action.traits)
            .map(|(&t, w)| i64::from(w) * (i64::from(t) - 128) / 128)
            .sum();
        let score = needs
            + pull
            + character
            + i64::from(context.statuses.leaning(row.status, kind))
            + (rng::mix(draw, k as u64) % noise) as i64;
        if score > best.0 {
            best = (score, kind);
        }
    }
    best.1
}
