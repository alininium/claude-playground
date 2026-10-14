//! People: every human is an entity of their own.
//!
//! A person has a sex, a birth date, a home city (whose culture they carry),
//! a character ([`Traits`]), [`Needs`], the technologies they know
//! personally ([`Skills`]), a [`Status`], a [`Family`], what they are doing
//! ([`Activity`]) and their own random stream ([`rng::PersonRng`]).
//!
//! # How a turn works
//! People are simulated in two kinds of steps:
//! - **Per-person steps** (needs, decisions) read only the person and shared
//!   read-only data, and write only the person. They may run in parallel
//!   ([`parallel::for_each`]) and give the same result on any number of
//!   threads.
//! - **Resolve steps** (land and food, crafting, mating, statuses, caravans)
//!   involve several people. They go through cities and people in id order.
//!
//! The schedule (see `Simulation::from_world`) runs, after the world:
//! `needs → decide → economy → tools → life → status → caravan → census`.
//! Each module exposes `systems()` for its slot and `install()` for its
//! resources, so modules can grow without touching each other.
//!
//! # Cost
//! Nothing here should do work for people who have nothing to do: decisions
//! happen when an activity ends (see [`Activity::until`]), needs can be
//! brought up to date lazily, and young children follow their mothers.

pub mod decide;
pub mod founding;
pub mod life;
pub mod needs;
pub use crate::parallel;
pub mod rng;
pub mod status;

use bevy_ecs::prelude::*;

use crate::caravan::Caravan;
use crate::city::{City, CityId, Population};
use crate::events::{EventLog, SimEvent};
use crate::map::Position;
use crate::tech::Knowledge;
use crate::time::Turn;
use rng::PersonRng;

/// Stable, never reused identifier of a person, in order of birth (or of
/// appearance at world creation). Everything order-sensitive sorts by it.
#[derive(
    Component,
    Debug,
    Clone,
    Copy,
    PartialEq,
    Eq,
    Hash,
    PartialOrd,
    Ord,
    serde::Serialize,
    serde::Deserialize,
)]
pub struct PersonId(pub u64);

/// The next [`PersonId`] to hand out.
#[derive(Resource, Debug, Default, Clone, Copy, PartialEq, Eq)]
pub struct NextPersonId(pub u64);

impl NextPersonId {
    pub fn take(&mut self) -> PersonId {
        let id = PersonId(self.0);
        self.0 += 1;
        id
    }
}

#[derive(
    Debug, Clone, Copy, PartialEq, Eq, Hash, PartialOrd, Ord, serde::Serialize, serde::Deserialize,
)]
pub enum Sex {
    Female,
    Male,
}

/// Who a person is.
#[derive(Component, Debug, Clone, Copy, PartialEq, Eq)]
pub struct Person {
    pub sex: Sex,
    /// Turn of birth; negative for people born before the simulation began.
    pub born_on: i64,
    /// The city (or tribe) they live in.
    pub home: CityId,
    /// The city whose culture they grew up in. Usually `home`; differs for
    /// migrants and for cities founded by caravans.
    pub culture: CityId,
}

impl Person {
    /// Age in whole years at `turn`.
    pub fn age_years(&self, turn: u64, turns_per_year: u64) -> u32 {
        ((turn as i64 - self.born_on).max(0) as u64 / turns_per_year.max(1)) as u32
    }
}

/// Character, each trait 0–255 with 128 average: the Big Five plus boldness
/// (appetite for risk).
#[derive(Component, Debug, Clone, Copy, PartialEq, Eq, Hash)]
pub struct Traits {
    pub openness: u8,
    pub conscientiousness: u8,
    pub extraversion: u8,
    pub agreeableness: u8,
    pub neuroticism: u8,
    pub boldness: u8,
}

impl Traits {
    pub fn as_array(&self) -> [u8; 6] {
        [
            self.openness,
            self.conscientiousness,
            self.extraversion,
            self.agreeableness,
            self.neuroticism,
            self.boldness,
        ]
    }

    pub fn from_array(a: [u8; 6]) -> Self {
        Self {
            openness: a[0],
            conscientiousness: a[1],
            extraversion: a[2],
            agreeableness: a[3],
            neuroticism: a[4],
            boldness: a[5],
        }
    }

    /// A random character: each trait the sum of four uniform draws, so
    /// roughly bell-shaped around 128.
    pub fn random(rng: &mut PersonRng) -> Self {
        Self::from_array(std::array::from_fn(|_| {
            (0..4).map(|_| rng.below(64)).sum::<u64>() as u8
        }))
    }

    /// A child's character: each trait the parents' average plus a random
    /// deviation of up to ±48 (from the child's own stream).
    pub fn inherit(mother: &Traits, father: &Traits, rng: &mut PersonRng) -> Self {
        let (m, f) = (mother.as_array(), father.as_array());
        Self::from_array(std::array::from_fn(|k| {
            let mid = (i64::from(m[k]) + i64::from(f[k])) / 2;
            (mid + rng.range(-48, 48)).clamp(0, 255) as u8
        }))
    }
}

