//! The journal: a short, readable record of the weather and volcanic
//! events that leave a mark, and of famines and technologies gained or
//! lost, each graded on five severity levels from [`Severity::Minor`] to
//! [`Severity::Cataclysmic`].
//!
//! The journal is written once per season, never per step. It reads the
//! season summary the climate keeps (mean temperature, precipitation and
//! flood days of every tile) and compares large regions against their
//! long-term climate normals for that season (floods: rivers overflowing
//! their bankfull level, see [`crate::climate::soil`]).
//! Regional weather is only judged once the climate normals rest on
//! [`MIN_RECORD_YEARS`] of record (as real climatology uses multi-decade
//! normals); oscillation phases, volcanic winters and eruptions are logged
//! from the start.
//! Thresholds are high, so entries are rare:
//!
//! | Happening | Minor | Moderate | Major | Severe | Cataclysmic |
//! |---|---|---|---|---|---|
//! | Heat wave / cold spell (regional season mean vs normal) | ±1.5 °C | ±2.5 | ±3.5 | ±5 | ±8 |
//! | Drought (regional season rain, % of normal) | ≤ 60% | ≤ 45 | ≤ 30 | ≤ 20 | ≤ 10 |
//! | Deluge (% of normal) | ≥ 170% | ≥ 200 | ≥ 250 | ≥ 300 | ≥ 400 |
//! | Floods (river flood days in a region) | ≥ 40 | ≥ 100 | ≥ 250 | ≥ 600 | ≥ 1500 |
//! | Warm / cold oscillation phase (index) | ±1.0 °C | ±1.5 | ±2.0 | ±2.5 | ±3.0 |
//! | Volcanic winter (hemispheric aerosol cooling) | 0.3 °C | 0.6 | 1.0 | 1.5 | 2.5 |
//! | Eruption | VEI 3 with deaths | VEI 4 | VEI 5 | VEI 6 | VEI 7 |
//!
//! | Famine (one city: share of the season's rations missing) | ≥ 10% | ≥ 40% | | | |
//! | Technology gained or lost (one city) | any | much builds on it (10+), or an invention | | | |
//! | Famine, or the same technology gained or lost, in several cities in a season | | | 2+ cities | ¼ of cities (3+) | ½ of cities (4+) |
//!
//! Society entries come from `record_society`, which runs after the people
//! modules; see there. Eruptions with 1000+ deaths are raised one level. Oscillation phases
//! and volcanic winters are logged when an episode reaches a new level,
//! not again every season it lasts. When more than four regions report the
//! same kind of anomaly in a season (a volcanic winter, an El Niño year),
//! they become one widespread entry with the highest severity, the worst
//! region's value and the number of regions.

use std::fmt;

use bevy_ecs::prelude::{Query, Res, ResMut, Resource};

use crate::city::{City, CityId};
use crate::climate::Climate;
use crate::config::SimConfig;
use crate::economy::Larder;
use crate::map::{Map, Position, Water};
use crate::tech::{Knowledge, TechId, TechTree};
use crate::time::{Date, TURNS_PER_DAY, TimeOfDay, Turn};
use crate::volcano::{Volcanism, VolcanoId, VolcanoKind};

#[derive(
    Debug, Clone, Copy, PartialEq, Eq, Hash, PartialOrd, Ord, serde::Serialize, serde::Deserialize,
)]
pub enum Severity {
    Minor,
    Moderate,
    Major,
    Severe,
    Cataclysmic,
}

impl Severity {
    const ALL: [Severity; 5] = [
        Severity::Minor,
        Severity::Moderate,
        Severity::Major,
        Severity::Severe,
        Severity::Cataclysmic,
    ];

    /// The highest level whose threshold `value` reaches (thresholds
    /// ascending), if any.
    fn grade(value: i64, thresholds: [i64; 5]) -> Option<Severity> {
        thresholds
            .iter()
            .rposition(|&t| value >= t)
            .map(|k| Severity::ALL[k])
    }

    fn raised(self) -> Severity {
        Severity::ALL[(self as usize + 1).min(4)]
    }
}

