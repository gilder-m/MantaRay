//! ROI reports, spectrum printouts and quantitative reports.

use chrono::NaiveDate;
use mantaray_core::{
    AnalysisOptions, EfficiencyCalibration, EnergyCalibration, NuclideLibrary, Roi, Spectrum,
    analyse,
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
