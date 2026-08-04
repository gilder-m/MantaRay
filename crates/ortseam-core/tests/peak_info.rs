//! Peak Info calculation, MAESTRO §4.3.5.1, equations (17)-(21).
//!
//! Reference case used throughout: an 11-channel ROI (l = 0, h = 10) with a flat
//! background of 10 counts and a symmetric triangle peak, with n = 3 background
//! points:
//!
//! ```text
//! channel: 0   1   2   3    4    5    6   7   8   9  10
//! counts:  10  10  10  10  100  200  100  10  10  10  10
//! ```
//!
//! Hand-worked expectations:
//!   B     = (30 + 30) * (10 - 0 + 1) / (2*3)   = 110
//!   A_g   = 480
//!   A_ag  = sum(ch 3..=7)                      = 420
//!   A_n   = 420 - 110 * 5 / 11                 = 370
//!   sigma = sqrt(420 + 110 * (5/6) * (5/11))   = sqrt(461.6667)
//!
//! The width of the adjusted region is the number of channels equation (19)
//! actually sums, `(h - l + 1) - 2n` = 5, not the `h - l - (n - 1)` = 8 printed
//! in the manual: the peak here holds exactly 370 counts above background
//! (90 + 190 + 90), and only the former reproduces that. The same substitution
//! makes a flat region report a net area of zero.

use ortseam_core::{
    AnalysisError, CalculationSettings, EnergyCalibration, PeakInfo, Roi, Spectrum, peak_info,
};

fn reference_spectrum() -> Spectrum {
    let mut s = Spectrum::from_counts(vec![10, 10, 10, 10, 100, 200, 100, 10, 10, 10, 10]);
    s.live_time = 100.0;
    s.real_time = 100.0;
    s
}

fn reference_info() -> PeakInfo {
    let s = reference_spectrum();
    peak_info(&s, Roi::new(0, 10), &CalculationSettings::default()).unwrap()
}

#[test]
fn background_follows_equation_17() {
    let info = reference_info();
    assert!(
        (info.background - 110.0).abs() < 1e-9,
        "got {}",
        info.background
    );
}

#[test]
fn gross_area_follows_equation_18() {
    let info = reference_info();
    assert!((info.gross_area - 480.0).abs() < 1e-9);
}

#[test]
fn adjusted_gross_area_follows_equation_19() {
    let info = reference_info();
    assert!((info.adjusted_gross_area - 420.0).abs() < 1e-9);
}

#[test]
fn net_area_follows_equation_20() {
    let info = reference_info();
    // 90 + 190 + 90 counts sit above the flat background.
    assert!(
        (info.net_area - 370.0).abs() < 1e-9,
        "got {}",
        info.net_area
    );
}

#[test]
fn net_area_uncertainty_follows_equation_21() {
    let info = reference_info();
    let variance: f64 = 420.0 + 110.0 * (5.0 / 6.0) * (5.0 / 11.0);
    let expected = variance.sqrt();
    assert!(
        (info.net_area_uncertainty - expected).abs() < 1e-9,
        "got {} expected {expected}",
        info.net_area_uncertainty
    );
}

#[test]
fn centroid_of_a_symmetric_peak_is_its_centre() {
    let info = reference_info();
    assert!((info.centroid - 5.0).abs() < 1e-6, "got {}", info.centroid);
}

#[test]
fn widths_are_interpolated_between_background_subtracted_channels() {
    let info = reference_info();
    // Net channel contents are 90, 190, 90 at channels 4, 5, 6.
    // Half maximum = 95 -> crossings at 4.05 and 5.95.
    assert!((info.fwhm - 1.9).abs() < 1e-6, "fwhm {}", info.fwhm);
    // Default x for FW(1/x)M is 5 -> level 190/5 = 38 -> crossings at
    // 3 + 38/90 and 6 + 52/90 by linear interpolation from the net-zero shoulders.
    let expected = (6.0 + 52.0 / 90.0) - (3.0 + 38.0 / 90.0);
    assert!(
        (info.fw_x_m - expected).abs() < 1e-6,
        "fw(1/5)m {} expected {expected}",
        info.fw_x_m
    );
    assert_eq!(info.fw_x, 5);
}

