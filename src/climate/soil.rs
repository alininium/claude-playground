//! Soil water and river discharge: a single-bucket water balance per tile
//! ("bucket model", Manabe 1969) with daily river routing.
//!
//! Each land tile's soil holds up to a capacity set by its soil type. Rain
//! (and snow or glacier meltwater) fills it; evapotranspiration empties it.
//! Potential evapotranspiration is temperature-based, scaled by day length,
//! in the spirit of Hamon (1961) and Thornthwaite (1948):
//! `PET = 0.2 mm/°C/day × T × daylight/12h`. Actual evaporation falls off as
//! the soil dries (below 70% of capacity). Water beyond capacity runs off.
//!
//! Runoff is routed down the drainage network once a day: a tile's
//! discharge is its own runoff plus everything flowing in from upstream
//! (instantaneous routing: no travel time). A tile floods on days when its
//! discharge exceeds three times its long-term (yearly moving average)
//! discharge.

use crate::map::{Map, Soil, Water};

/// Flood when discharge exceeds this multiple of the long-term mean.
const FLOOD_FACTOR: u64 = 3;

#[derive(Debug, Clone)]
pub struct SoilWater {
    /// Field capacity, tenths of a millimeter (0 for water tiles).
    capacity: Vec<u32>,
    /// Water in the soil, tenths of a millimeter.
    water: Vec<u32>,
    /// Runoff produced today, tenths of a millimeter.
    runoff: Vec<u32>,
    /// Yesterday's discharge: runoff from this tile and all tiles upstream,
    /// in tenths of a millimeter over one tile per day.
    discharge: Vec<u64>,
    /// Long-term mean discharge, same units.
    mean_discharge: Vec<u64>,
    flooding: Vec<bool>,
    /// Land and lake tiles, every tile before the tile it drains into.
    order: Vec<u32>,
}

impl SoilWater {
    pub fn new(map: &Map) -> Self {
        let n = map.tile_count() as usize;
        let capacity: Vec<u32> = map
            .tiles()
            .iter()
            .map(|t| match (t.water, t.soil) {
                (Water::Ocean | Water::Lake, _) => 0,
                (_, Soil::Alluvial) => 2_000,
                (_, Soil::Loam) => 1_500,
                (_, Soil::Sandy) => 600,
                (_, Soil::Rocky) => 500,
                (_, Soil::None) => 0,
            })
            .collect();
        Self {
            water: capacity.iter().map(|c| c / 2).collect(),
            capacity,
            runoff: vec![0; n],
            discharge: vec![0; n],
            mean_discharge: vec![0; n],
            flooding: vec![false; n],
            order: upstream_first(map),
        }
    }

    /// Soil water as a share of capacity, per mille (1000 for water tiles).
    pub fn wetness(&self, index: usize) -> u32 {
        match self.capacity[index] {
            0 => 1000,
            c => self.water[index] * 1000 / c,
        }
    }

    /// Yesterday's discharge, in cubic meters per second, for tiles of
    /// `tile_km` kilometers.
    pub fn discharge_m3s(&self, index: usize, tile_km: u64) -> u64 {
        // tenths of mm over a tile per day → m³/s
        self.discharge[index] * tile_km * tile_km * 100 / 86_400
    }

    pub fn is_flooding(&self, index: usize) -> bool {
        self.flooding[index]
    }

    /// Water balance for `quarters` quarter-days. `rain` is the liquid
    /// precipitation rate (mm/year), `melt` the meltwater released this step
    /// (tenths of a millimeter), `daylight` per-tile day length (minutes).
    #[allow(clippy::too_many_arguments)]
    pub fn step(
        &mut self,
        map: &Map,
        temperatures: &[i16],
        rain: &[u16],
        melt: &[u32],
        daylight: &[u16],
        quarters: u32,
        days_per_year: u32,
    ) {
        let q = i64::from(quarters);
        let per_year = i64::from(days_per_year.max(1)) * 4;
        for i in 0..self.water.len() {
            let water = map.tiles()[i].water;
            if water == Water::Ocean {
                continue;
            }
            let t = i64::from(temperatures[i]);
            let rain_in = if t > 10 {
                i64::from(rain[i]) * 10 * q / per_year
            } else {
                0
            };
            let input = rain_in + i64::from(melt[i]);
            let capacity = i64::from(self.capacity[i]);
            if capacity == 0 {
                self.runoff[i] = self.runoff[i].saturating_add(input as u32);
                continue;
            }
            let pet = t.max(0) * i64::from(daylight[i]) * q / 14_400;
            let mut w = i64::from(self.water[i]) + input;
            let aet = pet * w.min(capacity * 7 / 10) / (capacity * 7 / 10).max(1);
            w = (w - aet).max(0);
            if w > capacity {
                self.runoff[i] = self.runoff[i].saturating_add((w - capacity) as u32);
                w = capacity;
            }
            self.water[i] = w as u32;
        }
    }

    /// Routes the day's runoff down the rivers and flags floods.
    pub fn route(&mut self, map: &Map) {
        for d in self.discharge.iter_mut() {
            *d = 0;
        }
        for &i in &self.order {
            let i = i as usize;
            self.discharge[i] += u64::from(self.runoff[i]);
            if let Some(down) = map.tiles()[i].downstream {
                let flow = self.discharge[i];
                self.discharge[down as usize] += flow;
            }
            self.runoff[i] = 0;
        }
        for i in 0..self.discharge.len() {
            let (d, mean) = (self.discharge[i], &mut self.mean_discharge[i]);
            self.flooding[i] = *mean > 0 && d > *mean * FLOOD_FACTOR;
            // Yearly exponential moving average.
            *mean = (*mean * 363 + d) / 364;
        }
    }

    pub(crate) fn digest(&self, write: &mut impl FnMut(u64)) {
        for i in 0..self.water.len() {
            write(u64::from(self.water[i]) << 32 | u64::from(self.runoff[i]));
            write(self.discharge[i]);
            write(self.mean_discharge[i] << 1 | u64::from(self.flooding[i]));
        }
    }
}

/// Kahn's topological sort of the drainage graph (deterministic: sources in
/// index order, then first-come-first-served).
fn upstream_first(map: &Map) -> Vec<u32> {
    let n = map.tile_count() as usize;
    let mut inflows = vec![0u32; n];
    for t in map.tiles() {
        if let Some(d) = t.downstream {
            inflows[d as usize] += 1;
        }
    }
    let mut queue: std::collections::VecDeque<usize> =
        (0..n).filter(|&i| inflows[i] == 0).collect();
    let mut order = Vec::with_capacity(n);
    while let Some(i) = queue.pop_front() {
        if map.tiles()[i].water != Water::Ocean {
            order.push(i as u32);
        }
        if let Some(d) = map.tiles()[i].downstream {
            let d = d as usize;
            inflows[d] -= 1;
            if inflows[d] == 0 {
                queue.push_back(d);
            }
        }
    }
    order
}
