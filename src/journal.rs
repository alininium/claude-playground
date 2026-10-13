//! The climate journal: a short, readable record of the weather and
//! volcanic events that leave a mark, each graded on five severity levels
//! from [`Severity::Minor`] to [`Severity::Cataclysmic`].
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
//! Eruptions with 1000+ deaths are raised one level. Oscillation phases
//! and volcanic winters are logged when an episode reaches a new level,
//! not again every season it lasts.

use std::fmt;

use bevy_ecs::prelude::{Res, ResMut, Resource};

use crate::climate::Climate;
use crate::map::{Map, Position, Water};
use crate::time::{Date, Turn};
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
        };
        write!(f, "{when}: {:?}: {what} {place}", self.severity)
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

    pub(crate) fn digest(&self, write: &mut impl FnMut(u64)) {
        write(self.entries.len() as u64);
        for e in &self.entries {
            write(e.turn << 8 | e.severity as u64);
        }
    }
}

/// Regions are a coarse grid of about 16 × 8 cells over the map.
const REGION_COLUMNS: u32 = 16;
const REGION_ROWS: u32 = 8;
/// Regions with fewer land tiles than this are not reported.
const MIN_REGION_LAND: usize = 12;
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
    let mut write = |severity, happening, place| {
        journal.entries.push(JournalEntry {
            turn: turn.0,
            date,
            severity,
            happening,
            place,
        });
    };

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
