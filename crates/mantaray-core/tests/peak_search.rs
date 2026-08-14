//! Matched-filter peak search (Becquerel's method) and automatic ROI marking.

use mantaray_core::{
    CalculationSettings, EnergyCalibration, ShapeCalibration, Spectrum, mark_peaks, peak_search,
};

/// Deterministic synthetic spectrum: exponential continuum plus gaussians.
fn synthetic(peaks: &[(f64, f64, f64)], len: usize) -> Spectrum {
    let mut counts = vec![0f64; len];
    for (ch, c) in counts.iter_mut().enumerate() {
        *c = 200.0 * (-(ch as f64) / 400.0).exp() + 20.0;
    }
    for &(centre, sigma, area) in peaks {
        let amp = area / (sigma * (2.0 * std::f64::consts::PI).sqrt());
        for (ch, c) in counts.iter_mut().enumerate() {
            let x = (ch as f64 - centre) / sigma;
            *c += amp * (-0.5 * x * x).exp();
        }
    }
    Spectrum::from_counts(counts.into_iter().map(|c| c.round() as u64).collect())
}

#[test]
fn finds_isolated_peaks_at_the_right_channels() {
    let s = synthetic(&[(200.0, 3.0, 60_000.0), (600.0, 4.0, 80_000.0)], 1024);
    let found = peak_search(&s, &CalculationSettings::default());
    let centres: Vec<f64> = found.iter().map(|p| p.centroid).collect();
    assert_eq!(found.len(), 2, "expected two peaks, got {centres:?}");
    assert!((found[0].centroid - 200.0).abs() < 1.5, "{:?}", found[0]);
    assert!((found[1].centroid - 600.0).abs() < 1.5, "{:?}", found[1]);
    assert!(found[0].width > 0.0);
}

#[test]
fn peaks_are_returned_in_channel_order() {
    let s = synthetic(
        &[
            (150.0, 3.0, 90_000.0),
            (400.0, 3.5, 90_000.0),
            (800.0, 4.0, 90_000.0),
        ],
        1024,
    );
    let found = peak_search(&s, &CalculationSettings::default());
    assert!(found.len() >= 3);
    assert!(found.windows(2).all(|w| w[0].centroid < w[1].centroid));
}

#[test]
fn a_smooth_continuum_yields_no_peaks() {
    let s = synthetic(&[], 1024);
    let found = peak_search(&s, &CalculationSettings::default());
    assert!(found.is_empty(), "false positives: {found:?}");
}

#[test]
fn sensitivity_one_is_more_sensitive_than_five() {
    // A weak peak: ~120 net counts on a continuum of ~77 counts per channel is
    // a ~2-sigma matched-filter response, so a sensitivity of 1 catches it
    // and 5 rejects it.
    let s = synthetic(&[(500.0, 3.0, 120.0)], 1024);
    let sensitive = peak_search(&s, &CalculationSettings::default().with_sensitivity(1));
    let strict = peak_search(&s, &CalculationSettings::default().with_sensitivity(5));
    assert!(
        sensitive.len() >= strict.len(),
        "sensitivity 1 must not find fewer peaks than 5"
    );
    assert!(
        sensitive.iter().any(|p| (p.centroid - 500.0).abs() < 2.0),
        "sensitivity 1 should find the weak peak: {sensitive:?}"
    );
    assert!(
        !strict.iter().any(|p| (p.centroid - 500.0).abs() < 2.0),
        "sensitivity 5 should reject the weak peak: {strict:?}"
    );
}

#[test]
fn strong_peaks_survive_every_sensitivity() {
    let s = synthetic(&[(300.0, 3.0, 200_000.0)], 1024);
    for sensitivity in 1..=5 {
        let found = peak_search(
            &s,
            &CalculationSettings::default().with_sensitivity(sensitivity),
        );
        assert!(
            found.iter().any(|p| (p.centroid - 300.0).abs() < 1.5),
            "sensitivity {sensitivity} missed a strong peak"
        );
    }
}

#[test]
fn mark_peaks_puts_the_background_points_outside_three_fwhm() {
    let mut s = synthetic(&[(400.0, 4.0, 150_000.0)], 1024);
    s.energy_calibration = Some(EnergyCalibration::linear(0.0, 1.0));
    // Constant 9.42-channel FWHM (= 4.0 sigma) so the expected width is exact.
    s.shape_calibration = Some(ShapeCalibration::new([2.3548 * 4.0, 0.0, 0.0]));
    let settings = CalculationSettings::default();
    let n = mark_peaks(&mut s, &settings);
    assert_eq!(n, 1);
    let roi = s.rois.get(0).copied().expect("an ROI was marked");
    let width = roi.len() as f64;
    // Three FWHM is the peak's span; the background points get their own
    // room beyond it rather than being carved out of it.
    let expected = 3.0 * 2.3548 * 4.0 + 2.0 * settings.background_points as f64;
    assert!(
        (width - expected).abs() <= 2.0,
        "ROI width {width} should be 3x FWHM plus the background points ({expected})"
    );
    assert!(roi.contains(400));
    // The property those channels exist for: equation (17) reads the
    // background off them, so they must stand clear of the peak. Taken out
    // of the three-FWHM span instead, the innermost sat at three sigma - on
    // the peak's own shoulder - and the net area came back a seventh short.
    let sigma = 4.0;
    let innermost = (roi.end + 1 - settings.background_points) as f64;
    let clearance = (innermost - 400.0) / sigma;
    assert!(
        clearance >= 3.5,
        "the background points stand on the peak: innermost at {clearance:.2} sigma"
    );
}

#[test]
fn mark_peaks_falls_back_to_the_measured_width_without_a_calibration() {
    let mut s = synthetic(&[(400.0, 4.0, 150_000.0)], 1024);
    let n = mark_peaks(&mut s, &CalculationSettings::default());
    assert_eq!(n, 1);
    let roi = s.rois.get(0).copied().unwrap();
    assert!(roi.contains(400));
    assert!(roi.len() >= 3);
}

#[test]
fn mark_peaks_keeps_existing_regions() {
    // "Existing ROIs are not cleared."
    let mut s = synthetic(&[(400.0, 4.0, 150_000.0)], 1024);
    s.rois.mark(mantaray_core::Roi::new(10, 20));
    let n = mark_peaks(&mut s, &CalculationSettings::default());
    assert_eq!(n, 1);
    assert_eq!(s.rois.len(), 2);
    assert!(s.rois.at(15).is_some());
}
