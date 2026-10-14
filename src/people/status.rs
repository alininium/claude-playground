//! Statuses: the positions a culture offers (elder, chief, priest, scribe,
//! artisan, warrior, bonded...), unlocked by the city's technologies, and
//! who holds them. Also personal knowledge over time: learning from parents
//! and teachers, and a city knowing a technology only while someone in it
//! does.
//!
//! Three kinds of work, each on its own clock per city (see
//! [`StatusSchedule`]), so that a turn with nothing due costs a glance at
//! the cities:
//! - **Review** (seasonal): who holds which status. A resolve step that goes
//!   through the city's people in id order: holders who no longer qualify
//!   step down, and vacant positions go to the best-placed candidates. This
//!   is also how young people come into a position after they come of age.
//! - **Learning** (monthly): children pick up their parents' skills,
//!   adults what others in the city know, more readily the more open they
//!   are. Every learner decides from the same snapshot and draws only from
//!   their own stream, so this part runs in parallel.
//! - **Knowledge** (yearly): the city's [`Knowledge`] becomes what its
//!   people know between them, so a technology whose last knower died is
//!   lost.
//!
//! Every morning each person's Esteem need is eased by the standing of
//! their status.
//!
//! Tuning: the `status` section of `data/people.json`. Statuses are listed
//! there in order of precedence (the first is the ordinary member, then the
//! highest positions); a person holds one status at a time.

use std::collections::BTreeMap;

use bevy_ecs::prelude::*;
use bevy_ecs::schedule::ScheduleConfigs;
use bevy_ecs::system::ScheduleSystem;

use super::parallel::{self, Threads};
use super::rng::{PersonRng, mix};
use super::{
    ActionKind, Activity, Family, Need, Needs, Person, PersonId, Sex, Skills, Status, StatusId,
    Traits,
};
use crate::city::CityId;
use crate::config::SimConfig;
use crate::tech::{Knowledge, TechId, TechTree, VariantId};
use crate::time::{TURNS_PER_DAY, Turn};

/// How many kinds of action there are (the length of per-action tables).
pub const ACTIONS: usize = ActionKind::SettleAway as usize + 1;

/// Inserts this module's resources (its section of `data/people.json`).
pub fn install(world: &mut World) {
    world.insert_resource(StatusCatalog::builtin());
}

/// This module's systems, in order, for its slot in the turn.
pub fn systems() -> ScheduleConfigs<ScheduleSystem> {
    statuses_and_learning.into_configs()
}

// ---------------------------------------------------------------------------
// The catalog
// ---------------------------------------------------------------------------

/// The `status` section of `data/people.json`.
#[derive(Debug, Clone, Default, PartialEq, Eq, serde::Deserialize)]
#[serde(deny_unknown_fields)]
pub struct StatusConfig {
    /// Age from which people can hold a status (other than member) and
    /// learn from the whole city rather than from their parents.
    pub adult_age: u32,
    /// Age from which children start to pick up their parents' skills.
    pub learn_from_age: u32,
    /// Days between reviews of who holds which status.
    pub review_days: u32,
    /// Days between recomputations of what the city knows.
    pub knowledge_days: u32,
    /// Days between learning rounds.
    pub learn_days: u32,
    /// Per mille chance per round that a child learns one of a parent's
    /// skills.
    pub child_learning: u32,
    /// Per mille chance per round that an adult with openness 0 learns a
    /// technology known in the city...
    pub adult_learning: u32,
    /// ...plus this much more at openness 255 (in proportion between).
    pub openness_learning: u32,
    /// How much a day eases the Esteem need for a status of esteem 1000 (in
    /// proportion for lower esteem).
    pub esteem_relief: u16,
    pub statuses: Vec<StatusDef>,
    #[serde(default, rename = "notes")]
    _notes: Vec<String>,
}

