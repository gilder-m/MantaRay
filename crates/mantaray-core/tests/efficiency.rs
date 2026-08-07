//! Efficiency calibration, the basis of quantitative results.

use mantaray_core::{
    CalibrationError, EfficiencyCalibration, EfficiencyPoint, decay_factor, efficiency_from_source,
    source_activity_at,
};

fn approx(a: f64, b: f64, tol: f64) -> bool {
    (a - b).abs() <= tol
}

#[test]
fn interpolated_curve_passes_through_its_points() {
    let points = vec![
        EfficiencyPoint::new(59.5, 0.081),
        EfficiencyPoint::new(122.1, 0.062),
        EfficiencyPoint::new(661.7, 0.017),
        EfficiencyPoint::new(1332.5, 0.0095),
    ];
    let cal = EfficiencyCalibration::interpolated(points.clone());
    for p in &points {
        assert!(
            approx(cal.efficiency(p.energy), p.efficiency, 1e-9),
            "at {} keV",
            p.energy
        );
    }
    // Between two points the curve is monotone and bracketed.
    let mid = cal.efficiency(300.0);
    assert!(mid < 0.062 && mid > 0.017, "got {mid}");
}

#[test]
fn interpolation_clamps_outside_the_calibrated_range() {
    let cal = EfficiencyCalibration::interpolated(vec![
        EfficiencyPoint::new(100.0, 0.05),
        EfficiencyPoint::new(1000.0, 0.01),
    ]);
    assert!(approx(cal.efficiency(50.0), 0.05, 1e-12));
    assert!(approx(cal.efficiency(5000.0), 0.01, 1e-12));
}

#[test]
fn log_log_polynomial_recovers_a_known_curve() {
    // eff(E) = exp(a0 + a1*lnE + a2*(lnE)^2)
    let truth = |e: f64| (-1.2 - 0.85 * e.ln() + 0.01 * e.ln() * e.ln()).exp();
    let points: Vec<EfficiencyPoint> = [59.5, 122.1, 344.3, 661.7, 1173.2, 1332.5, 1408.0]
        .iter()
        .map(|&e| EfficiencyPoint::new(e, truth(e)))
        .collect();
    let cal = EfficiencyCalibration::fit_log_log(&points, 2).unwrap();
    for e in [80.0, 500.0, 1200.0] {
        let got = cal.efficiency(e);
        assert!(
            (got - truth(e)).abs() / truth(e) < 1e-6,
            "at {e} keV: {got} vs {}",
            truth(e)
        );
    }
    assert_eq!(cal.order(), 2);
}

#[test]
fn a_fit_needs_more_points_than_coefficients() {
    let points = vec![
        EfficiencyPoint::new(100.0, 0.05),
        EfficiencyPoint::new(1000.0, 0.01),
    ];
    assert!(matches!(
        EfficiencyCalibration::fit_log_log(&points, 3),
        Err(CalibrationError::NotEnoughPoints { .. })
    ));
    // Order 1 needs two points and works.
    let cal = EfficiencyCalibration::fit_log_log(&points, 1).unwrap();
    assert!(approx(cal.efficiency(100.0), 0.05, 1e-9));
    assert!(approx(cal.efficiency(1000.0), 0.01, 1e-9));
}

#[test]
fn non_positive_efficiencies_are_rejected() {
    let points = vec![
        EfficiencyPoint::new(100.0, 0.0),
        EfficiencyPoint::new(1000.0, 0.01),
        EfficiencyPoint::new(2000.0, 0.005),
    ];
    assert!(EfficiencyCalibration::fit_log_log(&points, 1).is_err());
}

#[test]
fn a_constant_efficiency_is_available_for_quick_work() {
    let cal = EfficiencyCalibration::constant(0.05);
    assert!(approx(cal.efficiency(59.5), 0.05, 1e-12));
    assert!(approx(cal.efficiency(2614.5), 0.05, 1e-12));
    assert!(!cal.is_empty());
    assert!(EfficiencyCalibration::default().is_empty());
    assert_eq!(EfficiencyCalibration::default().efficiency(661.7), 0.0);
}

#[test]
fn efficiency_points_can_be_derived_from_a_certified_source() {
    // 1000 Bq of a line with 85.1 % yield giving 42.55 net counts per second
    // means an absolute efficiency of 5 %.
    let eff = efficiency_from_source(42.55, 1000.0, 85.1);
    assert!(approx(eff, 0.05, 1e-9), "got {eff}");
    assert_eq!(efficiency_from_source(10.0, 0.0, 85.1), 0.0);
}

#[test]
fn source_activity_decays_to_the_measurement_date() {
    let half_life = 100.0;
    // Two half lives later a 1000 Bq source is at 250 Bq.
    assert!(approx(
        source_activity_at(1000.0, half_life, 200.0),
        250.0,
        1e-9
    ));
    assert!(approx(decay_factor(half_life, 100.0), 0.5, 1e-12));
    // A stable (unknown half life) source does not decay.
    assert!(approx(source_activity_at(1000.0, 0.0, 1e6), 1000.0, 1e-12));
}
