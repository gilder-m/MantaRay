//! IAEA / CTBTO ASCII `.Spe` files.

use chrono::NaiveDate;
use ortseam_core::{EnergyCalibration, Roi, ShapeCalibration, Spectrum};
use ortseam_formats::{FormatError, spe};

const SAMPLE: &str = "\
$SPEC_ID:
Detector 1 check source
$SPEC_REM:
DET# 1
DETDESC# IDM-200
AP# MAESTRO V7.00
$DATE_MEA:
07/29/2026 14:35:21
$MEAS_TIM:
3600 3661
$DATA:
0 7
10
20
30
40
50
60
70
$ROI:
1
2 5
$PRESETS:
None
0
0
$ENER_FIT:
1.250000 0.499900
$MCA_CAL:
3
1.250000E+000 4.999000E-001 1.500000E-007 keV
$SHAPE_CAL:
3
1.900000E+000 3.100000E-002 0.000000E+000
";

#[test]
fn reads_a_reference_file() {
    let s = spe::read(SAMPLE).unwrap();
    assert_eq!(s.channels, vec![10, 20, 30, 40, 50, 60, 70, 0]);
    assert_eq!(s.sample_description, "Detector 1 check source");
    assert_eq!(s.detector_description, "IDM-200");
    assert_eq!(s.detector_id, 1);
    assert_eq!(s.live_time, 3600.0);
    assert_eq!(s.real_time, 3661.0);
    assert_eq!(
        s.start_time,
        Some(
            NaiveDate::from_ymd_opt(2026, 7, 29)
                .unwrap()
                .and_hms_opt(14, 35, 21)
                .unwrap()
        )
    );
    let cal = s.energy_calibration.unwrap();
    assert!((cal.coefficients[0] - 1.25).abs() < 1e-9);
    assert!((cal.coefficients[1] - 0.4999).abs() < 1e-9);
    assert!((cal.coefficients[2] - 1.5e-7).abs() < 1e-15);
    assert_eq!(cal.units, "keV");
    let shape = s.shape_calibration.unwrap();
    assert!((shape.coefficients[1] - 0.031).abs() < 1e-9);
    assert_eq!(s.rois.to_pairs(), vec![(2, 5)]);
}

#[test]
fn data_header_gives_the_channel_range() {
    // "$DATA:" is followed by "first last"; the spectrum spans last+1 channels.
    let s = spe::read(SAMPLE).unwrap();
    assert_eq!(s.channels.len(), 8);
    assert_eq!(s.channel_offset, 0);
}

#[test]
fn round_trip_keeps_the_data_and_descriptors() {
    let mut original = Spectrum::from_counts((0..512u64).map(|c| c % 97).collect());
    original.live_time = 1234.5;
    original.real_time = 1300.0;
    original.detector_id = 12;
    original.detector_description = "GEM-30".into();
    original.sample_description = "Blank".into();
    original.energy_calibration = Some(EnergyCalibration::new([0.5, 0.25, 1e-8], "keV"));
    original.shape_calibration = Some(ShapeCalibration::new([2.0, 0.02, 0.0]));
    original.rois.mark(Roi::new(10, 20));
    original.rois.mark(Roi::new(100, 130));
    original.start_time = Some(
        NaiveDate::from_ymd_opt(2025, 12, 31)
            .unwrap()
            .and_hms_opt(23, 59, 58)
            .unwrap(),
    );

    let text = spe::write(&original).unwrap();
    let restored = spe::read(&text).unwrap();

    assert_eq!(restored.channels, original.channels);
    assert_eq!(restored.live_time, original.live_time);
    assert_eq!(restored.real_time, original.real_time);
    assert_eq!(restored.detector_id, original.detector_id);
    assert_eq!(restored.detector_description, original.detector_description);
    assert_eq!(restored.sample_description, original.sample_description);
    assert_eq!(restored.start_time, original.start_time);
    assert_eq!(restored.rois.to_pairs(), original.rois.to_pairs());
    let cal = restored.energy_calibration.unwrap();
    assert!((cal.coefficients[2] - 1e-8).abs() < 1e-15);
}

#[test]
fn writes_the_expected_keywords_in_order() {
    let text = spe::write(&Spectrum::from_counts(vec![1, 2, 3, 4])).unwrap();
    let keywords: Vec<&str> = text.lines().filter(|line| line.starts_with('$')).collect();
    assert_eq!(
        keywords,
        vec![
            "$SPEC_ID:",
            "$SPEC_REM:",
            "$DATE_MEA:",
            "$MEAS_TIM:",
            "$DATA:",
            "$PRESETS:",
            "$ENER_FIT:",
            "$MCA_CAL:",
            "$SHAPE_CAL:",
        ]
    );
    assert!(text.contains("$DATA:\n0 3\n"));
}

#[test]
fn a_file_without_data_is_rejected() {
    assert!(matches!(
        spe::read("$SPEC_ID:\nnothing here\n"),
        Err(FormatError::MissingField { .. })
    ));
}

#[test]
fn extra_unknown_keywords_are_ignored() {
    let text = format!("{SAMPLE}$GPS:\n1 2 3\n$FLAGS:\nxyz\n");
    let s = spe::read(&text).unwrap();
    assert_eq!(s.channels.len(), 8);
}

#[test]
fn tolerates_crlf_and_blank_lines() {
    let text = SAMPLE.replace('\n', "\r\n");
    let s = spe::read(&text).unwrap();
    assert_eq!(s.channels, vec![10, 20, 30, 40, 50, 60, 70, 0]);
}