/// One status of the catalog.
#[derive(Debug, Clone, Default, PartialEq, Eq, serde::Deserialize)]
#[serde(deny_unknown_fields)]
pub struct StatusDef {
    pub id: String,
    pub name: String,
    /// The city must know at least one of these technologies (empty: every
    /// culture has it).
    #[serde(default)]
    pub unlocked_by: Vec<String>,
    /// ...and none of these (a chief gives way to a king, a shaman to a
    /// priesthood).
    #[serde(default)]
    pub lost_with: Vec<String>,
    /// The holder must personally know at least one of these.
    #[serde(default)]
    pub skills_any: Vec<String>,
    /// A fixed number of positions...
    #[serde(default)]
    pub count: Option<u32>,
    /// ...or so many per mille of the city's adults (rounded).
    #[serde(default)]
    pub per_mille: Option<u32>,
    #[serde(default)]
    pub min_age: u32,
    #[serde(default)]
    pub max_age: Option<u32>,
    #[serde(default)]
    pub sex: Option<SexFilter>,
    /// Weight of each trait (0–255) in the choice among candidates, per
    /// mille; negative weights prefer a low trait.
    #[serde(default)]
    pub prefer: BTreeMap<TraitKind, i32>,
    /// Points per year of age in the choice among candidates.
    #[serde(default)]
    pub per_year_of_age: i32,
    /// Points for a candidate whose mother or father holds (or held at the
    /// last review) this status.
    #[serde(default)]
    pub inherited: i32,
    /// Weight of a candidate's fortune: a fixed number from 0 to 999 per
    /// person and status, from a hash of their seed, scaled by this per
    /// mille.
    #[serde(default)]
    pub luck: i32,
    /// Holders keep it for life (unless its technology is lost): they are
    /// never taken for another status.
    #[serde(default)]
    pub for_life: bool,
    /// Standing, 0–1000: how much holding it satisfies the Esteem need.
    pub esteem: u16,
    /// Added to the score of these actions for holders.
    #[serde(default)]
    pub leaning: BTreeMap<ActionKind, i32>,
    /// Actions holders may not take up.
    #[serde(default)]
    pub forbidden: Vec<ActionKind>,
}

#[derive(Debug, Clone, Copy, PartialEq, Eq, serde::Deserialize)]
#[serde(rename_all = "snake_case")]
pub enum SexFilter {
    Female,
    Male,
}

/// A trait of [`Traits`], by name.
#[derive(Debug, Clone, Copy, PartialEq, Eq, PartialOrd, Ord, serde::Deserialize)]
#[serde(rename_all = "snake_case")]
pub enum TraitKind {
    Openness,
    Conscientiousness,
    Extraversion,
    Agreeableness,
    Neuroticism,
    Boldness,
}

impl TraitKind {
    fn of(self, traits: &Traits) -> u8 {
        traits.as_array()[self as usize]
    }
}

/// What each status allows, read by `people::decide`, and the rest of this
/// module's tuning. The default (empty) catalog lets every status do
/// everything, with no preference, and assigns nothing.
#[derive(Resource, Debug, Clone, Default)]
pub struct StatusCatalog {
    pub config: StatusConfig,
    leaning: Vec<[i32; ACTIONS]>,
    forbidden: Vec<[bool; ACTIONS]>,
}

impl StatusCatalog {
    /// The catalog in `data/people.json`.
    pub fn builtin() -> Self {
        Self::new(super::section("status"))
    }

    /// Builds the catalog from its configuration. The first status must be
    /// the ordinary member, open to everyone; every other one needs either
    /// a `count` or a `per_mille`.
    pub fn new(config: StatusConfig) -> Self {
        assert!(
            config.statuses.len() <= 64,
            "data/people.json, status: at most 64 statuses"
        );
        for (i, s) in config.statuses.iter().enumerate() {
            if i == 0 {
                assert!(
                    s.unlocked_by.is_empty() && s.skills_any.is_empty(),
                    "data/people.json, status: the first status ({}) must be open to all",
                    s.id
                );
            } else {
                assert!(
                    s.count.is_some() != s.per_mille.is_some(),
                    "data/people.json, status {}: give exactly one of count and per_mille",
                    s.id
                );
            }
        }
        let leaning = config
            .statuses
            .iter()
            .map(|s| {
                let mut row = [0; ACTIONS];
                for (&a, &v) in &s.leaning {
                    row[a as usize] = v;
                }
                row
            })
            .collect();
        let forbidden = config
            .statuses
            .iter()
            .map(|s| {
                let mut row = [false; ACTIONS];
                for &a in &s.forbidden {
                    row[a as usize] = true;
                }
                row
            })
            .collect();
        Self {
            config,
            leaning,
            forbidden,
        }
    }

