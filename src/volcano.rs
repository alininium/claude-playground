//! Volcanoes: where they are, when they erupt, why, and what eruptions do.
//!
//! **Where (and why there).** Volcanoes are placed at world creation, each
//! with a geological setting that explains it:
//!
//! - [`VolcanoKind::Arc`]: high mountains near a coast, the pattern of
//!   subduction-zone volcanic arcs (the Andes, Japan, Indonesia). Viscous,
//!   gas-rich magma: these produce the big explosive eruptions, up to VEI 7.
//! - [`VolcanoKind::Hotspot`]: mantle plumes rising under a random spot of
//!   crust or ocean floor (Hawaii, Iceland, Yellowstone-like interiors).
//!   Mostly effusive: at most VEI 4 here.
//!
//! **When.** Eruptions of each size (Volcanic Explosivity Index, VEI 3-7)
//! happen at Earth-like planet-wide rates (after Newhall & Self, 1982):
//! roughly two VEI 3 a year, a VEI 4 every 7 years, a VEI 5 every 50, a VEI 6
//! every 125, a VEI 7 per millennium. Which volcano erupts is drawn in
//! proportion to the magma stored in each eligible volcano. Every volcano
//! recharges magma steadily (at heavy-tailed rates: a few systems dominate)
//! and an eruption of a given size needs at least that eruption's volume in
//! store, so the big eruptions come from the systems with huge magma
//! chambers (caldera volcanoes) and magma is conserved: nothing erupts more
//! than it has stored. If no volcano has enough magma, a drawn eruption
//! does not happen.
//!
//! **Effects.**
//! - Sulfate aerosols (VEI 4 and up) shade the planet: a tropical eruption
//!   spreads them over both hemispheres, an extratropical one mostly over
//!   its own. Cooling follows the aerosol optical depth with a ~2-month lag,
//!   and the aerosols wash out with a ~1-year e-folding time (Pinatubo,
//!   VEI 6, ≈ −0.5 °C; Tambora-like VEI 7 ≈ −1.5 °C: a "year without a
//!   summer").
//! - Ash buries crops around the volcano (worse closer in), cutting
//!   fertility for a year or two; in the long run volcanic soils are rich,
//!   so land next to a volcano gets a permanent soil bonus.
//! - Cities near the eruption lose part of their population.
//!
//! Every eruption is logged ([`SimEvent::Eruption`]) and kept in the
//! [`Volcanism`] history with its time, place, size, setting, repose time
//! and the magma it released.

use bevy_ecs::prelude::*;

use crate::city::{CityId, Population};
use crate::climate::Climate;
use crate::config::SimConfig;
use crate::events::{EventLog, SimEvent};
use crate::map::{Map, Position, Water};
use crate::rng::SimRng;
use crate::time::{TURNS_PER_DAY, Turn};

#[derive(Debug, Clone, PartialEq, Eq, serde::Serialize, serde::Deserialize)]
pub struct VolcanoConfig {
    /// Arc volcanoes per 10 000 map tiles.
    pub arcs_per_10k_tiles: u32,
    /// Hotspot volcanoes per 10 000 map tiles.
    pub hotspots_per_10k_tiles: u32,
    /// Planet-wide eruptions per 1000 years, for VEI 3, 4, 5, 6, 7.
    pub eruptions_per_millennium: [u32; 5],
    /// Cooling per unit of aerosol optical depth, tenths °C.
    pub cooling_per_optical_depth: u32,
}

impl Default for VolcanoConfig {
    fn default() -> Self {
        Self {
            arcs_per_10k_tiles: 10,
            hotspots_per_10k_tiles: 2,
            eruptions_per_millennium: [2000, 150, 20, 8, 1],
            cooling_per_optical_depth: 30,
        }
    }
}

#[derive(
    Component,
    Debug,
    Clone,
    Copy,
    PartialEq,
    Eq,
    Hash,
    PartialOrd,
    Ord,
    serde::Serialize,
    serde::Deserialize,
)]
pub struct VolcanoId(pub u32);

