use societies::map::{Landform, Map, Soil, TerrainConfig, Water};
use societies::rng::SimRng;

fn generate(seed: u64, width: u32, height: u32) -> Map {
    Map::generate(
        width,
        height,
        &TerrainConfig::default(),
        &mut SimRng::from_seed(seed),
    )
}

#[test]
fn same_seed_same_map() {
    let a = generate(7, 120, 60);
    let b = generate(7, 120, 60);
    assert_eq!(a.tiles(), b.tiles());
    assert_ne!(a.tiles(), generate(8, 120, 60).tiles());
}

#[test]
fn land_share_matches_config() {
    let map = generate(3, 160, 80);
    let config = TerrainConfig::default();
    let land = map.tiles().iter().filter(|t| t.altitude >= 0).count();
    let per_mille = (land * 1000 / map.tiles().len()) as i64;
    assert!(
        (per_mille - i64::from(config.land_per_mille)).abs() <= 50,
        "land share {per_mille}‰"
    );
    let highest = map.tiles().iter().map(|t| t.altitude).max().unwrap();
    let deepest = map.tiles().iter().map(|t| t.altitude).min().unwrap();
    assert!(
        highest > config.mountain_altitude,
        "no mountains, max {highest}m"
    );
    assert!(deepest < -3000, "no deep ocean, min {deepest}m");
}

#[test]
fn all_landforms_and_soils_appear() {
    let map = generate(1, 256, 128);
    for landform in [
        Landform::Water,
        Landform::Plains,
        Landform::Hills,
        Landform::Mountains,
    ] {
        assert!(
            map.tiles().iter().any(|t| t.landform == landform),
            "missing {landform:?}"
        );
    }
    for soil in [
        Soil::None,
        Soil::Loam,
        Soil::Sandy,
        Soil::Rocky,
        Soil::Alluvial,
    ] {
        assert!(
            map.tiles().iter().any(|t| t.soil == soil),
            "missing {soil:?}"
        );
    }
    assert!(
        map.tiles().iter().any(|t| t.water == Water::Lake),
        "no lakes"
    );
    assert!(
        map.tiles().iter().any(|t| t.water == Water::River),
        "no rivers"
    );
    for t in map.tiles().iter().filter(|t| t.water == Water::River) {
        assert_eq!(t.soil, Soil::Alluvial, "rivers deposit alluvium");
    }
}

#[test]
fn water_landform_iff_ocean_or_lake() {
    let map = generate(5, 128, 64);
    for t in map.tiles() {
        let standing = matches!(t.water, Water::Ocean | Water::Lake);
        assert_eq!(t.landform == Landform::Water, standing, "{t:?}");
        assert_eq!(t.soil == Soil::None, standing, "{t:?}");
        if t.water == Water::Ocean {
            assert!(t.altitude < 0);
        }
    }
}

#[test]
fn every_land_tile_drains_to_the_sea() {
    let map = generate(2, 128, 64);
    let n = map.tiles().len();
    for start in 0..n {
        let mut i = start;
        let mut steps = 0;
        while let Some(down) = map.tiles()[i].downstream {
            i = down as usize;
            steps += 1;
            assert!(steps <= n, "drainage cycle from tile {start}");
        }
        assert!(
            map.tiles()[i].water == Water::Ocean || map.is_edge(i),
            "tile {start} drains into a dead end at {i}"
        );
    }
}

#[test]
fn rivers_flow_into_rivers_lakes_or_ocean() {
    let map = generate(4, 160, 80);
    for t in map.tiles().iter().filter(|t| t.water == Water::River) {
        if let Some(down) = t.downstream {
            let next = &map.tiles()[down as usize];
            assert!(next.water != Water::Dry, "river ends on dry land");
            assert!(next.flow >= t.flow, "flow decreased downstream");
        }
    }
}

#[test]
fn odd_sizes_generate() {
    for (w, h) in [(2, 2), (3, 200), (300, 3), (17, 9)] {
        let map = generate(9, w, h);
        assert_eq!(map.tiles().len(), (w * h) as usize);
    }
}
