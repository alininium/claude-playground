//! The life cycle: ageing and death (Siler mortality, worsened by hunger
//! and cold), pairing up (in the resolve step, per city, in id order),
//! pregnancy, and births. A child's seed comes from `rng::child_seed`, so
//! the same parents always have the same children; its traits from
//! `Traits::inherit`.
//!
//! Almost everything happens once a day, in the morning turn: one death
//! draw per person from a per-age table computed once, a conception draw for
//! the few women who can conceive, births that are due, and pairing up. The
//! only per-turn work is noting who courts and who cares for children, which
//! is a field comparison per person.
//!
//! Tuning: the `life` section of `data/people.json`.

use bevy_ecs::prelude::*;
use bevy_ecs::schedule::ScheduleConfigs;
use bevy_ecs::system::ScheduleSystem;

use super::rng::{PersonRng, child_seed};
use super::{
    ActionKind, Activity, Family, Need, Needs, NextPersonId, Person, PersonBundle, PersonId, Sex,
    Skills, Status, Traits,
};
use crate::city::CityId;
use crate::config::SimConfig;
use crate::tech::{Category, Knowledge, TechTree};
use crate::time::{TURNS_PER_DAY, Turn};

/// Chances in the mortality tables are per billion per day: per million is
/// too coarse for the young adults' risk (a few dozen per million a day).
pub const PER_BILLION: u64 = 1_000_000_000;

/// The `life` section of `data/people.json`.
#[derive(Resource, Debug, Clone, PartialEq, Eq, serde::Deserialize)]
pub struct LifeConfig {
    pub mortality: MortalityConfig,
    pub pairing: PairingConfig,
    pub fertility: FertilityConfig,
    pub family: FamilyConfig,
}

/// The three Siler hazards, as yearly hazards per million (see
/// [`LifeTables`]), and what makes them worse.
#[derive(Debug, Clone, PartialEq, Eq, serde::Deserialize)]
pub struct MortalityConfig {
    /// Mean hazard over the first year of life.
    pub infant_per_million: u64,
    /// What is left of the infant hazard from one year of age to the next.
    pub infant_decay_per_mille: u64,
    /// The hazard that does not depend on age (accidents, violence, most
    /// infections).
    pub constant_per_million: u64,
    /// The senescent hazard at `senescent_age`...
    pub senescent_age: u32,
    pub senescent_per_million: u64,
    /// ...and its growth from one year of age to the next.
    pub senescent_growth_per_mille: u64,
    /// Nobody lives past this age.
    pub max_age: u32,
    /// Per mille of the infant and constant hazards that is infectious
    /// disease, which the technologies' `disease` effects scale.
    pub disease_share: u64,
    pub hunger: Exposure,
    pub cold: Exposure,
    /// Hazard multiplier, per mille, for an unweaned child whose mother is
    /// dead.
    pub orphan_per_mille: u64,
}

/// A need whose neglect kills: above `threshold` urgency, an extra daily
/// death chance that grows linearly to `extra_per_billion` at 1000.
#[derive(Debug, Clone, Copy, PartialEq, Eq, serde::Deserialize)]
pub struct Exposure {
    pub threshold: u16,
    pub extra_per_billion: u64,
}

impl Exposure {
    fn extra(&self, urgency: u16) -> u64 {
        if urgency <= self.threshold || self.threshold >= 1000 {
            return 0;
        }
        self.extra_per_billion * u64::from(urgency - self.threshold)
            / u64::from(1000 - self.threshold)
    }
}

#[derive(Debug, Clone, PartialEq, Eq, serde::Deserialize)]
pub struct PairingConfig {
    pub female_age: [u32; 2],
    pub male_age: [u32; 2],
    /// Daily chance, per mille, that an unpartnered woman looks for a
    /// partner; `court_chance` while she courts.
    pub daily_chance: u32,
    pub court_chance: u32,
    /// The age gap (man older) liked best, and the widest allowed.
    pub preferred_gap: i32,
    pub max_gap: i32,
    /// How many years younger than the woman a man may be.
    pub max_younger: i32,
    /// Compatibility lost per year away from the preferred gap.
    pub gap_penalty: i32,
    /// Compatibility lost per point of mean difference in openness,
    /// conscientiousness and extraversion, per mille.
    pub trait_penalty: i32,
    /// Compatibility gained by a man who courts.
    pub court_bonus: i32,
    /// No match below this compatibility (1000 is a perfect match).
    pub min_compatibility: i32,
}

