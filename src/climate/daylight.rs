//! Day length from latitude and date, in integer math.
//!
//! - Solar declination: δ = 23.44° · sin(2π · day / year), with day 0 the
//!   northern vernal equinox (a common first-order approximation; the real
//!   orbit's eccentricity is ignored).
//! - Sunrise equation: cos H₀ = −tan φ · tan δ, day length = 2·H₀ (in hours,
//!   H₀/π · 24). |cos H₀| ≥ 1 means polar night or midnight sun.
//! - arccos uses Abramowitz & Stegun 4.4.45 (error < 7·10⁻⁵ rad).

use super::trig::{self, FULL_TURN};
use crate::map::noise::ONE;

/// Axial tilt in tenths of a degree.
const TILT: i64 = 234;
/// π in fixed point.
const PI: i64 = 205_887;

/// Minutes of daylight (0..=1440) at a latitude (tenths of a degree) on a
/// day of the year.
pub fn day_length_minutes(latitude_tenths: i64, day_of_year: u32, days_per_year: u32) -> u32 {
    let year = i64::from(days_per_year.max(1));
    let declination = TILT * trig::sin(i64::from(day_of_year) * FULL_TURN / year) / ONE; // tenths of a degree
    // Degrees (tenths) to phase units: 3600 tenths per full turn.
    let to_phase = |tenths: i64| tenths * FULL_TURN / 3600;
    let (sin_phi, cos_phi) = (
        trig::sin(to_phase(latitude_tenths)),
        trig::cos(to_phase(latitude_tenths)),
    );
    let (sin_d, cos_d) = (
        trig::sin(to_phase(declination)),
        trig::cos(to_phase(declination)),
    );
    let denominator = cos_phi * cos_d / ONE;
    if denominator <= 0 {
        // At a pole: half a year of day, half of night.
        return if sin_phi * sin_d > 0 { 1440 } else { 0 };
    }
    let cos_h0 = -(sin_phi * sin_d / ONE) * ONE / denominator;
    if cos_h0 >= ONE {
        return 0;
    }
    if cos_h0 <= -ONE {
        return 1440;
    }
    let h0 = arccos(cos_h0);
    (h0 * 1440 / PI) as u32
}

/// arccos in fixed point (radians · ONE), for x in [−ONE, ONE].
fn arccos(x: i64) -> i64 {
    let negative = x < 0;
    let x = x.abs().min(ONE);
    // √(1−x) · (a0 + a1·x + a2·x² + a3·x³)
    let poly = 102_939 + x * (-13_901 + x * (4_867 - 1_227 * x / ONE) / ONE) / ONE;
    let root = ((ONE - x) * ONE).isqrt();
    let r = root * poly / ONE;
    if negative { PI - r } else { r }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn equinox_is_twelve_hours_everywhere() {
        for lat in [-800, -450, 0, 300, 600, 850] {
            let l = day_length_minutes(lat, 0, 364);
            assert!((715..=725).contains(&l), "lat {lat}: {l}");
        }
    }

    #[test]
    fn solstices_and_polar_day() {
        let summer = 91; // northern summer solstice
        let winter = 273;
        // ~60° N: about 18.5 h in June, 5.5 h in December.
        let june = day_length_minutes(600, summer, 364);
        let december = day_length_minutes(600, winter, 364);
        assert!((1080..=1140).contains(&june), "{june}");
        assert!((300..=360).contains(&december), "{december}");
        // Southern hemisphere mirrors it.
        assert_eq!(day_length_minutes(-600, winter, 364), june);
        // Inside the Arctic circle: midnight sun and polar night.
        assert_eq!(day_length_minutes(800, summer, 364), 1440);
        assert_eq!(day_length_minutes(800, winter, 364), 0);
        // The equator barely changes.
        let eq = day_length_minutes(0, summer, 364);
        assert!((715..=725).contains(&eq), "{eq}");
    }

    #[test]
    fn arccos_is_accurate() {
        for i in -100..=100 {
            let x = i as f64 / 100.0;
            let exact = x.acos();
            let approx = arccos((x * ONE as f64) as i64) as f64 / ONE as f64;
            assert!((exact - approx).abs() < 0.001, "{x}: {exact} vs {approx}");
        }
    }
}
