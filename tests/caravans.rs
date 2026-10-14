//! Caravans: people who choose to settle away leave together when there are
//! enough of them, travel, and found a new city at a valid site.

mod common;

use common::Fixture;
use societies::Simulation;
use societies::caravan::{self, CaravanConfig};
use societies::city::{CityId, can_settle, too_close};
use societies::events::SimEvent;
use societies::people::founding::FoundingConfig;
use societies::people::{ActionKind, Activity, PersonId};
use societies::time::TURNS_PER_DAY;

/// Makes `n` adults of `city` choose to settle away right now, mothers of
/// young children first. Returns who.
fn choose_to_leave(sim: &mut Simulation, city: CityId, n: usize) -> Vec<PersonId> {
    let (turn, tpy) = (sim.turn(), sim.config().turns_per_year());
    let adult = FoundingConfig::builtin().adult_age;
    let people = sim.people();
    let young_mothers: Vec<PersonId> = people
        .iter()
        .filter(|p| p.person.age_years(turn, tpy) < adult)
        .filter_map(|p| p.family.mother)
        .collect();
    let mut adults: Vec<_> = people
        .iter()
        .filter(|p| p.person.home == city && p.person.age_years(turn, tpy) >= adult)
        .map(|p| (!young_mothers.contains(&p.id), p.id))
        .collect();
    adults.sort();
    let chosen: Vec<PersonId> = adults.iter().take(n).map(|a| a.1).collect();
    let world = sim.world_mut();
    let mut query = world.query::<(&PersonId, &mut Activity)>();
    for (id, mut activity) in query.iter_mut(world) {
        if chosen.contains(id) {
            *activity = Activity {
                action: ActionKind::SettleAway,
                since: turn,
                until: turn + 40,
            };
        }
    }
    chosen
}

fn days(sim: &mut Simulation, n: u64) {
    sim.run(n * TURNS_PER_DAY);
}

#[test]
fn a_caravan_leaves_the_next_day_only_with_enough_adults() {
    let min = CaravanConfig::builtin().min_adults as usize;
    let adult = FoundingConfig::builtin().adult_age;

    // Too few: nobody leaves.
    let mut sim = common::sim(Fixture::Small);
    let before = sim.cities()[0].population;
    choose_to_leave(&mut sim, CityId(0), min - 1);
    days(&mut sim, 2);
    assert!(caravan::caravans(sim.world_mut()).is_empty());
    assert_eq!(sim.cities()[0].population, before);

    // Enough: they leave on the next day, with their young children.
    let mut sim = common::sim(Fixture::Small);
    let adults = choose_to_leave(&mut sim, CityId(0), min);
    sim.run(TURNS_PER_DAY - 1);
    assert!(
        caravan::caravans(sim.world_mut()).is_empty(),
        "left the same day"
    );
    sim.step();
    let caravans = caravan::caravans(sim.world_mut());
    assert_eq!(caravans.len(), 1);
    let c = &caravans[0];
    assert_eq!(c.origin, CityId(0));
    let (turn, tpy) = (sim.turn(), sim.config().turns_per_year());
    let people = sim.people();
    let mut expected = adults.clone();
    for p in &people {
        if p.person.age_years(turn, tpy) < adult
            && p.family.mother.is_some_and(|m| adults.contains(&m))
        {
            expected.push(p.id);
        }
    }
    expected.sort();
    assert!(expected.len() > adults.len(), "no children came along");
    assert_eq!(c.members, expected);
    for p in people.iter().filter(|p| expected.contains(&p.id)) {
        assert_eq!(p.activity.until, u64::MAX, "travellers are unavailable");
        assert_eq!(p.activity.action, ActionKind::SettleAway);
        assert_eq!(p.person.culture, CityId(0));
        assert_ne!(p.person.home, CityId(0));
    }
    // They no longer count as residents of their old city.
    let city = sim.cities()[0].clone();
    assert_eq!(city.population, before - expected.len() as u64);
    let living = people.iter().filter(|p| p.person.home == CityId(0)).count();
    assert_eq!(living as u64, city.population);
}