#[derive(Debug, Clone, PartialEq, Eq, serde::Deserialize)]
pub struct FertilityConfig {
    /// Daily chance of conception at the most fertile age, per million.
    pub daily_chance_per_million: u64,
    /// `[lowest age, fecundity per mille of the peak]`; below the first age
    /// and from an entry of 0 on, no conception.
    pub by_age: Vec<[u32; 2]>,
    pub pregnancy_days: u64,
    /// Births come up to this many days early or late.
    pub pregnancy_spread_days: u64,
    /// Infertility after a birth while the child is nursed (lactational
    /// amenorrhea); it ends early if the child dies.
    pub postpartum_days: u64,
    pub twins_per_mille: u32,
    pub maternal_death_per_mille: u32,
    pub male_births_per_mille: u32,
}

impl FertilityConfig {
    /// Fecundity at `age`, per mille of the peak.
    pub fn fecundity(&self, age: u32) -> u64 {
        self.by_age
            .iter()
            .rev()
            .find(|b| b[0] <= age)
            .map_or(0, |b| u64::from(b[1]))
    }
}

/// How much partners and children satisfy the Family need.
#[derive(Debug, Clone, PartialEq, Eq, serde::Deserialize)]
pub struct FamilyConfig {
    pub partner_daily: u16,
    pub children_daily: u16,
    pub care_per_turn: u16,
}

impl LifeConfig {
    pub fn builtin() -> Self {
        super::section("life")
    }
}

/// The daily death chance at each age in whole years, per billion,
/// computed once from [`MortalityConfig`]: the Siler model's infant hazard
/// falling geometrically, a constant hazard, and a senescent hazard growing
/// geometrically (Gompertz), each evaluated per year of age in integers.
#[derive(Resource, Debug, Clone, PartialEq, Eq)]
pub struct LifeTables {
    /// All causes.
    pub daily: Vec<u64>,
    /// The part of `daily` that is infectious disease.
    pub disease: Vec<u64>,
}

impl LifeTables {
    pub fn new(m: &MortalityConfig) -> Self {
        let ages = m.max_age as usize + 1;
        let mut infant = Vec::with_capacity(ages);
        let mut senescent = vec![0u64; ages];
        let mut h = m.infant_per_million * 1000;
        for _ in 0..ages {
            infant.push(h);
            h = h * m.infant_decay_per_mille / 1000;
        }
        // In per billion a year, so the young ages' tiny senescent hazard
        // keeps some precision.
        let reference = (m.senescent_age as usize).min(ages - 1);
        let growth = m.senescent_growth_per_mille.max(1);
        senescent[reference] = m.senescent_per_million * 1000;
        for a in reference + 1..ages {
            senescent[a] = senescent[a - 1] * growth / 1000;
        }
        for a in (0..reference).rev() {
            senescent[a] = senescent[a + 1] * 1000 / growth;
        }
        let days = 364;
        let mut daily: Vec<u64> = (0..ages)
            .map(|a| {
                ((infant[a] + m.constant_per_million * 1000 + senescent[a]) / days).min(PER_BILLION)
            })
            .collect();
        daily[ages - 1] = PER_BILLION;
        let disease = (0..ages)
            .map(|a| (infant[a] + m.constant_per_million * 1000) * m.disease_share / 1000 / days)
            .collect();
        Self { daily, disease }
    }

    /// Life expectancy at `age`, in thousandths of a year, if the table's
    /// chances applied day by day and nothing else (hunger, cold,
    /// technologies) changed them.
    pub fn life_expectancy(&self, age: u32) -> u64 {
        const ONE: u128 = 1 << 64;
        let mut alive = ONE;
        let mut days: u128 = 0;
        for a in (age as usize).min(self.daily.len() - 1)..self.daily.len() {
            let q = u128::from(self.daily[a]);
            for _ in 0..364 {
                days += alive;
                alive -= alive * q / u128::from(PER_BILLION);
            }
        }
        (days * 1000 / 364 / ONE) as u64
    }
}

