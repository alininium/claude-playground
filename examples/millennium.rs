//! Runs a world for many years and logs its people's history: a summary
//! every `every` years, and every famine and technology journal entry.
//!
//! Usage: cargo run --release --example millennium -- [seed] [years] [every]
use std::io::Write;
use std::time::Instant;

use societies::Simulation;
use societies::config::SimConfig;
use societies::people::life::Vitals;

fn main() {
    let args: Vec<u64> = std::env::args()
        .skip(1)
        .filter_map(|a| a.parse().ok())
        .collect();
    let seed = args.first().copied().unwrap_or(0);
    let years = args.get(1).copied().unwrap_or(1000);
    let every = args.get(2).copied().unwrap_or(10).max(1);
    let mut sim = Simulation::new(SimConfig {
        seed,
        ..SimConfig::default()
    });
    let tpy = sim.config().turns_per_year();
    let mut seen = 0;
    let start = Instant::now();
    let mut out = std::io::stdout();
    for year in 1..=years {
        sim.run(tpy);
        let entries = sim.journal().entries();
        for e in &entries[seen..] {
            if e.happening.is_society() {
                writeln!(out, "  {e}").unwrap();
            }
        }
        seen = entries.len();
        if year % every == 0 || year == years {
            let people = sim.people().len();
            let cities = sim.cities();
            let known: usize = cities.iter().map(|c| c.knowledge.count()).sum();
            let v = sim.world().resource::<Vitals>().clone();
            writeln!(
                out,
                "year {year:>4}: {people:>6} people in {:>3} cities, {:.1} technologies per city, {} births and {} deaths so far, {:.0} s",
                cities.len(),
                known as f64 / cities.len().max(1) as f64,
                v.births,
                v.deaths,
                start.elapsed().as_secs_f64()
            )
            .unwrap();
        }
        out.flush().unwrap();
    }
}