#[derive(Debug, Clone, Copy, PartialEq, Eq, Hash, serde::Serialize, serde::Deserialize)]
pub enum VolcanoKind {
    /// Subduction-zone volcanic arc: a high coastal mountain range.
    Arc,
    /// Mantle plume (hotspot) under the crust or the ocean floor.
    Hotspot,
}

impl VolcanoKind {
    /// Why a volcano of this kind is where it is.
    pub fn cause(self) -> &'static str {
        match self {
            VolcanoKind::Arc => "subduction arc along a coastal mountain range",
            VolcanoKind::Hotspot => "mantle plume (hotspot)",
        }
    }

    pub fn max_vei(self) -> u8 {
        match self {
            VolcanoKind::Arc => 7,
            VolcanoKind::Hotspot => 4,
        }
    }
}

#[derive(Component, Debug, Clone, PartialEq, Eq, serde::Serialize, serde::Deserialize)]
pub struct Volcano {
    pub id: VolcanoId,
    pub name: String,
    pub kind: VolcanoKind,
    /// Magma recharge, cubic meters per day.
    pub recharge: u64,
    /// Magma available to erupt, cubic meters.
    pub magma: u64,
    pub eruptions: u32,
    pub last_eruption: Option<u64>,
}

/// One eruption, as recorded in the history.
#[derive(Debug, Clone, PartialEq, Eq, serde::Serialize, serde::Deserialize)]
pub struct Eruption {
    pub turn: u64,
    pub volcano: VolcanoId,
    pub name: String,
    pub position: Position,
    pub kind: VolcanoKind,
    pub vei: u8,
    /// Years since this volcano's previous eruption (None: first one seen).
    pub repose_years: Option<u64>,
    /// Erupted volume, cubic meters.
    pub volume: u64,
    /// Aerosol optical depth added (thousandths), northern and southern.
    pub optical_depth: [u32; 2],
    /// People killed in nearby cities.
    pub casualties: u64,
}

/// Erupted volume for VEI 3..=7, cubic meters (0.01 to 100 km³).
pub fn eruption_volume(vei: u8) -> u64 {
    10_000_000 * 10u64.pow(u32::from(vei.clamp(3, 7)) - 3)
}

/// Global aerosol optical depth (thousandths) injected by an eruption.
fn optical_depth(vei: u8) -> u32 {
    match vei {
        0..=3 => 0,
        4 => 10,
        5 => 50,
        6 => 150,
        _ => 500,
    }
}

/// Radius of heavy ash fall, km.
fn ash_radius_km(vei: u8) -> i64 {
    match vei {
        0..=3 => 15,
        4 => 50,
        5 => 150,
        6 => 400,
        _ => 1000,
    }
}

/// Fertility lost under the volcano itself, per mille (half of it at the
/// edge of the ash radius).
fn ash_damage(vei: u8) -> i64 {
    match vei {
        0..=3 => 200,
        4 => 500,
        5 => 800,
        6 => 900,
        _ => 950,
    }
}

/// Share of a city's population killed at the volcano, per mille (half of
/// it at the edge of the ash radius).
fn lethality(vei: u8) -> u64 {
    match vei {
        0..=3 => 20,
        4 => 100,
        5 => 300,
        6 => 600,
        _ => 900,
    }
}

#[derive(Resource, Debug, Clone, serde::Serialize, serde::Deserialize)]
pub struct Volcanism {
    rng: SimRng,
    /// Stratospheric aerosol optical depth, millionths: north, south.
    optical_depth: [u64; 2],
    /// Cooling currently felt, millionths of a tenth °C: north, south.
    cooling: [i64; 2],
    /// Fertility lost to ash, per mille, per tile.
    ash: Vec<u16>,
    /// Permanent fertility bonus of volcanic soils, per mille, per tile.
    volcanic_soil: Vec<u16>,
    history: Vec<Eruption>,
    tile_km: i64,
}

impl Volcanism {
    pub fn history(&self) -> &[Eruption] {
        &self.history
    }

    /// Fertility lost to recent ash fall, per mille.
    pub fn ash(&self, index: usize) -> u16 {
        self.ash[index]
    }

    /// Long-term fertility bonus of volcanic soils, per mille.
    pub fn volcanic_soil(&self, index: usize) -> u16 {
        self.volcanic_soil[index]
    }