#[derive(Debug, Clone, Copy, PartialEq, Eq, Hash, serde::Serialize, serde::Deserialize)]
pub enum Hemisphere {
    Northern,
    Southern,
}

#[derive(Debug, Clone, PartialEq, Eq, serde::Serialize, serde::Deserialize)]
pub enum Happening {
    /// Season mean temperature above normal, tenths °C.
    HeatWave {
        anomaly: i64,
    },
    /// Season mean temperature below normal, tenths °C.
    ColdSpell {
        anomaly: i64,
    },
    /// Season precipitation, percent of normal.
    Drought {
        percent_of_normal: i64,
    },
    Deluge {
        percent_of_normal: i64,
    },
    /// River flood days summed over the region's tiles.
    Floods {
        flood_days: u64,
    },
    /// El Niño-like warm phase reached this index, tenths °C.
    WarmPhase {
        index: i64,
    },
    /// La Niña-like cold phase reached this index, tenths °C.
    ColdPhase {
        index: i64,
    },
    Eruption {
        volcano: VolcanoId,
        name: String,
        vei: u8,
        kind: VolcanoKind,
        casualties: u64,
    },
    /// Aerosol cooling of a hemisphere, tenths °C.
    VolcanicWinter {
        hemisphere: Hemisphere,
        cooling: i64,
    },
    /// Cities went hungry: over the season their people got this share of
    /// their rations short, per mille (the worst city's, which is named;
    /// `cities` in all).
    Famine {
        city: u32,
        city_name: String,
        shortfall: u32,
        cities: u32,
    },
    /// Cities came to know a technology (`cities` of them, the first by id
    /// named). `first` if no city knew it before (an invention). `more`:
    /// further technologies the one city gained that season, folded into
    /// this entry (the most important is named).
    TechnologyGained {
        city: u32,
        city_name: String,
        technology: String,
        first: bool,
        more: u32,
        cities: u32,
    },
    /// The last people in a city who knew a technology died or left
    /// (`cities` and `more` as for gains).
    TechnologyLost {
        city: u32,
        city_name: String,
        technology: String,
        more: u32,
        cities: u32,
    },
}

impl Happening {
    /// Famines and technologies, as opposed to weather and volcanoes.
    pub fn is_society(&self) -> bool {
        matches!(
            self,
            Happening::Famine { .. }
                | Happening::TechnologyGained { .. }
                | Happening::TechnologyLost { .. }
        )
    }
}

/// Where it happened.
#[derive(Debug, Clone, Copy, PartialEq, Eq, serde::Serialize, serde::Deserialize)]
pub enum Place {
    Planet,
    Hemisphere(Hemisphere),
    /// A large region, given by its center and its latitude/longitude
    /// (tenths of a degree; longitude 0 at the map's left edge, east +).
    Region {
        center: Position,
        latitude: i64,
        longitude: i64,
    },
    Site(Position),
    /// Many regions at once; `hemisphere` if they all lie in one.
    Widespread {
        regions: u32,
        hemisphere: Option<Hemisphere>,
    },
}

#[derive(Debug, Clone, PartialEq, Eq, serde::Serialize, serde::Deserialize)]
pub struct JournalEntry {
    /// Turn the entry was written (eruptions: the turn they happened).
    pub turn: u64,
    /// The date it refers to.
    pub date: Date,
    pub severity: Severity,
    pub happening: Happening,
    pub place: Place,
}

