//! Energy and peak-shape calibration, per MAESTRO Calculate/Calibration (§4.3.2):
//! two points give a linear fit, three or more (up to 96) give a quadratic fit.

use mantaray_core::{
    CalibrationError, CalibrationPoint, CalibrationTable, EnergyCalibration,
    MAX_CALIBRATION_POINTS, ShapeCalibration,
};

fn approx(a: f64, b: f64, tol: f64) -> bool {
    (a - b).abs() <= tol
}

#[test]
fn linear_calibration_maps_channels_to_energy() {
    let cal = EnergyCalibration::linear(1.5, 0.25);
    assert!(approx(cal.energy(0.0), 1.5, 1e-12));
    assert!(approx(cal.energy(100.0), 26.5, 1e-12));
    assert_eq!(cal.units, "keV");
}

#[test]
fn quadratic_calibration_includes_second_order_term() {
    let cal = EnergyCalibration::new([1.0, 0.5, 1e-6], "keV");
    assert!(approx(cal.energy(1000.0), 1.0 + 500.0 + 1.0, 1e-9));
}

#[test]
fn inverse_maps_energy_back_to_channel() {
    let lin = EnergyCalibration::linear(2.0, 0.5);
    assert!(approx(lin.channel(52.0).unwrap(), 100.0, 1e-9));

    let quad = EnergyCalibration::new([1.0, 0.5, 1e-6], "keV");
    let ch = quad.channel(quad.energy(1234.0)).unwrap();
    assert!(approx(ch, 1234.0, 1e-6));

    // A flat calibration cannot be inverted.
    assert!(
        EnergyCalibration::new([5.0, 0.0, 0.0], "keV")
            .channel(5.0)
            .is_none()
    );
}

#[test]
fn two_points_give_an_exact_linear_fit() {
    let cal = EnergyCalibration::fit(&[
        CalibrationPoint::new(100.0, 122.06),
        CalibrationPoint::new(1000.0, 1173.24),
    ])
    .unwrap();
    assert!(approx(cal.energy(100.0), 122.06, 1e-6));
    assert!(approx(cal.energy(1000.0), 1173.24, 1e-6));
    assert_eq!(
        cal.coefficients[2], 0.0,
        "no quadratic term from two points"
    );
    assert_eq!(cal.order(), 1);
}

#[test]
fn three_points_give_an_exact_quadratic_fit() {
    let truth = EnergyCalibration::new([2.0, 0.4, 5e-7], "keV");
    let pts: Vec<_> = [100.0, 2000.0, 4000.0]
        .iter()
        .map(|&c| CalibrationPoint::new(c, truth.energy(c)))
        .collect();
    let cal = EnergyCalibration::fit(&pts).unwrap();
    for c in [0.0, 137.0, 3999.0] {
        assert!(approx(cal.energy(c), truth.energy(c), 1e-6), "channel {c}");
    }
    assert_eq!(cal.order(), 2);
}

#[test]
fn many_points_are_least_squares_fitted() {
    let truth = EnergyCalibration::new([1.0, 0.3, 2e-7], "keV");
    // 20 points with small alternating perturbations: the fit must stay close.
    let pts: Vec<_> = (0..20)
        .map(|i| {
            let c = 50.0 + 200.0 * i as f64;
            let noise = if i % 2 == 0 { 0.05 } else { -0.05 };
            CalibrationPoint::new(c, truth.energy(c) + noise)
        })
        .collect();
    let cal = EnergyCalibration::fit(&pts).unwrap();
    for c in [100.0, 1500.0, 3800.0] {
        assert!(approx(cal.energy(c), truth.energy(c), 0.2), "channel {c}");
    }
}

#[test]
fn degenerate_and_short_point_sets_are_rejected() {
    assert!(matches!(
        EnergyCalibration::fit(&[]),
        Err(CalibrationError::NotEnoughPoints { .. })
    ));
    assert!(matches!(
        EnergyCalibration::fit(&[CalibrationPoint::new(10.0, 20.0)]),
        Err(CalibrationError::NotEnoughPoints { .. })
    ));
    assert!(matches!(
        EnergyCalibration::fit(&[
            CalibrationPoint::new(10.0, 20.0),
            CalibrationPoint::new(10.0, 40.0),
        ]),
        Err(CalibrationError::Degenerate)
    ));
}

