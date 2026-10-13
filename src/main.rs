use std::path::PathBuf;
use std::process::ExitCode;

use societies::Simulation;
use societies::config::SimConfig;
use societies::events::SimEvent;
use societies::map::{Biome, Water};
use societies::render;
use societies::time::{TURNS_PER_DAY, TimeOfDay};

const USAGE: &str = "usage: societies [--seed N] [--years N | --days N | --turns N] \
[--width N] [--height N] [--land PER_MILLE] [--png FILE] [--temp-png FILE] \
[--scale PIXELS_PER_TILE] [--quiet]

One turn is a quarter of a day (morning, day, evening, night). Default: 2 years.";

fn main() -> ExitCode {
    let mut config = SimConfig::default();
    let mut turns: Option<u64> = None;
    let mut days: Option<u64> = None;
    let mut years: Option<u64> = None;
    let mut png: Option<PathBuf> = None;
    let mut temp_png: Option<PathBuf> = None;
    let mut scale: u32 = 4;
    let mut quiet = false;

    let mut args = std::env::args().skip(1);
    while let Some(flag) = args.next() {
        match flag.as_str() {
            "-h" | "--help" => {
                println!("{USAGE}");
                return ExitCode::SUCCESS;
            }
            "--quiet" => {
                quiet = true;
                continue;
            }
            "--png" | "--temp-png" => {
                let Some(path) = args.next() else {
                    eprintln!("missing value for {flag}\n{USAGE}");
                    return ExitCode::FAILURE;
                };
                if flag == "--png" {
                    png = Some(path.into());
                } else {
                    temp_png = Some(path.into());
                }
                continue;
            }
            _ => {}
        }
        let Some(value) = args.next().and_then(|v| v.parse::<u64>().ok()) else {
            eprintln!("missing or invalid value for {flag}\n{USAGE}");
            return ExitCode::FAILURE;
        };
        match flag.as_str() {
            "--seed" => config.seed = value,
            "--turns" => turns = Some(value),
            "--days" => days = Some(value),
            "--years" => years = Some(value),
            "--width" => config.map_width = value as u32,
            "--height" => config.map_height = value as u32,
            "--land" => config.terrain.land_per_mille = value as u32,
            "--scale" => scale = value as u32,
            _ => {
                eprintln!("unknown argument {flag}\n{USAGE}");
                return ExitCode::FAILURE;
            }
        }
    }
    if config.map_width < 2 || config.map_height < 2 {
        eprintln!("map must be at least 2x2");
        return ExitCode::FAILURE;
    }

    let turns = turns
        .or(days.map(|d| d * TURNS_PER_DAY))
        .unwrap_or(years.unwrap_or(2) * config.turns_per_year());

    println!(
        "seed {}, {} turns, {}x{} map",
        config.seed, turns, config.map_width, config.map_height
    );
    let mut sim = Simulation::new(config);
    print_map_summary(&sim);
    if !quiet {
        println!("\nclimate at the start of each season (afternoon):");
    }
    for _ in 0..turns {
        sim.step();
        let date = sim.date();
        if !quiet && date.day_of_season == 0 && date.time_of_day == TimeOfDay::Day {
            print_climate_report(&sim);
        }
    }

    if !quiet {
        for event in sim.events().events() {
            match event {
                SimEvent::CityFounded {
                    turn,
                    id,
                    name,
                    position,
                } => println!(
                    "turn {turn:>4}: city #{} {name} founded at ({}, {})",
                    id.0, position.x, position.y
                ),
            }
        }
        println!("\nturn {} ({}):", sim.turn(), sim.date());
        println!("{}\n", sim.render_map());
    }

    let cities = sim.cities();
    if !quiet {
        for c in &cities {
            let tile = sim.map().tile(c.position);
            println!(
                "#{:<3} {:<12} ({:>3},{:>3})  pop {:>6}  founded turn {:<5}  {:>5.1}°C  {:?}{} {}m",
                c.id.0,
                c.name,
                c.position.x,
                c.position.y,
                c.population,
                c.founded_on,
                f64::from(sim.climate().temperature(sim.map().index(c.position))) / 10.0,
                tile.biome,
                if tile.water == Water::River {
                    " + river"
                } else {
                    ""
                },
                tile.altitude,
            );
        }
    }
    let total: u64 = cities.iter().map(|c| c.population).sum();
    println!("\n{} cities, total population {total}", cities.len());

    if let Some(path) = png {
        if let Err(e) = render::png(sim.map(), &cities, scale, &path) {
            eprintln!("failed to write {}: {e}", path.display());
            return ExitCode::FAILURE;
        }
        println!("map written to {}", path.display());
    }
    if let Some(path) = temp_png {
        if let Err(e) = render::temperature_png(sim.map(), sim.climate(), scale, &path) {
            eprintln!("failed to write {}: {e}", path.display());
            return ExitCode::FAILURE;
        }
        println!(
            "temperature map ({}) written to {}",
            sim.date(),
            path.display()
        );
    }
    println!("state digest {:016x}", sim.state_digest());
    ExitCode::SUCCESS
}

fn print_map_summary(sim: &Simulation) {
    let tiles = sim.map().tiles();
    let pct = |n: usize| n as f64 * 100.0 / tiles.len() as f64;
    let count = |b: Biome| tiles.iter().filter(|t| t.biome == b).count();
    let water = |w: Water| tiles.iter().filter(|t| t.water == w).count();
    let highest = tiles.iter().map(|t| t.altitude).max().unwrap_or(0);
    let deepest = tiles.iter().map(|t| t.altitude).min().unwrap_or(0);
    println!(
        "terrain: water {:.1}% (ocean {:.1}%, lakes {:.1}%), earth {:.1}%, sand {:.1}%, \
hills {:.1}%, mountains {:.1}%; rivers {:.1}%; altitude {deepest}m..{highest}m",
        pct(count(Biome::Water)),
        pct(water(Water::Ocean)),
        pct(water(Water::Lake)),
        pct(count(Biome::Earth)),
        pct(count(Biome::Sand)),
        pct(count(Biome::Hills)),
        pct(count(Biome::Mountains)),
        pct(water(Water::River)),
    );
}

fn print_climate_report(sim: &Simulation) {
    let climate = sim.climate();
    let config = &sim.config().climate.oscillation;
    let osc = climate.oscillation();
    let temps = climate.temperatures();
    let (hot, cold) = (
        temps.iter().copied().max().unwrap_or(0),
        temps.iter().copied().min().unwrap_or(0),
    );
    let date = sim.date();
    println!(
        "  year {:>3} {:<6}  global mean {:>5.1}°C  range {:>5.1}..{:>5.1}°C  \
oscillation {:>+5.1}°C ({:?})  drift {:>+4.1}°C",
        date.year,
        format!("{:?}", date.season),
        climate.global_mean() as f64 / 10.0,
        f64::from(cold) / 10.0,
        f64::from(hot) / 10.0,
        osc.index_tenths() as f64 / 10.0,
        osc.phase(config),
        climate.drift().anomaly_tenths() as f64 / 10.0,
    );
}