impl fmt::Display for JournalEntry {
    fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        let when = format!("year {} {:?}", self.date.year, self.date.season);
        let place = match self.place {
            Place::Planet => "worldwide".to_string(),
            Place::Hemisphere(h) => format!("across the {h:?} hemisphere").to_lowercase(),
            Place::Region {
                latitude,
                longitude,
                ..
            } => format!(
                "around {:.0}°{} {:.0}°E",
                latitude.abs() as f64 / 10.0,
                if latitude >= 0 { "N" } else { "S" },
                longitude as f64 / 10.0
            ),
            Place::Site(p) => format!("at ({}, {})", p.x, p.y),
            Place::Widespread {
                regions,
                hemisphere,
            } => match hemisphere {
                Some(h) => format!("in {regions} regions of the {h:?} hemisphere (worst shown)")
                    .replace("Northern", "northern")
                    .replace("Southern", "southern"),
                None => format!("in {regions} regions worldwide (worst shown)"),
            },
        };
        let what = match &self.happening {
            Happening::HeatWave { anomaly } => {
                format!(
                    "heat wave, season {:+.1} °C above normal",
                    *anomaly as f64 / 10.0
                )
            }
            Happening::ColdSpell { anomaly } => {
                format!(
                    "cold spell, season {:.1} °C below normal",
                    -*anomaly as f64 / 10.0
                )
            }
            Happening::Drought { percent_of_normal } => {
                format!("drought, {percent_of_normal}% of normal rain")
            }
            Happening::Deluge { percent_of_normal } => {
                format!("deluge, {percent_of_normal}% of normal rain")
            }
            Happening::Floods { flood_days } => format!("floods, {flood_days} river flood days"),
            Happening::WarmPhase { index } => {
                format!(
                    "El Niño-like warm phase, index {:+.1} °C",
                    *index as f64 / 10.0
                )
            }
            Happening::ColdPhase { index } => {
                format!(
                    "La Niña-like cold phase, index {:+.1} °C",
                    *index as f64 / 10.0
                )
            }
            Happening::Eruption {
                name,
                vei,
                kind,
                casualties,
                ..
            } => format!(
                "VEI {vei} eruption of {name} ({}), {casualties} dead",
                kind.cause()
            ),
            Happening::VolcanicWinter { cooling, .. } => {
                format!(
                    "volcanic winter, {:.1} °C of aerosol cooling",
                    *cooling as f64 / 10.0
                )
            }
            Happening::Famine {
                city_name,
                shortfall,
                cities,
                ..
            } => format!(
                "famine in {city_name}{}, {}{:.0}% of rations missing",
                and_others(*cities),
                if *cities > 1 { "up to " } else { "" },
                *shortfall as f64 / 10.0
            ),
            Happening::TechnologyGained {
                city_name,
                technology,
                first,
                more,
                cities,
                ..
            } => format!(
                "{city_name}{} {} {technology}{}",
                and_others(*cities),
                match (*first, *cities > 1) {
                    (true, false) => "invents",
                    (true, true) => "invent",
                    (false, false) => "learns",
                    (false, true) => "learn",
                },
                and_more(*more)
            ),
            Happening::TechnologyLost {
                city_name,
                technology,
                more,
                cities,
                ..
            } => format!(
                "{city_name}{} {} {technology}{}",
                and_others(*cities),
                if *cities > 1 { "forget" } else { "forgets" },
                and_more(*more)
            ),
        };
        write!(f, "{when}: {:?}: {what} {place}", self.severity)
    }
}

/// " and 3 other cities" (empty for one city).
fn and_others(cities: u32) -> String {
    match cities {
        0 | 1 => String::new(),
        2 => " and 1 other city".to_string(),
        n => format!(" and {} other cities", n - 1),
    }
}

/// " and 3 more technologies" (empty for 0).
fn and_more(more: u32) -> String {
    match more {
        0 => String::new(),
        1 => " and 1 more technology".to_string(),
        n => format!(" and {n} more technologies"),
    }
}

#[derive(Resource, Debug, Clone, Default, serde::Serialize, serde::Deserialize)]
pub struct Journal {
    entries: Vec<JournalEntry>,
    /// Eruptions already looked at.
    eruptions_seen: usize,
    /// Highest level reached by the current oscillation episode (sign, level).
    oscillation_episode: Option<(i64, Severity)>,
    /// Highest level reached by the current volcanic winter, per hemisphere.
    volcanic_winter: [Option<Severity>; 2],
}

impl Journal {
    pub fn entries(&self) -> &[JournalEntry] {
        &self.entries
    }