/// The needs a person tries to satisfy.
#[derive(
    Debug, Clone, Copy, PartialEq, Eq, Hash, PartialOrd, Ord, serde::Serialize, serde::Deserialize,
)]
#[serde(rename_all = "snake_case")]
pub enum Need {
    Food,
    Rest,
    Warmth,
    Safety,
    Company,
    Fun,
    Esteem,
    Family,
}

impl Need {
    pub const COUNT: usize = 8;
    pub const ALL: [Need; Need::COUNT] = [
        Need::Food,
        Need::Rest,
        Need::Warmth,
        Need::Safety,
        Need::Company,
        Need::Fun,
        Need::Esteem,
        Need::Family,
    ];
}

/// How urgent each need is: 0 fully satisfied, 1000 desperate.
#[derive(Component, Debug, Clone, Copy, PartialEq, Eq, Default)]
pub struct Needs {
    pub urgency: [u16; Need::COUNT],
}

impl Needs {
    pub fn get(&self, need: Need) -> u16 {
        self.urgency[need as usize]
    }

    pub fn set(&mut self, need: Need, value: u16) {
        self.urgency[need as usize] = value.min(1000);
    }

    /// Lowers the urgency of `need` by `amount` (not below 0).
    pub fn satisfy(&mut self, need: Need, amount: u16) {
        let u = &mut self.urgency[need as usize];
        *u = u.saturating_sub(amount);
    }

    /// Raises the urgency of `need` by `amount` (not above 1000).
    pub fn grow(&mut self, need: Need, amount: u16) {
        let u = &mut self.urgency[need as usize];
        *u = (*u + amount.min(1000)).min(1000);
    }
}

/// Parents, partner, and how many children this person has borne (mothers)
/// or fathered. A child's seed depends on its mother's `births` (see
/// [`rng::child_seed`]).
#[derive(Component, Debug, Clone, Copy, PartialEq, Eq, Default)]
pub struct Family {
    pub mother: Option<PersonId>,
    pub father: Option<PersonId>,
    pub partner: Option<PersonId>,
    pub births: u32,
}

/// The technologies (and variants) this person knows. A city's
/// [`Knowledge`] is what its people know between them.
#[derive(Component, Debug, Clone, PartialEq, Eq)]
pub struct Skills(pub Knowledge);

/// Index into the status catalog (see `status`). 0 is the plain member of
/// the community that every culture has.
#[derive(
    Debug,
    Clone,
    Copy,
    PartialEq,
    Eq,
    Hash,
    PartialOrd,
    Ord,
    Default,
    serde::Serialize,
    serde::Deserialize,
)]
pub struct StatusId(pub u16);

#[derive(Component, Debug, Clone, Copy, PartialEq, Eq, Default)]
pub struct Status(pub StatusId);

/// What a person can spend their time on. The decision step picks one (see
/// `decide`); the resolve steps act on them: `economy` on food work,
/// `tools` on crafting, `life` on courting and child care, `caravan` on
/// settling away. Their tuning lives in `data/people.json`.
#[derive(
    Debug,
    Clone,
    Copy,
    PartialEq,
    Eq,
    Hash,
    PartialOrd,
    Ord,
    Default,
    serde::Serialize,
    serde::Deserialize,
)]
#[serde(rename_all = "snake_case")]
pub enum ActionKind {
    #[default]
    Idle,
    Sleep,
    Rest,
    Forage,
    Farm,
    Herd,
    Fish,
    Craft,
    Build,
    Socialize,
    Worship,
    Court,
    CareForChildren,
    SettleAway,
}

/// What a person is doing, since when, and until when: they decide again on
/// turn `until` (or earlier if a need becomes urgent).
#[derive(Component, Debug, Clone, Copy, PartialEq, Eq, Default)]
pub struct Activity {
    pub action: ActionKind,
    pub since: u64,
    pub until: u64,
}

/// A snapshot of one person, for display and analysis.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct PersonSnapshot {
    pub id: PersonId,
    pub person: Person,
    pub traits: Traits,
    pub needs: Needs,
    pub family: Family,
    pub skills: Knowledge,
    pub status: StatusId,
    pub activity: Activity,
    pub rng: PersonRng,
}

/// Everything a new person is made of, for spawning.
#[derive(Bundle)]
pub struct PersonBundle {
    pub equipment: crate::tools::Equipment,
    pub id: PersonId,
    pub person: Person,
    pub traits: Traits,
    pub needs: Needs,
    pub family: Family,
    pub skills: Skills,
    pub status: Status,
    pub activity: Activity,
    pub rng: PersonRng,
}

/// The built-in `data/people.json`.
pub const PEOPLE_JSON: &str = include_str!("../../data/people.json");

