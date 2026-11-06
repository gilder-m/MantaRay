//! Quantitative nuclide analysis: identification, activity, MDA and QA charts.

use chrono::NaiveDate;
use mantaray_core::{
    AnalysisOptions, CalculationSettings, ControlLimits, EfficiencyCalibration, EfficiencyPoint,
    EnergyCalibration, LibraryPeak, Nuclide, NuclideLibrary, QaChart, QaMetric, QaRecord, QaStatus,
    Roi, SampleInfo, Spectrum, analyse, decay_during_acquisition, decay_to_reference,
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

/// A spectrum of Gaussian peaks on a flat background of 20, 1 keV per
/// channel, counted 1000 s live from 2026-07-01 12:00.
fn peaked(channels: usize, peaks: &[(f64, f64)]) -> Spectrum {
    let mut counts = vec![20f64; channels];
    for &(energy, area) in peaks {
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
    s.start_time = NaiveDate::from_ymd_opt(2026, 7, 1)
        .unwrap()
        .and_hms_opt(12, 0, 0);
    s
}

fn referred_to(year: i32, month: u32, day: u32) -> AnalysisOptions {
    let mut opts = options();
    opts.sample = SampleInfo {
        reference_date: NaiveDate::from_ymd_opt(year, month, day)
            .unwrap()
            .and_hms_opt(12, 0, 0),
        ..SampleInfo::default()
    };
    opts
}

#[test]
fn a_decay_correction_too_large_to_compute_is_left_out_by_name() {
    // Tl-208 (183 s half-life) three days after its reference date is 1400
    // half-lives: the correction overflowed to infinity and the activity, its
    // uncertainty and the report's total all came out NaN.
    let mut s = peaked(4096, &[(2614.511, 10_000.0)]);
    s.rois.mark(Roi::new(2602, 2627));
    let lib = NuclideLibrary::sample_for_tests();
    let report = analyse(&s, &lib, &referred_to(2026, 6, 28));
    let tl = report
        .nuclides
        .iter()
        .find(|n| n.nuclide == "Tl-208")
        .expect("Tl-208 is reported");
    assert!(
        tl.activity.is_finite() && tl.activity > 0.0,
        "{}",
        tl.activity
    );
    assert!(tl.uncertainty.is_finite(), "{}", tl.uncertainty);
    assert!(report.total_activity().is_finite());
    assert!(
        report
            .notes
            .iter()
            .any(|note| note.contains("reference date") && note.contains("Tl-208")),
        "{:?}",
        report.notes
    );
    // Reported at the time of the count: what no reference date gives.
    let uncorrected = analyse(&s, &lib, &options());
    let plain = uncorrected
        .nuclides
        .iter()
        .find(|n| n.nuclide == "Tl-208")
        .unwrap();
    assert!((tl.activity - plain.activity).abs() <= 1e-9 * plain.activity);
}

#[test]
fn a_large_finite_decay_correction_still_averages_its_lines() {
    // e^460 is a double, but its square is not: the weights 1/sigma^2 went to
    // zero and the weighted mean to 0/0.
    let mut s = calibrated_spectrum();
    s.rois.mark(Roi::new(1161, 1185));
    s.rois.mark(Roi::new(1320, 1344));
    let mut lib = NuclideLibrary::new("hot");
    lib.push(Nuclide::new(
        "Hot-1",
        86_400.0,
        vec![
            LibraryPeak::new(1173.228, 99.85).key(),
            LibraryPeak::new(1332.492, 99.98),
        ],
    ));
    // 664 days at a one-day half-life: a correction near 1e200.
    let referred = referred_to(2024, 9, 5);
    let report = analyse(&s, &lib, &referred);
    let hot = &report.nuclides[0];
    assert_eq!(hot.lines.len(), 2);
    assert!(hot.activity.is_finite(), "{}", hot.activity);
    assert!(hot.uncertainty.is_finite() && hot.uncertainty > 0.0);
    assert!(report.notes.is_empty(), "{:?}", report.notes);
    // Exactly the uncorrected activity times the reference-date factor.
    let plain = analyse(&s, &lib, &options());
    let factor = decay_to_reference(
        86_400.0,
        referred.sample.reference_date.unwrap(),
        s.start_time.unwrap(),
    );
    assert!(factor > 1e199 && factor.is_finite(), "{factor}");
    let ratio = hot.activity / plain.nuclides[0].activity / factor;
    assert!((ratio - 1.0).abs() < 1e-9, "{ratio}");
}

#[test]
fn the_lines_average_by_inverse_variance_and_the_library_term_adds_in_quadrature() {
    let mut s = calibrated_spectrum();
    s.rois.mark(Roi::new(1161, 1185));
    s.rois.mark(Roi::new(1320, 1344));
    let mut nuclide = Nuclide::new(
        "Two-1",
        0.0,
        vec![
            LibraryPeak::new(1173.228, 99.85).key(),
            LibraryPeak::new(1332.492, 99.98),
        ],
    );
    nuclide.uncertainty_percent = 4.0; // two sigma, so 2 % one sigma
    let mut lib = NuclideLibrary::new("two");
    lib.push(nuclide);
    let report = analyse(&s, &lib, &options());
    let two = &report.nuclides[0];
    let weights: Vec<f64> = two
        .lines
        .iter()
        .map(|line| 1.0 / line.activity_uncertainty.powi(2))
        .collect();
    let total: f64 = weights.iter().sum();
    let mean = two
        .lines
        .iter()
        .zip(&weights)
        .map(|(line, weight)| line.activity * weight)
        .sum::<f64>()
        / total;
    let sigma = (1.0 / total).sqrt().hypot(mean * 0.02);
    assert!(
        two.lines[0].activity_uncertainty != two.lines[1].activity_uncertainty,
        "the lines must differ for the weights to matter"
    );
    assert!(
        (two.activity / mean - 1.0).abs() < 1e-12,
        "{}",
        two.activity
    );
    assert!(
        (two.uncertainty / sigma - 1.0).abs() < 1e-12,
        "{}",
        two.uncertainty
    );
}

#[test]
fn the_note_names_ten_nuclides_and_counts_the_rest() {
    // MDA rows for short-lived nuclides three days past their reference date:
    // each has its correction left out, and the note must stay readable.
    let s = calibrated_spectrum();
    for count in [10usize, 11] {
        let mut lib = NuclideLibrary::new("short");
        for index in 1..=count {
            lib.push(Nuclide::new(
                &format!("S-{index}"),
                100.0,
                vec![LibraryPeak::new(200.0 + 50.0 * index as f64, 90.0).key()],
            ));
        }
        let mut opts = referred_to(2026, 6, 28);
        opts.report_mda_for_absent_nuclides = true;
        let report = analyse(&s, &lib, &opts);
        assert_eq!(report.nuclides.len(), count);
        let note = report
            .notes
            .iter()
            .find(|note| note.contains("reference date"))
            .expect("the note");
        assert!(note.contains("S-10"), "{note}");
        if count == 10 {
            assert!(!note.contains("more"), "{note}");
        } else {
            assert!(
                note.contains("and 1 more") && !note.contains("S-11"),
                "{note}"
            );
        }
    }
}

#[test]
fn a_line_with_no_yield_gives_no_activity_either() {
    let mut s = calibrated_spectrum();
    s.rois.mark(Roi::new(650, 674));
    let mut lib = NuclideLibrary::new("no yield");
    lib.push(Nuclide::new(
        "Cs-137",
        30.08 * 365.25 * 86_400.0,
        vec![LibraryPeak::new(661.657, 0.0).key()],
    ));
    let report = analyse(&s, &lib, &options());
    assert!(report.nuclides.is_empty(), "{:?}", report.nuclides);
    assert!(
        report.notes.iter().any(|note| note.contains("Cs-137")),
        "{:?}",
        report.notes
    );
}

#[test]
fn a_curve_that_gives_no_efficiency_is_not_taken_for_one() {
    // Both used to report Cs-137 detected at 0 Bq, with no note.
    let mut s = calibrated_spectrum();
    s.rois.mark(Roi::new(650, 674));
    let lib = NuclideLibrary::sample_for_tests();
    let plain = analyse(&s, &lib, &AnalysisOptions::default());
    let rate = plain.nuclides[0].activity;
    for curve in [
        EfficiencyCalibration::interpolated(vec![]),
        EfficiencyCalibration::constant(0.0),
    ] {
        let opts = AnalysisOptions {
            efficiency: Some(curve),
            ..AnalysisOptions::default()
        };
        let report = analyse(&s, &lib, &opts);
        assert_eq!(report.activity_unit(), "cps");
        let cs = report
            .nuclides
            .iter()
            .find(|n| n.nuclide == "Cs-137")
            .unwrap();
        assert!(rate > 0.0);
        assert_eq!(
            cs.activity, rate,
            "the count rate stands in, as with no curve"
        );
        assert!(
            report.notes.iter().any(|note| note.contains("efficiency")),
            "{:?}",
            report.notes
        );
    }
}

#[test]
fn a_nuclide_whose_lines_get_no_efficiency_is_withheld_with_a_note() {
    // A curve that is zero past 500 keV: Cs-137 at 661.7 keV used to come out
    // detected at 0 Bq.
    let mut s = calibrated_spectrum();
    s.rois.mark(Roi::new(650, 674));
    let opts = AnalysisOptions {
        efficiency: Some(EfficiencyCalibration::interpolated(vec![
            EfficiencyPoint::new(100.0, 0.1),
            EfficiencyPoint::new(500.0, 0.0),
        ])),
        ..AnalysisOptions::default()
    };
    let report = analyse(&s, &NuclideLibrary::sample_for_tests(), &opts);
    assert!(report.nuclides.iter().all(|n| n.nuclide != "Cs-137"));
    assert!(
        report
            .notes
            .iter()
            .any(|note| note.contains("Cs-137") && note.contains("661.7")),
        "{:?}",
        report.notes
    );
    assert!(
        report.peaks.iter().any(|peak| peak
            .identification
            .as_ref()
            .is_some_and(|id| id.nuclide == "Cs-137")),
        "the peak is still identified in the peak table"
    );
}

#[test]
fn a_spectrum_with_no_live_time_reports_no_activity_and_says_why() {
    let mut s = calibrated_spectrum();
    s.live_time = 0.0;
    s.rois.mark(Roi::new(650, 674));
    let report = analyse(&s, &NuclideLibrary::sample_for_tests(), &options());
    assert!(report.nuclides.is_empty(), "{:?}", report.nuclides);
    assert!(
        report.notes.iter().any(|note| note.contains("live time")),
        "{:?}",
        report.notes
    );
}
