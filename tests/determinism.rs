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
    let mut sim = Simulation::new(config(99));
    sim.run(400);
    let cities = sim.cities();
    assert!(!cities.is_empty(), "expected some cities after 400 turns");
    let cap = SimConfig::default().city_carrying_capacity;
    for c in &cities {
        assert!(c.population >= 10 && c.population <= cap);
        if c.founded_on == sim.turn() {
            assert_eq!(c.population, 10);
        }
    }
    let oldest = &cities[0];
    assert!(oldest.population > 10, "oldest city should have grown");
}

#[test]
fn cities_fill_only_habitable_tiles_once() {
    let mut sim = Simulation::new(SimConfig {
        seed: 5,
        map_width: 24,
        map_height: 12,
        city_spawn_chance_per_mille: 1000,
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
