//! Starting settlements: civilized cities in cradles, barbarian tribes,
//! spacing, their starting size and knowledge, and the founding events.

mod common;

use common::{Fixture, sim_with};

#[test]
fn settlements_start_with_people_and_stay_put() {
    use societies::city::People;
    let mut sim = common::sim(Fixture::Small);
    let start = sim.cities();
    assert!(!start.is_empty());
    let config = sim.config().clone();
    assert!(start.len() <= (config.initial_cities + config.initial_barbarians) as usize);
    let people = sim.people();
    for c in &start {
        assert_eq!(c.founded_on, 0);
        let [lo, hi] = match c.people {
            People::Civilized => config.city_population,
            People::Barbarian => config.barbarian_population,
        };
        assert!(
            (u64::from(lo)..=u64::from(hi)).contains(&c.population),
            "{} {:?} starts with {}",
            c.name,
            c.people,
            c.population
        );
        let living = people.iter().filter(|p| p.person.home == c.id).count();
        assert_eq!(living as u64, c.population, "{}", c.name);
    }

    // The starting settlements are all placed at world creation and never
    // move. Cities founded later (by caravans) come after them, with new
    // ids and a founding date during play.
    sim.run(sim.config().turns_per_year() / 4);
    let end = sim.cities();
    assert_eq!(
        end[..start.len()]
            .iter()
            .map(|c| (c.id, c.position, c.founded_on))
            .collect::<Vec<_>>(),
        start
            .iter()
            .map(|c| (c.id, c.position, c.founded_on))
            .collect::<Vec<_>>(),
        "starting settlements may not move or disappear"
    );
    for (k, c) in end.iter().enumerate().skip(start.len()) {
        assert_eq!(c.id.0 as usize, k, "ids are handed out in order");
        assert!(c.founded_on > 0, "{} founded at creation", c.name);
    }
}

#[test]
fn richer_land_starts_bigger() {
    use societies::city::{People, settlement_size};
    let sim = common::sim(Fixture::Medium);
    let (map, fertility, config) = (sim.map(), sim.fertility(), sim.config());
    let land: Vec<usize> = (0..map.tile_count() as usize)
        .filter(|&i| map.is_habitable(i))
        .collect();
    let poorest = *land
        .iter()
        .min_by_key(|&&i| fertility.fertility(i))
        .unwrap();
    let richest = *land
        .iter()
        .max_by_key(|&&i| fertility.fertility(i))
        .unwrap();
    for people in [People::Civilized, People::Barbarian] {
        let (small, big) = (
            settlement_size(map, fertility, config, people, poorest),
            settlement_size(map, fertility, config, people, richest),
        );
        assert!(small < big, "{people:?}: {small} vs {big}");
    }
    assert!(
        settlement_size(map, fertility, config, People::Barbarian, richest)
            <= settlement_size(map, fertility, config, People::Civilized, richest)
    );
}

#[test]
fn initial_settlements_are_spaced_out_on_settleable_land() {
    let mut sim = sim_with(Fixture::Medium, |c| {
        c.initial_cities = 10;
        c.initial_barbarians = 30;
    });
    let cities = sim.cities();
    assert_eq!(cities.len(), 40);
    let config = sim.config().clone();
    for (i, a) in cities.iter().enumerate() {
        let index = sim.map().index(a.position);
        assert!(societies::city::can_settle(sim.map(), sim.climate(), index));
        for b in &cities[i + 1..] {
            assert!(
                !societies::city::too_close(sim.map(), &config, a.position, b.position),
                "{} {:?} and {} {:?} are too close",
                a.name,
                a.position,
                b.name,
                b.position
            );
            let tiles = a
                .position
                .x
                .abs_diff(b.position.x)
                .max(a.position.y.abs_diff(b.position.y));
            assert!(tiles >= 2, "{} and {} are neighbors", a.name, b.name);
        }
    }
}

#[test]
fn spacing_is_measured_on_the_globe() {
    use societies::city::too_close;
    use societies::map::Position;
    let sim = common::sim(Fixture::Medium);
    let (map, config) = (sim.map(), sim.config());
    // 128×64 on a 40 000 km planet: 312.5 km per tile. 800 km takes three
    // tiles north-south or east-west near the equator, but near the poles
    // tiles are narrow and it takes many more east-west.
    let p = |x, y| Position { x, y };
    assert!(too_close(map, config, p(10, 32), p(10, 34)));
    assert!(!too_close(map, config, p(10, 32), p(10, 35)));
    assert!(too_close(map, config, p(10, 32), p(12, 32)));
    assert!(!too_close(map, config, p(10, 32), p(13, 32)));
    assert!(too_close(map, config, p(10, 3), p(20, 3)));
    assert!(!too_close(map, config, p(10, 3), p(40, 3)));
    // Never on neighboring tiles, however small the limit.
    let mut loose = config.clone();
    loose.min_city_distance_km = 0;
    assert!(too_close(map, &loose, p(10, 3), p(11, 4)));
    assert!(!too_close(map, &loose, p(10, 3), p(12, 3)));
}