/// Sends a caravan out of `city` and runs until it settles or gives up.
/// Returns the new city's id, if any.
fn send_out(sim: &mut Simulation, city: CityId) -> Option<CityId> {
    let min = CaravanConfig::builtin().min_adults as usize;
    let max_days = u64::from(CaravanConfig::builtin().max_days);
    let cities_before = sim.cities().len();
    choose_to_leave(sim, city, min + 3);
    for _ in 0..max_days + 2 {
        days(sim, 1);
        let cities = sim.cities();
        if cities.len() > cities_before {
            return Some(cities.last().unwrap().id);
        }
        if caravan::caravans(sim.world_mut()).is_empty() && sim.turn() > TURNS_PER_DAY {
            return None;
        }
    }
    None
}

#[test]
fn caravans_found_cities_at_valid_sites() {
    let mut sim = common::sim(Fixture::Medium);
    let start = sim.turn();
    let origin = CityId(0);
    let origin_city = sim.cities()[0].clone();
    let new = send_out(&mut sim, origin).expect("the caravan found no place to settle");
    let max_turns = u64::from(CaravanConfig::builtin().max_days + 1) * TURNS_PER_DAY;
    assert!(sim.turn() - start <= max_turns);

    let cities = sim.cities();
    let city = cities.iter().find(|c| c.id == new).unwrap();
    assert_eq!(city.founded_on, sim.turn());
    assert_eq!(city.people, origin_city.people);
    assert_ne!(city.position, origin_city.position);
    let index = sim.map().index(city.position);
    assert!(can_settle(sim.map(), sim.climate(), index));
    let config = sim.config().clone();
    for other in cities.iter().filter(|c| c.id != new) {
        assert!(
            !too_close(sim.map(), &config, city.position, other.position),
            "{} is too close to {}",
            city.name,
            other.name
        );
    }

    // The members live there now, available again, keeping their culture;
    // the census counts them there, and the city knows what they know.
    let people = sim.people();
    let residents: Vec<_> = people.iter().filter(|p| p.person.home == new).collect();
    assert!(!residents.is_empty());
    assert_eq!(residents.len() as u64, city.population);
    for p in &residents {
        assert_ne!(p.activity.until, u64::MAX);
        assert_eq!(p.person.culture, origin);
        for t in p.skills.techs() {
            assert!(city.knowledge.knows(t));
        }
    }
    assert!(caravan::caravans(sim.world_mut()).is_empty());
    let total: u64 = cities.iter().map(|c| c.population).sum();
    assert_eq!(total, people.len() as u64, "everyone lives somewhere");

    // The founding is logged exactly once.
    let logged: Vec<_> = sim
        .events()
        .events()
        .iter()
        .filter(|e| matches!(e, SimEvent::CityFounded { id, .. } if *id == new))
        .collect();
    assert_eq!(logged.len(), 1);
    let SimEvent::CityFounded {
        turn,
        name,
        position,
        ..
    } = logged[0]
    else {
        unreachable!()
    };
    assert_eq!(
        (*turn, name, *position),
        (city.founded_on, &city.name, city.position)
    );
}

#[test]
fn caravans_that_find_nothing_go_home() {
    let mut sim = common::sim(Fixture::Small);
    sim.world_mut().resource_mut::<CaravanConfig>().max_days = 2;
    let before = sim.cities();
    let adults = choose_to_leave(&mut sim, CityId(0), 6);
    days(&mut sim, 1);
    assert_eq!(caravan::caravans(sim.world_mut()).len(), 1);
    days(&mut sim, 3);
    assert!(caravan::caravans(sim.world_mut()).is_empty());
    assert_eq!(sim.cities().len(), before.len());
    assert_eq!(sim.cities()[0].population, before[0].population);
    for p in sim.people().iter().filter(|p| adults.contains(&p.id)) {
        assert_eq!(p.person.home, CityId(0));
        assert_ne!(p.activity.until, u64::MAX);
    }
}

#[test]
fn caravans_are_deterministic_on_any_number_of_threads() {
    let run = |threads| {
        let mut sim = common::sim(Fixture::Medium);
        sim.set_threads(threads);
        choose_to_leave(&mut sim, CityId(0), 6);
        choose_to_leave(&mut sim, CityId(1), 6);
        let mut digests = Vec::new();
        for _ in 0..12 {
            days(&mut sim, 5);
            digests.push((sim.state_digest(), caravan::caravans(sim.world_mut())));
        }
        (digests, sim.cities())
    };
    let one = run(1);
    assert!(!one.0[0].1.is_empty(), "no caravan left");
    assert_eq!(one, run(4));
}
