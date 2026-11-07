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

/// Above this every whole number is not exactly an `f64`: 2^53 + 1 reads as
/// 2^53. A whole count written as a decimal or with an exponent is believed
/// only below it.
const EXACT_F64: f64 = 9_007_199_254_740_992.0;

/// A whole, non-negative count written in `text`, exactly, or `None`.
///
/// One rule for every count a person or a job writes (count presets, raw
/// preset commands, channels): plain digits are read as an integer, so any
/// count that fits is exact; `1500.0` or `1e3` is believed only where an
/// `f64` holds every whole number. A fraction, a sign, NaN, infinity, or a
/// count too large to hold is `None`: refused, never truncated, rounded or
/// saturated.
pub fn whole_count(text: &str) -> Option<u64> {
    let text = text.trim();
    if let Ok(count) = text.parse::<u64>() {
        return Some(count);
    }
    let value = text.parse::<f64>().ok()?;
    if !(0.0..EXACT_F64).contains(&value) || value.fract() != 0.0 {
        return None;
    }
    // In range and whole, so the cast is exact.
    #[allow(clippy::cast_possible_truncation, clippy::cast_sign_loss)]
    Some(value as u64)
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn a_whole_count_is_exact_or_refused() {
        assert_eq!(whole_count("1500"), Some(1500));
        assert_eq!(whole_count("1500.0"), Some(1500));
        assert_eq!(whole_count("1e3"), Some(1000));
        assert_eq!(whole_count("0"), Some(0));
        assert_eq!(whole_count("18446744073709551615"), Some(u64::MAX));
        assert_eq!(whole_count("9007199254740993"), Some(9_007_199_254_740_993));
        for refused in [
            "1500.7",
            "-1",
            "-0.5",
            "NaN",
            "inf",
            "1e30",
            "18446744073709551616",
            "9007199254740993.0",
            "",
            "ten",
        ] {
            assert_eq!(whole_count(refused), None, "{refused}");
        }
    }

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