/// Carrying a child: due on turn `due`, fathered by `father`. The father's
/// seed and character are kept so that a child born after its father died
/// is the same child it would have been.
#[derive(Component, Debug, Clone, Copy, PartialEq, Eq)]
pub struct Pregnant {
    pub due: u64,
    pub father: PersonId,
    pub father_seed: u64,
    pub father_traits: Traits,
    /// Turn of the mother's previous birth, for birth intervals.
    pub previous_birth: Option<i64>,
}

/// Nursing after a birth: no conception before turn `until`.
#[derive(Component, Debug, Clone, Copy, PartialEq, Eq)]
pub struct PostPartum {
    pub until: i64,
    pub born: i64,
    pub child: PersonId,
}

/// People who courted or cared for children since the last daily step,
/// noted every turn (most people do neither, so this stays tiny).
#[derive(Resource, Debug, Clone, Default, PartialEq, Eq)]
pub struct Courting(pub Vec<PersonId>);

/// Vital statistics since the simulation began, for analysis and tests.
/// Never read by the simulation itself.
#[derive(Resource, Debug, Clone, Default, PartialEq, Eq)]
pub struct Vitals {
    pub births: u64,
    pub deaths: u64,
    pub twins: u64,
    pub maternal_deaths: u64,
    pub pairings: u64,
    pub widowed: u64,
    /// Deaths and person-days lived at each age in whole years.
    pub deaths_by_age: Vec<u64>,
    pub days_by_age: Vec<u64>,
    /// Woman-days lived and births by the mother's five-year age band
    /// (band 3 is 15–19).
    pub woman_days_by_band: Vec<u64>,
    pub births_by_band: Vec<u64>,
    /// Sum and count of intervals between a mother's births, in turns.
    pub birth_interval_turns: u64,
    pub birth_intervals: u64,
}

impl Vitals {
    /// Total fertility per mille: the children a woman would have at the
    /// observed age-specific rates (ages 15–49).
    pub fn total_fertility(&self) -> u64 {
        (3..10)
            .map(|b| {
                let days = self.woman_days_by_band.get(b).copied().unwrap_or(0);
                let births = self.births_by_band.get(b).copied().unwrap_or(0);
                // Five years of births per woman-year.
                births * 364 * 5 * 1000 / days.max(1)
            })
            .sum()
    }

    /// Life expectancy at `age`, in thousandths of a year, from the
    /// observed death rates (a period life table).
    pub fn life_expectancy(&self, age: u32) -> u64 {
        let daily: Vec<u64> = self
            .days_by_age
            .iter()
            .zip(&self.deaths_by_age)
            .map(|(&d, &n)| (n * PER_BILLION / d.max(1)).min(PER_BILLION))
            .collect();
        if daily.is_empty() {
            return 0;
        }
        LifeTables {
            daily,
            disease: Vec::new(),
        }
        .life_expectancy(age)
    }
}

/// Inserts this module's resources (its section of `data/people.json`).
pub fn install(world: &mut World) {
    let config = LifeConfig::builtin();
    world.insert_resource(LifeTables::new(&config.mortality));
    world.insert_resource(config);
    world.init_resource::<Courting>();
    world.init_resource::<Vitals>();
}

/// This module's systems, in order, for its slot in the turn.
pub fn systems() -> ScheduleConfigs<ScheduleSystem> {
    (tend, daily).chain()
}

/// Every turn: those caring for children satisfy their Family need, and
/// those courting are noted for the next pairing.
fn tend(
    config: Res<LifeConfig>,
    mut courting: ResMut<Courting>,
    mut people: Query<(&PersonId, &Activity, &mut Needs)>,
) {
    for (id, activity, mut needs) in &mut people {
        match activity.action {
            ActionKind::CareForChildren => needs.satisfy(Need::Family, config.family.care_per_turn),
            ActionKind::Court => courting.0.push(*id),
            _ => {}
        }
    }
}

/// What the daily step needs to know about one person.
#[derive(Clone, Copy)]
struct Row {
    entity: Entity,
    id: PersonId,
    sex: Sex,
    age: u32,
    home: CityId,
    away: bool,
    family: Family,
    traits: Traits,
}

type PersonQuery<'w, 's> = Query<
    'w,
    's,
    (
        Entity,
        &'static PersonId,
        &'static Person,
        &'static Traits,
        &'static mut Needs,
        &'static mut Family,
        &'static Activity,
        &'static mut PersonRng,
        Option<&'static Pregnant>,
        Option<&'static PostPartum>,
    ),