    /// Current aerosol optical depth, thousandths: north, south.
    pub fn optical_depth(&self) -> [u64; 2] {
        self.optical_depth.map(|d| d / 1000)
    }

    pub(crate) fn digest(&self, write: &mut impl FnMut(u64)) {
        write(self.optical_depth[0]);
        write(self.optical_depth[1]);
        write(self.cooling[0] as u64);
        write(self.cooling[1] as u64);
        for &a in &self.ash {
            write(u64::from(a));
        }
        write(self.history.len() as u64);
    }
}

/// Places volcanoes. Returns the [`Volcanism`] resource and the volcanoes
/// (to be spawned as entities).
pub fn place_volcanoes(
    config: &SimConfig,
    map: &Map,
    rng: &mut SimRng,
) -> (Volcanism, Vec<(Volcano, Position)>) {
    let mut rng = SimRng::from_seed(rng.next_u64());
    let n = map.tile_count() as usize;
    let tile_km = i64::from(config.climate.planet_circumference_km) / i64::from(map.width()).max(1);
    let v = &config.volcanoes;

    // Arc candidates: high ground (≥ 1200 m) within 4 tiles of the ocean.
    let coast_distance = ocean_distance(map, 4);
    let arc_candidates: Vec<usize> = (0..n)
        .filter(|&i| {
            let t = &map.tiles()[i];
            matches!(t.water, Water::Dry | Water::River)
                && t.altitude >= 1200
                && coast_distance[i] <= 4
        })
        .collect();
    let arcs = (n as u64 * u64::from(v.arcs_per_10k_tiles) / 10_000) as usize;
    let hotspots = ((n as u64 * u64::from(v.hotspots_per_10k_tiles)).div_ceil(10_000)) as usize;

    let mut taken: Vec<Position> = Vec::new();
    let mut sites: Vec<(usize, VolcanoKind)> = Vec::new();
    let far_enough = |taken: &[Position], p: Position| {
        taken
            .iter()
            .all(|q| q.x.abs_diff(p.x).max(q.y.abs_diff(p.y)) >= 3)
    };
    // Arcs: random draws from the candidates, kept at least 3 tiles apart.
    for _ in 0..arcs * 4 {
        if sites.len() >= arcs || arc_candidates.is_empty() {
            break;
        }
        let i = arc_candidates[rng.below(arc_candidates.len() as u64) as usize];
        let p = map.position_of(i as u32);
        if far_enough(&taken, p) {
            taken.push(p);
            sites.push((i, VolcanoKind::Arc));
        }
    }
    // Hotspots: anywhere.
    let mut placed = 0;
    for _ in 0..hotspots * 8 {
        if placed >= hotspots {
            break;
        }
        let i = rng.below(n as u64) as usize;
        let p = map.position_of(i as u32);
        if far_enough(&taken, p) {
            taken.push(p);
            sites.push((i, VolcanoKind::Hotspot));
            placed += 1;
        }
    }

    // Planet-wide long-term erupted volume per day, shared by activity.
    let rates = v.eruptions_per_millennium;
    let yearly_volume: u64 = (3..=7u8)
        .map(|vei| u64::from(rates[usize::from(vei - 3)]) * eruption_volume(vei) / 1000)
        .sum();
    let days = u64::from(config.climate.days_per_year.max(1));
    // Heavy-tailed activity (log-uniform over three orders of magnitude): a
    // few systems dominate eruptive output, as on Earth.
    let activity: Vec<u64> = sites
        .iter()
        .map(|_| {
            let tenths_of_decades = rng.below(31); // 10^(0..3.0)
            (0..tenths_of_decades).fold(1000u64, |a, _| a * 1259 / 1000)
        })
        .collect();
    let total_activity: u64 = activity.iter().sum::<u64>().max(1);

    let mut volcanic_soil = vec![0u16; n];
    let mut volcanoes = Vec::with_capacity(sites.len());
    for (k, &(i, kind)) in sites.iter().enumerate() {
        let recharge = yearly_volume * activity[k] / total_activity / days;
        // Start part-way through a repose: arcs up to 5000 years of stored
        // magma (caldera systems), hotspots up to 300 (they erupt often).
        let max_years = match kind {
            VolcanoKind::Arc => 5000,
            VolcanoKind::Hotspot => 300,
        };
        let magma = recharge * rng.below(max_years * days + 1);
        let position = map.position_of(i as u32);
        volcanoes.push((
            Volcano {
                id: VolcanoId(k as u32),
                name: crate::city::generate_name(&mut rng),
                kind,
                recharge,
                magma,
                eruptions: 0,
                last_eruption: None,
            },
            position,
        ));
        for j in tiles_within(map, position, 2) {
            volcanic_soil[j] = volcanic_soil[j].max(120);
        }
    }

    let volcanism = Volcanism {
        rng,
        optical_depth: [0, 0],
        cooling: [0, 0],
        ash: vec![0; n],
        volcanic_soil,
        history: Vec::new(),
        tile_km: tile_km.max(1),
    };
    (volcanism, volcanoes)
}