    /// Feeds the world's entries (weather and volcanoes) or society's
    /// (famines and technologies) to `write`.
    pub(crate) fn digest(&self, society: bool, write: &mut impl FnMut(u64)) {
        let entries = self
            .entries
            .iter()
            .filter(|e| e.happening.is_society() == society);
        write(entries.clone().count() as u64);
        for e in entries {
            write(e.turn << 8 | e.severity as u64);
        }
    }

    /// The journal without the society entries: what a saved world keeps,
    /// since worlds are saved without their cities and people.
    pub fn world_part(&self) -> Journal {
        Journal {
            entries: self
                .entries
                .iter()
                .filter(|e| !e.happening.is_society())
                .cloned()
                .collect(),
            ..self.clone()
        }
    }
}

/// Regions are a coarse grid of about 16 × 8 cells over the map.
const REGION_COLUMNS: u32 = 16;
const REGION_ROWS: u32 = 8;
/// Regions with fewer land tiles than this are not reported.
const MIN_REGION_LAND: usize = 12;
/// More regions than this with the same anomaly in a season are merged into
/// one widespread entry.
const MAX_SEPARATE_REGIONS: usize = 4;
/// Years of record before regional weather is judged against the normals.
pub const MIN_RECORD_YEARS: u64 = 20;

