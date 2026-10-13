//! Stage 2: oceans, rainfall, lakes and rivers.
//!
//! 1. Large connected areas below sea level are ocean.
//! 2. Rainfall follows Earth-like latitude bands (wet tropics, dry
//!    subtropics, wet temperate belt, dry poles), drops off with distance from
//!    the ocean, and gets some noise.
//! 3. A priority flood from the oceans (Barnes et al. 2014) finds where water
//!    would pool. Large depressions become lakes, small ones are filled in.
//!    Every land tile then drains to its steepest-descent neighbor; on flat
//!    ground (lake surfaces, filled pits) it drains toward the neighbor the
//!    flood reached it from, so water always finds its way to the sea.
//! 4. Rain is accumulated down that tree; tiles that collect enough become
//!    rivers.

use std::cmp::Reverse;
use std::collections::{BinaryHeap, VecDeque};

use super::noise::{Noise, ONE};
use super::{Map, TerrainConfig, Water};
use crate::rng::SimRng;

pub fn generate(map: &mut Map, config: &TerrainConfig, rng: &mut SimRng) {
    let rain_noise = Noise::new(rng.next_u64());
    mark_oceans(map);
    assign_moisture(map, &rain_noise);
    let order = flood_and_drain(map, config);
    accumulate_rivers(map, config, &order);
}

fn mark_oceans(map: &mut Map) {
    let n = map.tile_count() as usize;
    let min_ocean = (n / 256).max(16);
    let mut seen = vec![false; n];
    for start in 0..n {
        if seen[start] || map.tiles[start].altitude >= 0 {
            continue;
        }
        // Collect this below-sea-level basin.
        let mut component = vec![start];
        seen[start] = true;
        let mut i = 0;
        while i < component.len() {
            for nb in map.neighbors(component[i]) {
                if !seen[nb] && map.tiles[nb].altitude < 0 {
                    seen[nb] = true;
                    component.push(nb);
                }
            }
            i += 1;
        }
        let touches_edge = component.iter().any(|&t| map.is_edge(t));
        if touches_edge || component.len() >= min_ocean {
            for t in component {
                map.tiles[t].water = Water::Ocean;
            }
        }
    }
}

fn assign_moisture(map: &mut Map, noise: &Noise) {
    let (w, h) = (map.width as usize, map.height as usize);
    let n = w * h;

    // Distance (in tiles) to the nearest ocean.
    let mut dist = vec![u32::MAX; n];
    let mut queue = VecDeque::new();
    for (i, tile) in map.tiles.iter().enumerate() {
        if tile.water == Water::Ocean {
            dist[i] = 0;
            queue.push_back(i);
        }
    }
    while let Some(i) = queue.pop_front() {
        for nb in map.neighbors(i) {
            if dist[nb] == u32::MAX {
                dist[nb] = dist[i] + 1;
                queue.push_back(nb);
            }
        }
    }

    let inland_range = (w as i64 / 5).max(1);
    let latitudes: Vec<i64> = (0..h as u32).map(|y| map.latitude_tenths(y)).collect();
    for (i, tile) in map.tiles.iter_mut().enumerate() {
        let (x, y) = ((i % w) as i64, (i / w) as i64);
        let lat = latitudes[y as usize].abs();
        let mut m = latitude_rainfall(lat);
        let d = i64::from(dist[i].min(u32::MAX - 1));
        m = m * (1000 - (d * 650 / inland_range).min(650)) / 1000;
        let fx = (2 * x + 1) * ONE * 10 / (2 * w as i64);
        let fy = (2 * y + 1) * ONE * 10 / (2 * w as i64);
        m += noise.fbm(fx, fy, 6) * 450 / ONE;
        tile.moisture = m.clamp(0, 1000) as u16;
    }
}

/// Rainfall (0-1000) by latitude in tenths of a degree.
fn latitude_rainfall(lat: i64) -> i64 {
    const BANDS: [(i64, i64); 9] = [
        (0, 1000),
        (100, 900),
        (200, 450),
        (280, 280),
        (380, 550),
        (500, 850),
        (620, 700),
        (750, 350),
        (900, 150),
    ];
    for pair in BANDS.windows(2) {
        let ((l0, r0), (l1, r1)) = (pair[0], pair[1]);
        if lat <= l1 {
            return r0 + (r1 - r0) * (lat - l0) / (l1 - l0);
        }
    }
    BANDS[BANDS.len() - 1].1
}

