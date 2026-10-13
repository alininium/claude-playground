//! Integer sine/cosine. `f64::sin` is not guaranteed to round identically on
//! every platform, so periodic signals use Bhaskara I's rational
//! approximation (max error ~0.0016) in 16.16 fixed point instead.

use crate::map::noise::ONE;

/// A full circle in phase units.
pub const FULL_TURN: i64 = 1 << 16;

/// 2π in fixed point.
pub const TAU: i64 = 411_775;

/// Sine of `phase / FULL_TURN` turns, in `[-ONE, ONE]`.
pub fn sin(phase: i64) -> i64 {
    let half = FULL_TURN / 2;
    let p = phase.rem_euclid(FULL_TURN);
    let (t, sign) = if p < half { (p, 1) } else { (p - half, -1) };
    // sin(πt/H) ≈ 16·t(H−t) / (5H² − 4·t(H−t))
    let a = t * (half - t);
    sign * 16 * a * ONE / (5 * half * half - 4 * a)
}

pub fn cos(phase: i64) -> i64 {
    sin(phase + FULL_TURN / 4)
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn sine_is_accurate() {
        for i in 0..1000 {
            let phase = i * FULL_TURN / 1000 - FULL_TURN;
            let exact = (phase as f64 / FULL_TURN as f64 * std::f64::consts::TAU).sin();
            let approx = sin(phase) as f64 / ONE as f64;
            assert!(
                (exact - approx).abs() < 0.003,
                "phase {phase}: {exact} vs {approx}"
            );
        }
        assert_eq!(sin(0), 0);
        assert_eq!(cos(0), ONE);
        assert_eq!(sin(FULL_TURN / 4), ONE);
    }
}