#[test]
fn units_are_truncated_to_four_characters() {
    // MAESTRO asks for "up to four characters for the units".
    let cal = EnergyCalibration::new([0.0, 1.0, 0.0], "kiloelectronvolts");
    assert_eq!(cal.units, "kilo");
}

#[test]
fn calibration_table_collects_points_and_refits() {
    let mut table = CalibrationTable::default();
    assert!(table.calibration().is_none());
    assert_eq!(table.len(), 0);

    table.add(CalibrationPoint::new(100.0, 100.0));
    assert!(
        table.calibration().is_none(),
        "one point is not yet a calibration"
    );

    table.add(CalibrationPoint::new(200.0, 200.0));
    let cal = table.calibration().expect("two points calibrate");
    assert!(approx(cal.energy(150.0), 150.0, 1e-9));
    assert_eq!(cal.order(), 1);

    // A third point off the line switches the fit to a parabola, which passes
    // through all three points exactly.
    table.add(CalibrationPoint::new(300.0, 310.0));
    assert_eq!(table.len(), 3);
    let cal = table.calibration().unwrap();
    assert_eq!(cal.order(), 2);
    for (ch, value) in [(100.0, 100.0), (200.0, 200.0), (300.0, 310.0)] {
        assert!(approx(cal.energy(ch), value, 1e-6), "channel {ch}");
    }

    table.destroy();
    assert_eq!(table.len(), 0);
    assert!(table.calibration().is_none());
}

#[test]
fn calibration_table_is_capped_at_96_points() {
    let mut table = CalibrationTable::default();
    for i in 0..(MAX_CALIBRATION_POINTS + 10) {
        let _ = table.add(CalibrationPoint::new(i as f64 + 1.0, i as f64 + 1.0));
    }
    assert_eq!(table.len(), MAX_CALIBRATION_POINTS);
    assert_eq!(MAX_CALIBRATION_POINTS, 96);
}

#[test]
fn shape_calibration_predicts_fwhm() {
    // FWHM = c0 + c1*channel + c2*channel^2, in channels.
    let shape = ShapeCalibration::new([1.0, 0.002, 0.0]);
    assert!(approx(shape.fwhm(0.0), 1.0, 1e-12));
    assert!(approx(shape.fwhm(500.0), 2.0, 1e-12));
    assert!(approx(shape.sigma(500.0), 2.0 / 2.354_820_045, 1e-9));
    assert!(shape.is_usable());
    assert!(!ShapeCalibration::default().is_usable());
}

#[test]
fn shape_calibration_matches_a_real_maestro_pro_file() {
    // $SHAPE_CAL from a real HPGe spectrum at 16.0819 + 0.360998 keV/ch:
    // 1.80 keV at 662 keV and 2.27 keV at 1332 keV.
    let shape = ShapeCalibration::new([2.494_58, 1.745e-3, -1.935_63e-7]);
    let cal = EnergyCalibration::new([16.0819, 0.360_998, 2.707_29e-8], "keV");
    let channel_of = |energy: f64| cal.channel(energy).unwrap();
    let width_kev = |energy: f64| {
        let channel = channel_of(energy);
        cal.width(channel, shape.fwhm(channel))
    };
    assert!(
        approx(width_kev(661.657), 1.80, 0.05),
        "{}",
        width_kev(661.657)
    );
    assert!(
        approx(width_kev(1332.492), 2.27, 0.05),
        "{}",
        width_kev(1332.492)
    );
}

#[test]
fn shape_calibration_fits_measured_widths() {
    let truth = ShapeCalibration::new([2.5, 1.7e-3, -1.9e-7]);
    let pts: Vec<(f64, f64)> = [100.0, 900.0, 2500.0, 4900.0]
        .iter()
        .map(|&c| (c, truth.fwhm(c)))
        .collect();
    let fit = ShapeCalibration::fit(&pts).unwrap();
    for (c, w) in pts {
        assert!(approx(fit.fwhm(c), w, 1e-6), "channel {c}");
    }
    // Two points give a straight line through both.
    let line = ShapeCalibration::fit(&[(100.0, 3.0), (1100.0, 5.0)]).unwrap();
    assert!(approx(line.fwhm(600.0), 4.0, 1e-9));
    assert_eq!(line.coefficients[2], 0.0);
}
