//! Day-to-day weather: temperature and precipitation anomalies.
//!
//! Weather systems are smooth noise fields about 1500 km across. A new
//! independent field ("weather epoch") starts every few days, and each day
//! blends linearly from one epoch's field into the next, so a warm spell or
//! a rainy week builds up and fades over several days instead of flipping
//! daily. The anomalies average out to zero over time.
//!
//! - Temperature: ±1 °C typical in the tropics, ±4-5 °C at mid and high
//!   latitudes over land (where weather is most variable), damped at sea.
//! - Precipitation: a multiplier around 1 (0.1 to ~2.2), so dry spells and
//!   wet weeks alternate while the long-term total is unchanged.

use crate::map::noise::{Noise, ONE};

/// Days per weather epoch.
const EPOCH_DAYS: u64 = 4;

#[derive(Debug, Clone, serde::Serialize, serde::Deserialize)]
pub struct Weather {
    seed: u64,
    /// Temperature anomaly, tenths °C.
    temperature: Vec<i16>,
    /// Precipitation multiplier, per mille.
    precipitation: Vec<u16>,
    /// Noise grids of the last two epochs used. Each epoch's field is
    /// blended over eight days (as the next epoch, then the current one), so
    /// it is sampled once instead of every day. Not saved: recomputed on
    /// demand, with identical values.
    #[serde(skip)]
    grids: Vec<EpochGrid>,
}

/// The (temperature, precipitation) noise of one weather epoch at the
/// coarse grid points of a map `width` wide.
#[derive(Debug, Clone)]
struct EpochGrid {
    epoch: u64,
    width: i64,
    values: Vec<(i64, i64)>,
}

impl Weather {
    pub fn new(seed: u64, tiles: usize) -> Self {
        Self {
            seed,
            temperature: vec![0; tiles],
            precipitation: vec![1000; tiles],
            grids: Vec::new(),
        }
    }

    pub fn temperature(&self, index: usize) -> i16 {
        self.temperature[index]
    }

    pub fn precipitation_factor(&self, index: usize) -> u16 {
        self.precipitation[index]
    }

    /// Computes the weather for absolute day `day`. `amplitude` gives each
    /// tile's typical temperature anomaly, tenths °C.
    ///
    /// The noise is sampled on a coarse grid (four points per finest weather
    /// feature) and interpolated bilinearly, which looks the same as sampling
    /// every tile and is much cheaper on large maps.
    pub fn update(&mut self, day: u64, width: u32, amplitude: impl Fn(usize) -> i16) {
        let field = self.day_field(day, width);
        let w = field.width as usize;
        let rows = self
            .temperature
            .chunks_mut(w)
            .zip(self.precipitation.chunks_mut(w));
        for (y, (temperature, precipitation)) in rows.enumerate() {
            let row = field.row(y);
            for (x, (t, p)) in temperature.iter_mut().zip(precipitation).enumerate() {
                (*t, *p) = field.sample(&row, x, amplitude(y * w + x));
            }
        }
    }

    /// The weather field of absolute day `day`, to sample single tiles
    /// with: [`DayField::at`] gives exactly what [`Weather::update`] would
    /// store for that tile, without touching the stored weather.
    pub fn day_field(&mut self, day: u64, width: u32) -> DayField {
        let epoch = day / EPOCH_DAYS;
        let blend = ((day % EPOCH_DAYS) as i64 * ONE) / EPOCH_DAYS as i64;
        let w = i64::from(width.max(1));
        let h = (self.temperature.len() as i64 / w).max(1);
        let step = (w / 108).max(1);

        let a = self.epoch_grid(epoch, w, h, None);
        let b = self.epoch_grid(epoch + 1, w, h, Some(a));
        let (a, b) = (&self.grids[a].values, &self.grids[b].values);
        let grid = a
            .iter()
            .zip(b)
            .map(|(&(t0, p0), &(t1, p1))| {
                (t0 + (t1 - t0) * blend / ONE, p0 + (p1 - p0) * blend / ONE)
            })
            .collect();
        // Grid column and offset toward the next column, for every x.
        let columns = (0..w)
            .map(|x| ((x / step) as usize, (x % step) * ONE / step))
            .collect();
        DayField {
            grid,
            grid_width: ((w + step - 1) / step + 1) as usize,
            width: w,
            step,
            columns,
        }
    }

