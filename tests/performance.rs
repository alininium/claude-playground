//! Performance budgets. Timing only means something in optimized builds, so
//! these are skipped in debug builds: run `cargo test --release --test performance`.
//! Budgets are about 3x what a development container measures, to absorb
//! slower CI machines; a failure means something got several times slower.

use std::time::{Duration, Instant};

use societies::Simulation;
use societies::config::SimConfig;

mod common;

fn config(width: u32, height: u32) -> SimConfig {
    SimConfig {
        seed: 1,
        map_width: width,
        map_height: height,
        ..SimConfig::default()
    }
}

fn timed<T>(f: impl FnOnce() -> T) -> (T, Duration) {
    let start = Instant::now();
    let out = f();
    (out, start.elapsed())
}

fn within(what: &str, took: Duration, budget_ms: u64) {
    println!("{what}: {took:?} (budget {budget_ms} ms)");
    assert!(
        took <= Duration::from_millis(budget_ms),
        "{what} took {took:?}, budget {budget_ms} ms"
    );
}

#[test]
#[cfg_attr(debug_assertions, ignore = "timing needs `cargo test --release`")]
fn world_creation_256x128() {
    let (_, took) = timed(|| Simulation::new(config(256, 128)));
    within("create 256x128 world", took, BUDGET_CREATE_SMALL);
}

#[test]
#[cfg_attr(debug_assertions, ignore = "timing needs `cargo test --release`")]
fn world_creation_1024x512() {
    let (_, took) = timed(|| Simulation::new(config(1024, 512)));
    within("create 1024x512 world", took, BUDGET_CREATE_LARGE);
}

#[test]
#[cfg_attr(debug_assertions, ignore = "timing needs `cargo test --release`")]
fn one_year_on_256x128() {
    let mut sim = Simulation::new(config(256, 128));
    let turns = sim.config().turns_per_year();
    let (_, took) = timed(|| sim.run(turns));
    within("simulate 1 year on 256x128", took, BUDGET_YEAR_SMALL);
}

#[test]
#[cfg_attr(debug_assertions, ignore = "timing needs `cargo test --release`")]
fn one_day_on_1024x512() {
    let mut sim = Simulation::new(config(1024, 512));
    let (_, took) = timed(|| sim.run(4));
    within("simulate 1 day on 1024x512", took, BUDGET_DAY_LARGE);
}

/// The status step alone, on a day when every city reviews its statuses and
/// recounts its knowledge every turn and holds a learning round (normally
/// these come once a season, a year and a month), scaled to 10 000 people.
#[test]
#[cfg_attr(debug_assertions, ignore = "timing needs `cargo test --release`")]
fn a_busy_day_of_statuses_per_10000_people() {
    use societies::people::status::{self, StatusCatalog, StatusSchedule};
    let mut sim = Simulation::new(config(256, 128));
    sim.step();
    let people = sim.people().len() as u64;
    let world = sim.world_mut();
    let mut catalog = world.resource::<StatusCatalog>().config.clone();
    catalog.learn_days = 1;
    world.insert_resource(StatusCatalog::new(catalog));
    let mut schedule = bevy_ecs::schedule::Schedule::default();
    schedule.add_systems(status::systems());
    let (_, took) = timed(|| {
        for _ in 0..4 {
            let turn = world.resource::<societies::time::Turn>().0 + 1;
            world.resource_mut::<societies::time::Turn>().0 = turn;
            let mut due = world.query::<&mut StatusSchedule>();
            for mut s in due.iter_mut(world) {
                s.next_review = turn;
                s.next_knowledge = turn;
            }
            schedule.run(world);
        }
    });
    let scaled = took * 10_000 / people.max(1) as u32;
    println!("{people} people");
    within(
        "a busy day of statuses, per 10 000 people",
        scaled,
        BUDGET_STATUS_DAY,
    );
}

#[test]
#[cfg_attr(debug_assertions, ignore = "timing needs `cargo test --release`")]
fn loading_a_saved_256x128_world() {
    let bytes = Simulation::generate_world(&config(256, 128))
        .to_bytes()
        .unwrap();
    let (_, took) = timed(|| {
        let save = societies::save::WorldSave::from_bytes(&bytes).unwrap();
        Simulation::from_world(save, config(256, 128))
    });
    within("load a saved 256x128 world", took, BUDGET_LOAD_SMALL);
}

/// A day of needs and decisions with about 10 000 people: the small fixture
/// world (whose own day costs little) with every settlement made bigger.
#[test]
#[cfg_attr(debug_assertions, ignore = "timing needs `cargo test --release`")]
fn a_day_of_decisions_for_10000_people() {
    use societies::people::founding::{FoundingConfig, Settlement, spawn_settlement};
    let mut sim = common::sim(common::Fixture::Small);
    let cities = sim.cities();
    let (turn, tpy) = (sim.turn(), sim.config().turns_per_year());
    let tree = sim.technologies().clone();
    let founding = FoundingConfig::builtin();
    let mut rng = societies::rng::SimRng::from_seed(99);
    let have = sim.people().len() as u32;
    let extra = 10_000u32.saturating_sub(have) / cities.len() as u32;
    for city in &cities {
        let settlement = Settlement {
            city: city.id,
            size: extra,
            knowledge: &city.knowledge,
            turn,
            turns_per_year: tpy,
        };
        spawn_settlement(sim.world_mut(), &settlement, &tree, &founding, &mut rng);
    }
    assert!(sim.people().len() >= 9_900);
    sim.step(); // the first turn also gives everyone their decision state
    let (_, took) = timed(|| sim.run(4));
    within(
        "a day of decisions for 10 000 people",
        took,
        BUDGET_DAY_PEOPLE,
    );
}

// Measured in a development container (release build): creation 2.1 s and
// 47 s, one year 1.45 s, one large day 96 ms, loading 31 ms. Per-step speed
// matters most, so those budgets are the tight ones.
const BUDGET_CREATE_SMALL: u64 = 6_000;
const BUDGET_CREATE_LARGE: u64 = 150_000;
const BUDGET_YEAR_SMALL: u64 = 4_500;
const BUDGET_DAY_LARGE: u64 = 300;
const BUDGET_LOAD_SMALL: u64 = 300;
/// The people's own budget is 10 ms a day; the small world's turns add a
/// little.
const BUDGET_DAY_PEOPLE: u64 = 30;

// The status step's worst day (everything due, four times over): 30 ms in
// a debug build, not yet measured in release. An ordinary day does a small
// fraction of this work.
const BUDGET_STATUS_DAY: u64 = 30;