#[test]
fn background_point_count_changes_the_result() {
    let s = reference_spectrum();
    let settings = CalculationSettings::default().with_background_points(1);
    let info = peak_info(&s, Roi::new(0, 10), &settings).unwrap();
    // n = 1: B = (10 + 10) * 11 / 2 = 110, A_ag = sum(ch 1..=9) = 460,
    // A_n = 460 - 110 * 9/11 = 370 - the same net area as with n = 3, which is
    // what a consistent background model must give.
    assert!((info.background - 110.0).abs() < 1e-9);
    assert!((info.adjusted_gross_area - 460.0).abs() < 1e-9);
    assert!(
        (info.net_area - 370.0).abs() < 1e-9,
        "got {}",
        info.net_area
    );
}

#[test]
fn peak_info_reports_calibrated_values_when_available() {
    let mut s = reference_spectrum();
    s.energy_calibration = Some(EnergyCalibration::linear(0.0, 2.0));
    let info = peak_info(&s, Roi::new(0, 10), &CalculationSettings::default()).unwrap();
    let cal = s.energy_calibration.as_ref().unwrap();
    assert!((info.centroid_energy(cal) - 10.0).abs() < 1e-6);
    assert!((info.fwhm_energy(cal) - 3.8).abs() < 1e-6);
    assert!((info.fw_x_m_energy(cal) - 2.0 * info.fw_x_m).abs() < 1e-6);
}

#[test]
fn count_rates_use_the_live_time() {
    let info = reference_info();
    // live time is 100 s
    assert!((info.gross_count_rate(100.0) - 4.8).abs() < 1e-9);
    assert!((info.net_count_rate(100.0) - 3.7).abs() < 1e-9);
}

#[test]
fn a_flat_region_yields_no_gaussian_fit_but_still_reports_areas() {
    // "If the marker is in an ROI, peak information is displayed whether or not
    // the area is a detectable peak."
    let s = Spectrum::from_counts(vec![10; 21]);
    let info = peak_info(&s, Roi::new(0, 20), &CalculationSettings::default()).unwrap();
    assert!((info.gross_area - 210.0).abs() < 1e-9);
    assert!(info.net_area.abs() < 1e-6);
    assert!(info.fit.is_none(), "no gaussian fit on a flat region");
    assert_eq!(info.fwhm, 0.0, "no measurable width without a peak");
}

#[test]
fn gaussian_fit_recovers_a_known_peak() {
    // Gaussian with centroid 60.0 and sigma 3.0 on a flat background.
    let mut counts = vec![0u64; 128];
    for (ch, c) in counts.iter_mut().enumerate() {
        let x = (ch as f64 - 60.0) / 3.0;
        *c = (50.0 + 10_000.0 * (-0.5 * x * x).exp()).round() as u64;
    }
    let s = Spectrum::from_counts(counts);
    let info = peak_info(&s, Roi::new(45, 75), &CalculationSettings::default()).unwrap();
    let fit = info.fit.expect("a clean gaussian must fit");
    assert!(
        (fit.centroid - 60.0).abs() < 0.05,
        "centroid {}",
        fit.centroid
    );
    assert!((fit.sigma - 3.0).abs() < 0.1, "sigma {}", fit.sigma);
    // FWHM = 2.3548 * sigma
    assert!(
        (info.fwhm - 2.3548 * 3.0).abs() < 0.15,
        "fwhm {}",
        info.fwhm
    );
    // Net area of a gaussian: amplitude * sigma * sqrt(2*pi)
    let expected_area = 10_000.0 * 3.0 * (2.0 * std::f64::consts::PI).sqrt();
    assert!(
        (info.net_area - expected_area).abs() / expected_area < 0.02,
        "net {} expected {expected_area}",
        info.net_area
    );
}

#[test]
fn roi_must_be_wide_enough_for_the_background_points() {
    let s = reference_spectrum();
    let err = peak_info(&s, Roi::new(4, 6), &CalculationSettings::default()).unwrap_err();
    assert!(matches!(err, AnalysisError::RoiTooNarrow { .. }));
}

#[test]
fn roi_must_lie_inside_the_spectrum() {
    let s = reference_spectrum();
    let err = peak_info(&s, Roi::new(5, 500), &CalculationSettings::default()).unwrap_err();
    assert!(matches!(err, AnalysisError::RoiOutOfRange { .. }));
}
