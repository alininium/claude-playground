//! People: founding households, personal knowledge, deaths owed by
//! disasters, and determinism on any number of threads.

mod common;

use common::Fixture;
use societies::city::{CityId, Population};
use societies::people::Sex;
use societies::people::founding::FoundingConfig;

#[test]
fn settlements_start_as_households() {
    let mut sim = common::sim(Fixture::Medium);
    let people = sim.people();
    let (turn, tpy) = (sim.turn(), sim.config().turns_per_year());
    let config = FoundingConfig::builtin();
    let by_id: std::collections::HashMap<_, _> = people.iter().map(|p| (p.id, p)).collect();
    let age = |p: &societies::people::PersonSnapshot| p.person.age_years(turn, tpy);

    // A pre-modern age pyramid: lots of children, few elders.
    let children = people.iter().filter(|p| age(p) < 15).count();
    let elders = people.iter().filter(|p| age(p) >= 50).count();
    assert!(
        children * 100 > people.len() * 30 && children * 100 < people.len() * 50,
        "{children} children of {}",
        people.len()
    );
    assert!(elders * 100 < people.len() * 12, "{elders} elders");
    let women = people
        .iter()
        .filter(|p| p.person.sex == Sex::Female)
        .count();
    assert!(women * 100 > people.len() * 45 && women * 100 < people.len() * 55);

    let mut couples = 0;
    let mut with_mother = 0;
    for p in &people {
        if let Some(partner) = p.family.partner {
            let q = by_id[&partner];
            assert_eq!(q.family.partner, Some(p.id), "partners are mutual");
            assert_ne!(q.person.sex, p.person.sex);
            assert_eq!(q.person.home, p.person.home);
            couples += 1;
        }
        if let Some(mother) = p.family.mother {
            let m = by_id[&mother];
            assert_eq!(m.person.sex, Sex::Female);
            assert_eq!(m.person.home, p.person.home);
            let gap = age(m) - age(p);
            let [lo, hi] = config.mother_age_at_birth;
            assert!((lo..=hi).contains(&gap), "mother {gap} years older");
            assert_eq!(p.family.father, m.family.partner);
            with_mother += 1;
        }
    }
    assert!(couples > 0);
    let young = people.iter().filter(|p| age(p) < config.adult_age).count();
    assert!(
        with_mother * 10 >= young * 8,
        "{with_mother} of {young} children have a mother"
    );
}

#[test]
fn people_know_part_of_what_their_city_knows() {
    let mut sim = common::sim(Fixture::Medium);
    let cities = sim.cities();
    let tree = sim.technologies().clone();
    let fire = tree.id("fire").unwrap();
    for p in sim.people() {
        assert!(p.skills.knows(fire));
        let city = cities.iter().find(|c| c.id == p.person.home).unwrap();
        for t in p.skills.techs() {
            assert!(
                city.knowledge.knows(t),
                "{} beyond the city",
                tree.tech(t).id
            );
            assert!(tree.prerequisites_met(&p.skills, t));
        }
        for v in p.skills.variants() {
            assert!(city.knowledge.knows_variant(v));
        }
    }
}

#[test]
fn every_person_has_their_own_stream() {
    let mut sim = common::sim(Fixture::Medium);
    let people = sim.people();
    let mut seeds: Vec<u64> = people.iter().map(|p| p.rng.seed).collect();
    seeds.sort();
    seeds.dedup();
    assert_eq!(seeds.len(), people.len());
    let ids: Vec<u64> = people.iter().map(|p| p.id.0).collect();
    assert!(ids.windows(2).all(|w| w[0] < w[1]));
}

#[test]
fn disasters_kill_real_people() {
    let mut sim = common::sim(Fixture::Small);
    let before = sim.cities()[0].population;
    let mut query = sim.world_mut().query::<(&CityId, &mut Population)>();
    for (id, mut pop) in query.iter_mut(sim.world_mut()) {
        if id.0 == 0 {
            pop.pending_deaths = 7;
        }
    }
    sim.step();
    let after = sim.cities()[0].population;
    assert_eq!(after, before - 7);
    let living = sim
        .people()
        .iter()
        .filter(|p| p.person.home == CityId(0))
        .count() as u64;
    assert_eq!(living, after);
}

#[test]
fn the_number_of_threads_changes_nothing() {
    let mut one = common::sim(Fixture::Small);
    let mut many = common::sim(Fixture::Small);
    one.set_threads(1);
    many.set_threads(4);
    for _ in 0..8 {
        one.run(10);
        many.run(10);
        assert_eq!(
            one.state_digest(),
            many.state_digest(),
            "turn {}",
            one.turn()
        );
    }
}