/// BFS distance (in tiles) to the ocean, up to `max`.
fn ocean_distance(map: &Map, max: u32) -> Vec<u32> {
    let n = map.tile_count() as usize;
    let mut dist = vec![u32::MAX; n];
    let mut queue = std::collections::VecDeque::new();
    for (i, d) in dist.iter_mut().enumerate() {
        if map.tiles()[i].water == Water::Ocean {
            *d = 0;
            queue.push_back(i);
        }
    }
    while let Some(i) = queue.pop_front() {
        if dist[i] >= max {
            continue;
        }
        for nb in map.neighbors(i) {
            if dist[nb] == u32::MAX {
                dist[nb] = dist[i] + 1;
                queue.push_back(nb);
            }
        }
    }
    dist
}

/// Tile indices within Chebyshev distance `r` of a position.
fn tiles_within(map: &Map, p: Position, r: i64) -> Vec<usize> {
    let (w, h) = (i64::from(map.width()), i64::from(map.height()));
    let (cx, cy) = (i64::from(p.x), i64::from(p.y));
    let mut out = Vec::new();
    for y in (cy - r).max(0)..=(cy + r).min(h - 1) {
        for x in (cx - r).max(0)..=(cx + r).min(w - 1) {
            out.push((y * w + x) as usize);
        }
    }
    out
}

