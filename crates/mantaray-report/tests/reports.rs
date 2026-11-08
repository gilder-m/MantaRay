//! ROI reports, spectrum printouts and quantitative reports.

use chrono::NaiveDate;
use mantaray_core::{
    AnalysisOptions, EfficiencyCalibration, EfficiencyPoint, EnergyCalibration, LibraryPeak,
    Nuclide, NuclideLibrary, Roi, SampleInfo, Spectrum, Withheld, analyse,
};
use mantaray_report::{ReportOptions, nuclide_report, print_spectrum, results_csv, roi_report};

/// A calibrated spectrum with a clean Cs-137 peak at 1 keV per channel.
fn spectrum() -> Spectrum {
    let mut counts = vec![0f64; 1024];
    for c in counts.iter_mut() {
        *c = 25.0;
    }
    let sigma = 2.0;
    let amplitude = 40_000.0 / (sigma * (2.0 * std::f64::consts::PI).sqrt());
    for (channel, value) in counts.iter_mut().enumerate() {
        let x = (channel as f64 - 661.657) / sigma;
        *value += amplitude * (-0.5 * x * x).exp();
    }
    let mut s = Spectrum::from_counts(counts.into_iter().map(|c| c.round() as u64).collect());
    s.live_time = 1800.0;
    s.real_time = 1850.5;
    s.detector_id = 19;
    s.detector_description = "IDM-200".into();
    s.sample_description = "check source".into();
    s.energy_calibration = Some(EnergyCalibration::linear(0.0, 1.0));
    s.start_time = Some(
        NaiveDate::from_ymd_opt(2026, 6, 27)
            .unwrap()
            .and_hms_opt(19, 12, 55)
            .unwrap(),
    );
    s.rois.mark(Roi::new(650, 674));
    s
}

#[test]
fn the_header_names_the_detector_and_the_times() {
    let library = NuclideLibrary::sample_for_tests();
    let text = roi_report(
        &spectrum(),
        &ReportOptions::paragraph().with_library(&library),
    );
    assert!(text.contains("Detector #19"), "{text}");
    assert!(text.contains("27-Jun-2026"), "{text}");
    assert!(text.contains("19:12:55"), "{text}");
    assert!(text.contains("RT = 1850.5"), "{text}");
    assert!(text.contains("LT = 1800.0"), "{text}");
    assert!(text.contains("IDM-200"), "{text}");
    assert!(text.contains("check source"), "{text}");
}

#[test]
fn the_paragraph_report_has_the_maestro_fields() {
    let library = NuclideLibrary::sample_for_tests();
    let text = roi_report(
        &spectrum(),
        &ReportOptions::paragraph().with_library(&library),
    );
    assert!(text.contains("ROI # 1"), "{text}");
    assert!(text.contains("RANGE:"), "{text}");
    assert!(text.contains("AREA : Gross ="), "{text}");
    assert!(text.contains("Net ="), "{text}");
    assert!(text.contains("CENTROID:"), "{text}");
    assert!(text.contains("SHAPE: FWHM ="), "{text}");
    assert!(text.contains("Fw(1/5)M ="), "{text}");
    assert!(text.contains("ID: Cs-137 at 661.66keV"), "{text}");
    assert!(text.contains("Corrected Rate ="), "{text}");
    assert!(text.contains("cA"), "{text}");
    // 661.657 keV, within a keV of the marked peak.
    assert!(text.contains("661.6") || text.contains("661.7"), "{text}");
}

#[test]
fn the_column_report_has_a_header_row_and_one_line_per_region() {
    let library = NuclideLibrary::sample_for_tests();
    let mut s = spectrum();
    s.rois.mark(Roi::new(100, 130));
    let text = roi_report(&s, &ReportOptions::column().with_library(&library));
    let lines: Vec<&str> = text.lines().collect();
    let header = lines
        .iter()
        .position(|line| line.contains("ROI#") && line.contains("GROSS"))
        .expect("a column header");
    assert!(lines[header].contains("CENTROID"));
    assert!(lines[header].contains("FWHM"));
    assert!(lines[header].contains("LIBRARY"));
    // Two data rows follow, in channel order.
    let first: Vec<&str> = lines[header + 1].split_whitespace().collect();
    let second: Vec<&str> = lines[header + 2].split_whitespace().collect();
    assert_eq!(first[0], "1");
    assert_eq!(second[0], "2");
    let first_low: f64 = first[1].parse().expect("a low energy");
    let second_low: f64 = second[1].parse().expect("a low energy");
    assert!(first_low < second_low);
}

