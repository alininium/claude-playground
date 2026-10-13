//! Planet-wide climate variability.
//!
//! [`Oscillation`] is an ENSO-like coupled ocean-atmosphere mode, modeled as a
//! *recharge oscillator* (Jin 1997): the equatorial surface temperature
//! anomaly `T` and the ocean heat content `h` feed each other with a quarter
//! cycle delay, giving irregular warm (El Niño-like) and cold (La Niña-like)
//! episodes every few years. Damping keeps it bounded; random daily
//! "westerly wind bursts" perturb the heat content, which keeps the cycle
//! going and makes every episode different while the surface index itself
//! evolves smoothly.
//!
//! [`Drift`] is a much slower, mean-reverting random walk of the global mean
//! temperature (centuries), standing in for solar and orbital variability.
//!
//! Both are pure integer math, and both draw from their own RNG stream so
//! they never disturb other subsystems' random sequences.

use crate::map::noise::ONE;
use crate::rng::SimRng;

use super::trig::TAU;

#[derive(Debug, Clone, PartialEq, Eq)]
pub struct OscillationConfig {
    /// Mean length of a full warm-cold cycle, in days.
    pub period_days: u32,
    /// E-folding time of the damping, in days.
    pub damping_days: u32,
    /// Typical (standard deviation) index, in tenths of a degree.
    pub amplitude_tenths: u32,
    /// Index beyond which a warm or cold phase is declared, tenths of a degree.
    pub phase_threshold_tenths: u32,
    /// Time scale of the global drift, in days.
    pub drift_days: u32,
    /// Typical (standard deviation) global drift, in tenths of a degree.
    pub drift_amplitude_tenths: u32,
}

impl Default for OscillationConfig {
    fn default() -> Self {
        Self {
            period_days: 4 * 364,
            damping_days: 3 * 364,
            amplitude_tenths: 10,
            phase_threshold_tenths: 5,
            drift_days: 150 * 364,
            drift_amplitude_tenths: 5,
        }
    }
}

#[derive(Debug, Clone, Copy, PartialEq, Eq, Hash)]
pub enum OscillationPhase {
    /// El Niño-like: warm eastern tropical ocean.
    Warm,
    Neutral,
    /// La Niña-like: cold eastern tropical ocean.
    Cold,
}

#[derive(Debug, Clone, PartialEq, Eq)]
pub struct Oscillation {
    /// Surface temperature anomaly of the oscillation's core, in °C · ONE.
    index: i64,
    /// Ocean heat content anomaly (recharge), same units.
    heat: i64,
    rng: SimRng,
}

impl Oscillation {
    pub fn new(rng: SimRng) -> Self {
        Self {
            index: 0,
            heat: 0,
            rng,
        }
    }

    /// Core temperature anomaly in tenths of a degree.
    pub fn index_tenths(&self) -> i64 {
        self.index * 10 / ONE
    }

    pub fn phase(&self, config: &OscillationConfig) -> OscillationPhase {
        let threshold = i64::from(config.phase_threshold_tenths);
        match self.index_tenths() {
            i if i >= threshold => OscillationPhase::Warm,
            i if i <= -threshold => OscillationPhase::Cold,
            _ => OscillationPhase::Neutral,
        }
    }

    /// Advances one day (semi-implicit Euler, which keeps the rotation stable).
    pub fn step_day(&mut self, config: &OscillationConfig) {
        let omega = TAU / i64::from(config.period_days.max(1));
        let lambda = ONE / i64::from(config.damping_days.max(1));
        // Stationary sd of T is σ / (2√λ) for noise σ on one component.
        let sd = i64::from(config.amplitude_tenths) * ONE / 10;
        let sigma = sd * 2 * (lambda * ONE).isqrt() / ONE;
        let kick = uniform_unit_variance(&mut self.rng, sigma);

        self.heat += (-omega * self.index - lambda * self.heat) / ONE + kick;
        self.index += (-lambda * self.index + omega * self.heat) / ONE;
    }