>;

/// Once a day, in the morning: deaths, births, conceptions and pairing up.
#[allow(clippy::too_many_arguments)]
fn daily(
    mut commands: Commands,
    turn: Res<Turn>,
    sim: Res<SimConfig>,
    tree: Res<TechTree>,
    config: Res<LifeConfig>,
    tables: Res<LifeTables>,
    mut next_id: ResMut<NextPersonId>,
    mut courting: ResMut<Courting>,
    mut vitals: ResMut<Vitals>,
    cities: Query<(&CityId, &Knowledge)>,
    mut people: PersonQuery,
    mut started: Local<bool>,
) {
    let now = turn.0;
    if !now.is_multiple_of(TURNS_PER_DAY) {
        return;
    }
    let tpy = sim.turns_per_year();
    let tpd = TURNS_PER_DAY as i64;
    let ages = tables.daily.len();
    if vitals.days_by_age.len() < ages {
        vitals.days_by_age.resize(ages, 0);
        vitals.deaths_by_age.resize(ages, 0);
        vitals.woman_days_by_band.resize(10, 0);
        vitals.births_by_band.resize(10, 0);
    }

    // Everyone, in id order: the lookup table for links between people.
    let mut rows: Vec<Row> = people
        .iter()
        .map(
            |(entity, id, person, traits, _, family, activity, ..)| Row {
                entity,
                id: *id,
                sex: person.sex,
                age: person.age_years(now, tpy),
                home: person.home,
                away: activity.until == u64::MAX,
                family: *family,
                traits: *traits,
            },
        )
        .collect();
    rows.sort_unstable_by_key(|r| r.id);
    let find = |id: PersonId| rows.binary_search_by_key(&id, |r| r.id).ok();

    // Founders nursing on the first day (their new PostPartum only takes
    // effect tomorrow, so they are left out of today's conceptions).
    let mut nursing = Vec::new();
    if !*started {
        *started = true;
        nursing = settle_founders(&mut commands, &rows, &people, &config.fertility, now);
    }

    let modifiers = city_modifiers(&tree, &cities);
    let m = &config.mortality;
    let f = &config.fertility;
    let weaning_turns = f.postpartum_days as i64 * tpd;

    // Per person: the death draw, the Family need, and links to people who
    // are gone (killed by a disaster or by yesterday's step).
    let mut dead: Vec<PersonId> = Vec::new();
    let mut can_conceive: Vec<PersonId> = Vec::new();
    let mut due: Vec<PersonId> = Vec::new();
    for (_, id, person, _, mut needs, mut family, activity, mut rng, pregnant, postpartum) in
        &mut people
    {
        let age = (person.age_years(now, tpy) as usize).min(ages - 1);
        vitals.days_by_age[age] += 1;
        if person.sex == Sex::Female && (15..50).contains(&age) {
            vitals.woman_days_by_band[age / 5] += 1;
        }
        let (mortality, disease) = modifiers
            .get(person.home.0 as usize)
            .copied()
            .unwrap_or((1000, 1000));
        let base =
            tables.daily[age] as i64 * 1000 + tables.disease[age] as i64 * (disease as i64 - 1000);
        let mut hazard = base.max(0) as u64 / 1000 * mortality / 1000;
        let unweaned = (now as i64 - person.born_on) < weaning_turns;
        if unweaned && family.mother.is_some_and(|m| find(m).is_none()) {
            hazard = hazard * m.orphan_per_mille / 1000;
        }
        hazard += m.hunger.extra(needs.get(Need::Food)) + m.cold.extra(needs.get(Need::Warmth));
        if age == ages - 1 {
            hazard = PER_BILLION;
        }
        if rng.below(PER_BILLION) < hazard.min(PER_BILLION) {
            dead.push(*id);
            vitals.deaths_by_age[age] += 1;
            continue;
        }

        if family.partner.is_some_and(|p| find(p).is_none()) {
            family.partner = None;
            vitals.widowed += 1;
        }
        if family.partner.is_some() {
            needs.satisfy(Need::Family, config.family.partner_daily);
        }
        if family.births > 0 {
            needs.satisfy(Need::Family, config.family.children_daily);
        }

        if let Some(p) = pregnant {
            if p.due <= now {
                due.push(*id);
            }
        } else if person.sex == Sex::Female
            && family.partner.is_some()
            && f.fecundity(age as u32) > 0
            && postpartum.is_none_or(|pp| pp.until <= now as i64)
            && activity.until != u64::MAX
        {
            can_conceive.push(*id);
        }
    }
    dead.sort_unstable();
    let is_dead = |id: PersonId| dead.binary_search(&id).is_ok();

    // Births, in the mothers' id order, so ids are handed out in a fixed
    // order. A mother who died today loses the child with her.
    due.sort_unstable();
    let mut maternal: Vec<PersonId> = Vec::new();
    for mother_id in due {
        if is_dead(mother_id) {
            continue;
        }
        let Some(mi) = find(mother_id) else { continue };
        let mother_row = rows[mi];
        let Ok((_, _, mother, traits, _, mut family, _, mut rng, Some(pregnant), _)) =
            people.get_mut(mother_row.entity)
        else {
            continue;
        };
        let pregnant = *pregnant;
        let (mother, traits) = (*mother, *traits);
        let twins = rng.chance_per_mille(f.twins_per_mille);
        let dies = rng.chance_per_mille(f.maternal_death_per_mille);
        let mut last_child = None;
        for _ in 0..if twins { 2 } else { 1 } {
            let seed = child_seed(rng.seed, pregnant.father_seed, family.births);
            family.births += 1;
            let child = next_id.take();
            commands.spawn(newborn(
                child,
                seed,
                &mother,
                (mother_id, &traits),
                (pregnant.father, &pregnant.father_traits),
                f,
                &tree,
                now,
            ));
            last_child = Some(child);
        }
        let children = if twins { 2 } else { 1 };
        vitals.births += children;
        vitals.twins += u64::from(twins);
        vitals.births_by_band[(mother_row.age as usize / 5).min(9)] += children;
        if let Some(previous) = pregnant.previous_birth {
            vitals.birth_interval_turns += (now as i64 - previous).max(0) as u64;
            vitals.birth_intervals += 1;
        }
        commands
            .entity(mother_row.entity)
            .remove::<Pregnant>()
            .insert(PostPartum {
                until: now as i64 + weaning_turns,
                born: now as i64,
                child: last_child.expect("at least one child"),
            });
        if let Some(fi) = find(pregnant.father)
            && let Ok((.., mut father_family, _, _, _, _)) = people.get_mut(rows[fi].entity)
        {
            father_family.births += children as u32;
        }
        if dies {
            maternal.push(mother_id);
            vitals.maternal_deaths += 1;
            vitals.deaths_by_age[(mother_row.age as usize).min(ages - 1)] += 1;
        }
    }
    dead.extend(maternal);
    dead.sort_unstable();
    let is_dead = |id: PersonId| dead.binary_search(&id).is_ok();

    // Deaths: the partner is widowed, and a nursing mother whose child died
    // can conceive again. Children keep their parents' ids.
    for &id in &dead {
        let Some(i) = find(id) else { continue };
        let row = rows[i];
        commands.entity(row.entity).despawn();
        vitals.deaths += 1;
        if let Some(p) = row.family.partner
            && !is_dead(p)
            && let Some(pi) = find(p)
            && let Ok((.., mut family, _, _, _, _)) = people.get_mut(rows[pi].entity)
            && family.partner == Some(id)
        {
            family.partner = None;
            vitals.widowed += 1;
        }
        if let Some(mother) = row.family.mother
            && !is_dead(mother)
            && let Some(mi) = find(mother)
            && let Ok((.., Some(pp))) = people.get(rows[mi].entity)
            && pp.child == id
            && pp.until > now as i64
        {
            let pp = PostPartum {
                until: now as i64,
                ..*pp
            };
            commands.entity(rows[mi].entity).insert(pp);
        }
    }

    // Conception: her own draw, if her partner is alive and with her.
    can_conceive.sort_unstable();
    for woman in can_conceive {
        if nursing.binary_search(&woman).is_ok() {
            continue;
        }
        let row = rows[find(woman).expect("alive at the start of the day")];
        let Some(partner) = row.family.partner.filter(|&p| !is_dead(p)) else {
            continue;
        };
        let Some(pi) = find(partner) else { continue };
        let man = rows[pi];
        if man.home != row.home || man.away {
            continue;
        }
        let Ok((.., partner_rng, _, _)) = people.get(man.entity) else {
            continue;
        };
        let father_seed = partner_rng.seed;
        let Ok((.., mut rng, _, postpartum)) = people.get_mut(row.entity) else {
            continue;
        };
        let chance = f.daily_chance_per_million * f.fecundity(row.age);
        if rng.below(PER_BILLION) >= chance {
            continue;
        }
        let spread = f.pregnancy_spread_days as i64;
        let days = f.pregnancy_days as i64 + rng.range(-spread, spread);
        let previous_birth = postpartum.map(|pp| pp.born);
        let mut entity = commands.entity(row.entity);
        entity.insert(Pregnant {
            due: now + days.max(1) as u64 * TURNS_PER_DAY,
            father: partner,
            father_seed,
            father_traits: man.traits,
            previous_birth,
        });
        if postpartum.is_some() {
            entity.remove::<PostPartum>();
        }
    }

    // Pairing up, city by city, among the living who are at home.
    courting.0.sort_unstable();
    courting.0.dedup();
    let courts = |id: PersonId| courting.0.binary_search(&id).is_ok();
    let p = &config.pairing;
    let single = |r: &Row, [lo, hi]: [u32; 2]| {
        r.family.partner.is_none() && !r.away && !is_dead(r.id) && (lo..=hi).contains(&r.age)
    };
    let mut women: Vec<&Row> = rows
        .iter()
        .filter(|r| r.sex == Sex::Female && single(r, p.female_age))
        .collect();
    let mut men: Vec<&Row> = rows
        .iter()
        .filter(|r| r.sex == Sex::Male && single(r, p.male_age))
        .collect();
    // Stable sorts keep the id order within each city.
    women.sort_by_key(|r| r.home);
    men.sort_by_key(|r| r.home);
    let mut taken = vec![false; men.len()];
    let mut couples: Vec<(PersonId, PersonId)> = Vec::new();
    for w in women {
        let first = men.partition_point(|m| m.home < w.home);
        let last = men.partition_point(|m| m.home <= w.home);
        if first == last {
            continue;
        }
        let Ok((.., mut rng, _, _)) = people.get_mut(w.entity) else {
            continue;
        };
        let chance = if courts(w.id) {
            p.court_chance
        } else {
            p.daily_chance
        };
        if !rng.chance_per_mille(chance) {
            continue;
        }
        let best = (first..last)
            .filter(|&k| !taken[k] && !related(w, men[k]))
            .map(|k| (compatibility(p, w, men[k], courts(men[k].id)), k))
            .filter(|&(score, _)| score >= p.min_compatibility)
            // Highest score; among equals, the lowest id (men are in id
            // order within the city).
            .min_by_key(|&(score, k)| (-score, k));
        if let Some((_, k)) = best {
            taken[k] = true;
            couples.push((w.id, men[k].id));
        }
    }
    for (w, m) in couples {
        for (a, b) in [(w, m), (m, w)] {
            if let Ok((.., mut family, _, _, _, _)) = people.get_mut(rows[find(a).unwrap()].entity)
            {
                family.partner = Some(b);
            }
        }
        vitals.pairings += 1;
    }
    courting.0.clear();
}

