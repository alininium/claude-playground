//! The climate journal reports what matters, at the right severity, and
//! stays short.

mod common;

use common::{Fixture, sim_with};
use societies::climate::oscillation::OscillationConfig;
use societies::journal::{Happening, MIN_RECORD_YEARS, Place, Severity};
use societies::time::Date;
use societies::volcano::VolcanoConfig;

#[test]
fn the_journal_is_short_and_written_at_season_boundaries() {
    let mut sim = common::sim(Fixture::Medium);
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
        // Never more than four separate regions of one kind per season:
        // widespread anomalies are merged into one entry.
        if matches!(e.place, Place::Region { .. }) {
            let same = entries
                .iter()
                .filter(|o| {
                    o.turn == e.turn
                        && matches!(o.place, Place::Region { .. })
                        && std::mem::discriminant(&o.happening)
                            == std::mem::discriminant(&e.happening)
                })
                .count();
            assert!(same <= 4, "{same} separate entries like {e}");
        }
        // Regional weather waits for a record of climate normals.
        if matches!(e.place, Place::Region { .. } | Place::Widespread { .. }) {
            assert!(e.date.year > MIN_RECORD_YEARS, "{e}");
        }
    }
}

#[test]
fn a_big_eruption_is_a_severe_entry_followed_by_a_volcanic_winter() {
    let mut sim = sim_with(Fixture::Medium, |c| {
        c.volcanoes = VolcanoConfig {
            eruptions_per_millennium: [0, 0, 0, 1_000 * 364, 0],
            ..VolcanoConfig::default()
        }
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
    let mut sim = sim_with(Fixture::Medium, |c| {
        c.climate.oscillation = OscillationConfig {
            amplitude_tenths: 30,
            ..OscillationConfig::default()
        }
    });
    // The journal judges the index at each season boundary; follow along
    // and split the run into episodes the same way (a new one whenever the
    // index changes sign).
    let season = u64::from(sim.config().climate.days_per_year) / 4 * 4;
    let mut episode_of_turn = std::collections::HashMap::new();
    let (mut episode, mut sign) = (0, 0);
    for _ in 0..20 * 4 {
        sim.run(season);
        let s = sim.climate().oscillation().index_tenths().signum();
        if s != sign {
            episode += 1;
            sign = s;
        }
        episode_of_turn.insert(sim.turn(), episode);
    }
    let phases: Vec<(i64, Severity, u32)> = sim
        .journal()
        .entries()
        .iter()
        .filter_map(|e| match e.happening {
            Happening::WarmPhase { index } | Happening::ColdPhase { index } => {
                Some((index.signum(), e.severity, episode_of_turn[&e.turn]))
            }
            _ => None,
        })
        .collect();
    assert!(
        phases.iter().any(|&(_, s, _)| s >= Severity::Severe),
        "no severe phase in {phases:?}"
    );
    // Within an episode, each entry is a new, higher level.
    for pair in phases.windows(2) {
        if pair[0].2 == pair[1].2 {
            assert!(pair[1].1 > pair[0].1, "{phases:?}");
        }
    }
    assert!(phases.iter().any(|p| p.0 > 0) && phases.iter().any(|p| p.0 < 0));
}

#[test]
fn entries_read_well() {
    let mut sim = sim_with(Fixture::Medium, |c| {
        c.volcanoes = VolcanoConfig {
            eruptions_per_millennium: [0, 364_000, 0, 0, 0],
            ..VolcanoConfig::default()
        }
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
