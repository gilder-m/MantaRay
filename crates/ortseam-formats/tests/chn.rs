//! ORTEC `.Chn` integer spectrum files.
//!
//! Layout (little endian), as documented by the MAESTRO manual's Appendix B
//! reader:
//!
//! ```text
//! offset size field
//! 0      2    i16  file type, always -1
//! 2      2    i16  MCA number
//! 4      2    i16  segment number
//! 6      2    ascii seconds of the start time
//! 8      4    i32  real time in 20 ms ticks
//! 12     4    i32  live time in 20 ms ticks
//! 16     8    ascii start date "DDMMMYY" plus a century flag
//! 24     4    ascii start time "HHMM"
//! 28     2    u16  channel offset
//! 30     2    u16  number of channels
//! 32     4*n  i32  channel data
//! then   2    i16  footer type (-102 with a quadratic term, -101 without)
//! ```

use chrono::NaiveDate;
use ortseam_core::{EnergyCalibration, Roi, ShapeCalibration, Spectrum};
use ortseam_formats::{FormatError, chn};

fn sample() -> Spectrum {
    let mut s = Spectrum::from_counts((0..1024u64).map(|c| c * 3 + 1).collect());
    s.live_time = 3600.0;
    s.real_time = 3661.44; // an exact multiple of 20 ms
    s.detector_id = 7;
    s.segment = 1;
    s.channel_offset = 0;
    s.detector_description = "IDM-200 serial 4242".into();
    s.sample_description = "Soil sample 17, 1.0 kg".into();
    s.energy_calibration = Some(EnergyCalibration::new([1.25, 0.4999, 1.5e-7], "keV"));
    s.shape_calibration = Some(ShapeCalibration::new([1.9, 0.031, 0.0]));
    s.start_time = Some(
        NaiveDate::from_ymd_opt(2026, 7, 29)
            .unwrap()
            .and_hms_opt(14, 35, 21)
            .unwrap(),
    );
    s
}

#[test]
fn round_trip_preserves_everything_the_format_carries() {
    let original = sample();
    let bytes = chn::write(&original).unwrap();
    let restored = chn::read(&bytes).unwrap();

    assert_eq!(restored.channels, original.channels);
    assert_eq!(restored.detector_id, original.detector_id);
    assert_eq!(restored.segment, original.segment);
    assert_eq!(restored.detector_description, original.detector_description);
    assert_eq!(restored.sample_description, original.sample_description);
    assert!((restored.live_time - original.live_time).abs() < 0.02);
    assert!((restored.real_time - original.real_time).abs() < 0.02);
    assert_eq!(restored.start_time, original.start_time);

    let cal = restored.energy_calibration.unwrap();
    let expected = original.energy_calibration.unwrap();
    for i in 0..3 {
        assert!(
            (cal.coefficients[i] - expected.coefficients[i]).abs() < 1e-6,
            "coefficient {i}"
        );
    }
    let shape = restored.shape_calibration.unwrap();
    assert!((shape.coefficients[0] - 1.9).abs() < 1e-6);
    assert!((shape.coefficients[1] - 0.031).abs() < 1e-6);
}

#[test]
fn header_is_written_exactly_as_specified() {
    let bytes = chn::write(&sample()).unwrap();
    assert_eq!(i16::from_le_bytes([bytes[0], bytes[1]]), -1, "file type");
    assert_eq!(i16::from_le_bytes([bytes[2], bytes[3]]), 7, "MCA number");
    assert_eq!(i16::from_le_bytes([bytes[4], bytes[5]]), 1, "segment");
    assert_eq!(&bytes[6..8], b"21", "ascii seconds");

    let real_ticks = i32::from_le_bytes([bytes[8], bytes[9], bytes[10], bytes[11]]);
    let live_ticks = i32::from_le_bytes([bytes[12], bytes[13], bytes[14], bytes[15]]);
    let expected_real: f64 = 3661.44 / 0.02;
    assert_eq!(real_ticks, expected_real.round() as i32);
    assert_eq!(live_ticks, 3600 * 50);

    assert_eq!(&bytes[16..24], b"29JUL261", "date plus century flag");
    assert_eq!(&bytes[24..28], b"1435", "start time");
    assert_eq!(u16::from_le_bytes([bytes[28], bytes[29]]), 0, "offset");
    assert_eq!(u16::from_le_bytes([bytes[30], bytes[31]]), 1024, "channels");

    // First channel value follows the 32-byte header.
    assert_eq!(
        i32::from_le_bytes([bytes[32], bytes[33], bytes[34], bytes[35]]),
        1
    );
}

