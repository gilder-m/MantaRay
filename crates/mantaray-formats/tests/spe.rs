//! IAEA / CTBTO ASCII `.Spe` files.

use chrono::NaiveDate;
use mantaray_core::{EnergyCalibration, Roi, ShapeCalibration, Spectrum};
use mantaray_formats::{FormatError, spe};

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
    let mut spectrum = Spectrum::from_counts(vec![1, 2, 3, 4]);
    spectrum.start_time =
        chrono::NaiveDate::from_ymd_opt(2026, 8, 11).and_then(|day| day.and_hms_opt(15, 48, 20));
    let text = spe::write(&spectrum).unwrap();
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
    assert!(text.contains("08/11/2026 15:48:20"));
}

/// A spectrum whose start was never recorded says so by leaving the keyword
/// out, rather than by claiming the Unix epoch.
///
/// The placeholder that used to go here read back as a measurement made on
/// 1 January 1970 - a date nothing could tell from one that was meant, and one
/// a decay correction would use, wrong by decades and without complaint.
#[test]
fn a_spectrum_with_no_start_time_writes_no_date_at_all() {
    let text = spe::write(&Spectrum::from_counts(vec![1, 2, 3, 4])).unwrap();
    assert!(!text.contains("$DATE_MEA:"), "{text}");
    assert!(!text.contains("1970"), "{text}");
    assert_eq!(spe::read(&text).unwrap().start_time, None);
}

/// And a file already carrying the old placeholder reads as having no date.
#[test]
fn the_old_epoch_placeholder_reads_as_no_date() {
    let text = "$DATE_MEA:\n01/01/1970 00:00:00\n$DATA:\n0 3\n1\n2\n3\n4\n";
    assert_eq!(spe::read(text).unwrap().start_time, None);

    // A real date beside it still reads as one.
    let dated = "$DATE_MEA:\n08/11/2026 15:48:20\n$DATA:\n0 3\n1\n2\n3\n4\n";
    assert!(spe::read(dated).unwrap().start_time.is_some());
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

/// A live or real time that is not a finite number is refused on read, so it
/// is never carried into a native file that would not open again.
#[test]
fn a_measurement_time_that_is_not_finite_is_refused() {
    for times in ["NaN inf", "100 NaN", "inf 100", "-inf 100"] {
        let text = format!("$SPEC_ID:\nx\n$MEAS_TIM:\n{times}\n$DATA:\n0 3\n1\n2\n3\n4\n");
        match spe::read(&text) {
            Ok(spectrum) => panic!(
                "{times:?} read as live {} real {}",
                spectrum.live_time, spectrum.real_time
            ),
            Err(error) => assert!(error.to_string().contains("$MEAS_TIM"), "{times}: {error}"),
        }
    }
    // Finite times still read as before.
    let text = "$SPEC_ID:\nx\n$MEAS_TIM:\n10 12\n$DATA:\n0 1\n1\n2\n";
    let spectrum = spe::read(text).unwrap();
    assert_eq!((spectrum.live_time, spectrum.real_time), (10.0, 12.0));
}

/// The native writer refuses what its reader would refuse, naming the field.
#[test]
fn a_native_spectrum_with_a_time_that_is_not_finite_is_not_written() {
    let mut spectrum = Spectrum::from_counts(vec![1, 2, 3]);
    spectrum.live_time = f64::NAN;
    spectrum.real_time = f64::INFINITY;
    match mantaray_formats::native::write(&spectrum) {
        Ok(text) => assert!(
            mantaray_formats::native::read(&text).is_ok(),
            "wrote a spectrum that does not open again"
        ),
        Err(error) => assert!(error.to_string().contains("live time"), "{error}"),
    }
    let mut calibrated = Spectrum::from_counts(vec![1, 2, 3]);
    calibrated.energy_calibration = Some(EnergyCalibration::new([0.0, f64::NAN, 0.0], "keV"));
    match mantaray_formats::native::write(&calibrated) {
        Ok(text) => assert!(
            mantaray_formats::native::read(&text).is_ok(),
            "wrote a calibration that does not open again"
        ),
        Err(error) => assert!(error.to_string().contains("coefficients"), "{error}"),
    }
}

/// A corrupt `.Spc` whose time words hold NaN is refused the same way.
#[test]
fn an_spc_time_that_is_not_finite_is_refused() {
    let mut spectrum = Spectrum::from_counts(vec![5; 1024]);
    spectrum.live_time = 10.0;
    spectrum.real_time = 12.0;
    for offset in [90usize, 94] {
        let mut bytes = mantaray_formats::spc::write(&spectrum).unwrap();
        bytes[offset..offset + 4].copy_from_slice(&f32::NAN.to_le_bytes());
        match mantaray_formats::spc::read(&bytes) {
            Ok(back) => panic!(
                "offset {offset}: read as live {} real {}",
                back.live_time, back.real_time
            ),
            Err(error) => assert!(error.to_string().contains("time"), "{offset}: {error}"),
        }
    }
}
