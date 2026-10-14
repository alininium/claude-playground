//! Yearly climate bookkeeping and long-term climate normals.
//!
//! During a year every tile accumulates its temperature, precipitation (per
//! season), growing degree-days, soil wetness during the growing season,
//! flood days, and its frost-free season (the longest run of frost-free
//! nights, counted around the turn of the year so southern summers are not
//! cut in two). At the end of the year these become last
//! year's statistics and are folded into the tile's [`ClimateNormal`], an
//! exponential moving average with a time constant of about 10 years. Climate
//! normals change slowly, so things that depend on them (vegetation
//! biomes) do not flicker with every warm or cold year.

use crate::parallel::{self, Threads};

/// A tile's long-term climate.
#[derive(
    Debug, Clone, Copy, Default, PartialEq, Eq, Hash, serde::Serialize, serde::Deserialize,
)]
pub struct ClimateNormal {
    /// Mean annual temperature, hundredths of a degree C.
    pub mean: i32,
    /// Mean temperature per season (northern-hemisphere calendar),
    /// hundredths of a degree C.
    pub seasonal_mean: [i32; 4],
    /// Mean precipitation per season (northern-hemisphere calendar: spring,
    /// summer, autumn, winter), tenths of a millimeter.
    pub seasonal_precipitation: [i32; 4],
}

impl ClimateNormal {
    pub fn mean_tenths(&self) -> i64 {
        i64::from(self.mean) / 10
    }

    pub fn annual_precipitation_mm(&self) -> i64 {
        self.seasonal_precipitation
            .iter()
            .map(|&p| i64::from(p))
            .sum::<i64>()
            / 10
    }

    /// Mean precipitation of the driest month (a third of the driest
    /// season), millimeters.
    pub fn driest_month_mm(&self) -> i64 {
        i64::from(*self.seasonal_precipitation.iter().min().unwrap_or(&0)) / 30
    }

    /// Mean precipitation of the local summer and winter seasons, as
    /// monthly amounts (a third of the season), millimeters.
    pub fn summer_and_winter_month_mm(&self, northern: bool) -> (i64, i64) {
        let p = self.seasonal_precipitation.map(i64::from);
        let (summer, winter) = if northern { (p[1], p[3]) } else { (p[3], p[1]) };
        (summer / 30, winter / 30)
    }

    /// Share of the year's precipitation falling in the local warm half of
    /// the year (spring and summer of the tile's hemisphere), per mille.
    pub fn warm_half_share(&self, northern: bool) -> i64 {
        let p = self.seasonal_precipitation.map(i64::from);
        let warm = if northern { p[0] + p[1] } else { p[2] + p[3] };
        let total: i64 = p.iter().sum();
        if total == 0 { 500 } else { warm * 1000 / total }
    }
}

/// One finished season, every tile.
#[derive(Debug, Clone, Default, serde::Serialize, serde::Deserialize)]
pub struct SeasonSummary {
    /// Northern-hemisphere season index: 0 spring .. 3 winter.
    pub season: usize,
    /// Mean temperature, tenths °C.
    pub temperature: Vec<i16>,
    /// Precipitation, tenths of a millimeter.
    pub precipitation: Vec<i32>,
    pub flood_days: Vec<u16>,
}

#[derive(Debug, Clone, serde::Serialize, serde::Deserialize)]
pub(crate) struct YearStats {
    /// Sum of temperature (tenths °C) × quarter-days, per season.
    temperature: Vec<[i64; 4]>,
    /// Sum of precipitation rate (mm/year) × quarter-days, per season.
    precipitation: Vec<[u64; 4]>,
    /// Growing degree-days above 5 °C, tenths °C × quarter-days.
    growing: Vec<u32>,
    /// Quarter-days accumulated, per season.
    quarter_days: [u32; 4],
    /// Soil wetness (per mille) × quarter-days, and quarter-days, while
    /// warm enough to grow (above 5 °C).
    wetness: Vec<u64>,
    growing_quarters: Vec<u32>,
    /// Flood days, per season.
    flood_days: Vec<[u16; 4]>,
    frost: Vec<FrostRun>,
    days: u32,
    pub(crate) normals: Vec<ClimateNormal>,
    pub(crate) growing_last_year: Vec<u32>,
    /// Mean soil wetness during last year's growing season, per mille.
    pub(crate) wetness_last_year: Vec<u16>,
    pub(crate) flood_days_last_year: Vec<u16>,
    pub(crate) frost_free_last_year: Vec<u16>,
    have_normals: bool,
}

#[derive(Debug, Clone, Copy, Default, serde::Serialize, serde::Deserialize)]
struct FrostRun {
    current: u16,
    longest: u16,
    /// Frost-free nights at the start of the year, before the first frost.
    prefix: u16,
    had_frost: bool,
}

