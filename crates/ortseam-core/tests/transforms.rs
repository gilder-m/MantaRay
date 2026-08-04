//! Smooth (eq. 23), Strip, Sum, counting activity (eq. 22) and MDA.

use ortseam_core::{
    AnalysisError, Spectrum, StripFactor, counting_activity, mda_currie, smooth,
    statistical_uncertainty, strip, sum_channels,
};

#[test]
fn smooth_applies_the_five_point_binomial_kernel() {
    // Single spike of 16 counts spreads as 1,4,6,4,1 / 16.
    let mut s = Spectrum::from_counts(vec![0, 0, 0, 0, 16, 0, 0, 0, 0]);
    smooth(&mut s);
    assert_eq!(s.channels, vec![0, 0, 1, 4, 6, 4, 1, 0, 0]);
}

#[test]
fn smooth_preserves_area_on_a_flat_region() {
    let mut s = Spectrum::from_counts(vec![100; 32]);
    let before = s.total_counts();
    smooth(&mut s);
    // Interior channels are unchanged; only the two edge channels lose counts,
    // and the kernel is area-preserving elsewhere.
    assert_eq!(s.channels[5..27], vec![100u64; 22]);
    assert!(s.total_counts() <= before);
    assert!(s.total_counts() >= before - 4 * 100);
}

#[test]
fn smooth_keeps_times_and_calibration() {
    let mut s = Spectrum::from_counts(vec![10, 20, 30, 40, 50]);
    s.live_time = 42.0;
    s.real_time = 50.0;
    smooth(&mut s);
    assert_eq!(s.live_time, 42.0);
    assert_eq!(s.real_time, 50.0);
}

#[test]
fn strip_with_a_fixed_factor_subtracts_channel_by_channel() {
    let mut buffer = Spectrum::from_counts(vec![100, 200, 300]);
    let disk = Spectrum::from_counts(vec![10, 20, 30]);
    strip(&mut buffer, &disk, StripFactor::Fixed(2.0)).unwrap();
    assert_eq!(buffer.channels, vec![80, 160, 240]);
}

#[test]
fn strip_clamps_at_zero() {
    let mut buffer = Spectrum::from_counts(vec![10, 10]);
    let disk = Spectrum::from_counts(vec![100, 0]);
    strip(&mut buffer, &disk, StripFactor::Fixed(1.0)).unwrap();
    assert_eq!(buffer.channels, vec![0, 10]);
}

#[test]
fn a_negative_factor_adds_the_spectra() {
    // "Unmarking the Use Ratio box allows you to enter a factor, which can be
    // negative, in which case the spectra are added."
    let mut buffer = Spectrum::from_counts(vec![10, 10]);
    let disk = Spectrum::from_counts(vec![5, 5]);
    strip(&mut buffer, &disk, StripFactor::Fixed(-1.0)).unwrap();
    assert_eq!(buffer.channels, vec![15, 15]);
}

#[test]
fn strip_by_live_time_ratio() {
    let mut buffer = Spectrum::from_counts(vec![1000, 1000]);
    buffer.live_time = 200.0;
    let mut disk = Spectrum::from_counts(vec![100, 100]);
    disk.live_time = 100.0;
    strip(&mut buffer, &disk, StripFactor::LiveTimeRatio).unwrap();
    // factor = 200/100 = 2
    assert_eq!(buffer.channels, vec![800, 800]);
}

#[test]
fn strip_does_not_change_the_times() {
    let mut buffer = Spectrum::from_counts(vec![1000]);
    buffer.live_time = 200.0;
    buffer.real_time = 210.0;
    let mut disk = Spectrum::from_counts(vec![100]);
    disk.live_time = 100.0;
    strip(&mut buffer, &disk, StripFactor::LiveTimeRatio).unwrap();
    assert_eq!(buffer.live_time, 200.0);
    assert_eq!(buffer.real_time, 210.0);
}

#[test]
fn strip_requires_matching_lengths() {
    let mut buffer = Spectrum::from_counts(vec![1, 2, 3]);
    let disk = Spectrum::from_counts(vec![1, 2]);
    let err = strip(&mut buffer, &disk, StripFactor::Fixed(1.0)).unwrap_err();
    assert!(matches!(err, AnalysisError::LengthMismatch { .. }));
}

#[test]
fn strip_by_ratio_needs_a_live_time() {
    let mut buffer = Spectrum::from_counts(vec![1]);
    let disk = Spectrum::from_counts(vec![1]);
    let err = strip(&mut buffer, &disk, StripFactor::LiveTimeRatio).unwrap_err();
    assert!(matches!(err, AnalysisError::NoLiveTime));
}

#[test]
fn sum_counts_channels_inclusively() {
    let s = Spectrum::from_counts(vec![1, 2, 3, 4, 5]);
    assert_eq!(sum_channels(&s, 1, 3), 9);
    assert_eq!(sum_channels(&s, 0, 4), 15);
}

#[test]
fn counting_activity_follows_equation_22() {
    // cA = (100 / percent) * (net / live)
    let ca = counting_activity(7_589_080.0, 99.85, 22_500.0);
    let expected = (100.0 / 99.85) * (7_589_080.0 / 22_500.0);
    assert!((ca - expected).abs() < 1e-9);
    assert_eq!(counting_activity(100.0, 0.0, 10.0), 0.0);
    assert_eq!(counting_activity(100.0, 50.0, 0.0), 0.0);
}

#[test]
fn statistical_uncertainty_is_the_relative_error_of_the_net_area() {
    // Region with 10 000 gross counts over a flat background: the preset
    // compares this percentage against the requested limit.
    let mut s = Spectrum::from_counts(vec![100; 100]);
    s.live_time = 10.0;
    let pct = statistical_uncertainty(&s, 10, 89).unwrap();
    let gross: f64 = 80.0 * 100.0;
    assert!(
        (pct - 100.0 / gross.sqrt()).abs() < 1e-9,
        "got {pct}, expected {}",
        100.0 / gross.sqrt()
    );
    let empty = Spectrum::new(100);
    assert!(statistical_uncertainty(&empty, 10, 89).is_none());
}

#[test]
fn mda_uses_the_currie_expression() {
    // MDA = (2.71 + 4.65*sqrt(B)) / (eff * yield * live)
    let mda = mda_currie(100.0, 0.1, 0.85, 1000.0);
    let expected = (2.71 + 4.65 * 10.0) / (0.1 * 0.85 * 1000.0);
    assert!((mda - expected).abs() < 1e-12);
    assert!(mda_currie(100.0, 0.0, 1.0, 1000.0).is_infinite());
}
