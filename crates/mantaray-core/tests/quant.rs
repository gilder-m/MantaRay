//! Quantitative nuclide analysis: identification, activity, MDA and QA charts.

use chrono::NaiveDate;
use mantaray_core::{
    AnalysisOptions, CalculationSettings, ControlLimits, EfficiencyCalibration, EnergyCalibration,
    NuclideLibrary, QaChart, QaMetric, QaRecord, QaStatus, Roi, SampleInfo, Spectrum, analyse,
    decay_during_acquisition, decay_to_reference,
};

/// A calibrated spectrum with clean Cs-137 (661.657 keV) and Co-60 (1173.228 /
/// 1332.492 keV) peaks on a small flat background. 1 keV per channel.
fn calibrated_spectrum() -> Spectrum {
    let mut counts = vec![0f64; 2048];
    for c in counts.iter_mut() {
        *c = 20.0;
    }
    for (energy, area) in [
        (661.657, 50_000.0),
        (1173.228, 30_000.0),
        (1332.492, 27_000.0),
    ] {
        // ~4.7 keV FWHM, typical of a coaxial HPGe at 1 keV per channel.
        let sigma = 2.0;
        let amp = area / (sigma * (2.0 * std::f64::consts::PI).sqrt());
        for (ch, c) in counts.iter_mut().enumerate() {
            let x = (ch as f64 - energy) / sigma;
            *c += amp * (-0.5 * x * x).exp();
        }
    }
    let mut s = Spectrum::from_counts(counts.into_iter().map(|c| c.round() as u64).collect());
    s.live_time = 1000.0;
    s.real_time = 1010.0;
    s.energy_calibration = Some(EnergyCalibration::linear(0.0, 1.0));
    s.start_time = Some(
        NaiveDate::from_ymd_opt(2026, 7, 1)
            .unwrap()
            .and_hms_opt(12, 0, 0)
            .unwrap(),
    );
    s
}

fn options() -> AnalysisOptions {
    AnalysisOptions {
        efficiency: Some(EfficiencyCalibration::constant(0.05)),
        ..AnalysisOptions::default()
    }
}

#[test]
fn analysis_identifies_the_peaks_it_finds() {
    let mut s = calibrated_spectrum();
    let lib = NuclideLibrary::sample_for_tests();
    mantaray_core::mark_peaks(&mut s, &CalculationSettings::default());
    let report = analyse(&s, &lib, &options());

    let identified: Vec<&str> = report
        .peaks
        .iter()
        .filter_map(|p| p.identification.as_ref().map(|i| i.nuclide.as_str()))
        .collect();
    assert!(identified.contains(&"Cs-137"), "got {identified:?}");
    assert_eq!(
        identified.iter().filter(|n| **n == "Co-60").count(),
        2,
        "both Co-60 lines identified: {identified:?}"
    );
}

#[test]
fn activity_uses_efficiency_yield_and_live_time() {
    let mut s = calibrated_spectrum();
    let lib = NuclideLibrary::sample_for_tests();
    s.rois.mark(Roi::new(650, 674));
    let report = analyse(&s, &lib, &options());
    let cs = report
        .nuclides
        .iter()
        .find(|n| n.nuclide == "Cs-137")
        .expect("Cs-137 reported");

    // A = net / (live * eff * yield) = 50 000 / (1000 * 0.05 * 0.851) = 1175 Bq
    let expected = 50_000.0 / (1000.0 * 0.05 * 0.851);
    assert!(
        (cs.activity - expected).abs() / expected < 0.02,
        "got {} expected {expected}",
        cs.activity
    );
    assert!(cs.detected);
    assert!(cs.uncertainty > 0.0 && cs.uncertainty < cs.activity);
    assert!(cs.mda > 0.0 && cs.mda < cs.activity, "mda {}", cs.mda);
    assert_eq!(cs.lines.len(), 1);
}