/// How well a woman and a man suit each other: 1000 for a man
/// `preferred_gap` years older with the same character, less for a wider or
/// narrower gap and different characters, more for agreeable people and for
/// a man who courts. Below 0 if the gap is out of bounds.
fn compatibility(p: &PairingConfig, woman: &Row, man: &Row, man_courts: bool) -> i32 {
    let gap = man.age as i32 - woman.age as i32;
    if gap > p.max_gap || gap < -p.max_younger {
        return i32::MIN;
    }
    let (w, m) = (woman.traits, man.traits);
    let diff = (i32::from(w.openness) - i32::from(m.openness)).abs()
        + (i32::from(w.conscientiousness) - i32::from(m.conscientiousness)).abs()
        + (i32::from(w.extraversion) - i32::from(m.extraversion)).abs();
    let agreeable = (i32::from(w.agreeableness) + i32::from(m.agreeableness)) / 2 - 128;
    1000 - p.gap_penalty * (gap - p.preferred_gap).abs() - diff / 3 * p.trait_penalty / 1000
        + agreeable
        + if man_courts { p.court_bonus } else { 0 }
}

/// Parent and child, or siblings and half-siblings: never partners.
fn related(a: &Row, b: &Row) -> bool {
    let (fa, fb) = (a.family, b.family);
    fa.mother == Some(b.id)
        || fa.father == Some(b.id)
        || fb.mother == Some(a.id)
        || fb.father == Some(a.id)
        || (fa.mother.is_some() && fa.mother == fb.mother)
        || (fa.father.is_some() && fa.father == fb.father)
}

