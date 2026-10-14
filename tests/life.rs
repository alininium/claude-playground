//! The life cycle: the same parents have the same child anywhere, deaths
//! and births at realistic rates, hunger and cold kill, widowhood, pairing
//! up, and head counts that follow births and deaths.

mod common;

use bevy_ecs::prelude::*;
use common::Fixture;
use societies::Simulation;
use societies::city::{CityId, Population};
use societies::people::life::{LifeConfig, LifeTables, Pregnant, Vitals};
use societies::people::rng::PersonRng;
use societies::people::{
    ActionKind, Activity, Family, Need, Needs, Person, PersonId, PersonSnapshot, Sex, Traits,
};
use societies::time::TURNS_PER_DAY;

fn entity_of(sim: &mut Simulation, id: PersonId) -> Entity {
    let mut q = sim.world_mut().query::<(Entity, &PersonId)>();
    q.iter(sim.world())
        .find(|(_, p)| **p == id)
        .map(|(e, _)| e)
        .expect("alive")
}

/// Runs `days` days; before each, sets everyone's Food and Warmth urgency
/// to `food` and `warmth`, so other modules cannot change the conditions.
fn run_days(sim: &mut Simulation, days: u64, food: u16, warmth: u16) {
    for _ in 0..days {
        let mut q = sim.world_mut().query::<&mut Needs>();
        for mut needs in q.iter_mut(sim.world_mut()) {
            needs.set(Need::Food, food);
            needs.set(Need::Warmth, warmth);
        }
        sim.run(TURNS_PER_DAY);
    }
}

fn vitals(sim: &Simulation) -> Vitals {
    sim.world().resource::<Vitals>().clone()
}

/// The first couple (in the woman's id order) that is alive and at home.
fn a_couple(people: &[PersonSnapshot]) -> (PersonId, PersonId) {
    people
        .iter()
        .filter(|p| p.person.sex == Sex::Female)
        .find_map(|w| {
            let m = w.family.partner?;
            people.iter().any(|p| p.id == m).then_some((w.id, m))
        })
        .expect("a couple")
}

struct Parents {
    mother_seed: u64,
    father_seed: u64,
    mother_traits: Traits,
    father_traits: Traits,
    births: u32,
}

/// Makes the first couple of `sim` into `parents`, with the mother due
/// tomorrow, and returns the child she has.
fn child_of(mut sim: Simulation, parents: &Parents, counter: u64) -> PersonSnapshot {
    let (w, m) = a_couple(&sim.people());
    let (we, me) = (entity_of(&mut sim, w), entity_of(&mut sim, m));
    let due = sim.turn() + 1;
    let world = sim.world_mut();
    let mut father = world.entity_mut(me);
    *father.get_mut::<PersonRng>().unwrap() = PersonRng {
        seed: parents.father_seed,
        counter: counter * 3,
    };
    *father.get_mut::<Traits>().unwrap() = parents.father_traits;
    let mut mother = world.entity_mut(we);
    *mother.get_mut::<PersonRng>().unwrap() = PersonRng {
        seed: parents.mother_seed,
        counter,
    };
    *mother.get_mut::<Traits>().unwrap() = parents.mother_traits;
    mother.get_mut::<Family>().unwrap().births = parents.births;
    mother.insert(Pregnant {
        due,
        father: m,
        father_seed: parents.father_seed,
        father_traits: parents.father_traits,
        previous_birth: None,
    });
    sim.run(TURNS_PER_DAY);
    let people = sim.people();
    let mother = people.iter().find(|p| p.id == w).expect("mother alive");
    let child = people
        .iter()
        .filter(|p| p.family.mother == Some(w) && p.person.born_on >= due as i64 - 4)
        .min_by_key(|p| p.id)
        .expect("a newborn")
        .clone();
    assert!(mother.family.births > parents.births);
    assert_eq!(child.family.father, Some(m));
    assert_eq!(child.person.home, mother.person.home);
    assert_eq!(child.person.culture, mother.person.culture);
    assert_eq!(child.family.partner, None);
    let father = people.iter().find(|p| p.id == m).expect("father alive");
    assert!(father.family.births > 0);
    child
}