/// Writes the journal at the start of every season, for the season that
/// just ended. Does nothing on other turns.
pub fn update_journal(
    turn: Res<Turn>,
    map: Res<Map>,
    climate: Res<Climate>,
    volcanism: Res<Volcanism>,
    mut journal: ResMut<Journal>,
) {
    let Some((closed_on, summary)) = climate.last_season() else {
        return;
    };
    if *closed_on != turn.0 {
        return;
    }
    let days_per_year = climate.days_per_year();
    // The season that ended: the day before this turn.
    let date = Date::from_turn(turn.0.saturating_sub(4), days_per_year);
    let journal = &mut *journal;
    // Regional findings are collected first, then widespread ones merged.
    let mut regional: Vec<(Severity, Happening, Place)> = Vec::new();
    let mut write = |severity, happening, place| regional.push((severity, happening, place));

    // Regional weather, once the normals have a long enough record.
    let judge_weather = date.year > MIN_RECORD_YEARS;
    let (w, h) = (map.width(), map.height());
    let (cw, ch) = (
        w.div_ceil(REGION_COLUMNS).max(1),
        h.div_ceil(REGION_ROWS).max(1),
    );
    for ry in 0..h.div_ceil(ch) {
        for rx in 0..w.div_ceil(cw) {
            let mut land = Vec::new();
            for y in ry * ch..((ry + 1) * ch).min(h) {
                for x in rx * cw..((rx + 1) * cw).min(w) {
                    let i = (y * w + x) as usize;
                    if matches!(map.tiles()[i].water, Water::Dry | Water::River) {
                        land.push(i);
                    }
                }
            }
            if land.len() < MIN_REGION_LAND {
                continue;
            }
            let center = Position {
                x: (rx * cw + cw / 2).min(w - 1),
                y: (ry * ch + ch / 2).min(h - 1),
            };
            let place = Place::Region {
                center,
                latitude: map.latitude_tenths(center.y),
                longitude: i64::from(center.x) * 3600 / i64::from(w),
            };
            let n = land.len() as i64;
            let anomaly = land
                .iter()
                .map(|&i| {
                    i64::from(summary.temperature[i])
                        - i64::from(climate.normal(i).seasonal_mean[summary.season]) / 10
                })
                .sum::<i64>()
                / n;
            if !judge_weather {
            } else if let Some(s) = Severity::grade(anomaly, [15, 25, 35, 50, 80]) {
                write(s, Happening::HeatWave { anomaly }, place);
            } else if let Some(s) = Severity::grade(-anomaly, [15, 25, 35, 50, 80]) {
                write(s, Happening::ColdSpell { anomaly }, place);
            }

            let actual: i64 = land
                .iter()
                .map(|&i| i64::from(summary.precipitation[i]))
                .sum();
            let normal: i64 = land
                .iter()
                .map(|&i| i64::from(climate.normal(i).seasonal_precipitation[summary.season]))
                .sum();
            // Only where a dry spell matters: at least ~50 mm expected.
            if judge_weather && normal >= 500 * n {
                let percent = actual * 100 / normal;
                if let Some(s) = Severity::grade(-percent, [-60, -45, -30, -20, -10]) {
                    write(
                        s,
                        Happening::Drought {
                            percent_of_normal: percent,
                        },
                        place,
                    );
                } else if let Some(s) = Severity::grade(percent, [170, 200, 250, 300, 400]) {
                    write(
                        s,
                        Happening::Deluge {
                            percent_of_normal: percent,
                        },
                        place,
                    );
                }
            }

            let flood_days: u64 = land.iter().map(|&i| u64::from(summary.flood_days[i])).sum();
            if !judge_weather {
            } else if let Some(s) = Severity::grade(flood_days as i64, [40, 100, 250, 600, 1500]) {
                write(s, Happening::Floods { flood_days }, place);
            }
        }
    }

    // More than MAX_SEPARATE_REGIONS regions with the same kind of anomaly
    // (say, a volcanic winter) become one widespread entry: highest
    // severity, worst region's value, number of regions.
    let mut kinds: Vec<std::mem::Discriminant<Happening>> = Vec::new();
    for (_, happening, _) in &regional {
        let kind = std::mem::discriminant(happening);
        if !kinds.contains(&kind) {
            kinds.push(kind);
        }
    }
    for kind in kinds {
        let group: Vec<&(Severity, Happening, Place)> = regional
            .iter()
            .filter(|(_, h, _)| std::mem::discriminant(h) == kind)
            .collect();
        let mut push = |severity, happening: &Happening, place| {
            journal.entries.push(JournalEntry {
                turn: turn.0,
                date,
                severity,
                happening: happening.clone(),
                place,
            })
        };
        if group.len() <= MAX_SEPARATE_REGIONS {
            for (severity, happening, place) in group {
                push(*severity, happening, *place);
            }
            continue;
        }
        let worst = group
            .iter()
            .max_by_key(|(severity, ..)| *severity)
            .expect("non-empty group");
        let latitudes: Vec<i64> = group
            .iter()
            .filter_map(|(.., place)| match place {
                Place::Region { latitude, .. } => Some(*latitude),
                _ => None,
            })
            .collect();
        let hemisphere = if latitudes.iter().all(|&l| l >= 0) {
            Some(Hemisphere::Northern)
        } else if latitudes.iter().all(|&l| l < 0) {
            Some(Hemisphere::Southern)
        } else {
            None
        };
        let place = Place::Widespread {
            regions: group.len() as u32,
            hemisphere,
        };
        push(worst.0, &worst.1, place);
    }

    // The oscillation: log when an episode reaches a new level.
    let index = climate.oscillation().index_tenths();
    let sign = index.signum();
    if let Some((episode_sign, _)) = journal.oscillation_episode
        && (sign != episode_sign)
    {
        journal.oscillation_episode = None;
    }
    if let Some(level) = Severity::grade(index.abs(), [10, 15, 20, 25, 30]) {
        let previous = journal.oscillation_episode.map(|(_, l)| l);
        if previous.is_none_or(|p| level > p) {
            journal.oscillation_episode = Some((sign, level));
            let happening = if sign > 0 {
                Happening::WarmPhase { index }
            } else {
                Happening::ColdPhase { index }
            };
            journal.entries.push(JournalEntry {
                turn: turn.0,
                date,
                severity: level,
                happening,
                place: Place::Planet,
            });
        }
    }

    // Volcanic winters, per hemisphere.
    for (k, hemisphere) in [Hemisphere::Northern, Hemisphere::Southern]
        .into_iter()
        .enumerate()
    {
        let cooling = i64::from(climate.volcanic_cooling()[k]);
        match Severity::grade(cooling, [3, 6, 10, 15, 25]) {
            None => journal.volcanic_winter[k] = None,
            Some(level) if journal.volcanic_winter[k].is_none_or(|p| level > p) => {
                journal.volcanic_winter[k] = Some(level);
                journal.entries.push(JournalEntry {
                    turn: turn.0,
                    date,
                    severity: level,
                    happening: Happening::VolcanicWinter {
                        hemisphere,
                        cooling,
                    },
                    place: Place::Hemisphere(hemisphere),
                });
            }
            Some(_) => {}
        }
    }

    // Eruptions since the last look that leave a mark.
    let history = volcanism.history();
    for e in &history[journal.eruptions_seen.min(history.len())..] {
        let base = match e.vei {
            0..=3 if e.casualties > 0 => Some(Severity::Minor),
            0..=3 => None,
            4 => Some(Severity::Moderate),
            5 => Some(Severity::Major),
            6 => Some(Severity::Severe),
            _ => Some(Severity::Cataclysmic),
        };
        let Some(mut severity) = base else { continue };
        if e.casualties >= 1000 {
            severity = severity.raised();
        }
        journal.entries.push(JournalEntry {
            turn: e.turn,
            date: Date::from_turn(e.turn, days_per_year),
            severity,
            happening: Happening::Eruption {
                volcano: e.volcano,
                name: e.name.clone(),
                vei: e.vei,
                kind: e.kind,
                casualties: e.casualties,
            },
            place: Place::Site(e.position),
        });
    }
    journal.eruptions_seen = history.len();
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn grading() {
        let t = [15, 25, 35, 50, 80];
        assert_eq!(Severity::grade(10, t), None);
        assert_eq!(Severity::grade(15, t), Some(Severity::Minor));
        assert_eq!(Severity::grade(49, t), Some(Severity::Major));
        assert_eq!(Severity::grade(500, t), Some(Severity::Cataclysmic));
        // Descending scales are graded on the negated value.
        let drought = [-60, -45, -30, -20, -10];
        assert_eq!(Severity::grade(-70, drought), None);
        assert_eq!(Severity::grade(-55, drought), Some(Severity::Minor));
        assert_eq!(Severity::grade(-5, drought), Some(Severity::Cataclysmic));
        assert_eq!(Severity::Severe.raised(), Severity::Cataclysmic);
        assert_eq!(Severity::Cataclysmic.raised(), Severity::Cataclysmic);
    }
}