    /// Whether someone of `status` may take up `action`.
    pub fn allows(&self, status: StatusId, action: ActionKind) -> bool {
        self.forbidden
            .get(usize::from(status.0))
            .is_none_or(|row| !row[action as usize])
    }

    /// How much more (or less) someone of `status` leans toward `action`,
    /// added to its score (0 neutral).
    pub fn leaning(&self, status: StatusId, action: ActionKind) -> i32 {
        self.leaning
            .get(usize::from(status.0))
            .map_or(0, |row| row[action as usize])
    }

    pub fn len(&self) -> usize {
        self.config.statuses.len()
    }

    pub fn is_empty(&self) -> bool {
        self.config.statuses.is_empty()
    }

    /// The status with this id in the data (such as `"chief"`).
    pub fn id(&self, name: &str) -> Option<StatusId> {
        self.config
            .statuses
            .iter()
            .position(|s| s.id == name)
            .map(|i| StatusId(i as u16))
    }

    pub fn status(&self, id: StatusId) -> Option<&StatusDef> {
        self.config.statuses.get(usize::from(id.0))
    }

    /// How much holding `id` satisfies the Esteem need, 0–1000.
    pub fn esteem(&self, id: StatusId) -> u16 {
        self.status(id).map_or(0, |s| s.esteem)
    }

    /// Whether a city that knows `knowledge` offers `id`.
    pub fn unlocked(&self, tree: &TechTree, knowledge: &Knowledge, id: StatusId) -> bool {
        self.status(id).is_some_and(|s| {
            let knows = |name: &String| tree.id(name).is_some_and(|t| knowledge.knows(t));
            (s.unlocked_by.is_empty() || s.unlocked_by.iter().any(knows))
                && !s.lost_with.iter().any(knows)
        })
    }

    /// Every technology id the catalog names that `tree` does not have.
    pub fn unknown_technologies(&self, tree: &TechTree) -> Vec<String> {
        self.config
            .statuses
            .iter()
            .flat_map(|s| s.unlocked_by.iter().chain(&s.lost_with).chain(&s.skills_any))
            .filter(|name| tree.id(name).is_none())
            .cloned()
            .collect()
    }
}

// ---------------------------------------------------------------------------
// Per-city clocks
// ---------------------------------------------------------------------------

/// This module's bookkeeping on each city: when its next review and
/// knowledge recomputation are due, and who held each status at the last
/// review (so that a child can inherit a position from a parent who has
/// died since). Added to a city the first turn it is seen, which reviews
/// it and recomputes its knowledge at once.
#[derive(Component, Debug, Clone, PartialEq, Eq, Default)]
pub struct StatusSchedule {
    pub next_review: u64,
    pub next_knowledge: u64,
    /// Holders of each status at the last review, sorted (index 0, the
    /// member, is left empty).
    pub holders: Vec<Vec<PersonId>>,
}

// ---------------------------------------------------------------------------
// The step
// ---------------------------------------------------------------------------

struct Due {
    city: Entity,
    id: CityId,
    review: bool,
    knowledge: bool,
    learn: bool,
}

/// What a review and a learning round need to know about one person.
struct Resident {
    entity: Entity,
    id: PersonId,
    age: u32,
    sex: Sex,
    traits: Traits,
    family: Family,
    status: u16,
    seed: u64,
    /// Not travelling with a caravan.
    available: bool,
}

/// The whole module's turn (see the module docs).
fn statuses_and_learning(world: &mut World) {
    world.resource_scope(|world, catalog: Mut<StatusCatalog>| {
        if catalog.is_empty() {
            return;
        }
        world.resource_scope(|world, tree: Mut<TechTree>| step(world, &catalog, &tree));
    });
}