#[test]
fn the_same_parents_have_the_same_child_anywhere() {
    let parents = Parents {
        mother_seed: 0x1234_5678,
        father_seed: 0x9876_5432,
        mother_traits: Traits::from_array([200, 40, 128, 90, 160, 10]),
        father_traits: Traits::from_array([60, 220, 128, 170, 30, 250]),
        births: 2,
    };
    let here = child_of(common::sim(Fixture::Small), &parents, 5);

    // Another world, later, with everyone starving and the parents' streams
    // at other points.
    let mut elsewhere = common::sim(Fixture::Medium);
    run_days(&mut elsewhere, 3, 900, 700);
    elsewhere.run(2);
    let there = child_of(elsewhere, &parents, 777);

    assert_eq!(here.rng.seed, there.rng.seed);
    assert_eq!(here.person.sex, there.person.sex);
    assert_eq!(here.traits, there.traits);
    assert_eq!(here.skills, there.skills);
    // Traits lie around the parents' average.
    for ((c, m), f) in here
        .traits
        .as_array()
        .iter()
        .zip(parents.mother_traits.as_array())
        .zip(parents.father_traits.as_array())
    {
        let mid = (i32::from(m) + i32::from(f)) / 2;
        assert!((i32::from(*c) - mid).abs() <= 48);
    }

    // The next child of the same parents is someone else.
    let third = child_of(
        common::sim(Fixture::Small),
        &Parents {
            births: 3,
            ..parents
        },
        5,
    );
    assert_ne!(third.rng.seed, here.rng.seed);
}

#[test]
fn mortality_follows_forager_life_tables() {
    let tables = LifeTables::new(&LifeConfig::builtin().mortality);
    let e0 = tables.life_expectancy(0);
    let e15 = tables.life_expectancy(15);
    assert!((28_000..=35_000).contains(&e0), "life expectancy {e0}");
    assert!((40_000..=45_000).contains(&e15), "at 15: {e15} more");
    // Infants die far more often than children, the old more than adults.
    assert!(tables.daily[0] > tables.daily[10] * 10);
    assert!(tables.daily[70] > tables.daily[25] * 4);
    assert!(tables.daily[10] <= tables.daily[25]);
}

#[test]
fn deaths_and_births_happen_at_realistic_rates() {
    let mut sim = common::sim(Fixture::Small);
    let start = sim.people().len() as u64;
    run_days(&mut sim, 364 * 6, 0, 0);
    let v = vitals(&sim);
    let years = 6;
    let mean = (start + sim.people().len() as u64) / 2;
    // Per thousand a year: pre-modern birth rates 35–55, death rates 20–45.
    let cbr = v.births * 1000 / years / mean;
    let cdr = v.deaths * 1000 / years / mean;
    assert!((35..=55).contains(&cbr), "births {cbr}‰");
    assert!((15..=45).contains(&cdr), "deaths {cdr}‰");
    let tfr = v.total_fertility();
    assert!((5_000..=7_500).contains(&tfr), "total fertility {tfr}");
    let interval = v.birth_interval_turns / v.birth_intervals.max(1) / TURNS_PER_DAY;
    assert!(
        (2 * 364..=4 * 364).contains(&interval),
        "birth interval {interval} days"
    );
    let e0 = v.life_expectancy(0);
    assert!(
        (25_000..=42_000).contains(&e0),
        "observed life expectancy {e0}"
    );
    assert!(v.twins > 0 && v.twins * 20 < v.births);
    assert!(v.pairings > 0);
}

#[test]
fn hunger_and_cold_kill() {
    let deaths = |food, warmth| {
        let mut sim = common::sim(Fixture::Small);
        run_days(&mut sim, 30, food, warmth);
        vitals(&sim).deaths
    };
    let fed = deaths(0, 0);
    let starving = deaths(1000, 0);
    let freezing = deaths(0, 1000);
    assert!(starving > fed * 5, "starving {starving}, fed {fed}");
    assert!(freezing > fed * 3, "freezing {freezing}, fed {fed}");
    assert!(starving > freezing);
}

#[test]
fn the_dead_leave_widows_and_orphans_keep_their_parents() {
    let mut sim = common::sim(Fixture::Small);
    let people = sim.people();
    // A father, his partner and their children.
    let (w, m) = people
        .iter()
        .filter(|p| p.person.sex == Sex::Female)
        .find_map(|w| {
            let m = w.family.partner?;
            people
                .iter()
                .any(|c| c.family.father == Some(m) && c.family.mother == Some(w.id))
                .then_some((w.id, m))
        })
        .expect("a family");
    let me = entity_of(&mut sim, m);
    let ancient = sim.turn() as i64 - 120 * sim.config().turns_per_year() as i64;
    sim.world_mut()
        .entity_mut(me)
        .get_mut::<Person>()
        .unwrap()
        .born_on = ancient;
    run_days(&mut sim, 1, 0, 0);
    let people = sim.people();
    assert!(people.iter().all(|p| p.id != m), "too old to live");
    let widow = people.iter().find(|p| p.id == w).expect("widow alive");
    assert_eq!(widow.family.partner, None);
    assert!(
        people.iter().any(|c| c.family.father == Some(m)),
        "children keep their father"
    );
    assert!(vitals(&sim).widowed >= 1);

    // Deaths from a disaster (carried out by the census) widow too.
    let mut q = sim.world_mut().query::<(&CityId, &mut Population)>();
    for (_, mut pop) in q.iter_mut(sim.world_mut()) {
        pop.pending_deaths = 20;
    }
    run_days(&mut sim, 2, 0, 0);
    let people = sim.people();
    for p in &people {
        if let Some(partner) = p.family.partner {
            let q = people.iter().find(|q| q.id == partner);
            assert!(q.is_some(), "{:?} partnered to the dead", p.id);
            assert_eq!(q.unwrap().family.partner, Some(p.id));
        }
    }
}

