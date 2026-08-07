//! Spectrum model behaviour.

use mantaray_core::{AcquisitionMode, EnergyCalibration, Roi, Spectrum};

#[test]
fn new_spectrum_is_zeroed_and_sized() {
    let s = Spectrum::new(2048);
    assert_eq!(s.len(), 2048);
    assert_eq!(s.total_counts(), 0);
    assert_eq!(s.live_time, 0.0);
    assert_eq!(s.real_time, 0.0);
    assert!(s.energy_calibration.is_none());
    assert_eq!(s.mode, AcquisitionMode::Pha);
    assert!(s.rois.is_empty());
}

#[test]
fn from_counts_keeps_data() {
    let s = Spectrum::from_counts(vec![1, 2, 3, 4]);
    assert_eq!(s.len(), 4);
    assert_eq!(s.total_counts(), 10);
    assert_eq!(s.counts(2), 3);
    assert_eq!(s.counts(99), 0, "out of range reads as zero");
}

#[test]
fn sum_range_is_inclusive_and_clamped() {
    let s = Spectrum::from_counts(vec![1, 2, 3, 4, 5]);
    assert_eq!(s.sum_range(1, 3), 9);
    assert_eq!(s.sum_range(3, 1), 9, "reversed ranges are normalised");
    assert_eq!(s.sum_range(0, 99), 15, "clamped to the spectrum length");
}

#[test]
fn max_count_and_channel() {
    let s = Spectrum::from_counts(vec![1, 7, 3]);
    assert_eq!(s.max_count(), 7);
    assert_eq!(s.max_channel(), Some(1));
    assert_eq!(Spectrum::new(4).max_channel(), Some(0));
}

#[test]
fn clear_erases_data_and_times_but_keeps_presets_and_calibration() {
    let mut s = Spectrum::from_counts(vec![5, 5, 5]);
    s.live_time = 100.0;
    s.real_time = 120.0;
    s.energy_calibration = Some(EnergyCalibration::linear(0.0, 0.5));
    s.rois.mark(Roi::new(0, 2));
    s.clear();
    assert_eq!(s.total_counts(), 0);
    assert_eq!(s.live_time, 0.0);
    assert_eq!(s.real_time, 0.0);
    assert!(s.start_time.is_none());
    assert!(
        s.energy_calibration.is_some(),
        "Acquire/Clear does not destroy the calibration"
    );
    assert_eq!(s.rois.len(), 1, "Acquire/Clear does not erase ROI marks");
}

#[test]
fn dead_time_and_rates() {
    let mut s = Spectrum::from_counts(vec![100; 10]);
    s.live_time = 90.0;
    s.real_time = 100.0;
    assert!((s.dead_time_percent().unwrap() - 10.0).abs() < 1e-9);
    // 1000 counts / 90 s live
    assert!((s.count_rate().unwrap() - 1000.0 / 90.0).abs() < 1e-9);

    let idle = Spectrum::new(8);
    assert!(idle.dead_time_percent().is_none());
    assert!(idle.count_rate().is_none());
}

#[test]
fn energy_lookup_requires_calibration() {
    let mut s = Spectrum::new(1024);
    assert!(s.energy_of(100.0).is_none());
    s.energy_calibration = Some(EnergyCalibration::linear(1.0, 0.5));
    assert_eq!(s.energy_of(100.0), Some(51.0));
    assert_eq!(s.units(), "keV");
}

#[test]
fn resize_preserves_leading_channels() {
    let mut s = Spectrum::from_counts(vec![1, 2, 3, 4]);
    s.resize(2);
    assert_eq!(s.channels, vec![1, 2]);
    s.resize(4);
    assert_eq!(s.channels, vec![1, 2, 0, 0]);
}

#[test]
fn add_counts_accumulates() {
    let mut s = Spectrum::new(4);
    s.add_count(2);
    s.add_count(2);
    s.add_counts(3, 5);
    assert_eq!(s.counts(2), 2);
    assert_eq!(s.counts(3), 5);
    assert_eq!(s.total_counts(), 7);
}