fn step(world: &mut World, catalog: &StatusCatalog, tree: &TechTree) {
    let cfg = &catalog.config;
    let turn = world.resource::<Turn>().0;
    let tpy = world.resource::<SimConfig>().turns_per_year();
    let threads = *world.resource::<Threads>();
    let morning = turn % TURNS_PER_DAY == 0;
    let day = turn / TURNS_PER_DAY;

    // Which cities have something due this turn.
    let mut fresh = Vec::new();
    let mut due = Vec::new();
    let mut cities = world.query::<(Entity, &CityId, Option<&StatusSchedule>)>();
    for (city, &id, schedule) in cities.iter(world) {
        let learn = morning && (day + u64::from(id.0)) % u64::from(cfg.learn_days.max(1)) == 0;
        let (review, knowledge) = match schedule {
            None => {
                fresh.push(city);
                (true, true)
            }
            Some(s) => (s.next_review <= turn, s.next_knowledge <= turn),
        };
        if review || knowledge || learn {
            due.push(Due {
                city,
                id,
                review,
                knowledge,
                learn,
            });
        }
    }
    for city in fresh {
        world.entity_mut(city).insert(StatusSchedule {
            next_review: turn,
            next_knowledge: turn,
            holders: Vec::new(),
        });
    }
    due.sort_by_key(|d| d.id);

    if !due.is_empty() {
        let slot: BTreeMap<CityId, usize> = due.iter().enumerate().map(|(i, d)| (d.id, i)).collect();
        let mut residents: Vec<Vec<Resident>> = due.iter().map(|_| Vec::new()).collect();
        let mut people = world.query::<(
            Entity,
            &PersonId,
            &Person,
            &Traits,
            &Family,
            &Status,
            &Activity,
            &PersonRng,
        )>();
        for (entity, &id, person, traits, family, status, activity, rng) in people.iter(world) {
            if let Some(&i) = slot.get(&person.home) {
                residents[i].push(Resident {
                    entity,
                    id,
                    age: person.age_years(turn, tpy),
                    sex: person.sex,
                    traits: *traits,
                    family: *family,
                    status: status.0.0,
                    seed: rng.seed,
                    available: activity.until != u64::MAX,
                });
            }
        }
        for (d, mut list) in due.into_iter().zip(residents) {
            list.sort_by_key(|r| r.id);
            if d.knowledge {
                recompute_knowledge(world, tree, d.city, &list);
                let next = turn + u64::from(cfg.knowledge_days.max(1)) * TURNS_PER_DAY;
                world.get_mut::<StatusSchedule>(d.city).unwrap().next_knowledge = next;
            }
            if d.learn {
                learn(world, cfg, tree, d.city, &list, threads);
            }
            if d.review {
                review(world, catalog, tree, &d, &list, turn);
            }
        }
    }

    if morning && cfg.esteem_relief > 0 {
        let relief: Vec<u16> = cfg
            .statuses
            .iter()
            .map(|s| (u32::from(s.esteem.min(1000)) * u32::from(cfg.esteem_relief) / 1000) as u16)
            .collect();
        let mut people = world.query::<(&Status, &mut Needs)>();
        for (status, mut needs) in people.iter_mut(world) {
            let r = relief.get(usize::from(status.0.0)).copied().unwrap_or(0);
            if r > 0 && needs.get(Need::Esteem) > 0 {
                needs.satisfy(Need::Esteem, r);
            }
        }
    }
}

/// The city knows what its present people know between them: travellers
/// have left, and the dead take their knowledge with them.
fn recompute_knowledge(world: &mut World, tree: &TechTree, city: Entity, residents: &[Resident]) {
    let mut known = Knowledge::new(tree);
    for r in residents.iter().filter(|r| r.available) {
        let skills = &world.get::<Skills>(r.entity).expect("people have skills").0;
        for t in skills.techs() {
            known.learn(tree, t, None);
        }
        for v in skills.variants() {
            known.learn(tree, tree.variant_of(v), Some(v));
        }
    }
    let mut current = world.get_mut::<Knowledge>(city).expect("cities know things");
    if *current != known {
        *current = known;
    }
}

/// One learner's round: decided from a snapshot, so the order of learners
/// (and the number of threads) does not matter.
struct Lesson<'a> {
    rng: PersonRng,
    own: &'a Knowledge,
    /// Who they can learn from: a child's parents, or for an adult the
    /// whole city.
    teachers: [Option<&'a Knowledge>; 2],
    chance: u32,
    learned: Option<(TechId, Vec<VariantId>)>,
}

