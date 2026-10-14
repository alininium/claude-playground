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

// ---------------------------------------------------------------------------
// Less obvious consequences of leaving
// ---------------------------------------------------------------------------

/// Makes exactly the people in `ids` choose to settle away right now.
fn send_away(sim: &mut Simulation, ids: &[PersonId]) {
    let turn = sim.turn();
    let world = sim.world_mut();
    let mut query = world.query::<(&PersonId, &mut Activity)>();
    for (id, mut activity) in query.iter_mut(world) {
        if ids.contains(id) {
            *activity = Activity {
                action: ActionKind::SettleAway,
                since: turn,
                until: turn + 40,
            };
        }
    }
}

/// Adults of city 0 who have no partner and no young children, in id order.
fn free_adults(sim: &mut Simulation, skip: &[PersonId]) -> Vec<PersonId> {
    let (turn, tpy) = (sim.turn(), sim.config().turns_per_year());
    let adult = FoundingConfig::builtin().adult_age;
    let people = sim.people();
    let mothers: Vec<PersonId> = people
        .iter()
        .filter(|p| p.person.age_years(turn, tpy) < adult)
        .filter_map(|p| p.family.mother)
        .collect();
    people
        .iter()
        .filter(|p| {
            p.person.home == CityId(0)
                && p.person.age_years(turn, tpy) >= 16
                && p.family.partner.is_none()
                && !mothers.contains(&p.id)
                && !skip.contains(&p.id)
        })
        .map(|p| p.id)
        .collect()
}

/// A married man of city 0 (without young children, so nobody follows
/// him) and his wife.
fn a_couple(sim: &mut Simulation) -> (PersonId, PersonId) {
    let people = sim.people();
    let fathers: Vec<PersonId> = people.iter().filter_map(|p| p.family.father).collect();
    let husband = people
        .iter()
        .find(|p| {
            p.person.home == CityId(0)
                && p.person.sex == societies::people::Sex::Male
                && p.family.partner.is_some()
                && !fathers.contains(&p.id)
        })
        .expect("a married man without children");
    (husband.id, husband.family.partner.unwrap())
}

fn person(sim: &mut Simulation, id: PersonId) -> societies::people::PersonSnapshot {
    sim.people().into_iter().find(|p| p.id == id).unwrap()
}

#[test]
fn a_partner_left_behind_is_free_to_pair_again() {
    let mut sim = common::sim(Fixture::Small);
    let min = CaravanConfig::builtin().min_adults as usize;
    let (husband, wife) = a_couple(&mut sim);
    let mut leaving = vec![husband];
    leaving.extend(
        free_adults(&mut sim, &[husband, wife])
            .into_iter()
            .take(min - 1),
    );
    send_away(&mut sim, &leaving);
    days(&mut sim, 2);

    let (h, w) = (person(&mut sim, husband), person(&mut sim, wife));
    assert_eq!(h.activity.until, u64::MAX, "he left");
    assert_eq!(w.person.home, CityId(0), "she stayed");
    // Neither is still tied to someone they can no longer live with.
    assert_eq!(h.family.partner, None);
    assert_eq!(w.family.partner, None);
}

#[test]
fn couples_who_leave_together_stay_together() {
    let mut sim = common::sim(Fixture::Small);
    let min = CaravanConfig::builtin().min_adults as usize;
    let (husband, wife) = a_couple(&mut sim);
    let mut leaving = vec![husband, wife];
    leaving.extend(
        free_adults(&mut sim, &[husband, wife])
            .into_iter()
            .take(min - 2),
    );
    send_away(&mut sim, &leaving);
    days(&mut sim, 2);

    let (h, w) = (person(&mut sim, husband), person(&mut sim, wife));
    assert_eq!((h.activity.until, w.activity.until), (u64::MAX, u64::MAX));
    assert_eq!(h.family.partner, Some(wife));
    assert_eq!(w.family.partner, Some(husband));
}

