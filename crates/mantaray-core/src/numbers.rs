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

/// A whole, non-negative count written in `text`, exactly, or `None`.
///
/// One rule for every count a person or a job writes (count presets, raw
/// preset commands, channels). The text is read as a decimal, digit by
/// digit, never through an `f64`: `1500`, `1500.0`, `1.5e3` and `1e3` are
/// exact counts, and `4.9999999999999999` is a fraction even though an
/// `f64` would round it to 5. A fraction, a sign, NaN, infinity, or a count
/// too large to hold is `None`: refused, never truncated, rounded or
/// saturated.
pub fn whole_count(text: &str) -> Option<u64> {
    let text = text.trim();
    // A sign is refused even where it changes nothing (`+5`, `-0`).
    if text.starts_with(['+', '-']) {
        return None;
    }
    let (mantissa, exponent) = match text.split_once(['e', 'E']) {
        Some((mantissa, exponent)) => {
            // The exponent's own sign is allowed (`1500e-1`); its digits must
            // be there and be digits.
            let digits = exponent.strip_prefix(['+', '-']).unwrap_or(exponent);
            if digits.is_empty() || !digits.bytes().all(|byte| byte.is_ascii_digit()) {
                return None;
            }
            // Too long for an i64 is too large or too small to be a count,
            // unless the digits are zero (zero at any exponent is zero).
            (mantissa, exponent.parse::<i64>().ok())
        }
        None => (text, Some(0)),
    };
    let (whole, fraction) = mantissa.split_once('.').unwrap_or((mantissa, ""));
    let all_digits = |part: &str| part.bytes().all(|byte| byte.is_ascii_digit());
    if whole.len() + fraction.len() == 0 || !all_digits(whole) || !all_digits(fraction) {
        return None;
    }
    // The value is digits x 10^scale: drop the leading zeros, and turn the
    // trailing ones into scale, so only a nonzero digit past the point is a
    // fraction.
    let digits = format!("{whole}{fraction}");
    let digits = digits.trim_start_matches('0');
    let significant = digits.trim_end_matches('0');
    if significant.is_empty() {
        return Some(0);
    }
    let fraction_len = i64::try_from(fraction.len()).ok()?;
    let trailing = i64::try_from(digits.len() - significant.len()).ok()?;
    let scale = exponent?.checked_sub(fraction_len)?.checked_add(trailing)?;
    if scale < 0 {
        return None;
    }
    let mut count = significant.parse::<u64>().ok()?;
    for _ in 0..scale {
        count = count.checked_mul(10)?;
    }
    Some(count)
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
            "+5",
            "-0",
            "NaN",
            "inf",
            "1e30",
            "18446744073709551616",
            "",
            "ten",
        ] {
            assert_eq!(whole_count(refused), None, "{refused}");
        }
    }

    #[test]
    fn a_fraction_is_refused_even_where_an_f64_rounds_it_whole() {
        // 17 significant digits: as an f64 this is 5.0 exactly.
        for refused in [
            "4.9999999999999999",
            "5.0000000000000001",
            "1.23e1",
            "15e-1",
            "1e",
            "e3",
            ".",
            "1.2.3",
            "1e+",
            "0x10",
            "1_000",
        ] {
            assert_eq!(whole_count(refused), None, "{refused}");
        }
        // The decimal is read exactly, so no f64 limit applies to it.
        assert_eq!(
            whole_count("9007199254740993.0"),
            Some(9_007_199_254_740_993)
        );
        assert_eq!(whole_count("1.5e3"), Some(1500));
        assert_eq!(whole_count("1500e-1"), Some(150));
        assert_eq!(whole_count("0.0"), Some(0));
        assert_eq!(whole_count("0e-5"), Some(0));
        assert_eq!(whole_count("0e99999999999999999999"), Some(0));
        assert_eq!(whole_count("1e-99999999999999999999"), None);
        assert_eq!(whole_count("1E3"), Some(1000));
        assert_eq!(whole_count("1."), Some(1));
        assert_eq!(whole_count("1.8446744073709551615e19"), Some(u64::MAX));
        assert_eq!(whole_count("1.8446744073709551616e19"), None);
        assert_eq!(whole_count("1e99999999999999999999"), None);
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