#[test]
fn twentieth_century_dates_use_a_space_century_flag() {
    let mut s = Spectrum::from_counts(vec![1, 2, 3, 4]);
    s.start_time = Some(
        NaiveDate::from_ymd_opt(1998, 3, 5)
            .unwrap()
            .and_hms_opt(8, 7, 6)
            .unwrap(),
    );
    let bytes = chn::write(&s).unwrap();
    assert_eq!(&bytes[16..24], b"05MAR98 ");
    let back = chn::read(&bytes).unwrap();
    assert_eq!(back.start_time, s.start_time);
}

#[test]
fn a_spectrum_without_a_start_time_round_trips_as_none() {
    let s = Spectrum::from_counts(vec![5, 6, 7, 8]);
    let bytes = chn::write(&s).unwrap();
    let back = chn::read(&bytes).unwrap();
    assert!(back.start_time.is_none());
    assert_eq!(back.channels, vec![5, 6, 7, 8]);
}

#[test]
fn a_spectrum_without_a_calibration_round_trips_as_none() {
    let s = Spectrum::from_counts(vec![1; 16]);
    let bytes = chn::write(&s).unwrap();
    let back = chn::read(&bytes).unwrap();
    assert!(back.energy_calibration.is_none());
    assert!(back.shape_calibration.is_none());
}

#[test]
fn descriptions_are_truncated_to_the_field_width() {
    let mut s = Spectrum::from_counts(vec![1; 8]);
    s.sample_description = "x".repeat(200);
    s.detector_description = "y".repeat(200);
    let bytes = chn::write(&s).unwrap();
    let back = chn::read(&bytes).unwrap();
    assert_eq!(back.sample_description.len(), 63);
    assert_eq!(back.detector_description.len(), 63);
}

#[test]
fn regions_are_not_part_of_the_format() {
    // .Chn carries no ROI table; regions must be saved to a .Roi file.
    let mut s = Spectrum::from_counts(vec![1; 32]);
    s.rois.mark(Roi::new(4, 12));
    let back = chn::read(&chn::write(&s).unwrap()).unwrap();
    assert!(back.rois.is_empty());
}

#[test]
fn a_bad_magic_number_is_rejected() {
    let mut bytes = chn::write(&sample()).unwrap();
    bytes[0] = 9;
    bytes[1] = 0;
    assert!(matches!(
        chn::read(&bytes),
        Err(FormatError::BadMagic { .. })
    ));
}

#[test]
fn a_truncated_file_is_rejected() {
    let bytes = chn::write(&sample()).unwrap();
    assert!(matches!(
        chn::read(&bytes[..20]),
        Err(FormatError::Truncated { .. })
    ));
    // Header claims 1024 channels but the data is cut short.
    assert!(matches!(
        chn::read(&bytes[..600]),
        Err(FormatError::Truncated { .. })
    ));
}

#[test]
fn a_file_without_a_footer_still_reads() {
    let bytes = chn::write(&sample()).unwrap();
    let data_end = 32 + 4 * 1024;
    let back = chn::read(&bytes[..data_end]).unwrap();
    assert_eq!(back.channels.len(), 1024);
    assert!(back.energy_calibration.is_none());
}

#[test]
fn counts_above_the_32_bit_range_are_clamped() {
    let s = Spectrum::from_counts(vec![u64::MAX, 5]);
    let back = chn::read(&chn::write(&s).unwrap()).unwrap();
    assert_eq!(back.channels[0], i32::MAX as u64);
    assert_eq!(back.channels[1], 5);
}
