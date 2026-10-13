//! The climate journal reports what matters, at the right severity, and
//! stays short.

use societies::Simulation;
use societies::climate::oscillation::OscillationConfig;
use societies::config::SimConfig;
use societies::journal::{Happening, MIN_RECORD_YEARS, Place, Severity};
use societies::time::Date;
use societies::volcano::VolcanoConfig;

fn config(seed: u64) -> SimConfig {
    SimConfig {
        seed,
        map_width: 128,
        map_height: 64,
        ..SimConfig::default()
    }
}

#[test]
fn the_journal_is_short_and_written_at_season_boundaries() {
    let mut sim = Simulation::new(config(3));
    let years = 30;
    sim.run(years * sim.config().turns_per_year());
    let days = sim.config().climate.days_per_year;
    let entries = sim.journal().entries();
    assert!(
        entries.len() as u64 <= 4 * years,
        "{} entries in {years} years",
        entries.len()
    );
    for e in entries {
        match e.happening {
            // Eruptions keep the turn they happened on.
            Happening::Eruption { .. } => {}
            _ => {
                let written = Date::from_turn(e.turn, days);
                assert_eq!(written.day_of_season, 0, "{e}");
            }
        }
        // Regional weather waits for a record of climate normals.
        if matches!(e.place, Place::Region { .. }) {
            assert!(e.date.year > MIN_RECORD_YEARS, "{e}");
        }
    }
}

#[test]
fn a_big_eruption_is_a_severe_entry_followed_by_a_volcanic_winter() {
    let mut sim = Simulation::new(SimConfig {
        map_width: 192,
        map_height: 96,
        volcanoes: VolcanoConfig {
            eruptions_per_millennium: [0, 0, 0, 1_000 * 364, 0],
            ..VolcanoConfig::default()
        },
        ..config(3)
    });
    sim.run(sim.config().turns_per_year());
    let entries = sim.journal().entries();
    let eruption = entries
        .iter()
        .find(|e| matches!(e.happening, Happening::Eruption { vei: 6, .. }))
        .expect("the VEI 6 eruption is in the journal");
    assert!(eruption.severity >= Severity::Severe, "{eruption}");
    assert!(
        entries
            .iter()
            .any(|e| matches!(e.happening, Happening::VolcanicWinter { .. })
                && e.severity >= Severity::Minor),
        "no volcanic winter entry"
    );
    // Small eruptions without casualties stay out of the journal.
    let logged_eruptions = entries
        .iter()
        .filter(|e| matches!(e.happening, Happening::Eruption { .. }))
        .count();
    let impactful = sim
        .volcanism()
        .history()
        .iter()
        .filter(|e| e.vei >= 4 || e.casualties > 0)
        .count();
    assert_eq!(logged_eruptions, impactful);
}

#[test]
fn oscillation_episodes_escalate_without_repeating_levels() {
    // A wild oscillation (3 °C typical) reaches the top levels quickly.
    let mut sim = Simulation::new(SimConfig {
        climate: societies::climate::ClimateConfig {
            oscillation: OscillationConfig {
                amplitude_tenths: 30,
                ..OscillationConfig::default()
            },
            ..Default::default()
        },
        ..config(4)
    });
    sim.run(20 * sim.config().turns_per_year());
    let phases: Vec<(i64, Severity)> = sim
        .journal()
        .entries()
        .iter()
        .filter_map(|e| match e.happening {
            Happening::WarmPhase { index } | Happening::ColdPhase { index } => {
                Some((index.signum(), e.severity))
            }
            _ => None,
        })
        .collect();
    assert!(
        phases.iter().any(|&(_, s)| s >= Severity::Severe),
        "no severe phase in {phases:?}"
    );
    // Within an episode (same sign, consecutive entries) levels only rise.
    for pair in phases.windows(2) {
        if pair[0].0 == pair[1].0 {
            assert!(
                pair[1].1 > pair[0].1 || pair[1].1 == Severity::Minor,
                "{phases:?}"
            );
        }
    }
    assert!(phases.iter().any(|p| p.0 > 0) && phases.iter().any(|p| p.0 < 0));
}

#[test]
fn entries_read_well() {
    let mut sim = Simulation::new(SimConfig {
        volcanoes: VolcanoConfig {
            eruptions_per_millennium: [0, 364_000, 0, 0, 0],
            ..VolcanoConfig::default()
        },
        ..config(5)
    });
    // Eruptions are journaled at the next season boundary.
    sim.run(100);
    assert!(sim.journal().entries().is_empty());
    let season = u64::from(sim.config().climate.days_per_year) / 4 * 4;
    sim.run(season + 1 - 100);
    let entry = sim
        .journal()
        .entries()
        .iter()
        .find(|e| matches!(e.happening, Happening::Eruption { .. }))
        .expect("an eruption entry");
    let text = entry.to_string();
    assert!(
        text.starts_with("year 1 Spring: Moderate: VEI 4 eruption of "),
        "{text}"
    );
}
