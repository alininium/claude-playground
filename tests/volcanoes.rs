mod common;

use societies::config::SimConfig;
use societies::events::SimEvent;
use societies::map::Water;
use societies::volcano::{VolcanoConfig, VolcanoKind};

fn config(seed: u64, volcanoes: VolcanoConfig) -> SimConfig {
    SimConfig {
        seed,
        map_width: 128,
        map_height: 64,
        volcanoes,
        ..SimConfig::default()
    }
}

#[test]
fn volcanoes_sit_where_their_cause_says() {
    let mut sim = common::generated(config(3, VolcanoConfig::default()));
    let volcanoes = sim.volcanoes();
    assert!(volcanoes.iter().any(|(v, _)| v.kind == VolcanoKind::Arc));
    assert!(
        volcanoes
            .iter()
            .any(|(v, _)| v.kind == VolcanoKind::Hotspot)
    );
    let map = sim.map();
    for (v, p) in &volcanoes {
        if v.kind == VolcanoKind::Arc {
            let tile = map.tile(*p);
            assert!(tile.altitude >= 1200, "{} is not on high ground", v.name);
            assert!(matches!(tile.water, Water::Dry | Water::River));
            // Within 4 tiles of the sea.
            let near_sea = (0..map.tile_count() as usize).any(|i| {
                let q = map.position_of(i as u32);
                map.tiles()[i].water == Water::Ocean
                    && q.x.abs_diff(p.x).max(q.y.abs_diff(p.y)) <= 4
            });
            assert!(near_sea, "{} is not near a coast", v.name);
        }
    }
    for (i, (a, pa)) in volcanoes.iter().enumerate() {
        for (b, pb) in &volcanoes[i + 1..] {
            let d = pa.x.abs_diff(pb.x).max(pa.y.abs_diff(pb.y));
            assert!(d >= 3, "{} and {} are only {d} tiles apart", a.name, b.name);
        }
    }
}

/// `factor` times Earth's eruption rates (magma recharge scales with them),
/// so a few years show centuries' worth of eruptions.
fn busy(factor: u32) -> VolcanoConfig {
    VolcanoConfig {
        eruptions_per_millennium: [2_000, 150, 20, 8, 1].map(|r| r * factor),
        ..VolcanoConfig::default()
    }
}

#[test]
fn eruptions_follow_the_configured_rates_and_are_logged() {
    // Forty times Earth's rates for five years: two centuries of eruptions.
    let mut sim = common::generated(config(5, busy(40)));
    sim.run(5 * sim.config().turns_per_year());
    let history = sim.volcanism().history().to_vec();
    let count = |vei: u8| history.iter().filter(|e| e.vei == vei).count() as u64;
    // Expected: 400 VEI 3, 30 VEI 4.
    assert!((300..=500).contains(&count(3)), "{} VEI 3", count(3));
    assert!((10..=50).contains(&count(4)), "{} VEI 4", count(4));
    assert!(
        history
            .iter()
            .all(|e| e.kind == VolcanoKind::Arc || e.vei <= 4)
    );

    let logged: Vec<_> = sim
        .events()
        .events()
        .iter()
        .filter_map(|e| match e {
            SimEvent::Eruption {
                turn, volcano, vei, ..
            } => Some((*turn, *volcano, *vei)),
            _ => None,
        })
        .collect();
    let recorded: Vec<_> = history.iter().map(|e| (e.turn, e.volcano, e.vei)).collect();
    assert_eq!(
        logged, recorded,
        "every eruption is in the event log, in order"
    );
}

#[test]
fn magma_is_conserved() {
    let mut sim = common::generated(config(7, busy(20)));
    let start: Vec<u64> = sim.volcanoes().iter().map(|(v, _)| v.magma).collect();
    let days = 5 * u64::from(sim.config().climate.days_per_year);
    sim.run(days * 4);
    let history = sim.volcanism().history().to_vec();
    for (k, (v, _)) in sim.volcanoes().iter().enumerate() {
        let erupted: u64 = history
            .iter()
            .filter(|e| e.volcano == v.id)
            .map(|e| e.volume)
            .sum();
        assert_eq!(
            start[k] + v.recharge * days,
            v.magma + erupted,
            "{}: magma appeared or vanished",
            v.name
        );
    }
}

#[test]
fn a_large_eruption_cools_the_planet_and_buries_land_in_ash() {
    // Same world twice: one with a VEI 6 forced on day one, one with none.
    let eruptive = VolcanoConfig {
        eruptions_per_millennium: [0, 0, 0, 1_000 * 364, 0],
        ..VolcanoConfig::default()
    };
    let quiet = VolcanoConfig {
        eruptions_per_millennium: [0; 5],
        ..VolcanoConfig::default()
    };
    let mut hot = common::generated(config(3, eruptive));
    let mut calm = common::generated(config(3, quiet));
    hot.run(4); // first day
    let eruption = hot
        .volcanism()
        .history()
        .first()
        .cloned()
        .expect("a VEI 6 erupted");
    assert_eq!(eruption.vei, 6);
    let index = hot.map().index(eruption.position);
    assert!(hot.volcanism().ash(index) >= 500, "no ash at the volcano");
    let days = 180;
    hot.run(days * 4 - 4);
    calm.run(days * 4);
    let cooling = calm.climate().global_mean() - hot.climate().global_mean();
    assert!(cooling >= 3, "only {cooling} tenths °C of cooling");
}
