use std::process::ExitCode;

use societies::Simulation;
use societies::config::SimConfig;
use societies::events::SimEvent;

const USAGE: &str = "usage: societies [--seed N] [--turns N] [--width N] [--height N]";

fn main() -> ExitCode {
    let mut config = SimConfig::default();
    let mut turns: u64 = 200;

    let mut args = std::env::args().skip(1);
    while let Some(flag) = args.next() {
        if flag == "-h" || flag == "--help" {
            println!("{USAGE}");
            return ExitCode::SUCCESS;
        }
        let Some(value) = args.next().and_then(|v| v.parse::<u64>().ok()) else {
            eprintln!("missing or invalid value for {flag}\n{USAGE}");
            return ExitCode::FAILURE;
        };
        match flag.as_str() {
            "--seed" => config.seed = value,
            "--turns" => turns = value,
            "--width" => config.map_width = value as u32,
            "--height" => config.map_height = value as u32,
            _ => {
                eprintln!("unknown argument {flag}\n{USAGE}");
                return ExitCode::FAILURE;
            }
        }
    }

    println!(
        "seed {}, {} turns, {}x{} map",
        config.seed, turns, config.map_width, config.map_height
    );
    let mut sim = Simulation::new(config);
    sim.run(turns);

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

    println!("\nturn {}:", sim.turn());
    println!("{}\n", sim.render_map());
    let cities = sim.cities();
    for c in &cities {
        println!(
            "#{:<3} {:<12} ({:>2},{:>2})  pop {:>6}  founded turn {}",
            c.id.0, c.name, c.position.x, c.position.y, c.population, c.founded_on
        );
    }
    let total: u64 = cities.iter().map(|c| c.population).sum();
    println!("\n{} cities, total population {total}", cities.len());
    println!("state digest {:016x}", sim.state_digest());
    ExitCode::SUCCESS
}