/// The mortality and disease multipliers, per mille, of each city (indexed
/// by `CityId`), from the effects of the technologies and variants it
/// knows.
fn city_modifiers(tree: &TechTree, cities: &Query<(&CityId, &Knowledge)>) -> Vec<(u64, u64)> {
    let mut out = Vec::new();
    for (id, knowledge) in cities {
        let mut mortality = 1000u64;
        let mut disease = 1000u64;
        let effects = knowledge
            .techs()
            .flat_map(|t| tree.tech(t).effects.iter())
            .chain(
                knowledge
                    .variants()
                    .flat_map(|v| tree.variant(v).effects.iter()),
            );
        for effect in effects {
            let value = effect.value.max(0) as u64;
            match effect.kind.as_str() {
                "mortality" => mortality = mortality * value / 1000,
                "disease" => disease = disease * value / 1000,
                _ => {}
            }
        }
        let i = id.0 as usize;
        if out.len() <= i {
            out.resize(i + 1, (1000, 1000));
        }
        out[i] = (mortality, disease);
    }
    out
}

/// A newborn. Everything about it comes from its seed and its parents:
/// sex and character from its own stream, the baseline technologies, its
/// mother's home and culture.
#[allow(clippy::too_many_arguments)]
fn newborn(
    id: PersonId,
    seed: u64,
    mother: &Person,
    (mother_id, mother_traits): (PersonId, &Traits),
    (father_id, father_traits): (PersonId, &Traits),
    f: &FertilityConfig,
    tree: &TechTree,
    now: u64,
) -> PersonBundle {
    let mut rng = PersonRng::new(seed);
    let sex = if rng.chance_per_mille(f.male_births_per_mille) {
        Sex::Male
    } else {
        Sex::Female
    };
    let traits = Traits::inherit(mother_traits, father_traits, &mut rng);
    let mut skills = Knowledge::new(tree);
    for &t in tree.order() {
        if tree.tech(t).category == Category::Baseline {
            skills.learn(tree, t, None);
        }
    }
    PersonBundle {
        equipment: Default::default(),
        id,
        person: Person {
            sex,
            born_on: now as i64,
            home: mother.home,
            culture: mother.culture,
        },
        traits,
        needs: Needs::default(),
        family: Family {
            mother: Some(mother_id),
            father: Some(father_id),
            partner: None,
            births: 0,
        },
        skills: Skills(skills),
        status: Status::default(),
        activity: Activity {
            action: ActionKind::Idle,
            since: now,
            until: now,
        },
        rng,
    }
}