#[test]
fn an_uncalibrated_spectrum_is_reported_in_channels() {
    let mut s = spectrum();
    s.energy_calibration = None;
    let text = roi_report(&s, &ReportOptions::paragraph());
    assert!(!text.contains("keV"), "{text}");
    // No library identification and no corrected rate without a calibration.
    // ("CENTROID:" contains "ID:", so look for the nuclide fields themselves.)
    assert!(!text.contains("Cs-137"), "{text}");
    assert!(!text.contains("Corrected Rate"), "{text}");
    assert!(text.contains("RANGE:"), "{text}");
    assert!(text.contains("650"), "{text}");
}

#[test]
fn a_spectrum_without_regions_says_so() {
    let mut s = spectrum();
    s.rois.clear_all();
    let text = roi_report(&s, &ReportOptions::paragraph());
    assert!(text.to_lowercase().contains("no regions"), "{text}");
}

#[test]
fn the_report_can_be_limited_to_a_displayed_range() {
    let mut s = spectrum();
    s.rois.mark(Roi::new(100, 130));
    let options = ReportOptions::paragraph().displayed(600, 700);
    let text = roi_report(&s, &options);
    assert_eq!(text.matches("ROI #").count(), 1, "{text}");
}

#[test]
fn printing_a_spectrum_gives_seven_channels_a_line() {
    let s = spectrum();
    let text = print_spectrum(&s, Some((0, 20)));
    let data: Vec<&str> = text
        .lines()
        .filter(|line| line.trim_start().starts_with(|c: char| c.is_ascii_digit()))
        .collect();
    assert_eq!(data.len(), 3, "21 channels over 7 columns\n{text}");
    let first: Vec<&str> = data[0].split_whitespace().collect();
    assert_eq!(first.len(), 8, "a channel label and seven values");
    assert_eq!(first[0], "0");
    let second: Vec<&str> = data[1].split_whitespace().collect();
    assert_eq!(second[0], "7");
}

#[test]
fn printing_defaults_to_the_whole_spectrum() {
    let text = print_spectrum(&Spectrum::from_counts(vec![1; 14]), None);
    assert_eq!(
        text.lines()
            .filter(|line| line.trim_start().starts_with(|c: char| c.is_ascii_digit()))
            .count(),
        2
    );
}

#[test]
fn the_nuclide_report_lists_activities_and_detection_limits() {
    let s = spectrum();
    let library = NuclideLibrary::sample_for_tests();
    let options = AnalysisOptions {
        efficiency: Some(EfficiencyCalibration::constant(0.05)),
        report_mda_for_absent_nuclides: true,
        ..AnalysisOptions::default()
    };
    let analysis = analyse(&s, &library, &options);
    let text = nuclide_report(&analysis);

    assert!(text.contains("Cs-137"), "{text}");
    assert!(text.contains("Bq"), "{text}");
    assert!(text.to_uppercase().contains("MDA"), "{text}");
    assert!(
        text.contains("661.66") || text.contains("661.657"),
        "{text}"
    );
    // 40 000 net counts / (1800 s * 0.05 efficiency * 0.851 yield) = 522 Bq.
    assert!(
        text.contains("522."),
        "expected an activity near 522 Bq in\n{text}"
    );
    assert!(text.to_lowercase().contains("not detected"), "{text}");
}

