use std::path::PathBuf;
use std::process::ExitCode;

use societies::Simulation;
use societies::biome::Biome;
use societies::city::People;
use societies::climate::cryosphere::Cover;
use societies::config::SimConfig;
use societies::events::SimEvent;
use societies::map::{Landform, Water};
use societies::render;
use societies::tech::TechTree;
use societies::time::{TURNS_PER_DAY, TimeOfDay};

const USAGE: &str = "usage: societies [--seed N] [--years N | --days N | --turns N] \
[--width N] [--height N] [--land PER_MILLE] [--png FILE] [--temp-png FILE] \
[--fertility-png FILE] [--scale PIXELS_PER_TILE] [--load-world FILE] [--save-world FILE] [--technologies FILE] [--quiet]

--load-world starts from a saved world (its seed, size and terrain win over the flags);
--save-world saves the world at the end of the run (JSON if FILE ends in .json);
--technologies uses a technology file instead of the built-in data/technologies.json.

One turn is a quarter of a day (morning, day, evening, night). Default: 2 years.";

fn main() -> ExitCode {
    let mut config = SimConfig::default();
    let mut turns: Option<u64> = None;
    let mut days: Option<u64> = None;
    let mut years: Option<u64> = None;
    let mut png: Option<PathBuf> = None;
    let mut temp_png: Option<PathBuf> = None;
    let mut fertility_png: Option<PathBuf> = None;
    let mut load_world: Option<PathBuf> = None;
    let mut save_world: Option<PathBuf> = None;
    let mut technologies: Option<PathBuf> = None;
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
            "--png" | "--temp-png" | "--fertility-png" | "--load-world" | "--save-world"
            | "--technologies" => {
                let Some(path) = args.next() else {
                    eprintln!("missing value for {flag}\n{USAGE}");
                    return ExitCode::FAILURE;
                };
                match flag.as_str() {
                    "--png" => png = Some(path.into()),
                    "--temp-png" => temp_png = Some(path.into()),
                    "--load-world" => load_world = Some(path.into()),
                    "--save-world" => save_world = Some(path.into()),
                    "--technologies" => technologies = Some(path.into()),
                    _ => fertility_png = Some(path.into()),
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
    let tree = match technologies {
        Some(path) => match TechTree::load(&path) {
            Ok(tree) => tree,
            Err(e) => {
                eprintln!("failed to load {}: {e}", path.display());
                return ExitCode::FAILURE;
            }
        },
        None => TechTree::builtin(),
    };
    let save = match load_world {
        Some(path) => match Simulation::load_world(&path) {
            Ok(save) => save,
            Err(e) => {
                eprintln!("failed to load {}: {e}", path.display());
                return ExitCode::FAILURE;
            }
        },
        None => Simulation::generate_world(&config),
    };
    let mut sim = Simulation::from_world_with_technologies(save, config, tree);
    print_map_summary(&sim);
    print_volcano_summary(&mut sim);
    print_biome_summary(&sim);
    print_fertility_summary(&sim);
    print_frozen_summary(&sim);
    if !quiet {
        println!("\nclimate at the start of each season (afternoon):");
    }
    for _ in 0..turns {
        sim.step();
        let date = sim.date();
        if !quiet && date.day_of_season == 0 && date.time_of_day == TimeOfDay::Day {
            print_climate_report(&sim);
            print_frozen_summary(&sim);
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
                    people,
                } => println!(
                    "turn {turn:>4}: {} #{} {name} founded at ({}, {})",
                    match people {
                        People::Civilized => "city",
                        People::Barbarian => "barbarian tribe",
                    },
                    id.0,
                    position.x,
                    position.y
                ),
                SimEvent::CityAbandoned {
                    turn,
                    id,
                    name,
                    position,
                } => println!(
                    "turn {turn:>4}: #{} {name} at ({}, {}) abandoned",
                    id.0, position.x, position.y
                ),
                SimEvent::Eruption {
                    turn,
                    name,
                    position,
                    vei,
                    cause,
                    repose_years,
                    casualties,
                    ..
                } => {
                    let date =
                        societies::time::Date::from_turn(*turn, sim.config().climate.days_per_year);
                    let repose = repose_years.map_or("first recorded eruption".into(), |y| {
                        format!("quiet for {y} years")
                    });
                    println!(
                        "turn {turn:>4} ({date}): VEI {vei} eruption of {name} at ({}, {}); \
{cause}; {repose}; {casualties} dead",
                        position.x, position.y
                    );
                }
            }
        }
        if !sim.journal().entries().is_empty() {
            println!("\nclimate journal:");
            for entry in sim.journal().entries() {
                println!("  {entry}");
            }
        }
        println!("\nturn {} ({}):", sim.turn(), sim.date());
        println!("{}", render::ASCII_LEGEND);
        println!("{}\n", sim.render_map());
    }

    let cities = sim.cities();
    if !quiet {
        for c in &cities {
            let tile = sim.map().tile(c.position);
            println!(
                "#{:<3} {:<12} {:<9} ({:>3},{:>3})  pop {:>6}  {:>5.1}°C  {:?} {:?}{} {}m",
                c.id.0,
                c.name,
                match c.people {
                    People::Civilized => "city",
                    People::Barbarian => "barbarian",
                },
                c.position.x,
                c.position.y,
                c.population,
                f64::from(sim.climate().temperature(sim.map().index(c.position))) / 10.0,
                tile.landform,
                sim.biomes().biome(sim.map().index(c.position)),
                if tile.water == Water::River {
                    " + river"
                } else {
                    ""
                },
                tile.altitude,
            );
            let tree = sim.technologies();
            let variants: Vec<&str> = c
                .knowledge
                .variants()
                .map(|v| tree.variant(v).id.as_str())
                .collect();
            println!(
                "     knows {} of {} technologies; {}",
                c.knowledge.count(),
                tree.len(),
                if variants.is_empty() {
                    "no domesticates".to_string()
                } else {
                    variants.join(", ")
                }
            );
        }
    }
    let total: u64 = cities.iter().map(|c| c.population).sum();
    print_people_summary(&mut sim);
    let tribes = cities
        .iter()
        .filter(|c| c.people == People::Barbarian)
        .count();
    println!(
        "\n{} cities and {tribes} barbarian tribes, total population {total}",
        cities.len() - tribes
    );

    if let Some(path) = png {
        let volcanoes: Vec<_> = sim.volcanoes().into_iter().map(|(_, p)| p).collect();
        if let Err(e) = render::png(
            sim.map(),
            sim.climate(),
            sim.biomes(),
            &volcanoes,
            &cities,
            scale,
            &path,
        ) {
            eprintln!("failed to write {}: {e}", path.display());
            return ExitCode::FAILURE;
        }
        println!("map written to {}", path.display());
    }
    if let Some(path) = fertility_png {
        if let Err(e) = render::fertility_png(sim.map(), sim.fertility(), scale, &path) {
            eprintln!("failed to write {}: {e}", path.display());
            return ExitCode::FAILURE;
        }
        println!("fertility map written to {}", path.display());
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
    if let Some(path) = save_world {
        if let Err(e) = sim.save_world(&path) {
            eprintln!("failed to save {}: {e}", path.display());
            return ExitCode::FAILURE;
        }
        println!("world saved to {}", path.display());
    }
    println!("state digest {:016x}", sim.state_digest());
    ExitCode::SUCCESS
}

fn print_map_summary(sim: &Simulation) {
    let tiles = sim.map().tiles();
    let pct = |n: usize| n as f64 * 100.0 / tiles.len() as f64;
    let count = |l: Landform| tiles.iter().filter(|t| t.landform == l).count();
    let water = |w: Water| tiles.iter().filter(|t| t.water == w).count();
    let highest = tiles.iter().map(|t| t.altitude).max().unwrap_or(0);
    let deepest = tiles.iter().map(|t| t.altitude).min().unwrap_or(0);
    println!(
        "terrain: water {:.1}% (ocean {:.1}%, lakes {:.1}%), plains {:.1}%, \
hills {:.1}%, mountains {:.1}%; rivers {:.1}%; altitude {deepest}m..{highest}m",
        pct(count(Landform::Water)),
        pct(water(Water::Ocean)),
        pct(water(Water::Lake)),
        pct(count(Landform::Plains)),
        pct(count(Landform::Hills)),
        pct(count(Landform::Mountains)),
        pct(water(Water::River)),
    );
}

fn print_biome_summary(sim: &Simulation) {
    let map = sim.map();
    let land: Vec<usize> = (0..map.tile_count() as usize)
        .filter(|&i| matches!(map.tiles()[i].water, Water::Dry | Water::River))
        .collect();
    let area = |i: usize| map.area_weight(map.position_of(i as u32).y) as f64;
    let total: f64 = land.iter().map(|&i| area(i)).sum::<f64>().max(1.0);
    let mut shares: Vec<(Biome, f64)> = Biome::ALL
        .iter()
        .map(|&b| {
            let a: f64 = land
                .iter()
                .filter(|&&i| sim.biomes().biome(i) == b)
                .map(|&i| area(i))
                .sum();
            (b, a * 100.0 / total)
        })
        .filter(|&(_, share)| share > 0.0)
        .collect();
    shares.sort_by(|a, b| b.1.total_cmp(&a.1));
    let list: Vec<String> = shares
        .iter()
        .map(|(b, s)| format!("{b:?} {s:.1}%"))
        .collect();
    println!("biomes (share of land area): {}", list.join(", "));
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

fn print_frozen_summary(sim: &Simulation) {
    let map = sim.map();
    let cryo = sim.climate().cryosphere();
    let tiles = map.tiles();
    // Shares of true surface area, not of tile counts.
    let area = |i: usize| map.area_weight(map.position_of(i as u32).y);
    let share = |filter: &dyn Fn(usize) -> bool, of: &dyn Fn(usize) -> bool| {
        let total: i64 = (0..tiles.len()).filter(|&i| of(i)).map(area).sum();
        let part: i64 = (0..tiles.len())
            .filter(|&i| of(i) && filter(i))
            .map(area)
            .sum();
        part as f64 * 100.0 / total.max(1) as f64
    };
    let is = |w: Water| move |i: usize| tiles[i].water == w;
    let land = |i: usize| matches!(tiles[i].water, Water::Dry | Water::River);
    let cover = |c: Cover| move |i: usize| cryo.cover(map, i) == c;
    println!(
        "    frozen (by area): sea ice {:.1}% of ocean, lakes {:.0}%, rivers {:.0}%; land: glacier {:.1}%, \
snow {:.1}%, frozen ground {:.1}%, permafrost {:.1}%",
        share(&cover(Cover::SeaIce), &is(Water::Ocean)),
        share(&cover(Cover::LakeIce), &is(Water::Lake)),
        share(&cover(Cover::RiverIce), &is(Water::River)),
        share(&cover(Cover::Glacier), &land),
        share(&cover(Cover::Snow), &land),
        share(&cover(Cover::FrozenGround), &land),
        share(&|i| cryo.tile(i).permafrost, &land),
    );
}

fn print_fertility_summary(sim: &Simulation) {
    let map = sim.map();
    let fertility = sim.fertility();
    let land: Vec<u16> = (0..map.tile_count() as usize)
        .filter(|&i| matches!(map.tiles()[i].water, Water::Dry | Water::River))
        .map(|i| fertility.fertility(i))
        .collect();
    let n = land.len().max(1) as f64;
    let mean = land.iter().map(|&f| f64::from(f)).sum::<f64>() / n / 10.0;
    let pct =
        |lo: u16, hi: u16| land.iter().filter(|&&f| f >= lo && f < hi).count() as f64 * 100.0 / n;
    println!(
        "fertility (by area): land mean {mean:.0}%; barren (<10%) {:.1}%, poor {:.1}%, fair {:.1}%, rich (60%+) {:.1}%",
        pct(0, 100),
        pct(100, 300),
        pct(300, 600),
        pct(600, 1001),
    );
}

fn print_volcano_summary(sim: &mut Simulation) {
    let volcanoes = sim.volcanoes();
    let count =
        |k: societies::volcano::VolcanoKind| volcanoes.iter().filter(|(v, _)| v.kind == k).count();
    println!(
        "volcanoes: {} ({} subduction arc, {} hotspot)",
        volcanoes.len(),
        count(societies::volcano::VolcanoKind::Arc),
        count(societies::volcano::VolcanoKind::Hotspot),
    );
}

fn print_people_summary(sim: &mut Simulation) {
    use societies::people::Sex;
    let people = sim.people();
    if people.is_empty() {
        return;
    }
    let tpy = sim.config().turns_per_year();
    let turn = sim.turn();
    let ages: Vec<u32> = people
        .iter()
        .map(|p| p.person.age_years(turn, tpy))
        .collect();
    let band = |lo: u32, hi: u32| ages.iter().filter(|&&a| a >= lo && a < hi).count();
    let women = people
        .iter()
        .filter(|p| p.person.sex == Sex::Female)
        .count();
    let couples = people
        .iter()
        .filter(|p| p.person.sex == Sex::Female && p.family.partner.is_some())
        .count();
    let adults: Vec<_> = people
        .iter()
        .zip(&ages)
        .filter(|&(_, &a)| a >= 15)
        .map(|(p, _)| p.skills.count())
        .collect();
    println!(
        "\npeople: {} ({} women, {} men); children under 15 {}, adults 15-49 {}, 50+ {}; {} couples; adults know {:.1} technologies on average",
        people.len(),
        women,
        people.len() - women,
        band(0, 15),
        band(15, 50),
        band(50, 200),
        couples,
        adults.iter().sum::<usize>() as f64 / adults.len().max(1) as f64
    );
}
