//! Calibrating the way a person does it in the Calibrate dialog: put the
//! marker somewhere, type an energy, press Enter, repeat.

use mantaray_core::Spectrum;
use mantaray_gui::app::{Action, App};

/// A spectrum with a peak at `centre`, and a flat background.
fn with_peak(channels: usize, centre: usize) -> App {
    let mut spectrum = Spectrum::new(channels);
    spectrum.live_time = 100.0;
    spectrum.real_time = 105.0;
    for channel in 0..channels {
        spectrum.channels[channel] = 30;
    }
    for offset in -10i64..=10 {
        let value = (5000.0 * (-0.5 * (offset as f64 / 3.5).powi(2)).exp()) as u64;
        spectrum.channels[(centre as i64 + offset) as usize] += value;
    }
    let mut app = App::headless();
    app.open_buffer("Test".into(), spectrum, None);
    app
}

fn calibrated(app: &App) -> bool {
    app.active_spectrum()
        .and_then(|s| s.energy_calibration.as_ref())
        .is_some()
}

#[test]
fn two_points_calibrate_the_spectrum() {
    let mut app = with_peak(1024, 300);
    app.apply_action(Action::Marker(100));
    app.apply_action(Action::AddCalibrationPoint(200.0));
    assert!(
        !calibrated(&app),
        "one point cannot fix a line; it should still read uncalibrated"
    );
    app.apply_action(Action::Marker(800));
    app.apply_action(Action::AddCalibrationPoint(1600.0));
    assert!(
        calibrated(&app),
        "two points should calibrate it, but the spectrum still has no calibration"
    );
    let cal = app
        .active_spectrum()
        .and_then(|s| s.energy_calibration.clone())
        .expect("a calibration");
    assert!(
        (cal.energy(100.0) - 200.0).abs() < 1e-6 && (cal.energy(800.0) - 1600.0).abs() < 1e-6,
        "the fit does not pass through the points: {:?}",
        cal.coefficients
    );
}

#[test]
fn a_marker_inside_a_region_calibrates_on_the_peak_not_the_marker() {
    let mut app = with_peak(1024, 300);
    // A region around the peak, and the marker off-centre inside it.
    app.apply_action(Action::MarkRange(285, 315));
    app.apply_action(Action::Marker(292));
    app.apply_action(Action::AddCalibrationPoint(1000.0));
    app.apply_action(Action::Marker(800));
    app.apply_action(Action::AddCalibrationPoint(2000.0));

    let cal = app
        .active_spectrum()
        .and_then(|s| s.energy_calibration.clone())
        .expect("a calibration");
    // If the marker had been used, 292 would map to 1000. The centroid is at
    // the peak, near 300, so 292 must come out below 1000.
    let at_marker = cal.energy(292.0);
    assert!(
        at_marker < 999.0,
        "the marker channel was used instead of the region's peak: 292 -> {at_marker}"
    );
    assert!(
        (cal.energy(300.0) - 1000.0).abs() < 25.0,
        "the peak centroid should be what carries 1000 keV: 300 -> {}",
        cal.energy(300.0)
    );
}

#[test]
fn a_marker_outside_any_region_calibrates_on_the_marker() {
    let mut app = with_peak(1024, 300);
    app.apply_action(Action::MarkRange(285, 315));
    // Well away from the region.
    app.apply_action(Action::Marker(700));
    app.apply_action(Action::AddCalibrationPoint(1400.0));
    app.apply_action(Action::Marker(100));
    app.apply_action(Action::AddCalibrationPoint(200.0));
    let cal = app
        .active_spectrum()
        .and_then(|s| s.energy_calibration.clone())
        .expect("a calibration");
    assert!(
        (cal.energy(700.0) - 1400.0).abs() < 1e-6,
        "the marker itself should have been used: 700 -> {}",
        cal.energy(700.0)
    );
}
