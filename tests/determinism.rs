//! Same seed, same history: identical digests every turn; different seeds
//! diverge.

mod common;

use common::Fixture;
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
fn any_number_of_threads_gives_the_same_world_and_history() {
    // Split even this small map: the world must not depend on how the
    // per-tile work is divided (3 threads give uneven chunks).
    societies::parallel::set_min_tiles_per_thread(300);
    let generate = |threads| {
        Simulation::generate_world_with_threads(&config(77), threads)
            .to_bytes()
            .unwrap()
    };
    let one = generate(1);
    for threads in [3, 4] {
        assert!(
            generate(threads) == one,
            "world differs on {threads} threads"
        );
    }

    let run = |threads| {
        let save = societies::save::WorldSave::from_bytes(&one).unwrap();
        let mut sim = Simulation::from_world(save, config(77));
        sim.set_threads(threads);
        (0..200)
            .map(|_| {
                sim.step();
                sim.state_digest()
            })
            .collect::<Vec<_>>()
    };
    let one = run(1);
    assert_eq!(run(3), one);
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