/// Priority flood. Returns land tiles in the order they were reached
/// (downstream before upstream).
fn flood_and_drain(map: &mut Map, config: &TerrainConfig) -> Vec<usize> {
    let n = map.tile_count() as usize;
    let mut level: Vec<i32> = map.tiles.iter().map(|t| t.altitude).collect();
    let mut visited = vec![false; n];
    // Min-heap on (water level, insertion order): ties are first-come,
    // first-served, which is deterministic.
    let mut heap = BinaryHeap::new();
    let mut seq: u64 = 0;
    for i in 0..n {
        let tile = &map.tiles[i];
        if tile.water == Water::Ocean || map.is_edge(i) {
            visited[i] = true;
            heap.push(Reverse((level[i], seq, i)));
            seq += 1;
        }
    }

    // Pops come out in non-decreasing water level, so `order` lists every
    // tile after everything it could drain into.
    let mut order = Vec::with_capacity(n);
    let mut flood_parent: Vec<Option<u32>> = vec![None; n];
    while let Some(Reverse((lvl, _, i))) = heap.pop() {
        if map.tiles[i].water != Water::Ocean {
            order.push(i);
        }
        for nb in map.neighbors(i) {
            if visited[nb] {
                continue;
            }
            visited[nb] = true;
            level[nb] = level[nb].max(lvl);
            flood_parent[nb] = Some(i as u32);
            heap.push(Reverse((level[nb], seq, nb)));
            seq += 1;
        }
    }

    // Steepest descent on the filled surface. Strictly downhill or (on flats)
    // toward an earlier-flooded tile, so the drainage graph has no cycles.
    let w = map.width as usize;
    for &i in &order {
        let mut best: Option<(i64, usize)> = None;
        for nb in map.neighbors(i) {
            if level[nb] >= level[i] {
                continue;
            }
            let diagonal = nb % w != i % w && nb / w != i / w;
            let slope = i64::from(level[i] - level[nb]) * 1000 / if diagonal { 1414 } else { 1000 };
            if best.is_none_or(|(s, _)| slope > s) {
                best = Some((slope, nb));
            }
        }
        map.tiles[i].downstream = best.map(|(_, nb)| nb as u32).or(flood_parent[i]);
    }

    // Tiles where the water level is well above the ground hold standing
    // water. Shallower depressions (and lakes too small to matter) are filled
    // in, becoming flat land that drains through the flood's drainage tree.
    let depth = config.min_lake_depth;
    let deep = |map: &Map, t: usize| level[t] - map.tiles[t].altitude >= depth;
    let mut seen = vec![false; n];
    for start in 0..n {
        if seen[start] || !deep(map, start) {
            continue;
        }
        let mut component = vec![start];
        seen[start] = true;
        let mut i = 0;
        while i < component.len() {
            for nb in map.neighbors(component[i]) {
                if !seen[nb] && deep(map, nb) {
                    seen[nb] = true;
                    component.push(nb);
                }
            }
            i += 1;
        }
        let is_lake = component.len() as u32 >= config.min_lake_tiles;
        for t in component {
            if is_lake {
                map.tiles[t].water = Water::Lake;
            } else {
                map.tiles[t].altitude = level[t];
            }
        }
    }
    for (tile, &lvl) in map.tiles.iter_mut().zip(&level) {
        if tile.water != Water::Lake && tile.altitude < lvl {
            tile.altitude = lvl;
        }
        tile.surface = match tile.water {
            Water::Ocean => 0,
            Water::Lake => lvl,
            _ => tile.altitude,
        };
    }
    order
}

fn accumulate_rivers(map: &mut Map, config: &TerrainConfig, order: &[usize]) {
    for &i in order {
        map.tiles[i].flow = u32::from(map.tiles[i].moisture);
    }
    for &i in order.iter().rev() {
        if let Some(down) = map.tiles[i].downstream {
            let flow = map.tiles[i].flow;
            let target = &mut map.tiles[down as usize];
            target.flow = target.flow.saturating_add(flow);
        }
    }
    let threshold = river_threshold(map, config);
    for &i in order {
        let tile = &mut map.tiles[i];
        if tile.water == Water::Dry && tile.flow >= threshold {
            tile.water = Water::River;
        }
    }
}

/// Flow at which a tile becomes a river. Catchment is specified for a
/// 256x128 map and scaled with map area, so river density looks the same at
/// every size.
pub(crate) fn river_threshold(map: &Map, config: &TerrainConfig) -> u32 {
    let area_scale = u64::from(map.tile_count()).max(1);
    let catchment = (u64::from(config.river_catchment) * area_scale / (256 * 128)).max(4);
    u32::try_from(catchment * 1000).unwrap_or(u32::MAX)
}
