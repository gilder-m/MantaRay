//! `.Roi` region files, TRANSLT-style ASCII dumps and the native JSON format.

use chrono::NaiveDate;
use ortseam_core::{EnergyCalibration, Roi, RoiSet, Spectrum};
use ortseam_formats::{AsciiOptions, FormatError, ascii, native, roi};

#[test]
fn roi_file_round_trips_channel_pairs() {
    let set = RoiSet::from_pairs([(10, 20), (100, 130), (900, 901)]);
    let bytes = roi::write(&set);
    let back = roi::read(&bytes).unwrap();
    assert_eq!(back.to_pairs(), set.to_pairs());
}

#[test]
fn roi_file_stores_the_end_channel_exclusively() {
    // The manual's reader subtracts one from the stored end channel.
    let set = RoiSet::from_pairs([(10, 20)]);
    let bytes = roi::write(&set);
    // Two i16 of header, then begin and end.
    let begin = i16::from_le_bytes([bytes[4], bytes[5]]);
    let end = i16::from_le_bytes([bytes[6], bytes[7]]);
    assert_eq!(begin, 10);
    assert_eq!(end, 21, "stored end is one past the last channel");
}

#[test]
fn roi_file_stops_at_the_terminator() {
    let mut bytes = roi::write(&RoiSet::from_pairs([(10, 20)]));
    bytes.extend_from_slice(&[0u8; 16]); // padding after the terminator
    let back = roi::read(&bytes).unwrap();
    assert_eq!(back.len(), 1);
}

#[test]
fn an_empty_roi_file_reads_as_no_regions() {
    let back = roi::read(&roi::write(&RoiSet::default())).unwrap();
    assert!(back.is_empty());
}

#[test]
fn a_short_roi_file_is_rejected() {
    assert!(matches!(
        roi::read(&[1u8, 2]),
        Err(FormatError::Truncated { .. })
    ));
}

#[test]
fn ascii_dump_defaults_to_five_columns_with_channel_numbers() {
    let mut s = Spectrum::from_counts((0..10u64).collect());
    s.live_time = 120.0;
    s.real_time = 240.0;
    let text = ascii::write(&s, &AsciiOptions::default());
    assert!(text.contains("Real Time   240"), "{text}");
    assert!(text.contains("Live Time   120"), "{text}");
    let data: Vec<&str> = text.lines().filter(|line| line.contains(':')).collect();
    assert_eq!(data.len(), 2, "10 channels over 5 columns");
    // Channel labels are right-aligned in a six-character column.
    assert!(data[0].trim_start().starts_with("0:"), "{}", data[0]);
    assert!(data[1].trim_start().starts_with("5:"), "{}", data[1]);
}

#[test]
fn ascii_options_control_columns_channel_numbers_and_headers() {
    let s = Spectrum::from_counts((0..4u64).collect());
    let options = AsciiOptions {
        columns: 1,
        channel_numbers: false,
        header: false,
        acquisition_info: false,
    };
    let text = ascii::write(&s, &options);
    assert_eq!(
        text.lines().collect::<Vec<_>>(),
        vec!["0", "1", "2", "3"],
        "one bare column per channel"
    );
}

#[test]
fn ascii_dump_is_readable_back() {
    let mut s = Spectrum::from_counts((0..64u64).map(|c| c * 7).collect());
    s.live_time = 55.5;
    s.real_time = 60.0;
    let text = ascii::write(&s, &AsciiOptions::default());
    let back = ascii::read(&text).unwrap();
    assert_eq!(back.channels, s.channels);
    assert_eq!(back.live_time, 55.5);
    assert_eq!(back.real_time, 60.0);
}

#[test]
fn ascii_reader_accepts_bare_numbers() {
    let back = ascii::read("1 2 3\n4 5 6\n").unwrap();
    assert_eq!(back.channels, vec![1, 2, 3, 4, 5, 6]);
    assert_eq!(back.live_time, 0.0);
}

#[test]
fn ascii_reader_rejects_a_file_without_numbers() {
    assert!(matches!(
        ascii::read("no data here\n"),
        Err(FormatError::MissingField { .. })
    ));
}

#[test]
fn native_json_keeps_the_whole_model() {
    let mut s = Spectrum::from_counts(vec![1, 2, 3, 4, 5, 6, 7, 8]);
    s.live_time = 10.0;
    s.real_time = 11.0;
    s.sample_description = "native round trip".into();
    s.energy_calibration = Some(EnergyCalibration::new([1.0, 0.5, 1e-7], "keV"));
    s.rois.mark(Roi::new(1, 3));
    s.start_time = Some(
        NaiveDate::from_ymd_opt(2026, 2, 3)
            .unwrap()
            .and_hms_opt(4, 5, 6)
            .unwrap(),
    );

    let text = native::write(&s).unwrap();
    let back = native::read(&text).unwrap();
    assert_eq!(back, s, "the native format is lossless");
    assert!(text.contains("\"ortseam\""), "the file is self describing");
}

#[test]
fn native_json_rejects_a_foreign_document() {
    assert!(native::read("{\"hello\": 1}").is_err());
}