// ---------------------------------------------------------------------------
// Societies: famines and technologies
// ---------------------------------------------------------------------------

/// Famine thresholds: the share of a season's rations missing, per mille.
pub const FAMINE_SHORTFALL: [i64; 5] = [100, 250, 400, 600, 800];

/// Technology thresholds: how many technologies build on it, directly or
/// not (losing fire matters more than losing faience).
pub const TECHNOLOGY_WEIGHT: [i64; 5] = [0, 3, 10, 20, 40];

/// What the society part of the journal remembers between seasons. Not
/// saved: worlds are saved without their people.
#[derive(Resource, Debug, Clone, Default)]
pub struct SocietyRecord {
    /// By city id: the sum of each day's ration share (per mille) this
    /// season, and the days sampled.
    rations: Vec<(u64, u32)>,
    /// By city id: what it knew at the start of the season (`None` until a
    /// city has been seen at a season's start).
    known: Vec<Option<Knowledge>>,
    /// By technology: how many technologies build on it.
    weight: Vec<u32>,
}

/// Writes famines and technologies gained or lost into the journal, at the
/// start of every season, for the season that just ended. Every night it
/// notes how well each city ate that day (the economy serves meals in the
/// evening); it does nothing on other turns.
///
/// Something that happens in one city alone is at most Moderate: a famine
/// by the share of rations missing over the season (Moderate from 40%), a
/// technology Moderate when much of the tree builds on it or when nobody
/// knew it before (an invention), Minor otherwise; more than four
/// technologies gained or lost by one city in a season become one entry.
/// Major, Severe and Cataclysmic are for what happens in several cities at
/// once (see [`spread`]): famine in several cities, or the same technology
/// gained or lost by several cities in a season, is one entry graded by
/// how many.
pub fn record_society(
    turn: Res<Turn>,
    config: Res<SimConfig>,
    tree: Res<TechTree>,
    mut record: ResMut<SocietyRecord>,
    mut journal: ResMut<Journal>,
    cities: Query<(&CityId, &City, &Position, &Knowledge, Option<&Larder>)>,
) {
    let days_per_year = config.climate.days_per_year;
    let date = Date::from_turn(turn.0, days_per_year);
    let record = &mut *record;
    if date.time_of_day == TimeOfDay::Night {
        for (id, _, _, _, larder) in &cities {
            let Some(larder) = larder.filter(|l| l.residents > 0) else {
                continue;
            };
            let i = id.0 as usize;
            if record.rations.len() <= i {
                record.rations.resize(i + 1, (0, 0));
            }
            record.rations[i].0 += u64::from(larder.ration.min(1000));
            record.rations[i].1 += 1;
        }
        return;
    }
    if date.day_of_season != 0 || date.time_of_day != TimeOfDay::Morning || turn.0 == 0 {
        return;
    }
    if record.weight.is_empty() {
        record.weight = technology_weights(&tree);
    }
    let ended = Date::from_turn(turn.0 - TURNS_PER_DAY, days_per_year);

    let mut rows: Vec<_> = cities.iter().collect();
    rows.sort_by_key(|r| *r.0);
    // How many cities knew each technology at the start of the season.
    let mut knowers = vec![0u32; tree.len()];
    for known in record.known.iter().flatten() {
        for t in known.techs() {
            knowers[usize::from(t.0)] += 1;
        }
    }

    // What each city went through this season.
    let mut famines: Vec<(u32, u32, &str, Position)> = Vec::new();
    let mut gains: Vec<(TechId, u32, &str, Position)> = Vec::new();
    let mut losses: Vec<(TechId, u32, &str, Position)> = Vec::new();
    for &(id, city, position, knowledge, _) in &rows {
        let i = id.0 as usize;
        let (sum, days) = record.rations.get(i).copied().unwrap_or((0, 0));
        if days > 0 {
            let shortfall = 1000 - (sum / u64::from(days)) as u32;
            if i64::from(shortfall) >= FAMINE_SHORTFALL[0] {
                famines.push((shortfall, id.0, &city.name, *position));
            }
        }
        if let Some(Some(before)) = record.known.get(i) {
            for t in knowledge.techs().filter(|&t| !before.knows(t)) {
                gains.push((t, id.0, &city.name, *position));
            }
            for t in before.techs().filter(|&t| !knowledge.knows(t)) {
                losses.push((t, id.0, &city.name, *position));
            }
        }
    }

    let cities_now = rows.len() as u32;
    let weight = |t: TechId| record.weight[usize::from(t.0)];
    let invention = |t: TechId| knowers[usize::from(t.0)] == 0;
    let mut found: Vec<(Severity, Happening, Place)> = Vec::new();

    // Famine: one city alone is at most Moderate; several at once are one
    // entry graded by how many, naming the worst.
    famines.sort_by_key(|f| (std::cmp::Reverse(f.0), f.1));
    if let Some(&(shortfall, city, name, position)) = famines.first() {
        let n = famines.len() as u32;
        let severity = if n > 1 {
            spread(n, cities_now)
        } else {
            Severity::grade(i64::from(shortfall), FAMINE_SHORTFALL)
                .unwrap_or(Severity::Minor)
                .min(Severity::Moderate)
        };
        found.push((
            severity,
            Happening::Famine {
                city,
                city_name: name.to_string(),
                shortfall,
                cities: n,
            },
            Place::Site(position),
        ));
    }

    // Technologies: the same one gained (or lost) by several cities in a
    // season is one entry graded by how many. What a single city gains or
    // loses on its own is Minor, or Moderate when much builds on it (or it
    // is an invention); more than four in one city become one entry.
    for (changes, gained) in [(&mut gains, true), (&mut losses, false)] {
        changes.sort_by_key(|c| (c.0, c.1));
        let mut alone: Vec<(TechId, u32, &str, Position)> = Vec::new();
        for group in changes.chunk_by(|a, b| a.0 == b.0) {
            let (t, city, name, position) = group[0];
            let n = group.len() as u32;
            if n == 1 {
                alone.push(group[0]);
                continue;
            }
            found.push((
                spread(n, cities_now),
                technology_entry(&tree, t, gained, invention(t), city, name, 0, n),
                Place::Site(position),
            ));
        }
        alone.sort_by_key(|c| (c.1, std::cmp::Reverse(weight(c.0)), c.0));
        let single = |t: TechId| {
            let important = i64::from(weight(t)) >= TECHNOLOGY_WEIGHT[2];
            if important || (gained && invention(t)) {
                Severity::Moderate
            } else {
                Severity::Minor
            }
        };
        for city_changes in alone.chunk_by(|a, b| a.1 == b.1) {
            if city_changes.len() > MAX_SEPARATE_REGIONS {
                let (t, city, name, position) = city_changes[0];
                let severity = city_changes.iter().map(|c| single(c.0)).max().unwrap();
                let more = city_changes.len() as u32 - 1;
                found.push((
                    severity,
                    technology_entry(&tree, t, gained, invention(t), city, name, more, 1),
                    Place::Site(position),
                ));
            } else {
                for &(t, city, name, position) in city_changes {
                    found.push((
                        single(t),
                        technology_entry(&tree, t, gained, invention(t), city, name, 0, 1),
                        Place::Site(position),
                    ));
                }
            }
        }
    }

    for (severity, happening, place) in found {
        journal.entries.push(JournalEntry {
            turn: turn.0,
            date: ended,
            severity,
            happening,
            place,
        });
    }
    for &(id, _, _, knowledge, _) in &rows {
        let i = id.0 as usize;
        if record.known.len() <= i {
            record.known.resize(i + 1, None);
        }
        record.known[i] = Some(knowledge.clone());
    }
    record.rations.iter_mut().for_each(|r| *r = (0, 0));
}

