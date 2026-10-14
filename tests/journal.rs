//! The climate journal reports what matters, at the right severity, and
//! stays short.

mod common;

use common::{Fixture, sim_with};
use societies::climate::oscillation::OscillationConfig;
use societies::journal::{Happening, MIN_RECORD_YEARS, Place, Severity};
use societies::time::Date;
use societies::volcano::VolcanoConfig;

/// Weather tests over decades run without people, who are slow to
/// simulate and add nothing to the climate.
fn no_people(c: &mut societies::config::SimConfig) {
    c.city_population = [0, 0];
    c.barbarian_population = [0, 0];
}

#[test]
fn the_journal_is_short_and_written_at_season_boundaries() {
    let mut sim = sim_with(Fixture::Small, no_people);
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
    // A wild oscillation (3 °C typical) reaches the top levels quickly. It
    // is planet-wide, so the small world will do.
    let mut sim = sim_with(Fixture::Small, |c| {
        no_people(c);
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

// ---------------------------------------------------------------------------
// Societies: famines and technologies
// ---------------------------------------------------------------------------

/// Runs to the first morning of the next season (when the journal writes).
fn to_next_season(sim: &mut societies::Simulation) {
    use societies::time::TimeOfDay;
    loop {
        sim.step();
        let d = sim.date();
        if d.day_of_season == 0 && d.time_of_day == TimeOfDay::Morning {
            return;
        }
    }
}

/// Sets what the cities picked by `which` know to `f` of what they know now.
fn change_knowledge(
    sim: &mut societies::Simulation,
    which: impl Fn(u32) -> bool,
    f: impl Fn(&societies::tech::TechTree, &societies::tech::Knowledge) -> societies::tech::Knowledge,
) {
    use societies::city::CityId;
    use societies::tech::{Knowledge, TechTree};
    let world = sim.world_mut();
    let tree = world.resource::<TechTree>().clone();
    let mut query = world.query::<(&CityId, &mut Knowledge)>();
    for (id, mut knowledge) in query.iter_mut(world) {
        if which(id.0) {
            *knowledge = f(&tree, &knowledge);
        }
    }
}

/// `knowledge` without the technologies named in `drop`.
fn without(
    tree: &societies::tech::TechTree,
    knowledge: &societies::tech::Knowledge,
    drop: &[&str],
) -> societies::tech::Knowledge {
    let mut out = societies::tech::Knowledge::new(tree);
    for t in knowledge.techs() {
        if !drop.contains(&tree.tech(t).id.as_str()) {
            out.learn(tree, t, None);
        }
    }
    out
}

fn society_entries(sim: &societies::Simulation) -> Vec<societies::journal::JournalEntry> {
    sim.journal()
        .entries()
        .iter()
        .filter(|e| {
            matches!(
                e.happening,
                Happening::Famine { .. }
                    | Happening::TechnologyGained { .. }
                    | Happening::TechnologyLost { .. }
            )
        })
        .cloned()
        .collect()
}

#[test]
fn one_city_alone_is_at_most_moderate() {
    let mut sim = common::sim(Fixture::Small);
    to_next_season(&mut sim); // the journal notes what everyone knows
    assert!(society_entries(&sim).is_empty());
    // City 0 forgets fire (everything builds on it) and cordage, and comes
    // to know writing, which nobody knew.
    change_knowledge(
        &mut sim,
        |c| c == 0,
        |tree, k| {
            let mut k = without(tree, k, &["fire", "cordage"]);
            k.learn(tree, tree.id("writing").unwrap(), None);
            k
        },
    );
    to_next_season(&mut sim);
    let entries = society_entries(&sim);
    let severity = |what: &str| {
        entries
            .iter()
            .find(|e| e.to_string().contains(what))
            .unwrap_or_else(|| panic!("no entry for {what}: {entries:?}"))
            .severity
    };
    assert_eq!(severity("forgets Fire"), Severity::Moderate);
    assert_eq!(severity("forgets Cordage"), Severity::Minor);
    assert_eq!(severity("invents Writing"), Severity::Moderate);
    for e in &entries {
        assert!(e.severity <= Severity::Moderate, "{e}");
        assert!(matches!(e.place, Place::Site(_)));
        assert_eq!(e.turn, sim.turn());
    }
}

#[test]
fn the_same_loss_in_many_cities_is_one_big_entry() {
    let mut sim = common::sim(Fixture::Small);
    to_next_season(&mut sim);
    let cities = sim.cities().len() as u32;
    assert!(cities >= 4, "only {cities} cities");
    // Two cities forget pottery; every city forgets fire.
    change_knowledge(
        &mut sim,
        |c| c < 2,
        |tree, k| without(tree, k, &["pottery"]),
    );
    change_knowledge(&mut sim, |_| true, |tree, k| without(tree, k, &["fire"]));
    to_next_season(&mut sim);
    let entries = society_entries(&sim);
    let fire: Vec<_> = entries
        .iter()
        .filter(|e| e.to_string().contains("Fire"))
        .collect();
    assert_eq!(fire.len(), 1, "{entries:?}");
    assert_eq!(fire[0].severity, Severity::Cataclysmic);
    assert!(
        matches!(fire[0].happening, Happening::TechnologyLost { cities: c, .. } if c == cities)
    );
    assert!(
        fire[0].to_string().contains("other cities forget Fire"),
        "{}",
        fire[0]
    );
    let pottery: Vec<_> = entries
        .iter()
        .filter(|e| e.to_string().contains("Pottery"))
        .collect();
    if let [p] = pottery.as_slice() {
        // (only if both cities knew it)
        assert_eq!(p.severity, Severity::Major, "{p}");
    }
    assert_eq!(societies::journal::spread(2, 16), Severity::Major);
    assert_eq!(societies::journal::spread(4, 16), Severity::Severe);
    assert_eq!(societies::journal::spread(8, 16), Severity::Cataclysmic);
    assert_eq!(societies::journal::spread(3, 6), Severity::Severe);
    assert_eq!(societies::journal::spread(1, 16), Severity::Moderate);
}

#[test]
fn many_changes_in_a_season_become_one_entry() {
    let mut sim = common::sim(Fixture::Small);
    to_next_season(&mut sim);
    // Six technologies city 0 knows, beyond the baseline.
    let drop: Vec<String> = {
        let world = sim.world_mut();
        let tree = world.resource::<societies::tech::TechTree>().clone();
        let mut q = world.query::<(&societies::city::CityId, &societies::tech::Knowledge)>();
        let (_, k) = q.iter(world).find(|(id, _)| id.0 == 0).unwrap();
        k.techs()
            .filter(|&t| tree.tech(t).category != societies::tech::Category::Baseline)
            .map(|t| tree.tech(t).id.clone())
            .take(6)
            .collect()
    };
    assert_eq!(drop.len(), 6, "{drop:?}");
    let names: Vec<&str> = drop.iter().map(String::as_str).collect();
    change_knowledge(&mut sim, |c| c == 0, |tree, k| without(tree, k, &names));
    to_next_season(&mut sim);
    let lost: Vec<_> = society_entries(&sim)
        .into_iter()
        .filter(|e| matches!(e.happening, Happening::TechnologyLost { .. }))
        .collect();
    assert_eq!(lost.len(), 1, "{lost:?}");
    assert!(matches!(
        lost[0].happening,
        Happening::TechnologyLost { more: 5, .. }
    ));
    assert!(
        lost[0].to_string().contains("5 more technologies"),
        "{}",
        lost[0]
    );
}

#[test]
fn hunger_is_a_famine_graded_by_the_missing_rations() {
    let mut sim = common::sim(Fixture::Small);
    {
        // Nothing to eat anywhere: no stores, no yields.
        let world = sim.world_mut();
        let mut res = world.resource_mut::<societies::economy::EconomyConfig>();
        let config = &mut *res;
        config.initial_food_days = 0;
        for work in [&mut config.farm, &mut config.herd, &mut config.forage] {
            work.daily_yield = 0;
        }
        config.fish.daily_yield = 0;
    }
    to_next_season(&mut sim);
    let famines: Vec<_> = society_entries(&sim)
        .into_iter()
        .filter(|e| matches!(e.happening, Happening::Famine { .. }))
        .collect();
    // Starving everywhere at once: one entry for all the cities.
    let cities = sim.cities().len() as u32;
    assert_eq!(famines.len(), 1, "{famines:?}");
    let f = &famines[0];
    assert!(matches!(f.happening, Happening::Famine { cities: c, .. } if c == cities));
    assert_eq!(f.severity, Severity::Cataclysmic, "{f}");
    assert!(f.to_string().contains("other cities, up to"), "{f}");
}

#[test]
fn fed_cities_have_no_famine() {
    let mut sim = common::sim(Fixture::Small);
    to_next_season(&mut sim);
    to_next_season(&mut sim);
    assert!(
        !society_entries(&sim)
            .iter()
            .any(|e| matches!(e.happening, Happening::Famine { .. })),
        "{:?}",
        society_entries(&sim)
    );
}

#[test]
fn technology_weights_follow_the_tree() {
    let tree = societies::tech::TechTree::builtin();
    let w = societies::journal::technology_weights(&tree);
    let of = |name: &str| w[usize::from(tree.id(name).unwrap().0)];
    assert!(of("fire") > 30, "fire {}", of("fire"));
    assert!(of("writing") > of("law_codes"));
    assert_eq!(of("law_codes"), 0);
}