#[test]
fn civilized_cities_start_in_cradles() {
    use societies::city::{People, cradle_score};
    use societies::map::Soil;
    use societies::tech::Site;
    let mut sim = common::sim(Fixture::Medium);
    let cities = sim.cities();
    let civilized: Vec<_> = cities
        .iter()
        .filter(|c| c.people == People::Civilized)
        .collect();
    assert_eq!(civilized.len() as u32, sim.config().initial_cities);
    let (map, biomes) = (sim.map(), sim.biomes());
    let mut river = 0;
    let mut alluvial = 0;
    for c in &civilized {
        let i = map.index(c.position);
        assert!(
            cradle_score(map, sim.climate(), biomes, sim.fertility(), i) > 0,
            "{} is not on a cradle site",
            c.name
        );
        let site = Site::of(map, biomes, i);
        assert!(site.river || site.lake || site.coast);
        let t = sim.climate().normal(i).mean_tenths();
        assert!((100..=250).contains(&t), "{} has a mean of {t}", c.name);
        river += usize::from(site.river);
        alluvial += usize::from(site.soil == Soil::Alluvial);
    }
    // Most are river cities, many on floodplains.
    assert!(
        river * 4 >= civilized.len() * 3,
        "{river} of {} on rivers",
        civilized.len()
    );
    assert!(
        alluvial * 2 >= civilized.len(),
        "{alluvial} of {} on alluvium",
        civilized.len()
    );
    // Barbarians come after the cities.
    let first_barbarian = cities.iter().position(|c| c.people == People::Barbarian);
    assert_eq!(first_barbarian, Some(civilized.len()));
}

#[test]
fn barbarians_know_less_than_cities() {
    use societies::city::People;
    let mut sim = common::sim(Fixture::Medium);
    let cities = sim.cities();
    let mean = |people: People| {
        let known: Vec<usize> = cities
            .iter()
            .filter(|c| c.people == people)
            .map(|c| c.knowledge.count())
            .collect();
        assert!(!known.is_empty(), "no {people:?}");
        known.iter().sum::<usize>() * 10 / known.len()
    };
    let (civilized, barbarian) = (mean(People::Civilized), mean(People::Barbarian));
    assert!(
        barbarian * 3 < civilized * 2,
        "barbarians know {barbarian} vs {civilized} (tenths)"
    );
    // Nobody lacks the baseline.
    let fire = sim.technologies().id("fire").unwrap();
    assert!(cities.iter().all(|c| c.knowledge.knows(fire)));
}

#[test]
fn cities_prefer_fertile_land() {
    let mut sim = sim_with(Fixture::Medium, |c| {
        c.initial_cities = 0;
        c.initial_barbarians = 30;
    });
    let cities = sim.cities();
    let map = sim.map();
    let fertility = sim.fertility();
    let land: Vec<usize> = (0..map.tile_count() as usize)
        .filter(|&i| societies::city::can_settle(map, sim.climate(), i))
        .collect();
    let land_avg = land
        .iter()
        .map(|&i| u64::from(fertility.fertility(i)))
        .sum::<u64>()
        / land.len() as u64;
    let city_avg = cities
        .iter()
        .map(|c| u64::from(fertility.fertility(map.index(c.position))))
        .sum::<u64>()
        / cities.len() as u64;
    assert!(city_avg > land_avg, "cities {city_avg} vs land {land_avg}");
}

#[test]
fn event_log_records_every_city_founding_exactly_once() {
    use societies::events::SimEvent;
    let mut sim = common::sim(Fixture::Medium);
    let cities = sim.cities();
    let founded: Vec<_> = sim
        .events()
        .events()
        .iter()
        .filter_map(|e| match e {
            SimEvent::CityFounded {
                turn,
                id,
                name,
                position,
                people,
            } => Some((*turn, *id, name.clone(), *position, *people)),
            #[allow(unreachable_patterns)]
            _ => None,
        })
        .collect();
    let expected: Vec<_> = cities
        .iter()
        .map(|c| (0, c.id, c.name.clone(), c.position, c.people))
        .collect();
    assert_eq!(
        founded, expected,
        "one founding event per city, in id order"
    );

    // Playing on, with a caravan sent out to found a new city: the log is
    // append-only, and every city founded is logged exactly once, on the
    // turn it was founded.
    let before = sim.events().events().to_vec();
    send_caravan(&mut sim, cities[0].id);
    sim.run(sim.config().turns_per_year() / 2);
    let cities = sim.cities();
    let after = sim.events().events();
    assert_eq!(&after[..before.len()], &before[..], "history was rewritten");
    assert!(
        cities.len() > expected.len(),
        "no city was founded during play"
    );
    let founded: Vec<_> = after
        .iter()
        .filter_map(|e| match e {
            SimEvent::CityFounded {
                turn,
                id,
                name,
                position,
                people,
            } => Some((*turn, *id, name.clone(), *position, *people)),
            #[allow(unreachable_patterns)]
            _ => None,
        })
        .collect();
    let expected: Vec<_> = cities
        .iter()
        .map(|c| (c.founded_on, c.id, c.name.clone(), c.position, c.people))
        .collect();
    assert_eq!(
        founded, expected,
        "one founding event per city, in id order"
    );
}

/// Makes enough adults of `city` choose to settle away to send a caravan.
fn send_caravan(sim: &mut societies::Simulation, city: societies::city::CityId) {
    use societies::people::{ActionKind, Activity, Person};
    let (turn, tpy) = (sim.turn(), sim.config().turns_per_year());
    let adult = societies::people::founding::FoundingConfig::builtin().adult_age;
    let wanted = societies::caravan::CaravanConfig::builtin().min_adults as usize + 3;
    let world = sim.world_mut();
    let mut query = world.query::<(&societies::people::PersonId, &Person, &mut Activity)>();
    let mut adults: Vec<_> = query
        .iter_mut(world)
        .filter(|(_, p, _)| p.home == city && p.age_years(turn, tpy) >= adult)
        .collect();
    adults.sort_by_key(|(id, ..)| **id);
    for (_, _, activity) in adults.iter_mut().take(wanted) {
        **activity = Activity {
            action: ActionKind::SettleAway,
            since: turn,
            until: turn + 40,
        };
    }
}