#[test]
fn several_lines_are_combined_into_a_weighted_mean() {
    let mut s = calibrated_spectrum();
    let lib = NuclideLibrary::sample_for_tests();
    s.rois.mark(Roi::new(1161, 1185));
    s.rois.mark(Roi::new(1320, 1345));
    let report = analyse(&s, &lib, &options());
    let co = report
        .nuclides
        .iter()
        .find(|n| n.nuclide == "Co-60")
        .expect("Co-60 reported");
    assert_eq!(co.lines.len(), 2);

    // Both lines have ~100 % yield: 30 000 and 27 000 net counts give
    // 600 Bq and 540 Bq, so the weighted mean sits between them.
    let low = 27_000.0 / (1000.0 * 0.05 * 0.999826);
    let high = 30_000.0 / (1000.0 * 0.05 * 0.9985);
    assert!(
        co.activity > low * 0.98 && co.activity < high * 1.02,
        "weighted mean {} not between {low} and {high}",
        co.activity
    );
}

#[test]
fn activity_can_be_reported_per_unit_of_sample() {
    let mut s = calibrated_spectrum();
    let lib = NuclideLibrary::sample_for_tests();
    s.rois.mark(Roi::new(650, 674));
    let mut opts = options();
    opts.sample = SampleInfo {
        quantity: 2.0,
        unit: "kg".into(),
        ..SampleInfo::default()
    };
    let report = analyse(&s, &lib, &opts);
    let cs = report
        .nuclides
        .iter()
        .find(|n| n.nuclide == "Cs-137")
        .unwrap();
    let expected = 50_000.0 / (1000.0 * 0.05 * 0.851) / 2.0;
    assert!((cs.activity - expected).abs() / expected < 0.02);
    assert_eq!(report.activity_unit(), "Bq/kg");
}

#[test]
fn without_an_efficiency_curve_count_rates_stand_in_for_activities() {
    let mut s = calibrated_spectrum();
    let lib = NuclideLibrary::sample_for_tests();
    s.rois.mark(Roi::new(650, 674));
    let report = analyse(&s, &lib, &AnalysisOptions::default());
    let cs = report
        .nuclides
        .iter()
        .find(|n| n.nuclide == "Cs-137")
        .unwrap();
    // The unit says cps, so the numbers must be the rates - an all-zero
    // activity column under a "cps" heading is a contradiction, not a report.
    assert_eq!(report.activity_unit(), "cps");
    assert!(cs.lines[0].count_rate > 0.0);
    assert!(
        (cs.activity - cs.lines[0].count_rate).abs() < 1e-9,
        "one line: the nuclide's rate is that line's rate"
    );
    assert!(
        cs.uncertainty > 0.0,
        "the rate carries its counting uncertainty"
    );
    assert!(
        (cs.lines[0].activity - cs.lines[0].count_rate).abs() < 1e-9,
        "the line's activity column is its rate"
    );
    // The detection limit follows the fallback into count-rate units:
    // (2.71 + 4.65 sqrt(B)) / live, from Currie with unit efficiency and yield.
    assert!(
        cs.mda > 0.0 && cs.mda < cs.activity,
        "a strong peak's rate should dwarf its detection limit: mda {} rate {}",
        cs.mda,
        cs.activity
    );
}

#[test]
fn without_an_efficiency_curve_absent_nuclides_still_get_detection_limits() {
    let mut s = calibrated_spectrum();
    let lib = NuclideLibrary::sample_for_tests();
    s.rois.mark(Roi::new(650, 674));
    let opts = AnalysisOptions {
        report_mda_for_absent_nuclides: true,
        ..AnalysisOptions::default()
    };
    let report = analyse(&s, &lib, &opts);
    assert_eq!(report.activity_unit(), "cps");
    let k40 = report
        .nuclides
        .iter()
        .find(|n| n.nuclide == "K-40")
        .expect("K-40 reported with an MDA only");
    assert!(!k40.detected);
    // (2.71 + 4.65 sqrt(B)) / live over the K-40 line's region: with ~20
    // counts per channel of continuum the limit is small but decidedly not 0.
    assert!(k40.mda > 0.0, "got {}", k40.mda);
    assert!(k40.mda < 1.0, "a flat 20-count continuum: got {}", k40.mda);
}

