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
    let mda = cs.mda.expect("a detection limit");
    assert!(mda > 0.0 && mda < cs.activity, "mda {mda}");
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
    let mda = cs.mda.expect("a detection limit");
    assert!(
        mda > 0.0 && mda < cs.activity,
        "a strong peak's rate should dwarf its detection limit: mda {mda} rate {}",
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
    let mda = k40.mda.expect("a detection limit");
    assert!(mda > 0.0, "got {mda}");
    assert!(mda < 1.0, "a flat 20-count continuum: got {mda}");
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
    assert!(k40.mda.is_some_and(|mda| mda > 0.0));
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
    assert!(report.total_activity().is_some_and(f64::is_finite));
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
        report.withheld.iter().any(|w| w.nuclide == "Cs-137"),
        "{:?}",
        report.withheld
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
    assert_eq!(report.withheld.len(), 1, "{:?}", report.withheld);
    assert_eq!(report.withheld[0].nuclide, "Cs-137");
    assert!(
        report.withheld[0].reason.starts_with("no activity:")
            && report.withheld[0].reason.contains("661.7"),
        "{:?}",
        report.withheld
    );
    assert!(
        report.notes.iter().all(|note| !note.contains("Cs-137")),
        "a withheld nuclide is not repeated among the general notes: {:?}",
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
    // The withheld row carries the reason; no note repeats it.
    assert!(
        !report.notes.iter().any(|note| note.contains("live time")),
        "{:?}",
        report.notes
    );
    assert!(
        report
            .withheld
            .iter()
            .any(|w| w.nuclide == "Cs-137" && w.reason.contains("live time")),
        "{:?}",
        report.withheld
    );
}

#[test]
fn a_missing_live_time_is_said_once_whether_or_not_a_nuclide_was_found() {
    // Found: the withheld group says it. Not found (the library has no line
    // for the peak): the one global note says it. Either way, with the MDA
    // rows asked for too, exactly one place names the missing live time.
    let said = |report: &mantaray_core::QuantitativeReport| {
        let notes = report
            .notes
            .iter()
            .filter(|note| note.contains("live time"))
            .count();
        let groups = report
            .withheld_groups()
            .filter(|group| group[0].reason.contains("live time"))
            .count();
        (notes, groups)
    };
    let mut s = calibrated_spectrum();
    s.live_time = 0.0;
    s.rois.mark(Roi::new(650, 674));
    let opts = AnalysisOptions {
        report_mda_for_absent_nuclides: true,
        ..options()
    };
    let found = analyse(&s, &NuclideLibrary::sample_for_tests(), &opts);
    assert_eq!(said(&found), (0, 1), "{:?}", found.notes);
    let nothing_found = analyse(&s, &NuclideLibrary::new("empty"), &opts);
    assert_eq!(said(&nothing_found), (1, 0), "{:?}", nothing_found.notes);
}

#[test]
fn a_nuclide_that_was_not_found_and_has_no_detection_limit_is_a_note_not_a_withheld_row() {
    // An MDA row was asked for for every library nuclide, and the curve is
    // zero at Cs-137's 661.7 keV line. Cs-137 has no peak marked: nothing was
    // found, so there is no row to withhold; the missing limit is a note.
    let s = calibrated_spectrum();
    let opts = AnalysisOptions {
        report_mda_for_absent_nuclides: true,
        efficiency: Some(EfficiencyCalibration::interpolated(vec![
            EfficiencyPoint::new(100.0, 0.1),
            EfficiencyPoint::new(500.0, 0.0),
        ])),
        ..AnalysisOptions::default()
    };
    let report = analyse(&s, &NuclideLibrary::sample_for_tests(), &opts);
    assert!(report.withheld.is_empty(), "{:?}", report.withheld);
    let notes: Vec<&String> = report
        .notes
        .iter()
        .filter(|note| note.contains("no detection limit for"))
        .collect();
    assert_eq!(notes.len(), 1, "{:?}", report.notes);
    assert!(notes[0].contains("Cs-137"), "{}", notes[0]);
    assert!(notes[0].contains("library nuclide"), "{}", notes[0]);
}

#[test]
fn a_large_library_of_unmeasurable_nuclides_leaves_one_note_with_a_count_and_the_first_names() {
    // Every nuclide's line is far outside a 2048-channel spectrum, as most of
    // an evaluated library's are: one note, not 1500 rows.
    let mut lib = NuclideLibrary::new("big");
    for n in 0..1500 {
        lib.push(Nuclide::new(
            &format!("Ac-{n}"),
            1.0e9,
            vec![LibraryPeak::new(5000.0 + n as f64, 50.0).key()],
        ));
    }
    let opts = AnalysisOptions {
        report_mda_for_absent_nuclides: true,
        ..options()
    };
    let report = analyse(&calibrated_spectrum(), &lib, &opts);
    assert!(report.withheld.is_empty() && report.nuclides.is_empty());
    assert_eq!(report.notes.len(), 1, "{:?}", report.notes);
    let note = &report.notes[0];
    assert!(note.contains("1,500 library nuclides"), "{note}");
    assert!(note.contains("outside the spectrum"), "{note}");
    assert!(note.contains("Ac-0, Ac-1, Ac-2 and 1,497 more"), "{note}");
    assert!(note.len() < 300, "{} bytes", note.len());
}

/// A spectrum with the Cs-137 and Co-60 peaks marked, counted for no time.
fn marked_without_live_time() -> Spectrum {
    let mut s = calibrated_spectrum();
    s.live_time = 0.0;
    for (low, high) in [(650, 674), (1161, 1185), (1320, 1344)] {
        s.rois.mark(Roi::new(low, high));
    }
    s
}

#[test]
fn nuclides_withheld_for_one_global_reason_form_one_group() {
    let report = analyse(
        &marked_without_live_time(),
        &NuclideLibrary::sample_for_tests(),
        &options(),
    );
    let names: Vec<&str> = report.withheld.iter().map(|w| w.nuclide.as_str()).collect();
    assert_eq!(names, ["Co-60", "Cs-137"]);
    let groups: Vec<&[mantaray_core::Withheld]> = report.withheld_groups().collect();
    assert_eq!(groups.len(), 1, "{:?}", report.withheld);
    assert_eq!(mantaray_core::Withheld::names(groups[0]), "Co-60, Cs-137");
    // A single withheld nuclide is a group of one.
    let mut s = marked_without_live_time();
    s.rois = Default::default();
    s.rois.mark(Roi::new(650, 674));
    let report = analyse(&s, &NuclideLibrary::sample_for_tests(), &options());
    assert_eq!(
        report.withheld_groups().map(<[_]>::len).collect::<Vec<_>>(),
        [1]
    );
}

#[test]
fn a_withheld_nuclide_and_every_note_are_one_line_when_they_are_built() {
    let w = mantaray_core::Withheld::new("Cs-137\n fresh", "no activity:\r\n  the curve\tis zero");
    assert_eq!(w.nuclide, "Cs-137 fresh");
    assert_eq!(w.reason, "no activity: the curve is zero");
    // A library name with a line break quoted in a note and in a withheld row.
    let mut lib = NuclideLibrary::new("awkward");
    lib.push(Nuclide::new(
        "Far\nAway",
        1.0e9,
        vec![LibraryPeak::new(5000.0, 50.0).key()],
    ));
    lib.push(Nuclide::new(
        "Cs\n137",
        1.0e9,
        vec![LibraryPeak::new(661.657, 85.1).key()],
    ));
    let opts = AnalysisOptions {
        report_mda_for_absent_nuclides: true,
        ..options()
    };
    let mut s = marked_without_live_time();
    s.live_time = 1000.0;
    let report = analyse(&s, &lib, &opts);
    assert!(
        report.notes.iter().any(|note| note.contains("Far Away")),
        "{:?}",
        report.notes
    );
    assert!(report.notes.iter().all(|note| !note.contains('\n')));
    let report = analyse(&marked_without_live_time(), &lib, &options());
    assert_eq!(report.withheld.len(), 1);
    assert_eq!(report.withheld[0].nuclide, "Cs 137");
}

#[test]
fn a_reference_date_after_the_count_that_underflows_the_correction_is_left_out_by_name() {
    // Reference date three days after the count: ~1400 Tl-208 half-lives the
    // other way. The correction underflowed to 0.0, which is finite, and the
    // nuclide was reported detected at exactly 0 Bq, +/- 0, MDA 0, no note.
    let mut s = peaked(4096, &[(2614.511, 10_000.0)]);
    s.rois.mark(Roi::new(2602, 2627));
    let lib = NuclideLibrary::sample_for_tests();
    let report = analyse(&s, &lib, &referred_to(2026, 7, 4));
    let tl = report
        .nuclides
        .iter()
        .find(|n| n.nuclide == "Tl-208")
        .expect("Tl-208 is reported");
    assert!(
        tl.activity > 0.0 && tl.uncertainty > 0.0,
        "{} +/- {}",
        tl.activity,
        tl.uncertainty
    );
    assert!(
        report
            .notes
            .iter()
            .any(|note| note.contains("reference date") && note.contains("Tl-208")),
        "{:?}",
        report.notes
    );
    let plain = analyse(&s, &lib, &options());
    let plain = plain
        .nuclides
        .iter()
        .find(|n| n.nuclide == "Tl-208")
        .unwrap();
    assert!((tl.activity - plain.activity).abs() <= 1e-9 * plain.activity);
}

#[test]
fn an_activity_that_overflows_is_withheld_with_a_note_and_not_reported_as_nan() {
    // A correction of ~1e307 is finite, but the activity built from it is
    // not: it came out infinite, its uncertainty NaN, and the total with it.
    let mut s = peaked(2048, &[(1173.228, 30_000.0), (1332.492, 27_000.0)]);
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
    // 1020 days at a one-day half-life.
    let report = analyse(&s, &lib, &referred_to(2023, 9, 15));
    for n in &report.nuclides {
        assert!(
            n.activity.is_finite() && n.uncertainty.is_finite() && n.mda.is_none_or(f64::is_finite),
            "{} {} {:?}",
            n.activity,
            n.uncertainty,
            n.mda
        );
    }
    assert!(report.total_activity().is_some_and(f64::is_finite));
    assert!(
        report.notes.iter().any(|note| note.contains("Hot-1"))
            || report.withheld.iter().any(|w| w.nuclide == "Hot-1"),
        "{:?} {:?}",
        report.notes,
        report.withheld
    );
}

#[test]
fn a_line_with_no_efficiency_does_not_pull_the_fallback_mean_down() {
    // The only line inside the curve is left out of the weighted mean, so the
    // mean falls back to a plain average. The line past the curve's last
    // non-zero point measured nothing and must not count as a zero.
    let mut s = peaked(2048, &[(661.657, 50_000.0), (1173.228, 30_000.0)]);
    s.rois.mark(Roi::new(650, 674));
    s.rois.mark(Roi::new(1161, 1185));
    let mut live = LibraryPeak::new(661.657, 85.1);
    live.not_in_average = true;
    let mut lib = NuclideLibrary::new("two");
    lib.push(Nuclide::new(
        "Two-1",
        1.0e12,
        vec![live, LibraryPeak::new(1173.228, 99.85)],
    ));
    let opts = AnalysisOptions {
        efficiency: Some(EfficiencyCalibration::interpolated(vec![
            EfficiencyPoint::new(100.0, 0.05),
            EfficiencyPoint::new(1000.0, 0.05),
            EfficiencyPoint::new(1100.0, 0.0),
        ])),
        ..AnalysisOptions::default()
    };
    let report = analyse(&s, &lib, &opts);
    let n = &report.nuclides[0];
    let live_line = n
        .lines
        .iter()
        .find(|l| l.efficiency.is_some_and(|e| e > 0.0))
        .unwrap();
    assert!(live_line.activity > 0.0);
    assert!(
        (n.activity - live_line.activity).abs() <= 1e-6 * live_line.activity,
        "mean {} vs the one measuring line {}",
        n.activity,
        live_line.activity
    );
}

/// A library of one nuclide with a one-day half life, one key line per energy.
fn hot_library(lines: &[(&str, f64, f64)]) -> NuclideLibrary {
    let mut lib = NuclideLibrary::new("hot");
    for &(name, energy, yield_percent) in lines {
        lib.push(Nuclide::new(
            name,
            86_400.0,
            vec![LibraryPeak::new(energy, yield_percent).key()],
        ));
    }
    lib
}

/// Options that refer the activity `days` after the count (negative: before),
/// for a sample of `quantity`.
fn referred_days(
    s: &Spectrum,
    days: i64,
    quantity: f64,
    efficiency: Option<EfficiencyCalibration>,
) -> AnalysisOptions {
    AnalysisOptions {
        efficiency,
        sample: SampleInfo {
            quantity,
            reference_date: s.start_time.map(|t| t + chrono::Duration::days(days)),
            ..SampleInfo::default()
        },
        ..AnalysisOptions::default()
    }
}

fn one_line_spectrum() -> Spectrum {
    let mut s = peaked(2048, &[(1173.228, 30_000.0)]);
    s.rois.mark(Roi::new(1161, 1185));
    s
}

/// The sample quantity that puts the single Hot-1 line's `pick` figure at
/// `target`, found from a run at quantity 1: the figures scale as 1/quantity.
fn quantity_for(days: i64, target: f64, pick: impl Fn(&mantaray_core::LineResult) -> f64) -> f64 {
    let s = one_line_spectrum();
    let lib = hot_library(&[("Hot-1", 1173.228, 99.85)]);
    let opts = referred_days(&s, days, 1.0, Some(EfficiencyCalibration::constant(0.05)));
    let report = analyse(&s, &lib, &opts);
    pick(&report.nuclides[0].lines[0]) / target
}

#[test]
fn a_count_rate_keeps_its_value_when_the_decay_correction_ruins_the_detection_limit() {
    // No efficiency curve: the value is the count rate, which the decay
    // correction never touches - and neither is its detection limit, which is
    // the count rate's own. A correction of 1e304 on a tiny sample made the
    // limit infinite and withheld the whole finite count rate.
    let s = one_line_spectrum();
    let lib = hot_library(&[("Hot-1", 1173.228, 99.85)]);
    let report = analyse(&s, &lib, &referred_days(&s, -1010, 1e-9, None));
    assert_eq!(report.nuclides.len(), 1, "{:?}", report.notes);
    let hot = &report.nuclides[0];
    let plain = analyse(&s, &lib, &referred_days(&s, 0, 1.0, None));
    assert_eq!(hot.activity, plain.nuclides[0].activity);
    assert!(hot.activity > 0.0 && hot.uncertainty > 0.0);
    assert_eq!(hot.mda, plain.nuclides[0].mda, "the same limit");
    assert!(hot.mda.is_some());
    assert!(
        !report
            .notes
            .iter()
            .any(|note| note.contains("detection limit")),
        "{:?}",
        report.notes
    );
}

#[test]
fn the_count_rate_mode_detection_limit_is_the_plain_currie_limit_per_second() {
    // No efficiency curve: the activity column is the raw count rate, so the
    // limit is the count rate's: no decay correction (the reference date is
    // two half-lives off), no division by the sample quantity (2 g).
    let s = one_line_spectrum();
    let lib = hot_library(&[("Hot-1", 1173.228, 99.85)]);
    let report = analyse(&s, &lib, &referred_days(&s, 2, 2.0, None));
    let hot = &report.nuclides[0];
    let background = report.peaks[0].info.background;
    let expected = mantaray_core::mda_currie(background, 1.0, 1.0, s.live_time);
    assert_eq!(hot.mda, Some(expected), "{:?}", report.notes);
    assert_eq!(hot.lines[0].mda, Some(expected));
}

#[test]
fn a_detection_limit_that_underflows_is_not_shown_as_a_value() {
    // The detection limit is about 1.7 times smaller than the uncertainty, so
    // there is a window where it has lost its digits and the activity and its
    // uncertainty have not.
    let q = quantity_for(1020, 1.7e-308, |line| line.mda.unwrap());
    let s = one_line_spectrum();
    let lib = hot_library(&[("Hot-1", 1173.228, 99.85)]);
    let opts = referred_days(&s, 1020, q, Some(EfficiencyCalibration::constant(0.05)));
    let report = analyse(&s, &lib, &opts);
    let hot = &report.nuclides[0];
    assert!(hot.activity.is_normal() && hot.uncertainty.is_normal());
    assert_eq!(hot.mda, None);
    assert!(hot.lines.iter().all(|line| line.mda.is_none()));
    assert!(
        report
            .notes
            .iter()
            .any(|note| note.contains("detection limit")),
        "{:?}",
        report.notes
    );
}

#[test]
fn an_uncertainty_that_underflows_is_not_shown_as_plus_or_minus_zero() {
    let q = quantity_for(1020, 1.5e-308, |line| line.activity_uncertainty);
    let s = one_line_spectrum();
    let lib = hot_library(&[("Hot-1", 1173.228, 99.85)]);
    let opts = referred_days(&s, 1020, q, Some(EfficiencyCalibration::constant(0.05)));
    let report = analyse(&s, &lib, &opts);
    for n in &report.nuclides {
        assert!(
            n.uncertainty.is_normal(),
            "{} +/- {:e}",
            n.activity,
            n.uncertainty
        );
    }
    assert!(
        report.withheld.iter().any(|w| w.nuclide == "Hot-1"
            && w.reason.contains("uncertainty")
            && w.reason.contains("too small")),
        "{:?}",
        report.withheld
    );
}

#[test]
fn a_subnormal_activity_is_refused_and_the_note_names_the_quantity_and_the_way() {
    let q = quantity_for(1020, 1.5e-308, |line| line.activity);
    let s = one_line_spectrum();
    let lib = hot_library(&[("Hot-1", 1173.228, 99.85)]);
    let opts = referred_days(&s, 1020, q, Some(EfficiencyCalibration::constant(0.05)));
    let report = analyse(&s, &lib, &opts);
    assert!(report.nuclides.is_empty(), "{:?}", report.nuclides);
    assert!(
        report.withheld.iter().any(|w| w.nuclide == "Hot-1"
            && w.reason.contains("activity is too small")
            && w.reason.contains("decay correction")
            && w.reason.contains("reference date")),
        "{:?}",
        report.withheld
    );
}

#[test]
fn two_nuclides_whose_total_is_too_large_show_no_total_and_say_so() {
    let mut s = peaked(2048, &[(1173.228, 30_000.0), (1332.492, 27_000.0)]);
    s.rois.mark(Roi::new(1161, 1185));
    s.rois.mark(Roi::new(1320, 1344));
    let lib = hot_library(&[("Hot-A", 1173.228, 99.85), ("Hot-B", 1332.492, 99.98)]);
    let curve = Some(EfficiencyCalibration::constant(0.05));
    let at_one = analyse(&s, &lib, &referred_days(&s, -900, 1.0, curve.clone()));
    // Each activity 1.2e308: a double each, but 2.4e308 together.
    let q = at_one.nuclides[0].activity / 1.2e308;
    let report = analyse(&s, &lib, &referred_days(&s, -900, q, curve));
    assert_eq!(report.nuclides.len(), 2, "{:?}", report.notes);
    for n in &report.nuclides {
        assert!(n.detected && n.activity.is_normal(), "{}", n.activity);
    }
    assert!(
        report
            .nuclides
            .iter()
            .map(|n| n.activity)
            .sum::<f64>()
            .is_infinite()
    );
    assert_eq!(report.total_activity(), None);
    assert!(
        report.notes.iter().any(|note| note.contains("total")),
        "{:?}",
        report.notes
    );
}

#[test]
fn a_line_that_is_not_in_the_average_and_overflows_does_not_withhold_the_nuclide() {
    // The 661.7 keV line has a library yield so small that its activity
    // overflows. It is not in the average, and the 1173 keV line is: the
    // nuclide is reported from that one, and the other line is left out by name.
    let mut s = peaked(2048, &[(661.657, 50_000.0), (1173.228, 30_000.0)]);
    s.rois.mark(Roi::new(650, 674));
    s.rois.mark(Roi::new(1161, 1185));
    let mut bad = LibraryPeak::new(661.657, 1e-310);
    bad.not_in_average = true;
    let mut lib = NuclideLibrary::new("two");
    lib.push(Nuclide::new(
        "Two-1",
        1.0e12,
        vec![bad, LibraryPeak::new(1173.228, 99.85)],
    ));
    let report = analyse(&s, &lib, &options());
    assert_eq!(report.nuclides.len(), 1, "{:?}", report.notes);
    let n = &report.nuclides[0];
    assert_eq!(n.lines.len(), 1);
    assert!((n.lines[0].energy - 1173.228).abs() < 1e-9);
    assert!(n.activity.is_normal() && n.uncertainty.is_normal());
    assert!(
        report
            .notes
            .iter()
            .any(|note| note.contains("661.7") && note.contains("Two-1")),
        "{:?}",
        report.notes
    );
}

#[test]
fn a_line_that_is_in_the_average_and_overflows_still_withholds_the_nuclide() {
    let mut s = peaked(2048, &[(1173.228, 30_000.0)]);
    s.rois.mark(Roi::new(1161, 1185));
    let lib = hot_library(&[("Hot-1", 1173.228, 1e-310)]);
    let report = analyse(&s, &lib, &options());
    assert!(report.nuclides.is_empty());
    assert!(
        report
            .withheld
            .iter()
            .any(|w| w.nuclide == "Hot-1" && w.reason.contains("activity is too large")),
        "{:?}",
        report.withheld
    );
}

/// A spectrum with Gaussian peaks at 661.657 and 1173.228 keV, both marked.
fn two_peak_spectrum() -> Spectrum {
    let mut s = peaked(2048, &[(661.657, 50_000.0), (1173.228, 30_000.0)]);
    s.rois.mark(Roi::new(650, 674));
    s.rois.mark(Roi::new(1161, 1185));
    s
}

#[test]
fn a_peak_is_detected_when_it_is_there_whether_or_not_its_line_is_reported() {
    // The 661.7 keV key line overflows and is not in the average, so it is
    // left out of the nuclide's lines. The peak is plainly there (50 000
    // counts), and "detected" asks the peak, not the lines that were kept.
    let mut bad = LibraryPeak::new(661.657, 1e-310).key();
    bad.not_in_average = true;
    let mut lib = NuclideLibrary::new("two");
    lib.push(Nuclide::new(
        "Two-1",
        1.0e12,
        vec![bad, LibraryPeak::new(1173.228, 99.85)],
    ));
    let report = analyse(&two_peak_spectrum(), &lib, &options());
    let n = &report.nuclides[0];
    assert_eq!(n.lines.len(), 1, "the overflowing line is left out");
    assert!(n.detected, "a 50 000-count key peak is detected");
}

#[test]
fn a_line_with_no_uncertainty_to_weigh_by_does_not_withhold_a_nuclide_its_other_line_measures() {
    // The 661.7 keV region holds no counts at all: net area 0, uncertainty 0.
    // With a library yield of 1e-310 its activity is 0 * infinity, NaN. The
    // weighted mean skips a line without an uncertainty, so this line is not
    // in the mean and cannot be what withholds the nuclide.
    let mut s = peaked(2048, &[(1173.228, 30_000.0)]);
    for c in &mut s.channels[600..720] {
        *c = 0;
    }
    s.rois.mark(Roi::new(650, 674));
    s.rois.mark(Roi::new(1161, 1185));
    let mut lib = NuclideLibrary::new("two");
    lib.push(Nuclide::new(
        "Two-1",
        1.0e12,
        vec![
            LibraryPeak::new(661.657, 1e-310),
            LibraryPeak::new(1173.228, 99.85),
        ],
    ));
    let report = analyse(&s, &lib, &options());
    assert_eq!(report.nuclides.len(), 1, "{:?}", report.notes);
    let n = &report.nuclides[0];
    assert!(n.activity.is_normal() && n.activity > 0.0, "{}", n.activity);
}

#[test]
fn a_decay_correction_for_the_count_is_blamed_on_the_count_not_the_reference_date() {
    // A half life of 1e-300 s makes the correction for the decay during the
    // count about 7e302, and with a small sample the activity overflows. The
    // reference date equals the count's start, so its factor is exactly 1:
    // the note must not send the operator to check it.
    let s = one_line_spectrum();
    let mut lib = NuclideLibrary::new("fast");
    lib.push(Nuclide::new(
        "Hot-1",
        1e-300,
        vec![LibraryPeak::new(1173.228, 99.85).key()],
    ));
    let opts = referred_days(&s, 0, 1e-5, Some(EfficiencyCalibration::constant(0.05)));
    let report = analyse(&s, &lib, &opts);
    assert!(report.nuclides.is_empty(), "{:?}", report.nuclides);
    let note = &report
        .withheld
        .iter()
        .find(|w| w.nuclide == "Hot-1")
        .unwrap_or_else(|| panic!("{:?}", report.withheld))
        .reason;
    assert!(note.contains("for the count"), "{note}");
    assert!(!note.contains("reference date"), "{note}");
}

#[test]
fn a_nuclide_withheld_for_another_reason_still_says_its_reference_correction_was_left_out() {
    // 3 days at a 183 s half life: the reference-date correction cannot be
    // computed and is left out. The nuclide is then withheld anyway (its
    // library yield overflows the activity). Its reason says the correction
    // was left out too, and the note about the reference date names it.
    let s = peaked(4096, &[(2614.511, 10_000.0)]);
    let mut s = s;
    s.rois.mark(Roi::new(2602, 2627));
    let mut lib = NuclideLibrary::new("fast");
    lib.push(Nuclide::new(
        "Hot-1",
        183.0,
        vec![LibraryPeak::new(2614.511, 1e-310).key()],
    ));
    let report = analyse(&s, &lib, &referred_to(2026, 6, 28));
    assert!(report.nuclides.is_empty(), "{:?}", report.nuclides);
    assert!(
        report.withheld.iter().any(|w| w.nuclide == "Hot-1"
            && w.reason.starts_with("no activity")
            && w.reason.contains("reference date")
            && w.reason.contains("left out")),
        "{:?}",
        report.withheld
    );
    assert!(
        report.notes.iter().any(|note| note
            .starts_with("decay correction to the reference date left out for")
            && note.contains("Hot-1")),
        "{:?}",
        report.notes
    );
}

#[test]
fn a_withheld_nuclide_leaves_one_note_and_none_about_its_lines() {
    // The 661.7 keV line overflows but is not in the average: on its own that
    // is a note about a dropped line. The 1332.5 keV line overflows and is in
    // the average, so the nuclide has no row, and the dropped line's note
    // would describe a row that does not exist.
    let mut s = peaked(
        2048,
        &[
            (661.657, 50_000.0),
            (1173.228, 30_000.0),
            (1332.492, 27_000.0),
        ],
    );
    for (low, high) in [(650, 674), (1161, 1185), (1320, 1344)] {
        s.rois.mark(Roi::new(low, high));
    }
    let mut dropped = LibraryPeak::new(661.657, 1e-310);
    dropped.not_in_average = true;
    let mut lib = NuclideLibrary::new("three");
    lib.push(Nuclide::new(
        "Three-1",
        1.0e12,
        vec![
            dropped,
            LibraryPeak::new(1173.228, 99.85),
            LibraryPeak::new(1332.492, 1e-310),
        ],
    ));
    let report = analyse(&s, &lib, &options());
    assert!(report.nuclides.is_empty());
    // One reason in the withheld list and no note about a row that does not exist.
    let about: Vec<&mantaray_core::Withheld> = report
        .withheld
        .iter()
        .filter(|w| w.nuclide == "Three-1")
        .collect();
    assert_eq!(about.len(), 1, "{about:?}");
    assert!(about[0].reason.starts_with("no activity"), "{about:?}");
    assert!(
        !report.notes.iter().any(|note| note.contains("Three-1")),
        "{:?}",
        report.notes
    );
}

#[test]
fn the_total_says_when_it_has_no_value_and_is_zero_only_when_nothing_was_detected() {
    // Nothing detected: a real total of zero, not a missing one.
    let s = one_line_spectrum();
    let empty = NuclideLibrary::new("none");
    let report = analyse(&s, &empty, &options());
    assert_eq!(report.total_activity(), Some(0.0));
    assert!(report.notes.iter().all(|note| !note.contains("total")));
    // An ordinary report: the sum of the detected activities.
    let report = analyse(
        &calibrated_spectrum_marked(),
        &NuclideLibrary::sample_for_tests(),
        &options(),
    );
    let sum: f64 = report.detected().map(|n| n.activity).sum();
    assert!(sum > 0.0);
    assert_eq!(report.total_activity(), Some(sum));
    assert!(report.notes.iter().all(|note| !note.contains("total")));
}

fn calibrated_spectrum_marked() -> Spectrum {
    let mut s = calibrated_spectrum();
    s.rois.mark(Roi::new(650, 674));
    s.rois.mark(Roi::new(1161, 1185));
    s
}

fn no_live_time_notes(report: &mantaray_core::QuantitativeReport) -> usize {
    report
        .notes
        .iter()
        .filter(|note| note.contains("no live time"))
        .count()
}

#[test]
fn a_live_time_that_is_not_a_number_is_no_live_time() {
    // NaN, infinity and a negative time are each "the spectrum has no live
    // time": one global note, never a "not a number" reason per nuclide.
    for bad in [f64::NAN, f64::INFINITY, -5.0] {
        let mut s = calibrated_spectrum();
        s.live_time = bad;
        for (low, high) in [(650, 674), (1161, 1185), (1320, 1344)] {
            s.rois.mark(Roi::new(low, high));
        }
        let report = analyse(&s, &NuclideLibrary::sample_for_tests(), &options());
        assert!(report.nuclides.is_empty(), "{bad}: {:?}", report.nuclides);
        // Said once, by the withheld group; no second note repeats it.
        assert_eq!(no_live_time_notes(&report), 0, "{bad}: {:?}", report.notes);
        assert!(!report.withheld.is_empty(), "{bad}");
        for w in &report.withheld {
            assert!(
                w.reason.contains("no live time") && !w.reason.contains("not a number"),
                "{bad}: {w:?}"
            );
        }
        assert!(
            !report.notes.iter().any(|n| n.contains("not a number")),
            "{bad}: {:?}",
            report.notes
        );
    }
}

#[test]
fn a_curve_that_gives_no_efficiency_leaves_no_efficiency_on_the_lines() {
    // The count rates stand in for activities; the efficiency of a line is
    // missing, not a NaN or a negative number.
    let mut s = calibrated_spectrum();
    s.rois.mark(Roi::new(650, 674));
    let lib = NuclideLibrary::sample_for_tests();
    for curve in [
        EfficiencyCalibration::constant(f64::NAN),
        EfficiencyCalibration::constant(-0.1),
        EfficiencyCalibration::constant(0.0),
        EfficiencyCalibration::interpolated(vec![]),
    ] {
        let opts = AnalysisOptions {
            efficiency: Some(curve.clone()),
            ..AnalysisOptions::default()
        };
        let report = analyse(&s, &lib, &opts);
        let lines: Vec<_> = report.nuclides.iter().flat_map(|n| &n.lines).collect();
        assert!(!lines.is_empty(), "{curve:?}");
        for line in lines {
            assert_eq!(line.efficiency, None, "{curve:?}");
        }
    }
}

#[test]
fn the_empty_curve_note_names_what_is_wrong_with_the_curve() {
    use mantaray_core::EfficiencyModel;
    let mut s = calibrated_spectrum();
    s.rois.mark(Roi::new(650, 674));
    let lib = NuclideLibrary::sample_for_tests();
    let polynomial = |coefficients: Vec<f64>| EfficiencyCalibration {
        model: EfficiencyModel::LogLogPolynomial(coefficients),
        points: Vec::new(),
    };
    let cases = [
        (EfficiencyCalibration::constant(0.0), "constant"),
        (EfficiencyCalibration::constant(f64::NAN), "constant"),
        (EfficiencyCalibration::interpolated(vec![]), "no point"),
        (polynomial(vec![]), "no coefficients"),
        (polynomial(vec![-3.0, f64::NAN]), "not a finite number"),
    ];
    for (curve, cause) in cases {
        let opts = AnalysisOptions {
            efficiency: Some(curve.clone()),
            ..AnalysisOptions::default()
        };
        let report = analyse(&s, &lib, &opts);
        let note = report
            .notes
            .iter()
            .find(|note| note.contains("gives no efficiency"))
            .unwrap_or_else(|| panic!("{curve:?}: {:?}", report.notes));
        assert!(note.contains(cause), "{curve:?}: {note}");
    }
    // The wrong cause the note used to give for every model.
    let report = analyse(
        &s,
        &lib,
        &AnalysisOptions {
            efficiency: Some(polynomial(vec![])),
            ..AnalysisOptions::default()
        },
    );
    assert!(!report.notes.iter().any(|n| n.contains("above zero")));
}

#[test]
fn a_long_list_of_names_has_the_count_of_the_rest_with_thousands_separators() {
    let withheld: Vec<mantaray_core::Withheld> = (0..1_284)
        .map(|n| mantaray_core::Withheld::new(&format!("N-{n}"), "no activity"))
        .collect();
    let names = mantaray_core::Withheld::names(&withheld);
    assert!(names.starts_with("N-0, N-1, "), "{names}");
    assert!(names.ends_with(" and 1,274 more"), "{names}");
}

fn three_lines_marked() -> Spectrum {
    let mut s = calibrated_spectrum();
    s.rois.mark(Roi::new(650, 674));
    s.rois.mark(Roi::new(1161, 1185));
    s.rois.mark(Roi::new(1320, 1344));
    s
}

fn curve_through(points: &[(f64, f64)]) -> EfficiencyCalibration {
    EfficiencyCalibration::interpolated(
        points
            .iter()
            .map(|&(energy, efficiency)| EfficiencyPoint::new(energy, efficiency))
            .collect(),
    )
}

#[test]
fn a_curve_that_gives_infinity_at_every_line_withholds_by_name_not_as_zero() {
    // The curve has a usable point, so it is a curve (not the count-rate
    // fallback), and it gives infinity above 500 keV. That is not "no
    // efficiency": the reason names the energy and what the curve gave.
    let opts = AnalysisOptions {
        efficiency: Some(curve_through(&[(100.0, 0.05), (500.0, f64::INFINITY)])),
        ..AnalysisOptions::default()
    };
    let report = analyse(
        &three_lines_marked(),
        &NuclideLibrary::sample_for_tests(),
        &opts,
    );
    let reason = |name: &str| {
        report
            .withheld
            .iter()
            .find(|w| w.nuclide == name)
            .unwrap_or_else(|| panic!("{name} withheld: {:?} {:?}", report.withheld, report.notes))
            .reason
            .clone()
    };
    assert_eq!(
        reason("Cs-137"),
        "no activity: the efficiency curve gives infinity at 661.7 keV"
    );
    let co = reason("Co-60");
    assert!(
        co.contains("the efficiency curve gives infinity at 1173.2 keV"),
        "{co}"
    );
    assert!(
        co.contains("the efficiency curve gives infinity at 1332.5 keV"),
        "{co}"
    );
    assert!(!co.contains("is zero"), "{co}");
    assert!(report.nuclides.is_empty(), "{:?}", report.nuclides);
}

#[test]
fn a_line_the_curve_gives_a_bad_number_at_is_dropped_with_a_note_and_the_rest_report() {
    // Finite up to 1200 keV, infinity from 1300: Co-60's 1332.5 keV line is
    // left out by name and its 1173.2 keV line reports.
    let opts = AnalysisOptions {
        efficiency: Some(curve_through(&[
            (100.0, 0.05),
            (1200.0, 0.02),
            (1300.0, f64::INFINITY),
        ])),
        ..AnalysisOptions::default()
    };
    let report = analyse(
        &three_lines_marked(),
        &NuclideLibrary::sample_for_tests(),
        &opts,
    );
    let co = report
        .nuclides
        .iter()
        .find(|n| n.nuclide == "Co-60")
        .unwrap_or_else(|| panic!("{:?} {:?}", report.withheld, report.notes));
    assert_eq!(co.lines.len(), 1, "{:?}", co.lines);
    assert!((co.lines[0].energy - 1173.228).abs() < 1e-6);
    assert!(
        report
            .notes
            .iter()
            .any(|note| note.contains("1332.5 keV of Co-60 left out")
                && note.contains("the efficiency curve gives infinity at 1332.5 keV")),
        "{:?}",
        report.notes
    );
    assert!(co.activity.is_finite() && co.activity > 0.0);
}

#[test]
fn a_negative_efficiency_is_named_as_negative() {
    let opts = AnalysisOptions {
        efficiency: Some(curve_through(&[(100.0, 0.05), (2000.0, -0.05)])),
        ..AnalysisOptions::default()
    };
    let report = analyse(
        &three_lines_marked(),
        &NuclideLibrary::sample_for_tests(),
        &opts,
    );
    let co = report
        .withheld
        .iter()
        .find(|w| w.nuclide == "Co-60")
        .unwrap_or_else(|| panic!("{:?} {:?}", report.withheld, report.notes));
    assert!(
        co.reason.contains("gives a negative number"),
        "{}",
        co.reason
    );
    assert!(co.reason.contains("at 1173.2 keV"), "{}", co.reason);
}

#[test]
fn the_total_is_computed_in_one_pass_and_keeps_its_cancellation_rule() {
    // Detected nuclides of +x and -x total a real zero; one of them alone
    // does not make one.
    let nuclide = |name: &str, activity: f64| mantaray_core::NuclideResult {
        nuclide: name.into(),
        activity,
        uncertainty: 1.0,
        mda: None,
        detected: true,
        half_life_seconds: 1.0e9,
        lines: Vec::new(),
    };
    let report = |nuclides: Vec<mantaray_core::NuclideResult>| {
        let mut report = analyse(
            &Spectrum::from_counts(vec![0; 64]),
            &NuclideLibrary::new("none"),
            &AnalysisOptions::default(),
        );
        report.nuclides = nuclides;
        report
    };
    assert_eq!(
        report(vec![nuclide("A", 3.0), nuclide("B", -3.0)]).total_activity(),
        Some(0.0)
    );
    assert_eq!(report(vec![]).total_activity(), Some(0.0));
    assert_eq!(
        report(vec![nuclide("A", 2.0), nuclide("B", 3.0)]).total_activity(),
        Some(5.0)
    );
}

#[test]
fn a_log_log_curve_that_overflows_is_named_infinity_not_taken_for_zero() {
    use mantaray_core::EfficiencyModel;
    // exp(800) is infinity at every energy. It used to come back from the
    // curve as 0.0 and the nuclide was withheld as "the curve or the yield is
    // zero", which is not what the curve said.
    let opts = AnalysisOptions {
        efficiency: Some(EfficiencyCalibration {
            model: EfficiencyModel::LogLogPolynomial(vec![800.0]),
            points: Vec::new(),
        }),
        ..AnalysisOptions::default()
    };
    let report = analyse(
        &three_lines_marked(),
        &NuclideLibrary::sample_for_tests(),
        &opts,
    );
    let cs = report
        .withheld
        .iter()
        .find(|w| w.nuclide == "Cs-137")
        .unwrap_or_else(|| panic!("{:?} {:?}", report.withheld, report.notes));
    assert_eq!(
        cs.reason,
        "no activity: the efficiency curve gives infinity at 661.7 keV"
    );
    assert!(!cs.reason.contains("is zero"), "{}", cs.reason);
}

/// What a note says of Hot-1's bad half-life.
fn hot_half_life_words(half_life: f64) -> String {
    if half_life.is_nan() {
        "the library gives Hot-1 a half-life that is not a number".to_string()
    } else {
        format!("the library gives Hot-1 a half-life of {half_life} s")
    }
}

#[test]
fn a_degenerate_half_life_is_blamed_on_the_library_entry() {
    // Negative and NaN (zero and infinity are the library's "stable"): the
    // correction cannot be made, and neither the sample's reference date nor
    // "too many half-lives" is why.
    let s = one_line_spectrum();
    for half_life in [-5.0, f64::NAN, f64::NEG_INFINITY] {
        let mut lib = NuclideLibrary::new("odd");
        lib.push(Nuclide::new(
            "Hot-1",
            half_life,
            vec![LibraryPeak::new(1173.228, 99.85).key()],
        ));
        let curve = Some(EfficiencyCalibration::constant(0.05));
        let words = hot_half_life_words(half_life);

        // Reported, uncorrected, and the note says why.
        let report = analyse(&s, &lib, &referred_days(&s, 3, 1.0, curve.clone()));
        assert_eq!(report.nuclides.len(), 1, "{half_life}: {:?}", report.notes);
        assert!(
            report.notes.iter().any(|note| note.contains(&words)),
            "{half_life}: {:?}",
            report.notes
        );
        assert!(
            !report.notes.iter().any(|note| note.contains("half-lives")),
            "{half_life}: {:?}",
            report.notes
        );

        // Withheld for another reason (a quantity too small to divide by):
        // the reason still blames the entry.
        let report = analyse(&s, &lib, &referred_days(&s, 3, 1e-307, curve));
        let withheld = report
            .withheld
            .iter()
            .find(|w| w.nuclide == "Hot-1")
            .unwrap_or_else(|| panic!("{half_life}: {:?} {:?}", report.withheld, report.notes));
        assert!(
            withheld.reason.contains(&words),
            "{half_life}: {}",
            withheld.reason
        );
        assert!(
            !withheld.reason.contains("half-lives"),
            "{}",
            withheld.reason
        );
    }
}

#[test]
fn each_correction_that_breaks_the_figure_alone_is_named_separately() {
    // A figure near the top of the range (about 6e305) that the correction for
    // the decay during the count (about 3e5) breaks alone, and the correction
    // to the reference date one second back (about 1e130) breaks alone too.
    let s = one_line_spectrum();
    let mut lib = NuclideLibrary::new("fast");
    lib.push(Nuclide::new(
        "Hot-1",
        2.3e-3,
        vec![LibraryPeak::new(1173.228, 99.85).key()],
    ));
    let opts = AnalysisOptions {
        efficiency: Some(EfficiencyCalibration::constant(0.05)),
        sample: SampleInfo {
            quantity: 1e-303,
            reference_date: s.start_time.map(|t| t - chrono::Duration::seconds(1)),
            ..SampleInfo::default()
        },
        ..AnalysisOptions::default()
    };
    let report = analyse(&s, &lib, &opts);
    let reason = &report
        .withheld
        .iter()
        .find(|w| w.nuclide == "Hot-1")
        .unwrap_or_else(|| panic!("{:?} {:?}", report.withheld, report.notes))
        .reason;
    assert!(
        reason.contains(
            "the decay during the count and the decay to the reference date each break it"
        ),
        "{reason}"
    );
    assert!(!reason.contains("together"), "{reason}");
}

/// A library of a nuclide that is on the spectrum's line (`Hot-1`) and one that
/// is not (`Cold-1`, at 661.657 keV), both with `half_life` seconds.
fn hot_and_cold(half_life: f64) -> NuclideLibrary {
    let mut lib = NuclideLibrary::new("odd");
    lib.push(Nuclide::new(
        "Hot-1",
        half_life,
        vec![LibraryPeak::new(1173.228, 99.85).key()],
    ));
    lib.push(Nuclide::new(
        "Cold-1",
        half_life,
        vec![LibraryPeak::new(661.657, 85.0).key()],
    ));
    lib
}

fn with_decay_and_mda(mut options: AnalysisOptions) -> AnalysisOptions {
    options.correct_decay_during_count = true;
    options.decay_correct_to_reference = true;
    options.report_mda_for_absent_nuclides = true;
    options
}

#[test]
fn a_stable_or_unspecified_half_life_is_no_decay_found_or_absent() {
    // The library's own convention (library.rs): zero is "stable or
    // unspecified", infinity is stable. Neither is a bad entry, the figure is
    // uncorrected, and a FOUND nuclide says once that it was not corrected
    // (decay correction was asked for); an absent one says nothing.
    let s = one_line_spectrum();
    let curve = Some(EfficiencyCalibration::constant(0.05));
    let plain = analyse(
        &s,
        &hot_and_cold(86_400.0),
        &AnalysisOptions {
            report_mda_for_absent_nuclides: true,
            correct_decay_during_count: false,
            decay_correct_to_reference: false,
            ..referred_days(&s, 0, 1.0, curve.clone())
        },
    );
    for half_life in [0.0, f64::INFINITY] {
        let report = analyse(
            &s,
            &hot_and_cold(half_life),
            &with_decay_and_mda(referred_days(&s, 3, 1.0, curve.clone())),
        );
        let said: Vec<&String> = report
            .notes
            .iter()
            .filter(|note| note.contains("half-life") || note.contains("not decay-corrected"))
            .collect();
        let want = if half_life == 0.0 {
            "not decay-corrected: the library gives no half-life for Hot-1"
        } else {
            "not decay-corrected: the library lists Hot-1 as stable"
        };
        assert_eq!(said, vec![want], "{half_life}: {:?}", report.notes);
        assert_eq!(report.nuclides.len(), 2, "{half_life}: {:?}", report.notes);
        for (got, want) in report.nuclides.iter().zip(&plain.nuclides) {
            assert_eq!(got.nuclide, want.nuclide);
            assert_eq!(got.activity, want.activity, "{half_life} {}", got.nuclide);
            assert_eq!(got.mda, want.mda, "{half_life} {}", got.nuclide);
        }

        // Decay correction not asked for: nothing to say.
        let report = analyse(
            &s,
            &hot_and_cold(half_life),
            &AnalysisOptions {
                report_mda_for_absent_nuclides: true,
                correct_decay_during_count: false,
                decay_correct_to_reference: false,
                ..referred_days(&s, 3, 1.0, curve.clone())
            },
        );
        assert!(
            !report
                .notes
                .iter()
                .any(|note| note.contains("half-life") || note.contains("not decay-corrected")),
            "{half_life}: {:?}",
            report.notes
        );
    }
}

#[test]
fn bad_half_lives_across_a_large_absent_library_are_said_once() {
    let s = one_line_spectrum();
    let curve = Some(EfficiencyCalibration::constant(0.05));
    let mut lib = NuclideLibrary::new("big");
    lib.push(Nuclide::new(
        "Hot-1",
        -2.0,
        vec![LibraryPeak::new(1173.228, 99.85).key()],
    ));
    for i in 0..200 {
        let half_life = if i % 2 == 0 { -1.0 } else { f64::NAN };
        lib.push(Nuclide::new(
            &format!("Cold-{i}"),
            half_life,
            vec![LibraryPeak::new(661.657, 85.0).key()],
        ));
    }
    let report = analyse(
        &s,
        &lib,
        &with_decay_and_mda(referred_days(&s, 3, 1.0, curve)),
    );
    let absent_notes: Vec<&String> = report
        .notes
        .iter()
        .filter(|note| note.contains("Cold-"))
        .collect();
    assert_eq!(absent_notes.len(), 1, "{absent_notes:?}");
    assert!(
        absent_notes[0].contains("200 library nuclides"),
        "{absent_notes:?}"
    );
    assert!(
        absent_notes[0].contains("Cold-0: -1 s, Cold-1: not a number, Cold-2: -1 s and 197 more"),
        "{absent_notes:?}"
    );
    // The found nuclide keeps its own note.
    assert!(
        report
            .notes
            .iter()
            .any(|note| note.contains("the library gives Hot-1 a half-life of -2 s")),
        "{:?}",
        report.notes
    );
}

#[test]
fn a_negative_or_nan_half_life_is_one_note_found_and_one_grouped_note_absent() {
    let s = one_line_spectrum();
    let curve = Some(EfficiencyCalibration::constant(0.05));
    for half_life in [-1.0, f64::NAN] {
        let report = analyse(
            &s,
            &hot_and_cold(half_life),
            &with_decay_and_mda(referred_days(&s, 3, 1.0, curve.clone())),
        );
        let words = hot_half_life_words(half_life);
        let said = report
            .notes
            .iter()
            .filter(|note| note.contains(&words))
            .count();
        assert_eq!(said, 1, "{half_life}: {:?}", report.notes);
        let grouped = report
            .notes
            .iter()
            .filter(|note| note.contains("Cold-1"))
            .count();
        assert_eq!(grouped, 1, "{half_life}: {:?}", report.notes);
        // Both rows are still there, uncorrected.
        assert_eq!(report.nuclides.len(), 2, "{half_life}: {:?}", report.notes);
        assert!(report.nuclides.iter().all(|n| n.mda.is_some()));
    }
}

#[test]
fn the_count_rate_detection_limit_is_in_the_same_units_found_or_absent() {
    // No curve: every figure is a count rate. The limit of a nuclide that was
    // found and of one that was not are both the plain per-second Currie
    // limit: the sample quantity (10 g) and the decay correction do not enter
    // either.
    let s = one_line_spectrum();
    let lib = hot_and_cold(86_400.0);
    let asked = |quantity: f64, decay: bool| {
        let mut options = referred_days(&s, 3, quantity, None);
        options.report_mda_for_absent_nuclides = true;
        options.correct_decay_during_count = decay;
        options.decay_correct_to_reference = decay;
        analyse(&s, &lib, &options)
    };
    let plain = asked(1.0, false);
    let loaded = asked(10.0, true);
    for name in ["Hot-1", "Cold-1"] {
        let limit = |report: &mantaray_core::QuantitativeReport| {
            report
                .nuclides
                .iter()
                .find(|n| n.nuclide == name)
                .unwrap_or_else(|| panic!("{name}: {:?} {:?}", report.notes, report.withheld))
                .mda
        };
        assert!(limit(&plain).is_some(), "{name}");
        assert_eq!(limit(&loaded), limit(&plain), "{name}");
    }
}

#[test]
fn an_absent_nuclide_with_a_bad_half_life_reaches_the_note_even_when_its_limit_is_refused() {
    // The line used for the limit lies outside the spectrum, so no detection
    // limit can be built; the library's half-life is still to blame for the
    // missing decay correction and the note names the nuclide.
    let s = one_line_spectrum();
    let curve = Some(EfficiencyCalibration::constant(0.05));
    let mut lib = NuclideLibrary::new("odd");
    lib.push(Nuclide::new(
        "Cold-1",
        f64::NAN,
        vec![LibraryPeak::new(99_999.0, 85.0).key()],
    ));
    let report = analyse(
        &s,
        &lib,
        &with_decay_and_mda(referred_days(&s, 3, 1.0, curve)),
    );
    assert!(report.nuclides.is_empty(), "{:?}", report.nuclides);
    assert!(
        report
            .notes
            .iter()
            .any(|note| note.contains("Cold-1: not a number")),
        "{:?}",
        report.notes
    );
}

#[test]
fn the_grouped_bad_half_life_note_keeps_each_value_for_up_to_three_names() {
    let s = one_line_spectrum();
    let curve = Some(EfficiencyCalibration::constant(0.05));
    let mut lib = NuclideLibrary::new("odd");
    for (name, half_life) in [("Cold-1", -1.0), ("Cold-2", f64::NAN)] {
        lib.push(Nuclide::new(
            name,
            half_life,
            vec![LibraryPeak::new(661.657, 85.0).key()],
        ));
    }
    let report = analyse(
        &s,
        &lib,
        &with_decay_and_mda(referred_days(&s, 3, 1.0, curve)),
    );
    assert!(
        report
            .notes
            .iter()
            .any(|note| note.contains("(Cold-1: -1 s, Cold-2: not a number)")),
        "{:?}",
        report.notes
    );
}

#[test]
fn a_huge_or_tiny_half_life_reads_in_scientific_notation_and_nan_in_words() {
    let s = one_line_spectrum();
    let curve = Some(EfficiencyCalibration::constant(0.05));
    // Found (Hot-1): the per-nuclide note.
    for (half_life, said) in [
        (-1e300, "a half-life of -1e300 s"),
        (-5e-5, "a half-life of -5e-5 s"),
        (-5.0, "a half-life of -5 s"),
        (f64::NAN, "a half-life that is not a number"),
    ] {
        let report = analyse(
            &s,
            &hot_and_cold(half_life),
            &with_decay_and_mda(referred_days(&s, 3, 1.0, curve.clone())),
        );
        assert!(
            report
                .notes
                .iter()
                .any(|note| note.contains(&format!("the library gives Hot-1 {said}"))),
            "{half_life}: {:?}",
            report.notes
        );
        // Absent (Cold-1): the grouped note, same words.
        let grouped = report
            .notes
            .iter()
            .find(|note| note.contains("Cold-1"))
            .unwrap_or_else(|| panic!("{half_life}: {:?}", report.notes));
        let value = said
            .strip_prefix("a half-life of ")
            .unwrap_or("not a number");
        assert!(
            grouped.contains(&format!("Cold-1: {value}")),
            "{half_life}: {grouped}"
        );
        assert!(
            !grouped.contains("NaN") && !grouped.contains("000000000"),
            "{grouped}"
        );
    }
}

#[test]
fn an_absent_nuclide_whose_limit_is_refused_keeps_the_suffix_and_counts_in_the_grouped_note() {
    let s = one_line_spectrum();
    let curve = Some(EfficiencyCalibration::constant(0.05));
    let mut lib = NuclideLibrary::new("odd");
    for name in ["Cold-1", "Cold-2"] {
        lib.push(Nuclide::new(
            name,
            -1.0,
            vec![LibraryPeak::new(99_999.0, 85.0).key()],
        ));
    }
    let report = analyse(
        &s,
        &lib,
        &with_decay_and_mda(referred_days(&s, 3, 1.0, curve)),
    );
    let note = report
        .notes
        .iter()
        .find(|note| note.contains("no detection limit"))
        .unwrap_or_else(|| panic!("{:?}", report.notes));
    // The half-life value is said in the grouped note alone, never in the
    // reason, so the two nuclides' equal reasons still group and each fact
    // is stated once.
    assert!(
        !note.contains("-1 s") && !note.contains("half-life"),
        "{note}"
    );
    // Equal reasons still group: one clause for both names.
    assert!(note.contains("2 library nuclides"), "{note}");
    assert!(
        report
            .notes
            .iter()
            .any(|note| note.contains("2 library nuclides not found")
                && note.contains("Cold-1: -1 s")),
        "{:?}",
        report.notes
    );
    let said = report
        .notes
        .iter()
        .filter(|note| note.contains("-1 s"))
        .count();
    assert_eq!(said, 1, "the value is said once: {:?}", report.notes);
}

// ---------------------------------------------------------------------------
// Mutation-testing gaps (cargo-mutants over d3f5686..39c121b). Every expected
// number below is worked out by hand from the MAESTRO Peak Info equations
// (17)-(21) in `analysis::peak_info` and the Currie limit in `mda_currie`,
// on spectra built so those sums come out exact.
//
// The box spectrum: a flat 100 counts per channel, 1 keV per channel, 1000 s
// live and real, and regions 21 channels wide with 3 background points at
// each end. For such a region, equations (17)-(21) give:
//   background B = (300 + 300) * 21 / 6           = 2100
//   adjusted gross (15 middle channels) G = 1500 + bump
//   net = G - B * 15 / 21                          = bump
//   variance = G + B * (15 / 6) * (15 / 21)        = G + 3750
// ---------------------------------------------------------------------------

/// A flat 100 counts per channel with the given extra counts added, 512
/// channels, 1 keV per channel from 0 keV, counted 1000 s live and real.
fn boxed(bumps: &[(usize, u64)]) -> Spectrum {
    let mut counts = vec![100u64; 512];
    for &(channel, extra) in bumps {
        counts[channel] += extra;
    }
    let mut s = Spectrum::from_counts(counts);
    s.live_time = 1000.0;
    s.real_time = 1000.0;
    s.energy_calibration = Some(EnergyCalibration::linear(0.0, 1.0));
    s.start_time = NaiveDate::from_ymd_opt(2026, 7, 1)
        .unwrap()
        .and_hms_opt(12, 0, 0);
    s
}

/// A 1000-count peak spread 250/500/250 over three channels centred on
/// `channel`: a net area of exactly 1000.
fn thousand_at(channel: usize) -> [(usize, u64); 3] {
    [(channel - 1, 250), (channel, 500), (channel + 1, 250)]
}

/// No decay corrections, so every activity is net / (live * eff * yield).
fn exact_options(efficiency: Option<EfficiencyCalibration>) -> AnalysisOptions {
    AnalysisOptions {
        efficiency,
        correct_decay_during_count: false,
        decay_correct_to_reference: false,
        ..AnalysisOptions::default()
    }
}

/// One nuclide per (name, keV) with one 50 % key line and a long half-life.
fn lines_at(lines: &[(&str, f64)]) -> NuclideLibrary {
    let mut lib = NuclideLibrary::new("box");
    for &(name, energy) in lines {
        lib.push(Nuclide::new(
            name,
            1.0e9,
            vec![LibraryPeak::new(energy, 50.0).key()],
        ));
    }
    lib
}

fn close(got: f64, want: f64) -> bool {
    (got - want).abs() <= 1e-9 * want.abs().max(1e-300)
}

#[test]
fn a_key_line_under_three_sigma_is_found_but_not_detected() {
    // Weak-1: a 200-count bump at 100 keV. Net 200, variance 1700 + 3750 =
    // 5450, sigma 73.82; 200 is not above 3 sigma (221.5), so not detected -
    // close enough under the threshold that 3 + sigma (76.8) or 3 / sigma
    // in its place would call it detected.
    // Strong-1: net 1000, variance 2500 + 3750 = 6250, sigma 79.06; 1000 is
    // above 237.2, so detected. Activities at 5 % and 50 %: net / 25.
    let mut s = boxed(&[(100, 200), (299, 250), (300, 500), (301, 250)]);
    s.rois.mark(Roi::new(90, 110));
    s.rois.mark(Roi::new(290, 310));
    let lib = lines_at(&[("Weak-1", 100.0), ("Strong-1", 300.0)]);
    let report = analyse(
        &s,
        &lib,
        &exact_options(Some(EfficiencyCalibration::constant(0.05))),
    );
    let get = |name: &str| {
        report
            .nuclides
            .iter()
            .find(|n| n.nuclide == name)
            .unwrap_or_else(|| panic!("{name}: {:?} {:?}", report.nuclides, report.withheld))
    };
    let weak = get("Weak-1");
    assert!(
        close(weak.lines[0].net_area, 200.0),
        "{}",
        weak.lines[0].net_area
    );
    assert!(close(weak.lines[0].net_uncertainty, 5450f64.sqrt()));
    assert!(close(weak.activity, 200.0 / 25.0), "{}", weak.activity);
    assert!(
        !weak.detected,
        "200 counts against 3 sigma = 221.5 is not a detection"
    );
    let strong = get("Strong-1");
    assert!(close(strong.activity, 1000.0 / 25.0), "{}", strong.activity);
    assert!(strong.detected);
    // The total counts the detected nuclide only: 40 Bq, not 48.
    let total = report.total_activity().expect("a total");
    assert!(close(total, 40.0), "{total}");
}

#[test]
fn an_absent_nuclides_detection_limit_is_the_currie_limit_at_its_yield() {
    // Nothing found; the MDA row for Absent-1 (200 keV, 50 %) is built from
    // the counts within 1.5 FWHM (4 channels with no shape calibration, so 6
    // channels) either side: channels 194..=206, 13 * 100 = 1300 counts.
    // Currie: (2.71 + 4.65 sqrt(1300)) / (0.05 * 0.50 * 1000 s) = 6.8147 Bq.
    let s = boxed(&[]);
    let lib = lines_at(&[("Absent-1", 200.0)]);
    let options = AnalysisOptions {
        report_mda_for_absent_nuclides: true,
        ..exact_options(Some(EfficiencyCalibration::constant(0.05)))
    };
    let report = analyse(&s, &lib, &options);
    let absent = &report.nuclides[0];
    assert_eq!(absent.nuclide, "Absent-1");
    assert!(!absent.detected);
    let want = (2.71 + 4.65 * 1300f64.sqrt()) / (0.05 * 0.50 * 1000.0);
    assert!((want - 6.814_725).abs() < 1e-6, "hand value {want}");
    let mda = absent.mda.expect("a limit");
    assert!(close(mda, want), "mda {mda}, want {want}");
}

#[test]
fn without_a_curve_the_count_rate_uncertainty_is_the_net_area_sigma_per_second() {
    // Net 1000 +/- sqrt(6250) = 79.057 counts in 1000 s: 1 +/- 0.079057 cps.
    let mut s = boxed(&thousand_at(100));
    s.rois.mark(Roi::new(90, 110));
    let report = analyse(&s, &lines_at(&[("Rate-1", 100.0)]), &exact_options(None));
    assert_eq!(report.activity_unit(), "cps");
    let rate = &report.nuclides[0];
    let sigma = 6250f64.sqrt() / 1000.0;
    assert!((sigma - 0.079_057).abs() < 1e-6, "hand value {sigma}");
    assert!(
        close(rate.lines[0].activity, 1.0),
        "{}",
        rate.lines[0].activity
    );
    assert!(
        close(rate.lines[0].activity_uncertainty, sigma),
        "{}",
        rate.lines[0].activity_uncertainty
    );
    // One line, no library uncertainty: the nuclide's spread is the line's.
    assert!(close(rate.uncertainty, sigma), "{}", rate.uncertainty);
}

#[test]
fn a_region_with_a_net_area_of_exactly_zero_reports_zero_and_is_not_withheld() {
    // A flat region: net = 1500 - 2100 * 15 / 21 = 0 exactly, a measured
    // zero. Sigma sqrt(1500 + 3750) = 72.457 counts, / 25 = 2.8983 Bq.
    let mut s = boxed(&[]);
    s.rois.mark(Roi::new(90, 110));
    let report = analyse(
        &s,
        &lines_at(&[("Flat-1", 100.0)]),
        &exact_options(Some(EfficiencyCalibration::constant(0.05))),
    );
    assert!(report.withheld.is_empty(), "{:?}", report.withheld);
    let flat = &report.nuclides[0];
    assert_eq!(flat.lines[0].net_area, 0.0);
    assert_eq!(flat.activity, 0.0);
    assert!(
        close(flat.uncertainty, 5250f64.sqrt() / 25.0),
        "{}",
        flat.uncertainty
    );
    assert!(!flat.detected);
}

#[test]
fn an_empty_region_reports_zero_plus_or_minus_zero_and_is_not_withheld() {
    // No counts at all in channels 90..=110: net 0 and variance 0, both
    // measured zeros. The limit is the Currie constant alone:
    // 2.71 / (0.05 * 0.50 * 1000 s) = 0.1084 Bq.
    let mut s = boxed(&[]);
    s.channels[84..=116].fill(0);
    s.rois.mark(Roi::new(90, 110));
    let report = analyse(
        &s,
        &lines_at(&[("Empty-1", 100.0)]),
        &exact_options(Some(EfficiencyCalibration::constant(0.05))),
    );
    assert!(report.withheld.is_empty(), "{:?}", report.withheld);
    let empty = &report.nuclides[0];
    assert_eq!((empty.activity, empty.uncertainty), (0.0, 0.0));
    // 0 counts are not above 3 * 0: nothing is detected in an empty region.
    assert!(!empty.detected);
    assert!(close(empty.mda.expect("a limit"), 2.71 / 25.0));
}

#[test]
fn an_overflow_from_the_count_correction_alone_is_not_blamed_on_a_harmless_reference_date() {
    // Half-life 1000 s, counted 1000 s: the count correction is
    // ln2 / (1 - 1/2) = 2 ln2 = 1.3863. The reference date is 250 s before
    // the count: 2^(1/4) = 1.1892. The sample quantity puts the uncorrected
    // activity (1000 counts / 25) at 0.8 of the largest double, so:
    //   x1        = 0.80 max  fine
    //   x1.3863   = 1.11 max  overflows  <- the count correction breaks it
    //   x1.1892   = 0.95 max  fine       <- the reference date does not
    //   x1.6486   = 1.32 max  overflows  (both, as applied)
    // The blame is the count's alone.
    let mut s = boxed(&thousand_at(100));
    s.rois.mark(Roi::new(90, 110));
    let mut lib = NuclideLibrary::new("fast");
    lib.push(Nuclide::new(
        "Fast-1",
        1000.0,
        vec![LibraryPeak::new(100.0, 50.0).key()],
    ));
    let options = AnalysisOptions {
        efficiency: Some(EfficiencyCalibration::constant(0.05)),
        sample: SampleInfo {
            quantity: 40.0 / (0.8 * f64::MAX),
            reference_date: s.start_time.map(|t| t - chrono::Duration::seconds(250)),
            ..SampleInfo::default()
        },
        ..AnalysisOptions::default()
    };
    let report = analyse(&s, &lib, &options);
    assert!(report.nuclides.is_empty(), "{:?}", report.nuclides);
    assert_eq!(report.withheld.len(), 1, "{:?}", report.withheld);
    assert_eq!(
        report.withheld[0].reason,
        "no activity at its 100.0 keV line: its activity is too large to be a number \
         because of the decay correction for the count; none is reported"
    );
}

#[test]
fn with_no_live_time_and_regions_matching_nothing_the_one_note_says_so() {
    // Regions marked, none in the library, no MDA rows asked for: nothing is
    // withheld, so the global note is what says it. With no regions at all
    // there is nothing to compute and no note.
    let mut s = boxed(&thousand_at(100));
    s.live_time = 0.0;
    s.rois.mark(Roi::new(90, 110));
    let lib = lines_at(&[("Elsewhere-1", 400.0)]);
    let options = exact_options(Some(EfficiencyCalibration::constant(0.05)));
    let report = analyse(&s, &lib, &options);
    assert!(report.withheld.is_empty(), "{:?}", report.withheld);
    assert_eq!(
        report.notes,
        ["the spectrum has no live time, so no activity or count rate can be computed"]
    );
    s.rois = Default::default();
    let report = analyse(&s, &lib, &options);
    assert!(report.notes.is_empty(), "{:?}", report.notes);
}

#[test]
fn a_detection_limit_line_at_channel_zero_is_in_the_spectrum_and_below_it_is_not() {
    // 50 keV at channel 0 (zero offset 50 keV, 1 keV per channel). The sum
    // runs from channel max(0 - 6, 0) = 0 to 6: 7 * 100 = 700 counts, so
    // (2.71 + 4.65 sqrt(700)) / 25 = 5.0295 Bq. 45 keV is channel -5:
    // outside, no limit.
    let mut s = boxed(&[]);
    s.energy_calibration = Some(EnergyCalibration::linear(50.0, 1.0));
    let lib = lines_at(&[("Edge-0", 50.0), ("Below-1", 45.0)]);
    let options = AnalysisOptions {
        report_mda_for_absent_nuclides: true,
        ..exact_options(Some(EfficiencyCalibration::constant(0.05)))
    };
    let report = analyse(&s, &lib, &options);
    let names: Vec<&str> = report.nuclides.iter().map(|n| n.nuclide.as_str()).collect();
    assert_eq!(names, ["Edge-0"], "{:?}", report.notes);
    let want = (2.71 + 4.65 * 700f64.sqrt()) / 25.0;
    assert!((want - 5.029_497).abs() < 1e-6, "hand value {want}");
    let mda = report.nuclides[0].mda.expect("a limit");
    assert!(close(mda, want), "mda {mda}, want {want}");
    assert_eq!(
        report.notes,
        [
            "no detection limit for 1 library nuclide: the line used for the limit falls \
          outside the spectrum (Below-1)"
        ]
    );
}

#[test]
fn an_efficiency_of_exactly_zero_is_no_efficiency_not_a_fault_and_not_a_measurement() {
    // The curve is 0.1 at 40 keV and 0 from 80 keV up (interpolation holds
    // the last point beyond it): exactly 0 at both 100 and 200 keV. Zero is
    // "the curve says nothing here": not a negative number, and no activity.
    let mut s = boxed(&thousand_at(100));
    s.rois.mark(Roi::new(90, 110));
    let lib = lines_at(&[("Zero-1", 100.0), ("Zero-2", 200.0)]);
    let curve = EfficiencyCalibration::interpolated(vec![
        EfficiencyPoint::new(40.0, 0.1),
        EfficiencyPoint::new(80.0, 0.0),
    ]);
    assert_eq!(curve.efficiency(100.0), 0.0);
    let options = AnalysisOptions {
        report_mda_for_absent_nuclides: true,
        ..exact_options(Some(curve))
    };
    let report = analyse(&s, &lib, &options);
    assert!(report.nuclides.is_empty(), "{:?}", report.nuclides);
    assert_eq!(report.withheld.len(), 1, "{:?}", report.withheld);
    assert_eq!(report.withheld[0].nuclide, "Zero-1");
    assert_eq!(
        report.withheld[0].reason,
        "no activity: the efficiency curve or the library yield is zero at its 100.0 keV line(s)"
    );
    assert_eq!(
        report.notes,
        [
            "no detection limit for 1 library nuclide: the efficiency curve gives nothing at \
          the line used for the limit (Zero-2)"
        ]
    );
}

/// A sample quantity of 0, a negative one or NaN used to become 1 without a
/// word while the column still read Bq/kg: per-sample figures labelled per
/// kilogram. Now the label is dropped and a note says why.
#[test]
fn a_quantity_that_is_not_positive_is_not_reported_per_unit() {
    let mut s = calibrated_spectrum();
    let lib = NuclideLibrary::sample_for_tests();
    s.rois.mark(Roi::new(650, 674));
    for quantity in [0.0, -2.0, f64::NAN, f64::INFINITY] {
        let mut opts = options();
        opts.sample = SampleInfo {
            quantity,
            unit: "kg".into(),
            ..SampleInfo::default()
        };
        let report = analyse(&s, &lib, &opts);
        assert_eq!(report.activity_unit(), "Bq", "quantity {quantity}");
        assert!(
            report
                .notes
                .iter()
                .any(|note| note.contains("sample quantity") && note.contains("not per kg")),
            "quantity {quantity}: notes {:?}",
            report.notes
        );
        assert!(mantaray_core::sample_quantity_problem(quantity).is_some());
    }
    // No quantity (1, no unit) is plain Bq with nothing said, as before.
    let report = analyse(&s, &lib, &options());
    assert_eq!(report.activity_unit(), "Bq");
    assert!(
        !report
            .notes
            .iter()
            .any(|note| note.contains("sample quantity")),
        "{:?}",
        report.notes
    );
    assert!(mantaray_core::sample_quantity_problem(1.0).is_none());
    assert!(mantaray_core::sample_quantity_problem(0.25).is_none());
}