/// Parses section `name` of the built-in `data/people.json`.
pub fn section<T: serde::de::DeserializeOwned>(name: &str) -> T {
    let file: serde_json::Value =
        serde_json::from_str(PEOPLE_JSON).expect("data/people.json is valid JSON");
    serde_json::from_value(file[name].clone())
        .unwrap_or_else(|e| panic!("data/people.json, section {name}: {e}"))
}

/// Inserts the resources the people modules need.
pub fn install(world: &mut World) {
    world.init_resource::<NextPersonId>();
    world.init_resource::<parallel::Threads>();
    world.insert_resource(founding::FoundingConfig::builtin());
    needs::install(world);
    decide::install(world);
    life::install(world);
    status::install(world);
    crate::economy::install(world);
    crate::tools::install(world);
    crate::caravan::install(world);
}

/// The last step of every turn: carries out deaths owed by disasters (see
/// [`Population::pending_deaths`]) and counts each city's people. A city
/// left with nobody, and with no caravan of its own still on the road
/// (they come back to it if they give up), is abandoned: it disappears and
/// its land is free for others.
///
/// Who dies is decided by a hash of each resident's seed and the turn, so it
/// is deterministic and nobody's own random stream is touched.
pub fn census(
    mut commands: Commands,
    turn: Res<Turn>,
    people: Query<(Entity, &PersonId, &Person, &PersonRng)>,
    mut cities: Query<(Entity, &CityId, &mut Population, &City, &Position)>,
    caravans: Query<&Caravan>,
    mut log: ResMut<EventLog>,
) {
    let slots = cities
        .iter()
        .map(|(_, id, ..)| id.0 as usize + 1)
        .max()
        .unwrap_or(0);
    let mut counts = vec![0u64; slots];
    for (_, _, person, _) in &people {
        if let Some(c) = counts.get_mut(person.home.0 as usize) {
            *c += 1;
        }
    }
    let mut abandoned = Vec::new();
    for (entity, city, mut population, info, position) in &mut cities {
        let mut count = counts[city.0 as usize];
        if population.pending_deaths > 0 {
            let mut residents: Vec<(u64, PersonId, Entity)> = people
                .iter()
                .filter(|(_, _, p, _)| p.home == *city)
                .map(|(e, id, _, rng)| (rng::mix(rng.seed, turn.0 ^ 0xDEAD), *id, e))
                .collect();
            residents.sort();
            let deaths = population.pending_deaths.min(count);
            for &(_, _, entity) in residents.iter().take(deaths as usize) {
                commands.entity(entity).despawn();
            }
            count -= deaths;
            population.pending_deaths = 0;
        }
        if population.count != count {
            population.count = count;
        }
        if count == 0 && !caravans.iter().any(|c| c.origin == *city) {
            abandoned.push((*city, entity, info.name.clone(), *position));
        }
    }
    abandoned.sort_by_key(|a| a.0);
    for (id, entity, name, position) in abandoned {
        log.push(SimEvent::CityAbandoned {
            turn: turn.0,
            id,
            name,
            position,
        });
        commands.entity(entity).despawn();
    }
}

/// Feeds every person's state, in id order, to `write` (for digests).
pub fn digest(world: &mut World, write: &mut impl FnMut(u64)) {
    let mut people = people(world);
    people.sort_by_key(|p| p.id);
    for p in people {
        write(p.id.0);
        write(p.person.sex as u64);
        write(p.person.born_on as u64);
        write(u64::from(p.person.home.0) << 32 | u64::from(p.person.culture.0));
        write(u64::from_le_bytes({
            let mut b = [0u8; 8];
            b[..6].copy_from_slice(&p.traits.as_array());
            b
        }));
        for u in p.needs.urgency {
            write(u64::from(u));
        }
        let f = p.family;
        for v in [f.mother, f.father, f.partner] {
            write(v.map_or(u64::MAX, |id| id.0));
        }
        write(u64::from(f.births));
        p.skills.digest(&mut *write);
        write(u64::from(p.status.0));
        write(p.activity.action as u64);
        write(p.activity.since);
        write(p.activity.until);
        write(p.rng.seed);
        write(p.rng.counter);
    }
}

/// Snapshots of everyone alive, in id order.
pub fn people(world: &mut World) -> Vec<PersonSnapshot> {
    let mut query = world.query::<(
        &PersonId,
        &Person,
        &Traits,
        &Needs,
        &Family,
        &Skills,
        &Status,
        &Activity,
        &PersonRng,
    )>();
    let mut out: Vec<PersonSnapshot> = query
        .iter(world)
        .map(
            |(id, person, traits, needs, family, skills, status, activity, rng)| PersonSnapshot {
                id: *id,
                person: *person,
                traits: *traits,
                needs: *needs,
                family: *family,
                skills: skills.0.clone(),
                status: status.0,
                activity: *activity,
                rng: *rng,
            },
        )
        .collect();
    out.sort_by_key(|p| p.id);
    out
}