#[test]
fn regions_without_a_library_match_are_listed_separately() {
    let mut s = calibrated_spectrum();
    let lib = NuclideLibrary::new("empty");
    s.rois.mark(Roi::new(650, 674));
    let report = analyse(&s, &lib, &options());
    assert!(report.nuclides.is_empty());
    assert_eq!(report.unidentified.len(), 1);
    assert!(report.peaks[0].identification.is_none());
}

#[test]
fn undetected_library_nuclides_can_still_get_an_mda() {
    let mut s = calibrated_spectrum();
    let lib = NuclideLibrary::sample_for_tests();
    s.rois.mark(Roi::new(650, 674));
    let mut opts = options();
    opts.report_mda_for_absent_nuclides = true;
    let report = analyse(&s, &lib, &opts);
    let k40 = report
        .nuclides
        .iter()
        .find(|n| n.nuclide == "K-40")
        .expect("K-40 reported with an MDA only");
    assert!(!k40.detected);
    assert_eq!(k40.activity, 0.0);
    assert!(k40.mda > 0.0);
}

#[test]
fn decay_corrections_follow_the_exponential_law() {
    let reference = NaiveDate::from_ymd_opt(2026, 1, 1)
        .unwrap()
        .and_hms_opt(0, 0, 0)
        .unwrap();
    let measured = NaiveDate::from_ymd_opt(2026, 1, 11)
        .unwrap()
        .and_hms_opt(0, 0, 0)
        .unwrap();
    // Ten days later with a ten-day half life: the activity at the reference
    // date was twice what was measured.
    let factor = decay_to_reference(10.0 * 86400.0, reference, measured);
    assert!((factor - 2.0).abs() < 1e-9, "got {factor}");
    // Correcting to a later date reduces the inferred activity.
    assert!(decay_to_reference(10.0 * 86400.0, measured, reference) < 1.0);
    // A long-lived nuclide needs no correction.
    assert!((decay_to_reference(0.0, reference, measured) - 1.0).abs() < 1e-12);

    // Decay during the count: for a half life equal to the count time the
    // correction is ln2 / (1 - 1/2) = 1.386.
    let during = decay_during_acquisition(1000.0, 1000.0);
    assert!(
        (during - std::f64::consts::LN_2 / 0.5).abs() < 1e-9,
        "got {during}"
    );
    assert!((decay_during_acquisition(0.0, 1000.0) - 1.0).abs() < 1e-12);
    // A short count against a long half life is a negligible correction.
    assert!((decay_during_acquisition(1e9, 100.0) - 1.0).abs() < 1e-6);
}

#[test]
fn qa_charts_flag_drifting_detectors() {
    let mut chart = QaChart::new(QaMetric::Centroid, ControlLimits::default());
    let day = |d: u32| {
        NaiveDate::from_ymd_opt(2026, 6, d)
            .unwrap()
            .and_hms_opt(9, 0, 0)
            .unwrap()
    };
    for (index, value) in [1332.50, 1332.48, 1332.52, 1332.49, 1332.51]
        .iter()
        .enumerate()
    {
        chart.push(QaRecord::new(day(index as u32 + 1), *value));
    }
    let stats = chart.statistics().expect("statistics from five points");
    assert!((stats.mean - 1332.50).abs() < 0.01);
    assert!(stats.standard_deviation > 0.0);

    assert_eq!(chart.evaluate(1332.50), QaStatus::InControl);
    assert_eq!(
        chart.evaluate(1332.50 + 2.5 * stats.standard_deviation),
        QaStatus::Warning
    );
    assert_eq!(
        chart.evaluate(1332.50 - 4.0 * stats.standard_deviation),
        QaStatus::Action
    );
    assert_eq!(chart.status(), QaStatus::InControl);

    chart.push(QaRecord::new(
        day(6),
        1332.50 + 10.0 * stats.standard_deviation,
    ));
    assert_eq!(
        chart.status(),
        QaStatus::Action,
        "the last point is out of control"
    );
    assert_eq!(chart.len(), 6);
}
