//! Simulation time. One turn is a quarter of a day; a year has
//! `days_per_year` days split into four equal seasons. Turn 0 is the morning
//! of the first day of spring (the northern vernal equinox) of year 1.

use bevy_ecs::prelude::{ResMut, Resource};

pub const TURNS_PER_DAY: u64 = 4;

/// The current turn number. Turn 0 is the initial state, before any turn ran.
#[derive(Resource, Debug, Clone, Copy, Default, PartialEq, Eq)]
pub struct Turn(pub u64);

pub fn advance_turn(mut turn: ResMut<Turn>) {
    turn.0 += 1;
}

#[derive(Debug, Clone, Copy, PartialEq, Eq, Hash, PartialOrd, Ord)]
pub enum TimeOfDay {
    Morning,
    Day,
    Evening,
    Night,
}

#[derive(Debug, Clone, Copy, PartialEq, Eq, Hash, PartialOrd, Ord)]
pub enum Season {
    Spring,
    Summer,
    Autumn,
    Winter,
}

impl Season {
    /// The season as experienced in the southern hemisphere.
    pub fn opposite(self) -> Self {
        match self {
            Season::Spring => Season::Autumn,
            Season::Summer => Season::Winter,
            Season::Autumn => Season::Spring,
            Season::Winter => Season::Summer,
        }
    }
}

#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub struct Date {
    /// Starts at 1.
    pub year: u64,
    /// 0-based day within the year.
    pub day_of_year: u32,
    pub time_of_day: TimeOfDay,
    /// Northern-hemisphere season; see [`Season::opposite`].
    pub season: Season,
    /// 0-based day within the season.
    pub day_of_season: u32,
}

impl Date {
    pub fn from_turn(turn: u64, days_per_year: u32) -> Self {
        let days_per_year = u64::from(days_per_year.max(4));
        let day = turn / TURNS_PER_DAY;
        let day_of_year = day % days_per_year;
        let season_index = day_of_year * 4 / days_per_year;
        let season_start = (season_index * days_per_year).div_ceil(4);
        Self {
            year: day / days_per_year + 1,
            day_of_year: day_of_year as u32,
            time_of_day: match turn % TURNS_PER_DAY {
                0 => TimeOfDay::Morning,
                1 => TimeOfDay::Day,
                2 => TimeOfDay::Evening,
                _ => TimeOfDay::Night,
            },
            season: match season_index {
                0 => Season::Spring,
                1 => Season::Summer,
                2 => Season::Autumn,
                _ => Season::Winter,
            },
            day_of_season: (day_of_year - season_start) as u32,
        }
    }
}

impl std::fmt::Display for Date {
    fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        write!(
            f,
            "year {}, {:?} day {}, {:?}",
            self.year,
            self.season,
            self.day_of_season + 1,
            self.time_of_day
        )
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn calendar_cycles() {
        let d = Date::from_turn(0, 364);
        assert_eq!((d.year, d.day_of_year, d.season), (1, 0, Season::Spring));
        assert_eq!(d.time_of_day, TimeOfDay::Morning);
        assert_eq!(Date::from_turn(3, 364).time_of_day, TimeOfDay::Night);
        assert_eq!(Date::from_turn(4, 364).day_of_year, 1);
        assert_eq!(Date::from_turn(91 * 4, 364).season, Season::Summer);
        assert_eq!(Date::from_turn(91 * 4, 364).day_of_season, 0);
        assert_eq!(Date::from_turn(273 * 4, 364).season, Season::Winter);
        let next_year = Date::from_turn(364 * 4, 364);
        assert_eq!((next_year.year, next_year.season), (2, Season::Spring));
    }

    #[test]
    fn uneven_years_still_have_four_seasons() {
        let seasons: Vec<_> = (0..365)
            .map(|d| Date::from_turn(d * 4, 365).season)
            .collect();
        assert_eq!(seasons[0], Season::Spring);
        assert_eq!(seasons[364], Season::Winter);
        assert!(seasons.windows(2).all(|w| w[0] <= w[1]));
    }
}
