//! Very long runs: the climate must stay stable (no runaway snowball or
//! hothouse, no unbounded ice growth, no counter overflow) over centuries.
//!
//! The 1000-year run takes a while, so it only runs in optimized builds:
//! `cargo test --release --test long_run`. The 100-year run always runs.

mod common;

use societies::climate::cryosphere::{Cover, GLACIER};
use societies::map::Water;

#[derive(Debug)]
struct Year {
    global_mean: i64,
    sea_ice_pct: f64,
    glacier_pct: f64,
    oscillation: i64,
    drift: i64,
}

/// Runs `years` years on a small map, checking invariants every year, and
/// returns one sample per year (taken at the start of each year).
fn run(years: u64, seed: u64) -> Vec<Year> {
    let _ = seed;
    let mut sim = common::sim(common::Fixture::Small);
    let tpy = sim.config().turns_per_year();
    let cap = sim.config().city_carrying_capacity;
    let n = sim.map().tile_count() as usize;
    let area: Vec<i64> = (0..n)
        .map(|i| sim.map().area_weight(sim.map().position_of(i as u32).y))
        .collect();
    let water: Vec<Water> = sim.map().tiles().iter().map(|t| t.water).collect();
    let share = |of: &dyn Fn(usize) -> bool, part: &dyn Fn(usize) -> bool| {
        let total: i64 = (0..n).filter(|&i| of(i)).map(|i| area[i]).sum();
        let hit: i64 = (0..n).filter(|&i| of(i) && part(i)).map(|i| area[i]).sum();
        hit as f64 * 100.0 / total.max(1) as f64
    };

    let mut samples = Vec::new();
    for year in 0..years {
        sim.run(tpy);
        let climate = sim.climate();
        let cryo = climate.cryosphere();
        for (i, &t) in climate.temperatures().iter().enumerate() {
            assert!(
                (-900..=600).contains(&t),
                "year {year}: tile {i} at {t} tenths °C"
            );
        }
        for f in cryo.tiles() {
            assert!(f.ice <= 50_000 && f.frost <= 30_000, "year {year}: {f:?}");
            assert!(
                f.glacier < u32::MAX / 2,
                "year {year}: glacier counter near overflow"
            );
        }
        let sample = Year {
            global_mean: climate.global_mean(),
            sea_ice_pct: share(&|i| water[i] == Water::Ocean, &|i| {
                cryo.cover(sim.map(), i) == Cover::SeaIce
            }),
            glacier_pct: share(&|i| matches!(water[i], Water::Dry | Water::River), &|i| {
                cryo.tile(i).glacier >= GLACIER
            }),
            oscillation: climate.oscillation().index_tenths(),
            drift: climate.drift().anomaly_tenths(),
        };
        // Spring-equinox global mean of an Earth-like planet: no snowball, no hothouse.
        assert!(
            (80..=250).contains(&sample.global_mean),
            "year {year}: {sample:?}"
        );
        assert!(sample.sea_ice_pct < 30.0, "year {year}: {sample:?}");
        assert!(sample.glacier_pct < 50.0, "year {year}: {sample:?}");
        samples.push(sample);
        for c in sim.cities() {
            assert!(c.population <= cap);
        }
    }
    samples
}

fn check_long_term(samples: &[Year]) {
    let n = samples.len();
    // Both oscillation phases happen, and the drift stays within a few sigma.
    assert!(samples.iter().any(|s| s.oscillation >= 5));
    assert!(samples.iter().any(|s| s.oscillation <= -5));
    let max_drift = samples.iter().map(|s| s.drift.abs()).max().unwrap();
    assert!(max_drift <= 40, "drift reached {max_drift} tenths °C");
    // Ice settles: averaged over the second and the last fifth of the run,
    // glaciers and sea ice differ by only a few points.
    let fifth = n / 5;
    let avg = |range: std::ops::Range<usize>, f: &dyn Fn(&Year) -> f64| {
        samples[range.clone()].iter().map(f).sum::<f64>() / range.len() as f64
    };
    let (g1, g2) = (
        avg(fifth..2 * fifth, &|s| s.glacier_pct),
        avg(n - fifth..n, &|s| s.glacier_pct),
    );
    let (s1, s2) = (
        avg(fifth..2 * fifth, &|s| s.sea_ice_pct),
        avg(n - fifth..n, &|s| s.sea_ice_pct),
    );
    assert!(
        (g1 - g2).abs() < 5.0,
        "glaciers kept changing: {g1:.1}% -> {g2:.1}%"
    );
    assert!(
        (s1 - s2).abs() < 5.0,
        "sea ice kept changing: {s1:.1}% -> {s2:.1}%"
    );
}

#[test]
fn a_century_stays_stable() {
    check_long_term(&run(100, 21));
}

#[test]
#[cfg_attr(
    debug_assertions,
    ignore = "takes minutes in debug builds; run with `cargo test --release`"
)]
fn a_millennium_stays_stable() {
    check_long_term(&run(1000, 7));
}