fn learn(
    world: &mut World,
    cfg: &StatusConfig,
    tree: &TechTree,
    city: Entity,
    residents: &[Resident],
    threads: Threads,
) {
    let pool = world.get::<Knowledge>(city).expect("cities know things").clone();
    let skills: Vec<Knowledge> = residents
        .iter()
        .map(|r| world.get::<Skills>(r.entity).expect("people have skills").0.clone())
        .collect();
    let present = |id: Option<PersonId>| {
        let i = residents.binary_search_by_key(&id?, |r| r.id).ok()?;
        residents[i].available.then_some(&skills[i])
    };
    let mut lessons: Vec<(usize, Lesson)> = residents
        .iter()
        .enumerate()
        .filter(|(_, r)| r.available && r.age >= cfg.learn_from_age)
        .map(|(i, r)| {
            let rng = *world.get::<PersonRng>(r.entity).expect("people have streams");
            let (teachers, chance) = if r.age < cfg.adult_age {
                let parents = [present(r.family.mother), present(r.family.father)];
                (parents, cfg.child_learning)
            } else {
                let openness = u32::from(r.traits.openness);
                let chance = cfg.adult_learning + cfg.openness_learning * openness / 255;
                ([Some(&pool), None], chance)
            };
            let lesson = Lesson {
                rng,
                own: &skills[i],
                teachers,
                chance,
                learned: None,
            };
            (i, lesson)
        })
        .filter(|(_, l)| l.teachers.iter().any(Option::is_some))
        .collect();

    parallel::for_each(&mut lessons, threads, |(_, lesson)| {
        if !lesson.rng.chance_per_mille(lesson.chance) {
            return;
        }
        let teachers: Vec<&Knowledge> = lesson.teachers.iter().flatten().copied().collect();
        let teacher = teachers[lesson.rng.below(teachers.len() as u64) as usize];
        let options: Vec<TechId> = tree
            .order()
            .iter()
            .copied()
            .filter(|&t| {
                teacher.knows(t) && !lesson.own.knows(t) && tree.prerequisites_met(lesson.own, t)
            })
            .collect();
        if options.is_empty() {
            return;
        }
        let t = options[lesson.rng.below(options.len() as u64) as usize];
        let variants = tree.variants(t).filter(|&v| teacher.knows_variant(v)).collect();
        lesson.learned = Some((t, variants));
    });

    for (i, lesson) in lessons {
        let entity = residents[i].entity;
        *world.get_mut::<PersonRng>(entity).unwrap() = lesson.rng;
        if let Some((t, variants)) = lesson.learned {
            let mut skills = world.get_mut::<Skills>(entity).unwrap();
            if variants.is_empty() {
                skills.0.learn(tree, t, None);
            }
            for v in variants {
                skills.0.learn(tree, t, Some(v));
            }
        }
    }
}