#[test]
fn results_export_as_csv() {
    let s = spectrum();
    let library = NuclideLibrary::sample_for_tests();
    let options = AnalysisOptions {
        efficiency: Some(EfficiencyCalibration::constant(0.05)),
        ..AnalysisOptions::default()
    };
    let analysis = analyse(&s, &library, &options);
    let csv = results_csv(&analysis);
    let mut lines = csv.lines();
    let header = lines.next().expect("a header row");
    assert!(header.starts_with("nuclide,"), "{header}");
    assert!(header.contains("activity"));
    assert!(header.contains("mda"));
    let row = lines.next().expect("a data row");
    let fields: Vec<&str> = row.split(',').collect();
    assert_eq!(fields[0], "Cs-137");
    assert_eq!(fields.len(), header.split(',').count());
    // Every numeric field parses.
    for field in &fields[1..] {
        if !field.is_empty() && field.chars().next().unwrap().is_ascii_digit() {
            assert!(field.parse::<f64>().is_ok(), "{field:?} is not a number");
        }
    }
}

#[test]
fn a_comma_in_a_nuclide_name_does_not_shift_the_csv_columns() {
    // Nuclide names come from user-written library files; a comma must be
    // quoted, not silently push every later column over by one.
    let s = spectrum();
    let mut awkward = NuclideLibrary::sample_for_tests()
        .nuclide("Cs-137")
        .expect("the standard library knows Cs-137")
        .clone();
    awkward.name = "Cs-137, fresh".to_string();
    let mut library = NuclideLibrary::new("awkward");
    library.push(awkward);
    let options = AnalysisOptions {
        efficiency: Some(EfficiencyCalibration::constant(0.05)),
        ..AnalysisOptions::default()
    };
    let analysis = analyse(&s, &library, &options);
    let csv = results_csv(&analysis);
    let mut lines = csv.lines();
    let header = lines.next().expect("a header row");
    let row = lines.next().expect("a data row");
    assert!(row.starts_with("\"Cs-137, fresh\""), "{row}");
    // Splitting outside quotes must give exactly the header's column count.
    let mut fields = 1;
    let mut quoted = false;
    for c in row.chars() {
        match c {
            '"' => quoted = !quoted,
            ',' if !quoted => fields += 1,
            _ => {}
        }
    }
    assert_eq!(fields, header.split(',').count(), "{row}");
}

#[test]
fn a_detection_limit_that_is_not_a_number_is_a_dash_in_the_text_and_empty_in_the_csv() {
    // A weak peak on a huge background, for a tiny sample: the activity is
    // large but fits a double, the detection limit (about 60 times the
    // activity here) does not. Shown as 0 it would read as a limit of
    // nothing; it must read as missing.
    let mut s = spectrum();
    for count in s.channels.iter_mut() {
        *count += 10_000_000_000;
    }
    let mut library = NuclideLibrary::new("hot");
    library.push(Nuclide::new(
        "Hot-1",
        86_400.0,
        vec![LibraryPeak::new(661.657, 85.1).key()],
    ));
    let options = AnalysisOptions {
        efficiency: Some(EfficiencyCalibration::constant(0.05)),
        sample: SampleInfo {
            quantity: 8.7e-305,
            ..SampleInfo::default()
        },
        ..AnalysisOptions::default()
    };
    let analysis = analyse(&s, &library, &options);
    assert_eq!(
        analysis.nuclides.len(),
        1,
        "{:?} {:?}",
        analysis.notes,
        analysis.withheld
    );
    assert_eq!(analysis.nuclides[0].mda, None, "{:?}", analysis.nuclides);

    let text = nuclide_report(&analysis);
    let row = text
        .lines()
        .find(|line| line.starts_with("Hot-1"))
        .expect("a Hot-1 row");
    let cells: Vec<&str> = row.split_whitespace().collect();
    assert_eq!(cells[3], "\u{2014}", "{row}");

    let csv = results_csv(&analysis);
    let mut lines = csv.lines();
    let columns = lines.next().unwrap().split(',').count();
    let row = lines.next().expect("a data row");
    let fields: Vec<&str> = row.split(',').collect();
    assert_eq!(fields.len(), columns, "{row}");
    assert_eq!(fields[3], "", "{row}");
}