/// For each technology, how many technologies build on it, directly or
/// through others (through `requires` or `requires_any`).
pub fn technology_weights(tree: &TechTree) -> Vec<u32> {
    let mut builds_on: Vec<std::collections::BTreeSet<u16>> = vec![Default::default(); tree.len()];
    // Dependents come after their prerequisites in `order`, so going
    // backwards each technology's set is complete before it is passed on.
    for &id in tree.order().iter().rev() {
        let mut passed = builds_on[usize::from(id.0)].clone();
        passed.insert(id.0);
        for p in tree.requires(id).iter().chain(tree.requires_any(id)) {
            builds_on[usize::from(p.0)].extend(passed.iter().copied());
        }
    }
    builds_on.iter().map(|s| s.len() as u32).collect()
}

/// The severity of something that happened in `n` of `total` cities in
/// one season: Major in two or more, Severe in a quarter of them (at least
/// three), Cataclysmic in half of them (at least four). A single city never
/// rises above Moderate.
pub fn spread(n: u32, total: u32) -> Severity {
    if n >= 4.max(total.div_ceil(2)) {
        Severity::Cataclysmic
    } else if n >= 3.max(total.div_ceil(4)) {
        Severity::Severe
    } else if n >= 2 {
        Severity::Major
    } else {
        Severity::Moderate
    }
}

/// A technology gained or lost, as a journal happening.
#[allow(clippy::too_many_arguments)]
fn technology_entry(
    tree: &TechTree,
    t: TechId,
    gained: bool,
    first: bool,
    city: u32,
    name: &str,
    more: u32,
    cities: u32,
) -> Happening {
    let technology = tree.tech(t).name.clone();
    let city_name = name.to_string();
    if gained {
        Happening::TechnologyGained {
            city,
            city_name,
            technology,
            first,
            more,
            cities,
        }
    } else {
        Happening::TechnologyLost {
            city,
            city_name,
            technology,
            more,
            cities,
        }
    }
}