#[test]
fn head_counts_follow_births_and_deaths() {
    let mut sim = common::sim(Fixture::Small);
    let start = sim.people().len() as u64;
    run_days(&mut sim, 364, 0, 0);
    let v = vitals(&sim);
    let people = sim.people();
    assert!(v.births > 0 && v.deaths > 0);
    assert_eq!(people.len() as u64, start + v.births - v.deaths);
    for city in sim.cities() {
        let living = people.iter().filter(|p| p.person.home == city.id).count() as u64;
        assert_eq!(city.population, living, "city {:?}", city.id);
    }
    // Ids stay unique and newborns get fresh ones.
    let mut ids: Vec<_> = people.iter().map(|p| p.id).collect();
    ids.dedup();
    assert_eq!(ids.len(), people.len());
}

#[test]
fn couples_are_suitable_and_courting_helps() {
    let mut sim = common::sim(Fixture::Small);
    let (turn, tpy) = (sim.turn(), sim.config().turns_per_year());
    let pairing = LifeConfig::builtin().pairing;
    // Half the single women court for two months.
    let singles: Vec<PersonId> = sim
        .people()
        .iter()
        .filter(|p| {
            let age = p.person.age_years(turn, tpy);
            p.person.sex == Sex::Female
                && p.family.partner.is_none()
                && (pairing.female_age[0]..=pairing.female_age[1]).contains(&age)
        })
        .map(|p| p.id)
        .collect();
    assert!(singles.len() >= 20, "{} single women", singles.len());
    let until = turn + 60 * TURNS_PER_DAY;
    for &id in singles.iter().step_by(2) {
        let e = entity_of(&mut sim, id);
        *sim.world_mut().get_mut::<Activity>(e).unwrap() = Activity {
            action: ActionKind::Court,
            since: turn,
            until,
        };
    }
    run_days(&mut sim, 60, 0, 0);
    let people = sim.people();
    let paired = |ids: &mut dyn Iterator<Item = &PersonId>| {
        ids.filter(|id| {
            people
                .iter()
                .any(|p| p.id == **id && p.family.partner.is_some())
        })
        .count()
    };
    let courted = paired(&mut singles.iter().step_by(2));
    let waited = paired(&mut singles.iter().skip(1).step_by(2));
    assert!(courted > waited, "courting {courted}, waiting {waited}");

    let now = sim.turn();
    for p in &people {
        let Some(partner) = p.family.partner else {
            continue;
        };
        let q = people.iter().find(|q| q.id == partner).expect("alive");
        assert_eq!(q.family.partner, Some(p.id));
        assert_ne!(q.person.sex, p.person.sex);
        assert_eq!(q.person.home, p.person.home);
        assert_ne!(p.family.mother, Some(q.id));
        assert_ne!(p.family.father, Some(q.id));
        if p.family.mother.is_some() {
            assert_ne!(p.family.mother, q.family.mother, "siblings");
        }
        let min = match p.person.sex {
            Sex::Female => pairing.female_age[0],
            Sex::Male => pairing.male_age[0],
        };
        assert!(p.person.age_years(now, tpy) >= min);
    }
}

/// One day of the life cycle for about 11 000 people. Timing only means
/// something in a release build.
#[test]
#[cfg_attr(debug_assertions, ignore = "timing needs `cargo test --release`")]
fn a_day_of_life_for_ten_thousand_people() {
    let mut sim = common::sim_with(Fixture::Small, |c| {
        c.city_population = [650, 750];
        c.barbarian_population = [650, 750];
    });
    let count = sim.people().len();
    assert!(count >= 9_000, "{count} people");
    let mut schedule = bevy_ecs::schedule::Schedule::default();
    schedule.add_systems(societies::people::life::systems());
    let world = sim.world_mut();
    // Warm up (first-day setup), then time an ordinary day.
    for _ in 0..TURNS_PER_DAY {
        world.resource_mut::<societies::time::Turn>().0 += 1;
        schedule.run(world);
    }
    let start = std::time::Instant::now();
    for _ in 0..TURNS_PER_DAY {
        world.resource_mut::<societies::time::Turn>().0 += 1;
        schedule.run(world);
    }
    let took = start.elapsed();
    println!("a day of life for {count} people: {took:?}");
    assert!(took < std::time::Duration::from_millis(5), "{took:?}");
}
