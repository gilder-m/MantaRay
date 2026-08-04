//! Automatic energy calibration: peaks in, calibration out.

use ortseam_core::{CalibrationError, auto_calibrate};

/// Eu-152's strong lines, keV.
const EU152: [f64; 7] = [121.78, 244.70, 344.28, 778.90, 964.08, 1112.07, 1408.01];

/// Channels those lines sit at for a given zero and gain.
fn channels(zero: f64, gain: f64, energies: &[f64]) -> Vec<f64> {
    energies
        .iter()
        .map(|energy| (energy - zero) / gain)
        .collect()
}

#[test]
fn a_europium_spectrum_calibrates_itself() {
    // 0.5 keV per channel, a small offset - a typical HPGe setup.
    let found = channels(1.2, 0.5, &EU152);
    let result = auto_calibrate(&found, &EU152, "keV").expect("should calibrate");
    let cal = &result.calibration;
    assert!(
        (cal.coefficients[1] - 0.5).abs() < 0.001,
        "gain should be recovered, got {}",
        cal.coefficients[1]
    );
    assert!(
        (cal.coefficients[0] - 1.2).abs() < 0.5,
        "zero should be recovered, got {}",
        cal.coefficients[0]
    );
    assert_eq!(result.matches.len(), 7, "every line should be matched");
    // The proof: each found channel maps back to its line.
    for (channel, energy) in found.iter().zip(EU152) {
        assert!(
            (cal.energy(*channel) - energy).abs() < 0.5,
            "channel {channel} should read {energy}"
        );
    }
}

#[test]
fn the_strongest_peaks_need_not_be_in_energy_order() {
    // Callers pass centroids strongest-first; for Eu-152 that is 121.78 first,
    // then 344, 1408... - nothing like energy order.
    let ordered = channels(0.0, 0.3611, &EU152);
    let by_strength = [
        ordered[0], ordered[2], ordered[6], ordered[1], ordered[3], ordered[5], ordered[4],
    ];
    let result = auto_calibrate(&by_strength, &EU152, "keV").expect("should calibrate");
    assert!((result.calibration.coefficients[1] - 0.3611).abs() < 0.001);
}

#[test]
fn stray_peaks_do_not_spoil_the_fit() {
    // A background line (1460.8, K-40) and a random artefact ride along.
    let mut found = channels(0.0, 0.5, &EU152);
    found.push(1460.8 / 0.5);
    found.push(333.0);
    let result = auto_calibrate(&found, &EU152, "keV").expect("should calibrate");
    assert!((result.calibration.coefficients[1] - 0.5).abs() < 0.002);
    assert!(
        result.matches.len() >= 6,
        "the real lines should still be matched"
    );
}

#[test]
fn a_spectrum_of_unrelated_peaks_is_refused() {
    // Peaks that correspond to no assignment of the reference lines. With
    // enough of them at coprime spacings no linear map drops three onto lines.
    let found = [100.0, 217.0, 471.0, 1023.0];
    let error =
        auto_calibrate(&found, &[121.78, 244.70, 344.28], "keV").expect_err("nothing should match");
    assert!(matches!(error, CalibrationError::NoMatch { .. }));
}

#[test]
fn too_few_peaks_are_refused_with_the_count() {
    let error = auto_calibrate(&[100.0, 200.0], &EU152, "keV").expect_err("two is not enough");
    match error {
        CalibrationError::NotEnoughPoints { needed, got } => {
            assert_eq!(needed, 3);
            assert_eq!(got, 2);
        }
        other => panic!("unexpected error: {other}"),
    }
}

#[test]
fn cobalt_60_with_its_two_lines_is_not_enough_either() {
    // Two lines can always be made to fit two peaks; three are the minimum for
    // the fit to mean anything, so Co-60 alone is refused rather than guessed.
    let found = channels(0.0, 0.5, &[1173.23, 1332.49]);
    assert!(auto_calibrate(&found, &[1173.23, 1332.49], "keV").is_err());
}

#[test]
fn a_coarse_scintillator_gain_works_too() {
    // A NaI detector: 3 MeV across 1024 channels.
    let lines = [511.0, 661.66, 1173.23, 1332.49, 1460.8];
    let found = channels(-5.0, 2.93, &lines);
    let result = auto_calibrate(&found, &lines, "keV").expect("should calibrate");
    assert!((result.calibration.coefficients[1] - 2.93).abs() < 0.01);
}