impl YearStats {
    pub(crate) fn new(n: usize) -> Self {
        Self {
            temperature: vec![[0; 4]; n],
            precipitation: vec![[0; 4]; n],
            growing: vec![0; n],
            quarter_days: [0; 4],
            wetness: vec![0; n],
            growing_quarters: vec![0; n],
            flood_days: vec![[0; 4]; n],
            frost: vec![FrostRun::default(); n],
            days: 0,
            normals: vec![ClimateNormal::default(); n],
            growing_last_year: vec![0; n],
            wetness_last_year: vec![1000; n],
            flood_days_last_year: vec![0; n],
            frost_free_last_year: vec![0; n],
            have_normals: false,
        }
    }

    /// Adds `quarters` quarter-days at the given temperatures (tenths °C),
    /// precipitation rates (mm/year) and soil wetness (per mille), in
    /// northern-hemisphere `season` 0-3.
    pub(crate) fn accumulate(
        &mut self,
        temperatures: &[i16],
        precipitation: &[u16],
        wetness: impl Fn(usize) -> u32 + Sync,
        quarters: u32,
        season: usize,
        threads: Threads,
    ) {
        let q = i64::from(quarters);
        let c = parallel::chunk_len(
            temperatures.len(),
            threads,
            parallel::min_tiles_per_thread(),
        );
        let parts = self
            .temperature
            .chunks_mut(c)
            .zip(self.precipitation.chunks_mut(c))
            .zip(self.growing.chunks_mut(c))
            .zip(self.wetness.chunks_mut(c))
            .zip(self.growing_quarters.chunks_mut(c))
            .zip(temperatures.chunks(c).zip(precipitation.chunks(c)))
            .enumerate();
        parallel::run(parts, |(part, (sums, (temperatures, precipitation)))| {
            let ((((temp, prec), growing), wet), growing_q) = sums;
            let rows = temp
                .iter_mut()
                .zip(prec)
                .zip(growing)
                .zip(wet)
                .zip(growing_q)
                .zip(temperatures.iter().zip(precipitation))
                .enumerate();
            for (k, (((((temp, prec), growing), wet), growing_q), (&t, &p))) in rows {
                temp[season] += i64::from(t) * q;
                prec[season] += u64::from(p) * u64::from(quarters);
                if t > 50 {
                    *growing += (i32::from(t) - 50) as u32 * quarters;
                    // Soil wetness counts while it is warm enough to grow.
                    *wet += u64::from(wetness(part * c + k)) * u64::from(quarters);
                    *growing_q += quarters;
                }
            }
        });
        self.quarter_days[season] += quarters;
    }

    /// Records one day of `season`: the night's temperatures and today's
    /// floods.
    pub(crate) fn record_day(
        &mut self,
        night: &[i16],
        flooding: impl Fn(usize) -> bool + Sync,
        season: usize,
        threads: Threads,
    ) {
        let c = parallel::chunk_len(night.len(), threads, parallel::min_tiles_per_thread());
        let parts = self
            .frost
            .chunks_mut(c)
            .zip(self.flood_days.chunks_mut(c))
            .zip(night.chunks(c))
            .enumerate();
        parallel::run(parts, |(part, ((frost, flood_days), night))| {
            let rows = frost.iter_mut().zip(flood_days).zip(night).enumerate();
            for (k, ((run, floods), &t)) in rows {
                if t > 0 {
                    run.current += 1;
                } else {
                    if !run.had_frost {
                        run.prefix = run.current;
                        run.had_frost = true;
                    }
                    run.longest = run.longest.max(run.current);
                    run.current = 0;
                }
                if flooding(part * c + k) {
                    floods[season] += 1;
                }
            }
        });
        self.days += 1;
    }

    /// What one season of this year was like, for every tile: mean
    /// temperature (tenths °C), precipitation (tenths of a mm) and flood days.
    pub(crate) fn season_summary(&self, season: usize, days_per_year: u32) -> SeasonSummary {
        let quarters = i64::from(self.quarter_days[season].max(1));
        let per_year = u64::from(days_per_year.max(1)) * 4;
        SeasonSummary {
            season,
            temperature: self
                .temperature
                .iter()
                .map(|t| (t[season] / quarters) as i16)
                .collect(),
            precipitation: self
                .precipitation
                .iter()
                .map(|p| (p[season] * 10 / per_year) as i32)
                .collect(),
            flood_days: self.flood_days.iter().map(|f| f[season]).collect(),
        }
    }