/// The analysis of the standard spectrum with a curve that is zero at Cs-137's
/// 661.7 keV line: Cs-137 is withheld.
fn analysis_with_a_withheld_nuclide() -> mantaray_core::QuantitativeReport {
    let options = AnalysisOptions {
        efficiency: Some(EfficiencyCalibration::interpolated(vec![
            EfficiencyPoint::new(100.0, 0.1),
            EfficiencyPoint::new(500.0, 0.0),
        ])),
        ..AnalysisOptions::default()
    };
    let analysis = analyse(&spectrum(), &NuclideLibrary::sample_for_tests(), &options);
    assert!(analysis.nuclides.is_empty());
    assert_eq!(
        analysis.withheld.len(),
        1,
        "precondition: Cs-137 is withheld"
    );
    analysis
}

/// Reads CSV the way a spreadsheet or Python's csv module does: a field in
/// quotes may hold commas, doubled quotes and line breaks. A line that starts
/// with `#` is not special to those readers, so it is a row here too.
fn parse_csv(text: &str) -> Vec<Vec<String>> {
    let mut rows = Vec::new();
    let mut row = Vec::new();
    let mut field = String::new();
    let mut quoted = false;
    let mut chars = text.chars().peekable();
    while let Some(c) = chars.next() {
        match (c, quoted) {
            ('"', true) if chars.peek() == Some(&'"') => {
                field.push('"');
                chars.next();
            }
            ('"', _) => quoted = !quoted,
            (',', false) => row.push(std::mem::take(&mut field)),
            ('\n', false) => {
                row.push(std::mem::take(&mut field));
                rows.push(std::mem::take(&mut row));
            }
            (c, _) => field.push(c),
        }
    }
    assert!(!quoted, "a quote was never closed");
    rows
}
#[test]
fn a_withheld_nuclide_has_its_own_csv_row_and_the_table_stays_rectangular() {
    let mut analysis = analysis_with_a_withheld_nuclide();
    // A reason with the three things that break a CSV row, and a nuclide whose
    // name has a comma.
    analysis.withheld[0] = Withheld::new(
        "Cs-137, fresh",
        "no activity: the curve says \"zero\",\nsee it",
    );
    let rows = parse_csv(&results_csv(&analysis));
    let header = &rows[0];
    assert_eq!(header.len(), 13);
    // The twelve columns scripts already read, in their order, then `note`.
    assert_eq!(
        header.join(","),
        "nuclide,activity,uncertainty,mda,unit,detected,energy_kev,net_area,net_uncertainty,count_rate,efficiency,yield_percent,note"
    );
    for row in &rows {
        assert_eq!(row.len(), 13, "{row:?}");
    }
    let row = rows.last().unwrap();
    assert_eq!(row[0], "Cs-137, fresh");
    // The numbers are empty; the text and boolean columns keep their types.
    for number in [1, 2, 3, 6, 7, 8, 9, 10, 11] {
        assert_eq!(row[number], "", "column {number} of {row:?}");
    }
    assert_eq!(row[4], analysis.activity_unit(), "{row:?}");
    assert_eq!(row[5], "false", "{row:?}");
    assert_eq!(row[12], "no activity: the curve says \"zero\", see it");
}

#[test]
fn an_ordinary_csv_row_has_an_empty_note() {
    let options = AnalysisOptions {
        efficiency: Some(EfficiencyCalibration::constant(0.05)),
        ..AnalysisOptions::default()
    };
    let analysis = analyse(&spectrum(), &NuclideLibrary::sample_for_tests(), &options);
    assert!(analysis.withheld.is_empty() && !analysis.nuclides.is_empty());
    let rows = parse_csv(&results_csv(&analysis));
    assert!(rows.len() > 1);
    for row in &rows {
        assert_eq!(row.len(), 13, "{row:?}");
    }
    assert!(rows[1..].iter().all(|row| row[12].is_empty()));
    // The columns before `note` are what they were: same values as ever.
    assert_eq!(rows[1][0], "Cs-137");
    assert!(rows[1][1].parse::<f64>().unwrap() > 0.0);
}

