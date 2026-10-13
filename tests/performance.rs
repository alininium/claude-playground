//! Performance budgets. Timing only means something in optimized builds, so
//! these are skipped in debug builds: run `cargo test --release --test performance`.
//! Budgets are about 3x what a development container measures, to absorb
//! slower CI machines; a failure means something got several times slower.

use std::time::{Duration, Instant};

use societies::Simulation;
use societies::config::SimConfig;

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

const BUDGET_CREATE_SMALL: u64 = 10_000;
const BUDGET_CREATE_LARGE: u64 = 100_000;
const BUDGET_YEAR_SMALL: u64 = 10_000;
const BUDGET_DAY_LARGE: u64 = 10_000;
