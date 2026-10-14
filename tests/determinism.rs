mod common;

use common::{Fixture, sim_with};
use societies::Simulation;
use societies::config::SimConfig;

/// Fresh generation (not a fixture): these tests cover generation too.
fn config(seed: u64) -> SimConfig {
    SimConfig {
        seed,
        map_width: 64,
        map_height: 32,
        ..SimConfig::default()
    }
}

#[test]
fn same_seed_produces_identical_history() {
    let mut a = Simulation::new(config(1234));
    let mut b = Simulation::new(config(1234));
    for _ in 0..500 {
        a.step();
        b.step();
        assert_eq!(
            a.state_digest(),
            b.state_digest(),
            "diverged at turn {}",
            a.turn()
        );
    }
    assert_eq!(a.cities(), b.cities());
    assert_eq!(a.events().events(), b.events().events());
}

#[test]
fn different_seeds_diverge() {
    let mut a = Simulation::new(config(1));
    let mut b = Simulation::new(config(2));
    a.run(300);
    b.run(300);
    assert_ne!(a.state_digest(), b.state_digest());
}

#[test]
fn cities_are_placed_at_generation_and_only_grow() {
    let mut sim = sim_with(Fixture::Small, |c| c.city_growth_per_mille_per_year = 300);
    let start = sim.cities();
    assert!(!start.is_empty());
    let config = sim.config();
    assert!(start.len() <= (config.initial_cities + config.initial_barbarians) as usize);
    for c in &start {
        assert_eq!((c.founded_on, c.population), (0, 10));
    }

    let years = 3;
    sim.run(years * sim.config().turns_per_year());
    let end = sim.cities();
    assert_eq!(
        end.iter().map(|c| (c.id, c.position)).collect::<Vec<_>>(),
        start.iter().map(|c| (c.id, c.position)).collect::<Vec<_>>(),
        "no cities may appear (or move) during play"
    );
    for c in &end {
        assert!(c.population > 10, "{} did not grow", c.name);
        // 30% a year, compounded every turn: at most 10 · e^0.9 ≈ 24.6 in three years.
        assert!(
            c.population <= 24,
            "{} grew too fast: {}",
            c.name,
            c.population
        );
    }
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
fn seasons_and_days_change_temperatures() {
    let mut sim = common::sim(Fixture::Medium);
    let days_per_season = u64::from(sim.config().climate.days_per_year) / 4;
    let day = societies::time::TURNS_PER_DAY;
    // A land tile far north of the equator.
    let map = sim.map();
    let north_land = (0..map.tile_count() as usize)
        .find(|&i| map.latitude_tenths(map.position_of(i as u32).y) > 450 && map.is_habitable(i))
        .expect("northern land");

    // Mid-summer afternoon vs mid-summer night, and mid-winter afternoon.
    sim.run((days_per_season * 3 / 2) * day + 1);
    let summer_day = sim.climate().temperature(north_land);
    sim.run(2);
    let summer_night = sim.climate().temperature(north_land);
    sim.run(2 * days_per_season * day - 2);
    let winter_day = sim.climate().temperature(north_land);
    assert!(summer_day > summer_night, "{summer_day} vs {summer_night}");
    assert!(summer_day > winter_day + 50, "{summer_day} vs {winter_day}");
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

    // Playing on never founds (or logs) another city, and the log is append-only.
    let before = sim.events().events().to_vec();
    sim.run(sim.config().turns_per_year());
    let after = sim.events().events();
    assert_eq!(&after[..before.len()], &before[..], "history was rewritten");
    assert!(
        after[before.len()..]
            .iter()
            .all(|e| !matches!(e, SimEvent::CityFounded { .. })),
        "a city was founded during play"
    );
}
