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
}

impl Weather {
    pub fn new(seed: u64, tiles: usize) -> Self {
        Self {
            seed,
            temperature: vec![0; tiles],
            precipitation: vec![1000; tiles],
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
        let epoch = day / EPOCH_DAYS;
        let blend = ((day % EPOCH_DAYS) as i64 * ONE) / EPOCH_DAYS as i64;
        let field = |epoch: u64, layer: u64| {
            Noise::new(splitmix(
                self.seed ^ epoch.wrapping_mul(0x9E37_79B9_7F4A_7C15) ^ layer,
            ))
        };
        let (t0, t1) = (field(epoch, 1), field(epoch + 1, 1));
        let (p0, p1) = (field(epoch, 2), field(epoch + 1, 2));
        let w = i64::from(width.max(1));
        let h = (self.temperature.len() as i64 / w).max(1);
        let step = (w / 108).max(1);
        let (gw, gh) = ((w + step - 1) / step + 1, (h + step - 1) / step + 1);

        // Noise values (temperature, precipitation) at the grid points.
        let mut grid = Vec::with_capacity((gw * gh) as usize);
        for gy in 0..gh {
            for gx in 0..gw {
                // About 27 weather cells across the planet.
                let fx = (2 * gx * step + 1) * ONE * 27 / (2 * w);
                let fy = (2 * gy * step + 1) * ONE * 27 / (2 * w);
                let sample = |a: &Noise, b: &Noise| {
                    let (va, vb) = (a.fbm(fx, fy, 2), b.fbm(fx, fy, 2));
                    va + (vb - va) * blend / ONE
                };
                grid.push((sample(&t0, &t1), sample(&p0, &p1)));
            }
        }

        for i in 0..self.temperature.len() {
            let (x, y) = (i as i64 % w, i as i64 / w);
            let (gx, gy) = ((x / step) as usize, (y / step) as usize);
            let (sx, sy) = ((x % step) * ONE / step, (y % step) * ONE / step);
            let at = |dx: usize, dy: usize| grid[(gy + dy) * gw as usize + gx + dx];
            let lerp2 = |k: fn((i64, i64)) -> i64| {
                let top = k(at(0, 0)) + (k(at(1, 0)) - k(at(0, 0))) * sx / ONE;
                let bottom = k(at(0, 1)) + (k(at(1, 1)) - k(at(0, 1))) * sx / ONE;
                top + (bottom - top) * sy / ONE
            };
            // fbm values are typically ±0.3; scale so `amplitude` is typical.
            let t = lerp2(|v| v.0) * 3 * i64::from(amplitude(i)) / ONE;
            self.temperature[i] = t.clamp(-300, 300) as i16;
            let p = 1000 + lerp2(|v| v.1) * 3 * 1000 / ONE * 2 / 5;
            self.precipitation[i] = p.clamp(100, 2200) as u16;
        }
    }

    pub(crate) fn digest(&self, write: &mut impl FnMut(u64)) {
        for (&t, &p) in self.temperature.iter().zip(&self.precipitation) {
            write((t as u64) << 16 | u64::from(p));
        }
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