#[test]
fn the_text_report_and_the_csv_say_a_withheld_reason_the_same_way() {
    let mut analysis = analysis_with_a_withheld_nuclide();
    analysis.withheld[0] = Withheld::new(
        "Cs-137",
        "no activity: line one\r\n  line   two\nline three",
    );
    let said = "no activity: line one line two line three";
    assert_eq!(analysis.withheld[0].reason, said);
    let text = nuclide_report(&analysis);
    assert!(
        text.lines()
            .any(|line| line == format!("Note: Cs-137: {said}")),
        "{text}"
    );
    let rows = parse_csv(&results_csv(&analysis));
    assert_eq!(rows.last().unwrap()[12], said);
}

/// The standard spectrum with Co-60 peaks as well, counted for no time: both
/// Cs-137 and Co-60 are found, and both are withheld for the same reason.
fn analysis_with_two_nuclides_withheld_for_one_reason() -> mantaray_core::QuantitativeReport {
    let mut counts = vec![25u64; 2048];
    for (energy, area) in [
        (661.657, 40_000.0),
        (1173.228, 30_000.0),
        (1332.492, 27_000.0),
    ] {
        let amplitude = area / (2.0 * (2.0 * std::f64::consts::PI).sqrt());
        for (channel, count) in counts.iter_mut().enumerate() {
            let x = (channel as f64 - energy) / 2.0;
            *count += (amplitude * (-0.5 * x * x).exp()).round() as u64;
        }
    }
    let mut s = Spectrum::from_counts(counts);
    s.live_time = 0.0;
    s.energy_calibration = Some(EnergyCalibration::linear(0.0, 1.0));
    for (low, high) in [(650, 674), (1161, 1185), (1320, 1344)] {
        s.rois.mark(Roi::new(low, high));
    }
    let options = AnalysisOptions {
        efficiency: Some(EfficiencyCalibration::constant(0.05)),
        ..AnalysisOptions::default()
    };
    let analysis = analyse(&s, &NuclideLibrary::sample_for_tests(), &options);
    assert_eq!(analysis.withheld.len(), 2, "{:?}", analysis.withheld);
    analysis
}

#[test]
fn a_cause_every_found_nuclide_shares_is_said_once_in_the_text_and_per_nuclide_in_the_csv() {
    let analysis = analysis_with_two_nuclides_withheld_for_one_reason();
    let text = nuclide_report(&analysis);
    let said: Vec<&str> = text
        .lines()
        .filter(|line| line.contains("Co-60") || line.contains("Cs-137"))
        .collect();
    assert_eq!(
        said,
        ["Note: not reported (no activity: the spectrum has no live time): Co-60, Cs-137"],
        "{text}"
    );
    // The CSV is for programs: a row for each, each with its reason.
    let rows = parse_csv(&results_csv(&analysis));
    let reasons: Vec<(&str, &str)> = rows[1..]
        .iter()
        .map(|row| (row[0].as_str(), row[12].as_str()))
        .collect();
    assert_eq!(
        reasons,
        [
            ("Co-60", "no activity: the spectrum has no live time"),
            ("Cs-137", "no activity: the spectrum has no live time"),
        ]
    );
}

#[test]
fn a_missing_live_time_is_named_on_one_line_of_the_text_report() {
    // A found nuclide, a live time of 0 and MDA rows asked for too: the
    // withheld line says it, and no second note repeats it.
    let mut s = spectrum();
    s.live_time = 0.0;
    let options = AnalysisOptions {
        efficiency: Some(EfficiencyCalibration::constant(0.05)),
        report_mda_for_absent_nuclides: true,
        ..AnalysisOptions::default()
    };
    let analysis = analyse(&s, &NuclideLibrary::sample_for_tests(), &options);
    let text = nuclide_report(&analysis);
    let naming: Vec<&str> = text
        .lines()
        .filter(|line| line.contains("live time"))
        .collect();
    assert_eq!(naming.len(), 1, "{text}");
}