    /// Index in `grids` of the noise grid of `epoch`, sampling it if needed
    /// in place of the oldest grid other than `keep`.
    fn epoch_grid(&mut self, epoch: u64, w: i64, h: i64, keep: Option<usize>) -> usize {
        if let Some(i) = self
            .grids
            .iter()
            .position(|g| g.epoch == epoch && g.width == w)
        {
            return i;
        }
        let field = |layer: u64| {
            Noise::new(splitmix(
                self.seed ^ epoch.wrapping_mul(0x9E37_79B9_7F4A_7C15) ^ layer,
            ))
        };
        let (t, p) = (field(1), field(2));
        let step = (w / 108).max(1);
        let (gw, gh) = ((w + step - 1) / step + 1, (h + step - 1) / step + 1);
        let mut values = Vec::with_capacity((gw * gh) as usize);
        for gy in 0..gh {
            for gx in 0..gw {
                // About 27 weather cells across the planet.
                let fx = (2 * gx * step + 1) * ONE * 27 / (2 * w);
                let fy = (2 * gy * step + 1) * ONE * 27 / (2 * w);
                values.push((t.fbm(fx, fy, 2), p.fbm(fx, fy, 2)));
            }
        }
        let grid = EpochGrid {
            epoch,
            width: w,
            values,
        };
        if self.grids.len() < 2 {
            self.grids.push(grid);
            self.grids.len() - 1
        } else {
            let replace = match keep {
                Some(k) => 1 - k,
                None => usize::from(self.grids[1].epoch < self.grids[0].epoch),
            };
            self.grids[replace] = grid;
            replace
        }
    }

    pub(crate) fn digest(&self, write: &mut impl FnMut(u64)) {
        for (&t, &p) in self.temperature.iter().zip(&self.precipitation) {
            write((t as u64) << 16 | u64::from(p));
        }
    }
}

/// One day's weather noise, interpolated to tiles on demand.
pub struct DayField {
    /// Blended (temperature, precipitation) noise at the grid points.
    grid: Vec<(i64, i64)>,
    grid_width: usize,
    width: i64,
    step: i64,
    columns: Vec<(usize, i64)>,
}

/// Grid row and offset toward the next row of one map row.
struct FieldRow {
    gy: usize,
    sy: i64,
}

impl DayField {
    /// The (temperature anomaly, precipitation factor) of tile `index`,
    /// whose typical anomaly is `amplitude`.
    pub fn at(&self, index: usize, amplitude: i16) -> (i16, u16) {
        let w = self.width as usize;
        self.sample(&self.row(index / w), index % w, amplitude)
    }

    fn row(&self, y: usize) -> FieldRow {
        let y = y as i64;
        FieldRow {
            gy: (y / self.step) as usize,
            sy: (y % self.step) * ONE / self.step,
        }
    }

    fn sample(&self, row: &FieldRow, x: usize, amplitude: i16) -> (i16, u16) {
        let (gx, sx) = self.columns[x];
        let sy = row.sy;
        let at = |dx: usize, dy: usize| self.grid[(row.gy + dy) * self.grid_width + gx + dx];
        let (v00, v10, v01, v11) = (at(0, 0), at(1, 0), at(0, 1), at(1, 1));
        let lerp2 = |k: fn((i64, i64)) -> i64| {
            let top = k(v00) + (k(v10) - k(v00)) * sx / ONE;
            let bottom = k(v01) + (k(v11) - k(v01)) * sx / ONE;
            top + (bottom - top) * sy / ONE
        };
        // fbm values are typically ±0.3; scale so `amplitude` is typical.
        let t = lerp2(|v| v.0) * 3 * i64::from(amplitude) / ONE;
        let p = 1000 + lerp2(|v| v.1) * 3 * 1000 / ONE * 2 / 5;
        (t.clamp(-300, 300) as i16, p.clamp(100, 2200) as u16)
    }
}

fn splitmix(mut z: u64) -> u64 {
    z = z.wrapping_add(0x9E37_79B9_7F4A_7C15);
    z = (z ^ (z >> 30)).wrapping_mul(0xBF58_476D_1CE4_E5B9);
    z = (z ^ (z >> 27)).wrapping_mul(0x94D0_49BB_1331_11EB);
    z ^ (z >> 31)
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn weather_averages_out_and_changes_gradually() {
        let width = 64;
        let n = 64 * 32;
        let mut weather = Weather::new(5, n);
        let amplitude = |_| 50i16;
        let (mut sum_t, mut sum_p, mut count) = (0i64, 0i64, 0i64);
        let mut previous: Option<i64> = None;
        let mut biggest_daily_jump = 0i64;
        for day in 0..400 {
            weather.update(day, width, amplitude);
            for i in (0..n).step_by(37) {
                sum_t += i64::from(weather.temperature(i));
                sum_p += i64::from(weather.precipitation_factor(i));
                count += 1;
            }
            let t = weather.temperature(100);
            if let Some(p) = previous {
                biggest_daily_jump = biggest_daily_jump.max((i64::from(t) - p).abs());
            }
            previous = Some(i64::from(t));
        }
        assert!((sum_t / count).abs() <= 8, "mean anomaly {}", sum_t / count);
        assert!(
            (sum_p / count - 1000).abs() <= 80,
            "mean rain factor {}",
            sum_p / count
        );
        assert!(
            biggest_daily_jump <= 60,
            "jumped {biggest_daily_jump} tenths in a day"
        );
    }
}
