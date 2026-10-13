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

/// A tile's long-term climate.
#[derive(Debug, Clone, Copy, Default, PartialEq, Eq, Hash)]
pub struct ClimateNormal {
    /// Mean annual temperature, hundredths of a degree C.
    pub mean: i32,
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

#[derive(Debug, Clone)]
pub(crate) struct YearStats {
    /// Sum of temperature (tenths °C) × quarter-days.
    temperature: Vec<i64>,
    /// Sum of precipitation rate (mm/year) × quarter-days, per season.
    precipitation: Vec<[u64; 4]>,
    /// Growing degree-days above 5 °C, tenths °C × quarter-days.
    growing: Vec<u32>,
    quarter_days: u32,
    /// Soil wetness (per mille) × quarter-days, and quarter-days, while
    /// warm enough to grow (above 5 °C).
    wetness: Vec<u64>,
    growing_quarters: Vec<u32>,
    flood_days: Vec<u16>,
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

#[derive(Debug, Clone, Copy, Default)]
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
            temperature: vec![0; n],
            precipitation: vec![[0; 4]; n],
            growing: vec![0; n],
            quarter_days: 0,
            wetness: vec![0; n],
            growing_quarters: vec![0; n],
            flood_days: vec![0; n],
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
        wetness: impl Fn(usize) -> u32,
        quarters: u32,
        season: usize,
    ) {
        let q = i64::from(quarters);
        for (i, (&t, &p)) in temperatures.iter().zip(precipitation).enumerate() {
            self.temperature[i] += i64::from(t) * q;
            self.precipitation[i][season] += u64::from(p) * u64::from(quarters);
            if t > 50 {
                self.growing[i] += (i32::from(t) - 50) as u32 * quarters;
                // Soil wetness counts while it is warm enough to grow.
                self.wetness[i] += u64::from(wetness(i)) * u64::from(quarters);
                self.growing_quarters[i] += quarters;
            }
        }
        self.quarter_days += quarters;
    }

    /// Records one day: the night's temperatures and today's floods.
    pub(crate) fn record_day(&mut self, night: &[i16], flooding: impl Fn(usize) -> bool) {
        for (i, (run, &t)) in self.frost.iter_mut().zip(night).enumerate() {
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
            if flooding(i) {
                self.flood_days[i] += 1;
            }
        }
        self.days += 1;
    }

    /// Closes the year: updates the normals (the first year sets them).
    pub(crate) fn end_year(&mut self, days_per_year: u32) {
        let quarters = i64::from(self.quarter_days.max(1));
        let per_year = u64::from(days_per_year.max(1)) * 4;
        for i in 0..self.normals.len() {
            let mean = (self.temperature[i] * 10 / quarters) as i32;
            let precipitation = self.precipitation[i].map(|p| (p * 10 / per_year) as i32);
            let normal = &mut self.normals[i];
            if self.have_normals {
                normal.mean += (mean - normal.mean) / 10;
                for (n, p) in normal.seasonal_precipitation.iter_mut().zip(precipitation) {
                    *n += (p - *n) / 10;
                }
            } else {
                *normal = ClimateNormal {
                    mean,
                    seasonal_precipitation: precipitation,
                };
            }
            self.growing_last_year[i] = self.growing[i] / 40;
            self.wetness_last_year[i] = match self.growing_quarters[i] {
                0 => 1000,
                q => (self.wetness[i] / u64::from(q)) as u16,
            };
            self.flood_days_last_year[i] = self.flood_days[i];
            let run = self.frost[i];
            self.frost_free_last_year[i] = if run.had_frost {
                run.longest.max(run.prefix + run.current)
            } else {
                self.days as u16
            };
            self.temperature[i] = 0;
            self.precipitation[i] = [0; 4];
            self.growing[i] = 0;
            self.wetness[i] = 0;
            self.growing_quarters[i] = 0;
            self.flood_days[i] = 0;
            self.frost[i] = FrostRun::default();
        }
        self.quarter_days = 0;
        self.days = 0;
        self.have_normals = true;
    }

    pub(crate) fn digest(&self, write: &mut impl FnMut(u64)) {
        write(u64::from(self.quarter_days));
        for i in 0..self.normals.len() {
            let n = &self.normals[i];
            write(self.temperature[i] as u64);
            write(u64::from(self.growing[i]) << 32 | u64::from(self.growing_last_year[i]));
            write(n.mean as u64);
            let f = self.frost[i];
            write(
                u64::from(f.current) << 48
                    | u64::from(f.longest) << 32
                    | u64::from(f.prefix) << 16
                    | u64::from(f.had_frost),
            );
            write(self.wetness[i] ^ u64::from(self.growing_quarters[i]) << 40);
            write(
                u64::from(self.flood_days[i]) << 48
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
            stats.accumulate(&[100], &[1000], |_| 1000, 364, season);
        }
        stats.end_year(364);
        assert_eq!(stats.normals[0].mean, 1000);
        assert_eq!(stats.normals[0].annual_precipitation_mm(), 1000);
        assert_eq!(stats.growing_last_year[0], 5 * 364);

        // A much warmer year moves the normal a tenth of the way.
        for season in 0..4 {
            stats.accumulate(&[300], &[1000], |_| 1000, 364, season);
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
            stats.record_day(&[night], |_| false);
        }
        stats.end_year(364);
        assert_eq!(stats.frost_free_last_year[0], 364 - 50);

        for _ in 0..364 {
            stats.record_day(&[30], |_| false);
        }
        stats.end_year(364);
        assert_eq!(stats.frost_free_last_year[0], 364, "no frost at all");
    }

    #[test]
    fn seasonal_shares() {
        let normal = ClimateNormal {
            mean: 0,
            seasonal_precipitation: [300, 600, 50, 50],
        };
        assert_eq!(normal.warm_half_share(true), 900);
        assert_eq!(normal.warm_half_share(false), 100);
        assert_eq!(normal.driest_month_mm(), 1);
    }
}