#[test]
fn a_chief_who_leaves_gives_up_the_office_but_the_bonded_stay_bonded() {
    use societies::people::status::StatusCatalog;
    use societies::people::{Status, StatusId};
    let mut sim = common::sim(Fixture::Small);
    let min = CaravanConfig::builtin().min_adults as usize;
    let catalog = sim.world().resource::<StatusCatalog>().clone();
    let (chief, bonded) = (catalog.id("chief").unwrap(), catalog.id("bonded").unwrap());
    // Past each city's first review (on its first day), so nothing but
    // leaving changes these statuses.
    days(&mut sim, 1);
    let leaving: Vec<PersonId> = free_adults(&mut sim, &[]).into_iter().take(min).collect();
    {
        let world = sim.world_mut();
        let mut query = world.query::<(&PersonId, &mut Status)>();
        for (id, mut status) in query.iter_mut(world) {
            if *id == leaving[0] {
                status.0 = chief;
            } else if *id == leaving[1] {
                status.0 = bonded;
            }
        }
    }
    send_away(&mut sim, &leaving);
    days(&mut sim, 2);
    let (a, b) = (person(&mut sim, leaving[0]), person(&mut sim, leaving[1]));
    assert_eq!(a.activity.until, u64::MAX, "they left");
    assert_eq!(a.status, StatusId(0), "the office stays with the city");
    assert_eq!(b.status, bonded, "bondage is for life");
}

#[test]
fn a_caravan_that_takes_the_last_knowers_leaves_the_city_without_the_technology() {
    use societies::journal::Happening;
    use societies::people::Skills;
    use societies::people::status::{StatusCatalog, StatusSchedule};
    use societies::tech::{Knowledge, TechTree};
    let mut sim = common::sim(Fixture::Small);
    // Nobody learns anything meanwhile, so the knowers stay the knowers.
    sim.world_mut()
        .resource_mut::<StatusCatalog>()
        .config
        .learn_days = 1_000_000;
    // A season boundary, so the journal has seen what the city knows.
    loop {
        sim.step();
        let d = sim.date();
        if d.day_of_season == 0 && d.time_of_day == societies::time::TimeOfDay::Morning {
            break;
        }
    }

    // Pottery is known in city 0 only by the adults who will leave.
    let min = CaravanConfig::builtin().min_adults as usize;
    let leaving: Vec<PersonId> = free_adults(&mut sim, &[]).into_iter().take(min).collect();
    let tree = sim.world().resource::<TechTree>().clone();
    let pottery = tree.id("pottery").unwrap();
    {
        let world = sim.world_mut();
        let mut query = world.query::<(&PersonId, &societies::people::Person, &mut Skills)>();
        for (id, p, mut skills) in query.iter_mut(world) {
            if p.home != CityId(0) {
                continue;
            }
            let mut k = Knowledge::new(&tree);
            for t in skills.0.techs() {
                if t != pottery || leaving.contains(id) {
                    k.learn(&tree, t, None);
                }
            }
            if leaving.contains(id) {
                k.learn(&tree, pottery, None);
            }
            skills.0 = k;
        }
        let mut cities = world.query::<(&CityId, &mut Knowledge)>();
        for (id, mut knowledge) in cities.iter_mut(world) {
            if *id == CityId(0) {
                knowledge.learn(&tree, pottery, None);
            }
        }
    }
    send_away(&mut sim, &leaving);
    days(&mut sim, 2);
    // The city takes stock of what its people know now.
    let turn = sim.turn();
    {
        let world = sim.world_mut();
        let mut q = world.query::<(&CityId, &mut StatusSchedule)>();
        for (id, mut schedule) in q.iter_mut(world) {
            if *id == CityId(0) {
                schedule.next_knowledge = turn;
            }
        }
    }
    days(&mut sim, 1);
    let knows = |sim: &mut Simulation| {
        sim.cities()
            .into_iter()
            .find(|c| c.id == CityId(0))
            .unwrap()
            .knowledge
            .knows(pottery)
    };
    assert!(!knows(&mut sim), "the city still knows pottery");

    // ... which the journal records at the next season boundary.
    loop {
        sim.step();
        let d = sim.date();
        if d.day_of_season == 0 && d.time_of_day == societies::time::TimeOfDay::Morning {
            break;
        }
    }
    let lost = sim.journal().entries().iter().any(|e| {
        matches!(&e.happening, Happening::TechnologyLost { city: 0, technology, .. } if technology == "Pottery")
    });
    assert!(lost, "{:?}", sim.journal().entries());
}
