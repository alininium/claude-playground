//! Stage 2: oceans, rainfall, lakes and rivers.
//!
//! 1. Large connected areas below sea level are ocean.
//! 2. Rainfall follows Earth-like latitude bands (wet tropics, dry
//!    subtropics, wet temperate belt, dry poles). Moist air comes in from the
//!    sea with the prevailing winds (easterly trade winds below 30°,
//!    westerlies to 60°, polar easterlies beyond), so rainfall drops with the
//!    distance the wind has travelled over land. Mountains upwind cast a rain
//!    shadow, and slopes facing the wind get extra (orographic) rain.
//!    Subtropical west coasts, washed by cold upwelling currents, are dry.
//!    The tropical rain belt follows the *thermal equator*, which wanders
//!    with longitude and bulges toward the hemisphere with more land (see
//!    [`tropical_shift`]), so tropical bands are not straight lines. Plus
//!    some noise.
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
    let equator_noise = Noise::new(rng.next_u64());
    mark_oceans(map);
    tropical_shift(map, &equator_noise);
    land_share(map);
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
    let surface: Vec<i64> = map
        .tiles
        .iter()
        .map(|t| i64::from(t.altitude.max(0)))
        .collect();
    let water: Vec<Water> = map.tiles.iter().map(|t| t.water).collect();
    for (i, tile) in map.tiles.iter_mut().enumerate() {
        let (x, y) = ((i % w) as i64, (i / w) as i64);
        let lat = latitudes[y as usize];
        let effective = tropical_latitude(lat, tile.tropical_shift);
        let mut m = latitude_rainfall(effective.abs());

        // Walk upwind (against the prevailing wind) to the sea, noting the
        // highest ground the air had to cross.
        let upwind = -prevailing_wind(lat);
        let (mut upwind_land, mut barrier) = (0i64, 0i64);
        let mut cx = x;
        while upwind_land < inland_range && water[(y as usize) * w + cx as usize] != Water::Ocean {
            let next = cx + upwind;
            if next < 0 || next >= w as i64 {
                break; // the map edge counts as open sea
            }
            cx = next;
            upwind_land += 1;
            barrier = barrier.max(surface[(y as usize) * w + cx as usize]);
        }
        tile.upwind_land = upwind_land as u16;
        let d_bfs = i64::from(dist[i].min(u32::MAX - 1));
        let d = (upwind_land + 2 * d_bfs) / 3;
        m = m * (1000 - (d * 600 / inland_range).min(600)) / 1000;

        // Rain shadow behind mountains higher than the tile itself.
        let shadow = (barrier - surface[i] - 300).max(0);
        m = m * (1000 - (shadow * 600 / 2500).min(600)) / 1000;
        // Orographic rain on slopes rising into the wind.
        let windward_x = x + upwind;
        if (0..w as i64).contains(&windward_x) {
            let rise = surface[i] - surface[(y as usize) * w + windward_x as usize];
            m = m * (1000 + (rise * 300 / 1000).clamp(0, 300)) / 1000;
        }

        // Cold upwelling currents along subtropical west coasts (Atacama,
        // Namib, Baja California): sea to the west, 12-32° from the equator.
        let cold_coast = (1..=2)
            .any(|dx| x - dx >= 0 && water[(y as usize) * w + (x - dx) as usize] == Water::Ocean);
        if cold_coast {
            let band = interpolate(&COLD_CURRENT, lat.abs());
            m = m * (1000 - band) / 1000;
        }

        let fx = (2 * x + 1) * ONE * 10 / (2 * w as i64);
        let fy = (2 * y + 1) * ONE * 10 / (2 * w as i64);
        m += noise.fbm(fx, fy, 6) * 450 / ONE;
        tile.moisture = m.clamp(0, 1000) as u16;
    }
}

/// How much drier subtropical west coasts are, per mille, by |latitude|.
const COLD_CURRENT: [(i64, i64); 4] = [(100, 0), (160, 450), (280, 450), (340, 0)];

fn interpolate(table: &[(i64, i64)], x: i64) -> i64 {
    for pair in table.windows(2) {
        let ((x0, y0), (x1, y1)) = (pair[0], pair[1]);
        if x <= x1 {
            return y0 + (y1 - y0) * (x - x0).max(0) / (x1 - x0);
        }
    }
    table[table.len() - 1].1
}

