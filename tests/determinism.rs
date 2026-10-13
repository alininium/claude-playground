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
fn cities_start_at_ten_and_grow_slowly() {
    let mut sim = Simulation::new(SimConfig {
        map_width: 64,
        map_height: 32,
        city_growth_per_mille_per_year: 300,
        ..config(99)
    });
    let years = 3;
    sim.run(years * sim.config().turns_per_year());
    let cities = sim.cities();
    assert!(
        !cities.is_empty(),
        "expected some cities after {years} years"
    );
    let cap = SimConfig::default().city_carrying_capacity;
    for c in &cities {
        assert!(c.population >= 10 && c.population <= cap);
        if c.founded_on == sim.turn() {
            assert_eq!(c.population, 10);
        }
    }
    let oldest = &cities[0];
    assert!(oldest.population > 10, "oldest city should have grown");
    // 30% a year compounds to at most 10 · 1.3³ ≈ 22 in three years.
    assert!(
        oldest.population <= 22,
        "grew too fast: {}",
        oldest.population
    );
}

#[test]
fn cities_fill_only_habitable_tiles_once() {
    let mut sim = Simulation::new(SimConfig {
        seed: 5,
        map_width: 24,
        map_height: 12,
        city_foundings_per_year: u32::MAX,
        ..SimConfig::default()
    });
    let habitable = (0..sim.map().tile_count() as usize)
        .filter(|&i| sim.map().is_habitable(i))
        .count();
    assert!(habitable > 0);
    sim.run(habitable as u64 + 10);
    let cities = sim.cities();
    assert_eq!(
        cities.len(),
        habitable,
        "every habitable tile should be settled"
    );
    let mut positions: Vec<_> = cities.iter().map(|c| c.position).collect();
    positions.sort();
    positions.dedup();
    assert_eq!(positions.len(), habitable, "two cities share a tile");
    for c in &cities {
        assert!(sim.map().is_habitable(sim.map().index(c.position)));
    }
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