/// On the first day: founders who are still nursing a young child cannot
/// conceive yet, as if they had been through the daily step all along.
/// Returns those mothers, in id order.
fn settle_founders(
    commands: &mut Commands,
    rows: &[Row],
    people: &PersonQuery,
    f: &FertilityConfig,
    now: u64,
) -> Vec<PersonId> {
    let tpd = TURNS_PER_DAY as i64;
    let weaning = f.postpartum_days as i64 * tpd;
    let mut nursing: Vec<(PersonId, i64, PersonId)> = Vec::new();
    for row in rows {
        let Some(mother) = row.family.mother else {
            continue;
        };
        let Ok((_, _, person, ..)) = people.get(row.entity) else {
            continue;
        };
        if now as i64 - person.born_on < weaning {
            nursing.push((mother, person.born_on, row.id));
        }
    }
    // The youngest child decides, so later entries win.
    nursing.sort_unstable();
    for &(mother, born, child) in &nursing {
        if let Ok(i) = rows.binary_search_by_key(&mother, |r| r.id) {
            commands.entity(rows[i].entity).insert(PostPartum {
                until: born + weaning,
                born,
                child,
            });
        }
    }
    let mut mothers: Vec<PersonId> = nursing.into_iter().map(|(m, ..)| m).collect();
    mothers.dedup();
    mothers
}