/// Runs once a day (at the morning turn): recharges magma, ages the
/// aerosols and ash, rolls for eruptions, applies their effects, and tells
/// the climate how much the aerosols cool each hemisphere.
#[allow(clippy::too_many_arguments)] // Bevy systems declare their data access as parameters.
pub fn volcanic_activity(
    turn: Res<Turn>,
    config: Res<SimConfig>,
    map: Res<Map>,
    mut volcanism: ResMut<Volcanism>,
    mut climate: ResMut<Climate>,
    mut log: ResMut<EventLog>,
    mut volcanoes: Query<(&mut Volcano, &Position)>,
    mut cities: Query<(&CityId, &Position, &mut Population)>,
) {
    if !turn.0.is_multiple_of(TURNS_PER_DAY) {
        return;
    }
    let v = &mut *volcanism;
    for mut volcano in volcanoes.iter_mut().map(|(volcano, _)| volcano) {
        volcano.magma = volcano.magma.saturating_add(volcano.recharge);
    }
    // Aerosols wash out (e-folding ~1 year); cooling follows with ~2 months lag.
    let days = i64::from(config.climate.days_per_year.max(1));
    let k = i64::from(config.volcanoes.cooling_per_optical_depth);
    for h in 0..2 {
        v.optical_depth[h] -= v.optical_depth[h] / days as u64;
        // optical depth (millionths) × k (tenths °C per unit) → millionths of tenths.
        let target = v.optical_depth[h] as i64 * k;
        v.cooling[h] += (target - v.cooling[h]) / 60;
    }
    for a in v.ash.iter_mut() {
        *a -= (*a).div_ceil(250).min(*a);
    }

    let rates = config.volcanoes.eruptions_per_millennium;
    for vei in 3..=7u8 {
        let rate = u64::from(rates[usize::from(vei - 3)]);
        if v.rng.below(1000 * days as u64) >= rate {
            continue;
        }
        // Eligible: big enough setting and enough stored magma; weighted by magma.
        let volume = eruption_volume(vei);
        let mut eligible: Vec<(VolcanoId, u64)> = volcanoes
            .iter()
            .filter(|(volcano, _)| volcano.kind.max_vei() >= vei && volcano.magma >= volume)
            .map(|(volcano, _)| (volcano.id, volcano.magma))
            .collect();
        eligible.sort();
        let total: u64 = eligible.iter().map(|e| e.1).sum();
        if total == 0 {
            continue;
        }
        let mut r = v.rng.below(total);
        let chosen = eligible
            .iter()
            .find(|e| {
                if r < e.1 {
                    true
                } else {
                    r -= e.1;
                    false
                }
            })
            .expect("r < total")
            .0;
        let Some((mut volcano, &position)) = volcanoes.iter_mut().find(|(vol, _)| vol.id == chosen)
        else {
            continue;
        };
        volcano.magma -= volume;
        volcano.eruptions += 1;
        let tpy = days as u64 * TURNS_PER_DAY;
        let repose_years = volcano.last_eruption.map(|t| (turn.0 - t) / tpy);
        volcano.last_eruption = Some(turn.0);
        let (name, kind) = (volcano.name.clone(), volcano.kind);

        // Aerosols (`depth` is the global mean): tropical eruptions spread
        // evenly over both hemispheres, others keep 85% in their own.
        let lat = map.latitude_tenths(position.y);
        let depth = optical_depth(vei);
        let split = if lat.abs() < 200 {
            [depth, depth]
        } else if lat > 0 {
            [depth * 170 / 100, depth * 30 / 100]
        } else {
            [depth * 30 / 100, depth * 170 / 100]
        };
        for (depth, &added) in v.optical_depth.iter_mut().zip(&split) {
            *depth += u64::from(added) * 1000;
        }

        // Ash and casualties within the ash radius.
        let radius = (ash_radius_km(vei) / v.tile_km).max(0);
        for i in tiles_within(&map, position, radius) {
            let p = map.position_of(i as u32);
            let d = i64::from(p.x.abs_diff(position.x).max(p.y.abs_diff(position.y)));
            let near = 1000 - d * 500 / (radius + 1);
            let damage = (ash_damage(vei) * near / 1000) as u16;
            v.ash[i] = v.ash[i].max(damage);
        }
        let mut casualties = 0;
        let mut hit: Vec<(CityId, Mut<Population>, i64)> = cities
            .iter_mut()
            .filter_map(|(id, p, pop)| {
                let d = i64::from(p.x.abs_diff(position.x).max(p.y.abs_diff(position.y)));
                (d <= radius).then_some((*id, pop, d))
            })
            .collect();
        hit.sort_by_key(|h| h.0);
        for (_, pop, d) in hit.iter_mut() {
            let share = lethality(vei) * (1000 - (*d as u64) * 500 / (radius as u64 + 1)) / 1000;
            let deaths = pop.count * share / 1000;
            pop.count -= deaths;
            casualties += deaths;
        }

        let eruption = Eruption {
            turn: turn.0,
            volcano: chosen,
            name: name.clone(),
            position,
            kind,
            vei,
            repose_years,
            volume,
            optical_depth: split,
            casualties,
        };
        log.push(SimEvent::Eruption {
            turn: turn.0,
            volcano: chosen,
            name,
            position,
            vei,
            cause: kind.cause(),
            repose_years,
            casualties,
        });
        v.history.push(eruption);
    }

    let to_tenths = |c: i64| (c / 1_000_000).clamp(0, i64::from(i16::MAX)) as i16;
    climate.set_volcanic_cooling(to_tenths(v.cooling[0]), to_tenths(v.cooling[1]));
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn bigger_eruptions_are_ten_times_bigger() {
        for vei in 3..7 {
            assert_eq!(eruption_volume(vei + 1), eruption_volume(vei) * 10);
        }
        assert_eq!(eruption_volume(6), 10_000_000_000); // 10 km³
    }
}
