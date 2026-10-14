//! Runs a world for some years and prints a yearly summary of its people.
//!
//! Usage: cargo run --release --example society -- [seed] [years]
use std::collections::BTreeMap;

use societies::Simulation;
use societies::config::SimConfig;
use societies::economy::{Larder, Outlook, Stores};
use societies::people::ActionKind;
use societies::people::life::Vitals;

fn main() {
    let args: Vec<u64> = std::env::args()
        .skip(1)
        .filter_map(|a| a.parse().ok())
        .collect();
    let seed = args.first().copied().unwrap_or(0);
    let years = args.get(1).copied().unwrap_or(10);
    let mut sim = Simulation::new(SimConfig {
        seed,
        ..SimConfig::default()
    });
    let tpy = sim.config().turns_per_year();
    println!(
        "year  people  kids  cities  caravans  births/y deaths/y  food_days(min/med)  free_land0  idle%  settle  craft  statuses  tools"
    );
    let mut last = (0u64, 0u64);
    for year in 0..=years {
        if year > 0 {
            let t0 = std::time::Instant::now();
            sim.run(tpy);
            let _ = t0;
        }
        let turn = sim.turn();
        let people = sim.people();
        let kids = people
            .iter()
            .filter(|p| p.person.age_years(turn, tpy) < 12)
            .count();
        let cities = sim.cities();
        let world = sim.world_mut();
        let caravans = societies::caravan::caravans(world).len();
        let v = world.resource::<Vitals>().clone();
        let mut q = world.query::<(&Outlook, &Stores, Option<&Larder>)>();
        let mut food: Vec<u32> = Vec::new();
        let mut no_land = 0;
        let mut idle = (0u64, 0u64);
        let mut tools = 0u64;
        for (o, s, l) in q.iter(world) {
            food.push(o.food_days);
            if o.free_land == 0 {
                no_land += 1;
            }
            tools += s.items.iter().map(|&n| u64::from(n)).sum::<u64>();
            if let Some(l) = l {
                idle.0 += u64::from(l.last_day.idle.iter().sum::<u32>());
                idle.1 += u64::from(l.last_day.workers.iter().sum::<u32>())
                    + u64::from(l.last_day.idle.iter().sum::<u32>());
            }
        }
        food.sort();
        let mut statuses: BTreeMap<u16, usize> = BTreeMap::new();
        for p in &people {
            *statuses.entry(p.status.0).or_default() += 1;
        }
        let settle = people
            .iter()
            .filter(|p| p.activity.action == ActionKind::SettleAway)
            .count();
        let craft = people
            .iter()
            .filter(|p| p.activity.action == ActionKind::Craft)
            .count();
        println!(
            "{year:>4} {:>7} {:>5} {:>7} {:>9} {:>9} {:>8} {:>8}/{:<8} {:>10} {:>6} {:>7} {:>6}  {:?} {}",
            people.len(),
            kids,
            cities.len(),
            caravans,
            v.births - last.0,
            v.deaths - last.1,
            food.first().unwrap_or(&0),
            food.get(food.len() / 2).unwrap_or(&0),
            no_land,
            (idle.0 * 100).checked_div(idle.1).unwrap_or(0),
            settle,
            craft,
            statuses,
            tools
        );
        last = (v.births, v.deaths);
    }
}