/// Latitude (tenths of a degree) as the tropical climate sees it: shifted by
/// the local displacement of the thermal equator, fully within 25° of the
/// equator and fading out by 45°.
pub fn tropical_latitude(latitude_tenths: i64, shift: i16) -> i64 {
    let fade = (1000 - (latitude_tenths.abs() - 250).max(0) * 1000 / 200).clamp(0, 1000);
    latitude_tenths - i64::from(shift) * fade / 1000
}

/// The thermal equator — where the tropical rain belt sits — is not the
/// geographic equator. Over each stretch of longitude it is pulled toward
/// the hemisphere with more land in the tropics (land heats up more), up to
/// about 10°, plus a wander of typically ±5-8° that varies over a few
/// hundred kilometers. Stored per tile in
/// tenths of a degree (positive: north).
fn tropical_shift(map: &mut Map, noise: &Noise) {
    let (w, h) = (map.width as i64, map.height as i64);
    let latitudes: Vec<i64> = (0..h as u32).map(|y| map.latitude_tenths(y)).collect();
    // Tropical land per column, north and south of the equator.
    let mut north = vec![0i64; w as usize];
    let mut south = vec![0i64; w as usize];
    for y in 0..h {
        let lat = latitudes[y as usize];
        if lat.abs() > 300 {
            continue;
        }
        for x in 0..w {
            if map.tiles[(y * w + x) as usize].water != Water::Ocean {
                if lat >= 0 {
                    north[x as usize] += 1;
                } else {
                    south[x as usize] += 1;
                }
            }
        }
    }
    let window = (w / 24).max(1);
    let pull: Vec<i64> = (0..w)
        .map(|x| {
            let (mut n, mut s) = (0, 0);
            for dx in -window..=window {
                let c = (x + dx).clamp(0, w - 1) as usize;
                n += north[c];
                s += south[c];
            }
            (n - s) * 100 / (n + s + 2 * window + 1).max(1)
        })
        .collect();
    for (i, tile) in map.tiles.iter_mut().enumerate() {
        let (x, y) = (i as i64 % w, i as i64 / w);
        let fx = (2 * x + 1) * ONE * 6 / (2 * w);
        let fy = (2 * y + 1) * ONE * 6 / (2 * w);
        let wander = noise.fbm(fx, fy, 4) * 250 / ONE;
        tile.tropical_shift = (pull[x as usize] + wander).clamp(-150, 150) as i16;
    }
}

/// Share of land (per mille) within about 1700 km of each tile: large
/// continents drive strong monsoons, maritime regions weak ones.
fn land_share(map: &mut Map) {
    let (w, h) = (map.width as usize, map.height as usize);
    let r = (w / 24).max(1) as i64;
    // Summed-area table of land tiles.
    let mut sum = vec![0i64; (w + 1) * (h + 1)];
    for y in 0..h {
        for x in 0..w {
            let land = i64::from(map.tiles[y * w + x].water != Water::Ocean);
            sum[(y + 1) * (w + 1) + x + 1] =
                land + sum[y * (w + 1) + x + 1] + sum[(y + 1) * (w + 1) + x] - sum[y * (w + 1) + x];
        }
    }
    for (i, tile) in map.tiles.iter_mut().enumerate() {
        let (x, y) = ((i % w) as i64, (i / w) as i64);
        let (x0, x1) = ((x - r).max(0) as usize, ((x + r + 1) as usize).min(w));
        let (y0, y1) = ((y - r).max(0) as usize, ((y + r + 1) as usize).min(h));
        let land = sum[y1 * (w + 1) + x1] - sum[y0 * (w + 1) + x1] - sum[y1 * (w + 1) + x0]
            + sum[y0 * (w + 1) + x0];
        let area = ((x1 - x0) * (y1 - y0)) as i64;
        tile.land_share = (land * 1000 / area.max(1)) as u16;
    }
}

/// Direction the prevailing wind blows toward, east-west, at a latitude in
/// tenths of a degree: −1 (westward: trade winds and polar easterlies) or +1
/// (eastward: the westerlies between 30° and 60°).
pub fn prevailing_wind(latitude_tenths: i64) -> i64 {
    if (300..600).contains(&latitude_tenths.abs()) {
        1
    } else {
        -1
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