/// Who holds which status. Statuses are filled in order of precedence;
/// candidates are ordinary members and holders of lower statuses (unless
/// those are for life), best score first, then lowest id.
fn review(
    world: &mut World,
    catalog: &StatusCatalog,
    tree: &TechTree,
    due: &Due,
    residents: &[Resident],
    turn: u64,
) {
    let cfg = &catalog.config;
    let n = catalog.len();
    let knowledge = world.get::<Knowledge>(due.city).expect("cities know things");
    let unlocked: Vec<bool> = (0..n)
        .map(|s| catalog.unlocked(tree, knowledge, StatusId(s as u16)))
        .collect();
    let skill_lists: Vec<Vec<TechId>> = cfg
        .statuses
        .iter()
        .map(|s| s.skills_any.iter().filter_map(|t| tree.id(t)).collect())
        .collect();
    let schedule = world.get::<StatusSchedule>(due.city).expect("added above");
    let former = schedule.holders.clone();

    // Which statuses each person could hold, as a bit mask.
    let qualified: Vec<u64> = residents
        .iter()
        .map(|r| {
            if !r.available || r.age < cfg.adult_age {
                return 0;
            }
            let skills = &world.get::<Skills>(r.entity).expect("people have skills").0;
            let mut mask = 0u64;
            for (s, def) in cfg.statuses.iter().enumerate().skip(1) {
                let ok = unlocked[s]
                    && r.age >= def.min_age
                    && def.max_age.is_none_or(|m| r.age <= m)
                    && def.sex.is_none_or(|sex| match sex {
                        SexFilter::Female => r.sex == Sex::Female,
                        SexFilter::Male => r.sex == Sex::Male,
                    })
                    && (skill_lists[s].is_empty() || skill_lists[s].iter().any(|&t| skills.knows(t)));
                if ok {
                    mask |= 1 << s;
                }
            }
            mask
        })
        .collect();
    let adults = residents
        .iter()
        .filter(|r| r.available && r.age >= cfg.adult_age)
        .count() as u64;

    let mut status: Vec<u16> = residents.iter().map(|r| r.status).collect();
    let held = |s: usize, id: PersonId, status: &[u16]| {
        former.get(s).is_some_and(|h| h.binary_search(&id).is_ok())
            || residents
                .binary_search_by_key(&id, |r| r.id)
                .is_ok_and(|i| usize::from(status[i]) == s)
    };
    let score = |i: usize, s: usize, status: &[u16]| -> i64 {
        let (r, def) = (&residents[i], &cfg.statuses[s]);
        let mut total: i64 = def
            .prefer
            .iter()
            .map(|(k, &w)| i64::from(k.of(&r.traits)) * i64::from(w) / 1000)
            .sum();
        total += i64::from(r.age) * i64::from(def.per_year_of_age);
        let parents = [r.family.mother, r.family.father];
        if parents.iter().flatten().any(|&p| held(s, p, status)) {
            total += i64::from(def.inherited);
        }
        let fortune = (mix(r.seed, 0x5747_0000 ^ s as u64) % 1000) as i64;
        total + i64::from(def.luck) * fortune / 1000
    };

    // Holders who no longer qualify step down; so do travellers, except
    // from a status held for life.
    for (i, r) in residents.iter().enumerate() {
        let s = usize::from(status[i]);
        if s == 0 || s >= n {
            status[i] = 0;
            continue;
        }
        let keeps = if r.available {
            qualified[i] >> s & 1 == 1
        } else {
            cfg.statuses[s].for_life && unlocked[s]
        };
        if !keeps {
            status[i] = 0;
        }
    }

    for s in 1..n {
        if !unlocked[s] {
            continue;
        }
        let def = &cfg.statuses[s];
        let slots = def
            .count
            .map(u64::from)
            .unwrap_or_else(|| (adults * u64::from(def.per_mille.unwrap_or(0)) + 500) / 1000)
            as usize;
        let rank = |list: &mut Vec<usize>, status: &[u16]| {
            let mut keyed: Vec<(i64, PersonId, usize)> = list
                .iter()
                .map(|&i| (-score(i, s, status), residents[i].id, i))
                .collect();
            keyed.sort();
            *list = keyed.into_iter().map(|(.., i)| i).collect();
        };
        let mut holders: Vec<usize> = (0..residents.len())
            .filter(|&i| usize::from(status[i]) == s && residents[i].available)
            .collect();
        let away = (0..residents.len())
            .filter(|&i| usize::from(status[i]) == s && !residents[i].available)
            .count();
        let room = slots.saturating_sub(away);
        if holders.len() > room {
            rank(&mut holders, &status);
            for &i in &holders[room..] {
                status[i] = 0;
            }
            continue;
        }
        let vacant = room - holders.len();
        if vacant == 0 {
            continue;
        }
        let mut candidates: Vec<usize> = (0..residents.len())
            .filter(|&i| {
                let current = usize::from(status[i]);
                qualified[i] >> s & 1 == 1
                    && (current == 0 || (current > s && !cfg.statuses[current].for_life))
            })
            .collect();
        rank(&mut candidates, &status);
        for &i in candidates.iter().take(vacant) {
            status[i] = s as u16;
        }
    }

    let mut holders = vec![Vec::new(); n];
    for (i, r) in residents.iter().enumerate() {
        if status[i] != 0 {
            holders[usize::from(status[i])].push(r.id);
        }
        if status[i] != r.status {
            world.get_mut::<Status>(r.entity).unwrap().0 = StatusId(status[i]);
        }
    }
    let period = u64::from(cfg.review_days.max(1));
    let mut schedule = world.get_mut::<StatusSchedule>(due.city).unwrap();
    // The first review is the same turn for every city; spread the next
    // ones over the season.
    let stagger = if schedule.holders.is_empty() {
        u64::from(due.id.0) * 7 % period
    } else {
        0
    };
    schedule.next_review = turn + (period + stagger) * TURNS_PER_DAY;
    schedule.holders = holders;
}
