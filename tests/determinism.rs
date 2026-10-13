use societies::Simulation;
use societies::config::SimConfig;

fn config(seed: u64) -> SimConfig {
    SimConfig {
        seed,
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
    let mut sim = Simulation::new(SimConfig {
        map_width: 64,
        map_height: 32,
        city_growth_per_mille_per_year: 300,
        ..config(99)
    });
    let start = sim.cities();
    assert!(!start.is_empty());
    assert!(start.len() <= sim.config().initial_cities as usize);
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
fn initial_cities_are_spaced_out_on_settleable_land() {
    let mut sim = Simulation::new(SimConfig {
        initial_cities: 40,
        ..config(5)
    });
    let cities = sim.cities();
    assert_eq!(cities.len(), 40);
    let spacing = sim.config().min_city_distance;
    for (i, a) in cities.iter().enumerate() {
        let index = sim.map().index(a.position);
        assert!(societies::city::can_settle(sim.map(), sim.climate(), index));
        for b in &cities[i + 1..] {
            let d = a
                .position
                .x
                .abs_diff(b.position.x)
                .max(a.position.y.abs_diff(b.position.y));
            assert!(
                d >= spacing,
                "{} and {} are {d} tiles apart",
                a.name,
                b.name
            );
        }
    }
}

#[test]
fn cities_prefer_fertile_land() {
    let mut sim = Simulation::new(SimConfig {
        initial_cities: 30,
        ..config(8)
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
    let mut sim = Simulation::new(config(11));
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
