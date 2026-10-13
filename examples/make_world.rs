//! Generates a world and saves it: `make_world <seed> <width> <height> <file>`.
use societies::Simulation;
use societies::config::SimConfig;

fn main() {
    let args: Vec<String> = std::env::args().skip(1).collect();
    let [seed, width, height, path] = args.as_slice() else {
        eprintln!("usage: make_world <seed> <width> <height> <file>");
        std::process::exit(2);
    };
    let config = SimConfig {
        seed: seed.parse().expect("seed"),
        map_width: width.parse().expect("width"),
        map_height: height.parse().expect("height"),
        ..SimConfig::default()
    };
    Simulation::generate_world(&config)
        .write(std::path::Path::new(path))
        .expect("write world");
}