    pub(crate) fn state(&self) -> (i64, i64) {
        (self.index, self.heat)
    }
}

#[derive(Debug, Clone, PartialEq, Eq)]
pub struct Drift {
    /// Global temperature anomaly, in °C · ONE.
    anomaly: i64,
    rng: SimRng,
}

impl Drift {
    pub fn new(rng: SimRng) -> Self {
        Self { anomaly: 0, rng }
    }

    pub fn anomaly_tenths(&self) -> i64 {
        self.anomaly * 10 / ONE
    }

    pub(crate) fn anomaly(&self) -> i64 {
        self.anomaly
    }

    /// Ornstein-Uhlenbeck step: `x += -x/τ + σ·ξ`, stationary sd = σ·√(τ/2).
    pub fn step_day(&mut self, config: &OscillationConfig) {
        let tau = i64::from(config.drift_days.max(1));
        let sd = i64::from(config.drift_amplitude_tenths) * ONE / 10;
        let sigma = sd * (2 * ONE * ONE / tau).isqrt() / ONE;
        let kick = uniform_unit_variance(&mut self.rng, sigma);
        self.anomaly += -self.anomaly / tau + kick;
    }
}

/// Uniform noise with standard deviation `sigma` (range ±σ√3).
fn uniform_unit_variance(rng: &mut SimRng, sigma: i64) -> i64 {
    let half_width = sigma * 113_512 / ONE; // √3 · ONE = 113_512
    if half_width <= 0 {
        return 0;
    }
    rng.below(2 * half_width as u64 + 1) as i64 - half_width
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn index_changes_smoothly() {
        let series = sample(2, 364 * 50);
        let max_step = series
            .windows(2)
            .map(|w| (w[1] - w[0]).abs())
            .max()
            .unwrap();
        assert!(max_step <= 1, "index jumped {max_step} tenths in a day");
    }

    fn sample(seed: u64, days: usize) -> Vec<i64> {
        let config = OscillationConfig::default();
        let mut osc = Oscillation::new(SimRng::from_seed(seed));
        (0..days)
            .map(|_| {
                osc.step_day(&config);
                osc.index_tenths()
            })
            .collect()
    }

    #[test]
    fn oscillates_with_realistic_amplitude() {
        let config = OscillationConfig::default();
        let series = sample(1, 364 * 200);
        let n = series.len() as i64;
        let mean = series.iter().sum::<i64>() / n;
        let var = series.iter().map(|v| (v - mean) * (v - mean)).sum::<i64>() / n;
        let sd = var.isqrt();
        assert!(mean.abs() <= 3, "mean {mean}");
        let target = i64::from(config.amplitude_tenths);
        assert!(sd >= target / 2 && sd <= target * 2, "sd {sd} tenths");

        // Count warm episodes: reaching the threshold after having been
        // below zero since the previous one.
        let threshold = i64::from(config.phase_threshold_tenths);
        let mut warm_onsets = 0;
        let mut armed = true;
        for &v in &series {
            if armed && v >= threshold {
                warm_onsets += 1;
                armed = false;
            } else if v < 0 {
                armed = true;
            }
        }
        // Roughly one every 2-8 years over 200 years.
        assert!(
            (25..=100).contains(&warm_onsets),
            "{warm_onsets} warm episodes"
        );
    }

    #[test]
    fn drift_is_slow_and_bounded() {
        let config = OscillationConfig::default();
        let mut drift = Drift::new(SimRng::from_seed(3));
        let mut max = 0;
        let mut biggest_daily_change = 0;
        let mut prev = 0;
        for _ in 0..364 * 1000 {
            drift.step_day(&config);
            max = max.max(drift.anomaly_tenths().abs());
            biggest_daily_change = biggest_daily_change.max((drift.anomaly() - prev).abs());
            prev = drift.anomaly();
        }
        assert!(max > 0 && max <= 40, "max drift {max} tenths");
        assert!(biggest_daily_change < ONE / 100, "drift moved too fast");
    }
}
