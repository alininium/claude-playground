//! Premade worlds for tests that are not about world generation, so they
//! skip the generation (and its climate spin-up) and start right away.
//!
//! The worlds live in `tests/fixtures/`. A missing or unreadable fixture
//! (for example after the save format changed) is regenerated
//! automatically. After changing how worlds are generated, delete the
//! fixtures so they are rebuilt.

#![allow(dead_code)]

use std::path::PathBuf;

use societies::Simulation;
use societies::config::SimConfig;
use societies::save::WorldSave;

#[derive(Debug, Clone, Copy)]
pub enum Fixture {
    /// 48×24, seed 7: for long runs.
    Small,
    /// 128×64, seed 3: for statistics over many tiles.
    Medium,
}

impl Fixture {
    fn spec(self) -> (&'static str, u64, u32, u32) {
        match self {
            Fixture::Small => ("small", 7, 48, 24),
            Fixture::Medium => ("medium", 3, 128, 64),
        }
    }

    /// The generation config of this fixture.
    pub fn config(self) -> SimConfig {
        let (_, seed, width, height) = self.spec();
        SimConfig {
            seed,
            map_width: width,
            map_height: height,
            ..SimConfig::default()
        }
    }
}

/// Loads (or, if needed, generates and stores) a fixture world.
pub fn world(fixture: Fixture) -> WorldSave {
    let (name, ..) = fixture.spec();
    let path = PathBuf::from(env!("CARGO_MANIFEST_DIR"))
        .join("tests/fixtures")
        .join(format!("{name}.world"));
    let expected = fixture.config();
    if let Ok(save) = WorldSave::read(&path)
        && save.config == expected
    {
        return save;
    }
    let save = Simulation::generate_world(&expected);
    // Write atomically: other test binaries may be reading it right now.
    let temp = path.with_extension(format!("tmp{}", std::process::id()));
    if save.write(&temp).is_ok() {
        std::fs::rename(&temp, &path).ok();
    }
    save
}

/// A simulation on a fixture world with the default settings.
pub fn sim(fixture: Fixture) -> Simulation {
    sim_with(fixture, |_| {})
}

/// A simulation on a fixture world, with runtime settings (cities, eruption
/// rates, the oscillation) adjusted by `adjust`.
pub fn sim_with(fixture: Fixture, adjust: impl FnOnce(&mut SimConfig)) -> Simulation {
    let mut config = fixture.config();
    adjust(&mut config);
    Simulation::from_world(world(fixture), config)
}