    /// Closes the year: updates the normals (the first year sets them).
    pub(crate) fn end_year(&mut self, days_per_year: u32) {
        let quarters = i64::from(self.quarter_days.iter().sum::<u32>().max(1));
        let per_year = u64::from(days_per_year.max(1)) * 4;
        for i in 0..self.normals.len() {
            let mean = (self.temperature[i].iter().sum::<i64>() * 10 / quarters) as i32;
            let seasonal_mean: [i32; 4] = std::array::from_fn(|s| {
                (self.temperature[i][s] * 10 / i64::from(self.quarter_days[s].max(1))) as i32
            });
            let precipitation = self.precipitation[i].map(|p| (p * 10 / per_year) as i32);
            let normal = &mut self.normals[i];
            if self.have_normals {
                normal.mean += (mean - normal.mean) / 10;
                for (n, m) in normal.seasonal_mean.iter_mut().zip(seasonal_mean) {
                    *n += (m - *n) / 10;
                }
                for (n, p) in normal.seasonal_precipitation.iter_mut().zip(precipitation) {
                    *n += (p - *n) / 10;
                }
            } else {
                *normal = ClimateNormal {
                    mean,
                    seasonal_mean,
                    seasonal_precipitation: precipitation,
                };
            }
            self.growing_last_year[i] = self.growing[i] / 40;
            self.wetness_last_year[i] = match self.growing_quarters[i] {
                0 => 1000,
                q => (self.wetness[i] / u64::from(q)) as u16,
            };
            self.flood_days_last_year[i] = self.flood_days[i].iter().sum();
            let run = self.frost[i];
            self.frost_free_last_year[i] = if run.had_frost {
                run.longest.max(run.prefix + run.current)
            } else {
                self.days as u16
            };
            self.temperature[i] = [0; 4];
            self.precipitation[i] = [0; 4];
            self.growing[i] = 0;
            self.wetness[i] = 0;
            self.growing_quarters[i] = 0;
            self.flood_days[i] = [0; 4];
            self.frost[i] = FrostRun::default();
        }
        self.quarter_days = [0; 4];
        self.days = 0;
        self.have_normals = true;
    }

    pub(crate) fn digest(&self, write: &mut impl FnMut(u64)) {
        for q in self.quarter_days {
            write(u64::from(q));
        }
        for i in 0..self.normals.len() {
            let n = &self.normals[i];
            for t in self.temperature[i] {
                write(t as u64);
            }
            write(u64::from(self.growing[i]) << 32 | u64::from(self.growing_last_year[i]));
            write(n.mean as u64);
            for m in n.seasonal_mean {
                write(m as u64);
            }
            let f = self.frost[i];
            write(
                u64::from(f.current) << 48
                    | u64::from(f.longest) << 32
                    | u64::from(f.prefix) << 16
                    | u64::from(f.had_frost),
            );
            write(self.wetness[i] ^ u64::from(self.growing_quarters[i]) << 40);
            write(
                u64::from(self.flood_days[i].iter().sum::<u16>()) << 48
                    | u64::from(self.wetness_last_year[i]) << 32
                    | u64::from(self.flood_days_last_year[i]) << 16
                    | u64::from(self.frost_free_last_year[i]),
            );
            for k in 0..4 {
                write(self.precipitation[i][k]);
                write(n.seasonal_precipitation[k] as u64);
            }
        }
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn first_year_sets_normals_then_they_move_slowly() {
        let mut stats = YearStats::new(1);
        for season in 0..4 {
            stats.accumulate(&[100], &[1000], |_| 1000, 364, season, Threads(1));
        }
        stats.end_year(364);
        assert_eq!(stats.normals[0].mean, 1000);
        assert_eq!(stats.normals[0].annual_precipitation_mm(), 1000);
        assert_eq!(stats.growing_last_year[0], 5 * 364);

        // A much warmer year moves the normal a tenth of the way.
        for season in 0..4 {
            stats.accumulate(&[300], &[1000], |_| 1000, 364, season, Threads(1));
        }
        stats.end_year(364);
        assert_eq!(stats.normals[0].mean, 1200);
    }

    #[test]
    fn frost_free_season_wraps_around_the_year() {
        let mut stats = YearStats::new(1);
        // Frost only in the middle of the year (a southern winter): the
        // frost-free season runs from late in the year into the next.
        for day in 0..364 {
            let night = if (150..200).contains(&day) { -20 } else { 30 };
            stats.record_day(&[night], |_| false, 0, Threads(1));
        }
        stats.end_year(364);
        assert_eq!(stats.frost_free_last_year[0], 364 - 50);

        for _ in 0..364 {
            stats.record_day(&[30], |_| false, 0, Threads(1));
        }
        stats.end_year(364);
        assert_eq!(stats.frost_free_last_year[0], 364, "no frost at all");
    }

    #[test]
    fn seasonal_shares() {
        let normal = ClimateNormal {
            mean: 0,
            seasonal_mean: [0; 4],
            seasonal_precipitation: [300, 600, 50, 50],
        };
        assert_eq!(normal.warm_half_share(true), 900);
        assert_eq!(normal.warm_half_share(false), 100);
        assert_eq!(normal.driest_month_mm(), 1);
    }
}
