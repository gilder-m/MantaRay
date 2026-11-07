//! The two number rules every layer shares: what counts as a change, and
//! what a zero looks like. They live here, once, so the analysis, the
//! instrument layer and the dialogs cannot drift apart.

/// Zero, never negative zero: `-0.0 == 0.0`, so no check or comparison sees the
/// difference, but `{:.0}` prints it as `-0`, which is what must never be sent
/// or shown.
pub fn plain_zero(x: f64) -> f64 {
    if x == 0.0 { 0.0 } else { x }
}

/// Whether a setting differs from what the settings hold. A NaN that is still
/// NaN is not a change - `NaN != NaN` would make every edit that leaves it
/// alone look like one.
pub fn differs(new: f64, old: f64) -> bool {
    !(new == old || (new.is_nan() && old.is_nan()))
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn a_negative_zero_is_a_plain_zero() {
        assert_eq!(plain_zero(-0.0).to_bits(), 0.0f64.to_bits());
        assert_eq!(plain_zero(0.0).to_bits(), 0.0f64.to_bits());
        assert_eq!(plain_zero(-3.5), -3.5);
        assert!(plain_zero(f64::NAN).is_nan());
    }

    #[test]
    fn a_nan_that_is_still_nan_is_not_a_change() {
        assert!(!differs(f64::NAN, f64::NAN));
        assert!(!differs(-0.0, 0.0));
        assert!(differs(f64::NAN, 1.0));
        assert!(differs(1.0, f64::NAN));
        assert!(differs(20.0, 20.4));
    }
}