#[test]
fn nuclides_that_were_not_found_do_not_flood_the_csv_or_the_text_report() {
    // 1500 nuclides, none found, none with a detection limit: no rows.
    let mut library = NuclideLibrary::new("big");
    for n in 0..1500 {
        library.push(mantaray_core::Nuclide::new(
            &format!("Ac-{n}"),
            1.0e9,
            vec![mantaray_core::LibraryPeak::new(5000.0 + n as f64, 50.0).key()],
        ));
    }
    let options = AnalysisOptions {
        efficiency: Some(EfficiencyCalibration::constant(0.05)),
        report_mda_for_absent_nuclides: true,
        ..AnalysisOptions::default()
    };
    let analysis = analyse(&spectrum(), &library, &options);
    assert_eq!(parse_csv(&results_csv(&analysis)).len(), 1, "header only");
    let text = nuclide_report(&analysis);
    assert!(text.lines().count() < 20, "{}", text.lines().count());
    assert!(text.contains("no detection limit for 1,500 library nuclides"));
}

#[test]
fn a_curve_that_gives_no_efficiency_prints_no_efficiency_and_no_nan() {
    let s = spectrum();
    let library = NuclideLibrary::sample_for_tests();
    for curve in [
        EfficiencyCalibration::constant(f64::NAN),
        EfficiencyCalibration::constant(-0.1),
    ] {
        let options = AnalysisOptions {
            efficiency: Some(curve.clone()),
            ..AnalysisOptions::default()
        };
        let analysis = analyse(&s, &library, &options);
        let csv = results_csv(&analysis);
        let text = nuclide_report(&analysis);
        for out in [&csv, &text] {
            assert!(
                !out.to_lowercase().contains("nan") && !out.contains("-0.1"),
                "{curve:?}: {out}"
            );
        }
        let columns: Vec<&str> = csv.lines().next().unwrap().split(',').collect();
        let at = columns.iter().position(|c| *c == "efficiency").unwrap();
        let row: Vec<&str> = csv.lines().nth(1).unwrap().split(',').collect();
        assert_eq!(row.len(), columns.len(), "{csv}");
        assert_eq!(row[at], "", "{curve:?}: {csv}");
    }
}

/// A report holding the given (activity, detected) nuclides in becquerels.
fn report_of(nuclides: &[(&str, f64, bool)]) -> mantaray_core::QuantitativeReport {
    mantaray_core::QuantitativeReport {
        peaks: Vec::new(),
        nuclides: nuclides
            .iter()
            .map(|&(name, activity, detected)| mantaray_core::NuclideResult {
                nuclide: name.to_string(),
                activity,
                uncertainty: activity / 10.0,
                mda: Some(1.0),
                detected,
                half_life_seconds: 1.0e9,
                lines: Vec::new(),
            })
            .collect(),
        unidentified: Vec::new(),
        has_efficiency: true,
        sample_unit: String::new(),
        live_time: 1000.0,
        real_time: 1000.0,
        notes: Vec::new(),
        withheld: Vec::new(),
    }
}

#[test]
fn the_total_detected_activity_line_sums_the_detected_nuclides_and_is_absent_for_zero() {
    // 100 + 250 Bq detected; the 1000 Bq nuclide was not detected and is not
    // part of the total: 350 Bq.
    let text = nuclide_report(&report_of(&[
        ("A-1", 100.0, true),
        ("B-2", 250.0, true),
        ("C-3", 1000.0, false),
    ]));
    let totals: Vec<&str> = text
        .lines()
        .filter(|line| line.starts_with("Total detected activity"))
        .collect();
    assert_eq!(totals.len(), 1, "{text}");
    let number: f64 = totals[0]
        .trim_start_matches("Total detected activity: ")
        .trim_end_matches(" Bq")
        .parse()
        .unwrap_or_else(|_| panic!("{:?}", totals[0]));
    assert_eq!(number, 350.0, "{text}");

    // Nothing detected: a total of 0 is not printed as a line.
    let text = nuclide_report(&report_of(&[("C-3", 1000.0, false)]));
    assert!(!text.contains("Total detected activity"), "{text}");
}
