use societies::Simulation;
use societies::config::SimConfig;
use societies::save::WorldSave;

fn config() -> SimConfig {
    SimConfig {
        seed: 9,
        map_width: 64,
        map_height: 32,
        ..SimConfig::default()
    }
}

fn temp_file(name: &str) -> std::path::PathBuf {
    std::env::temp_dir().join(format!("societies-{}-{name}.world", std::process::id()))
}

#[test]
fn a_loaded_world_is_identical_to_a_fresh_one() {
    let path = temp_file("fresh");
    let mut fresh = Simulation::new(config());
    fresh.save_world(&path).unwrap();
    let mut loaded = Simulation::from_world(Simulation::load_world(&path).unwrap(), config());
    std::fs::remove_file(&path).ok();

    assert_eq!(
        fresh.state_digest(),
        loaded.state_digest(),
        "cities included"
    );
    let year = fresh.config().turns_per_year();
    fresh.run(year);
    loaded.run(year);
    assert_eq!(fresh.state_digest(), loaded.state_digest(), "after a year");
}

#[test]
fn a_world_saved_mid_run_continues_exactly() {
    let mut original = Simulation::new(config());
    original.run(1000);
    let bytes = original.world_save().to_bytes().unwrap();
    let mut resumed = Simulation::from_world(WorldSave::from_bytes(&bytes).unwrap(), config());
    assert_eq!(resumed.turn(), 1000);
    assert_eq!(original.world_digest(), resumed.world_digest());
    original.run(2000);
    resumed.run(2000);
    // Cities are refounded on load, but the world itself carries on exactly.
    assert_eq!(original.world_digest(), resumed.world_digest());
    assert_eq!(original.journal().entries(), resumed.journal().entries());
}

#[test]
fn world_shaping_settings_come_from_the_save() {
    let save = Simulation::generate_world(&config());
    let other = SimConfig {
        seed: 12345,
        map_width: 300,
        initial_cities: 3,
        ..SimConfig::default()
    };
    let mut sim = Simulation::from_world(save, other);
    assert_eq!(sim.config().seed, 9);
    assert_eq!(sim.map().width(), 64);
    assert_eq!(
        sim.config().initial_cities,
        3,
        "society settings come from the caller"
    );
    assert!(sim.cities().len() <= 3);
}

#[test]
fn bad_files_are_errors_not_panics() {
    assert!(WorldSave::from_bytes(b"").is_err());
    assert!(WorldSave::from_bytes(b"not a world file at all").is_err());
    let mut bytes = Simulation::generate_world(&config()).to_bytes().unwrap();
    bytes[8] = bytes[8].wrapping_add(1); // another format version
    assert!(WorldSave::from_bytes(&bytes).is_err());
    bytes[8] = bytes[8].wrapping_sub(1);
    let len = bytes.len();
    bytes.truncate(len / 2); // cut off
    assert!(WorldSave::from_bytes(&bytes).is_err());
}

#[test]
fn json_worlds_round_trip_too() {
    let save = Simulation::generate_world(&config());
    let json = save.to_json().unwrap();
    assert!(json.starts_with(b"{\"format\":\"societies-world\""));
    // Loading detects the format from the content.
    let loaded = WorldSave::from_bytes(&json).unwrap();
    let mut a = Simulation::from_world(save, config());
    let mut b = Simulation::from_world(loaded, config());
    assert_eq!(a.state_digest(), b.state_digest());

    let path = temp_file("json").with_extension("json");
    a.save_world(&path).unwrap();
    let text = std::fs::read_to_string(&path).unwrap();
    std::fs::remove_file(&path).ok();
    assert!(text.contains("\"climate\""), "written as JSON");
    assert!(WorldSave::from_bytes(br#"{"format":"other","version":1,"world":{}}"#).is_err());
}
