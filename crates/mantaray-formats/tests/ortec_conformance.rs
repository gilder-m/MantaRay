//! Conformance with ORTEC's own layouts, from the 2026-11-08 format audit.
//!
//! The files in `golden/ortec-audit-2026-11-08/` are the audit's repro files,
//! made by hand to the spec (no MAESTRO-written `.Roi`, `.Chn` or `.Spc` was
//! at hand): a `.Roi` laid out as the File Structure Manual (§3.2) and
//! MAESTRO's sample reader give it, a `.Chn` carrying ORTEC's "uncalibrated"
//! trailer values, and a real-format (FILTYP 5) `.Spc`.

use chrono::{NaiveDate, NaiveDateTime};
use mantaray_core::{EnergyCalibration, RoiSet, Spectrum};
use mantaray_formats::{SpectrumFormat, chn, roi, spc, spe};

const MAESTRO_ROI: &[u8] = include_bytes!("golden/ortec-audit-2026-11-08/F_maestro_layout.Roi");
const ORTEC_UNCALIBRATED_CHN: &[u8] =
    include_bytes!("golden/ortec-audit-2026-11-08/I_spec_uncalibrated.Chn");
const REAL_FORMAT_SPC: &[u8] = include_bytes!("golden/ortec-audit-2026-11-08/G_real_filtyp5.Spc");

fn words(bytes: &[u8]) -> Vec<i16> {
    bytes
        .as_chunks::<2>()
        .0
        .iter()
        .map(|w| i16::from_le_bytes(*w))
        .collect()
}

fn start() -> NaiveDateTime {
    NaiveDate::from_ymd_opt(2026, 11, 9)
        .unwrap()
        .and_hms_opt(9, 31, 5)
        .unwrap()
}

fn sample() -> Spectrum {
    let mut s = Spectrum::from_counts((0..1024u64).map(|c| c % 50 + 1).collect());
    s.live_time = 1798.42;
    s.real_time = 1800.0;
    s.start_time = Some(start());
    s
}

// --- B1: `.Roi` has one header word ---

#[test]
fn a_maestro_roi_file_reads_as_its_regions_not_the_gaps_between_them() {
    let set = roi::read(MAESTRO_ROI).unwrap();
    assert_eq!(set.to_pairs(), vec![(320, 340), (600, 620)]);
}

#[test]
fn a_written_roi_file_has_ortecs_one_word_header_and_terminator() {
    let set = RoiSet::from_pairs([(320, 340), (600, 620)]);
    assert_eq!(words(&roi::write(&set)), vec![-2, 320, 341, 600, 621, -1]);
    assert_eq!(words(&roi::write(&RoiSet::default())), vec![-2, -1]);
}

#[test]
fn a_two_word_header_from_0_2_4_still_reads() {
    // 0.2.4 wrote the count, a zero, then the pairs. A real ORTEC file never
    // has a positive header word followed by a zero start channel.
    let mut bytes = Vec::new();
    for word in [2i16, 0, 320, 341, 600, 621, 0, 0] {
        bytes.extend_from_slice(&word.to_le_bytes());
    }
    assert_eq!(
        roi::read(&bytes).unwrap().to_pairs(),
        vec![(320, 340), (600, 620)]
    );
}

// --- S1: `.Chn` uncalibrated is slope 1, FWHM 1 ---

fn chn_trailer_floats(bytes: &[u8], channels: usize) -> Vec<f32> {
    let trailer = 32 + 4 * channels;
    bytes[trailer + 4..trailer + 28]
        .as_chunks::<4>()
        .0
        .iter()
        .map(|w| f32::from_le_bytes(*w))
        .collect()
}

#[test]
fn an_uncalibrated_chn_is_written_with_ortecs_uncalibrated_values() {
    let bytes = chn::write(&sample()).unwrap();
    assert_eq!(
        chn_trailer_floats(&bytes, 1024),
        vec![0.0, 1.0, 0.0, 1.0, 0.0, 0.0]
    );
    let back = chn::read(&bytes).unwrap();
    assert_eq!(back.energy_calibration, None);
    assert_eq!(back.shape_calibration, None);
}

#[test]
fn ortecs_uncalibrated_chn_reads_as_uncalibrated() {
    let spectrum = chn::read(ORTEC_UNCALIBRATED_CHN).unwrap();
    assert_eq!(spectrum.energy_calibration, None, "not 1 keV per channel");
    assert_eq!(spectrum.shape_calibration, None, "not a 1-channel FWHM");
}

#[test]
fn a_zero_filled_chn_trailer_from_0_2_4_still_reads_as_uncalibrated() {
    let mut bytes = chn::write(&sample()).unwrap();
    let trailer = 32 + 4 * 1024;
    bytes[trailer + 4..trailer + 28].fill(0);
    assert_eq!(chn::read(&bytes).unwrap().energy_calibration, None);
}

#[test]
fn a_chn_segment_of_zero_is_written_as_one() {
    // FSM §3.1: "Segment number (set to 1 in UMCBI)".
    let mut s = sample();
    s.segment = 0;
    let bytes = chn::write(&s).unwrap();
    assert_eq!(words(&bytes[4..6]), vec![1]);
}

// --- S3/S4: `.Spc` dates ---

fn decday(at: NaiveDateTime) -> f64 {
    let epoch = NaiveDate::from_ymd_opt(1979, 1, 1)
        .unwrap()
        .and_hms_opt(0, 0, 0)
        .unwrap();
    (at - epoch).num_milliseconds() as f64 / 86_400_000.0
}

#[test]
fn spc_acqtim_holds_the_start_as_a_decday_date() {
    let bytes = spc::write(&sample()).unwrap();
    let acqtim = f32::from_le_bytes(bytes[68..72].try_into().unwrap());
    let acqti8 = f64::from_le_bytes(bytes[72..80].try_into().unwrap());
    let expected = decday(start());
    assert!((acqti8 - expected).abs() < 1e-6, "{acqti8} vs {expected}");
    assert!((f64::from(acqtim) - expected).abs() < 0.01, "{acqtim}");
    // The FSM example: 2-Jan-79 12:00 is 1.5.
    let noon = NaiveDate::from_ymd_opt(1979, 1, 2)
        .unwrap()
        .and_hms_opt(12, 0, 0)
        .unwrap();
    assert!((decday(noon) - 1.5).abs() < 1e-12);
}

#[test]
fn spc_acqtim_is_zero_when_there_is_no_start() {
    let mut s = sample();
    s.start_time = None;
    let bytes = spc::write(&s).unwrap();
    assert_eq!(&bytes[68..80], &[0u8; 12]);
}

#[test]
fn spc_sample_collection_dates_are_not_invented() {
    // FSM §4.11.2: collection start date at byte 90, start time 102, stop
    // date 110, stop time 120. MantaRay has no collection time to put there.
    let bytes = spc::write(&sample()).unwrap();
    let record = &bytes[2 * spc::RECORD_SIZE..3 * spc::RECORD_SIZE];
    assert!(
        record[88..128].iter().all(|b| *b == 0 || *b == b' '),
        "{:?}",
        String::from_utf8_lossy(&record[88..128])
    );
    // The acquisition date itself stays where it was.
    assert_eq!(&record[16..26], b"09-NOV-261");
    assert_eq!(spc::read(&bytes).unwrap().start_time, Some(start()));
}

// --- S5: `.Spc` real format is FILTYP 5 ---

#[test]
fn a_real_format_spc_with_filtyp_5_opens() {
    let spectrum = spc::read(REAL_FORMAT_SPC).unwrap();
    assert_eq!(spectrum.len(), 1024);
    let cal = spectrum.energy_calibration.expect("calibrated");
    assert!((cal.coefficients[1] - 1.9985).abs() < 1e-4, "{cal:?}");
}

// --- S6: units ---

fn mev() -> Spectrum {
    let mut s = sample();
    s.energy_calibration = Some(EnergyCalibration::new([0.00021, 0.0019985, 1.2e-9], "MeV"));
    s
}

fn assert_kev(cal: &EnergyCalibration) {
    assert!(cal.units.eq_ignore_ascii_case("keV"), "{}", cal.units);
    let want = [0.21, 1.9985, 1.2e-6];
    for (got, want) in cal.coefficients.iter().zip(want) {
        assert!((got - want).abs() <= want.abs() * 1e-5, "{cal:?}");
    }
}

#[test]
fn a_mev_calibration_is_stored_in_kev_in_chn() {
    let back = chn::read(&chn::write(&mev()).unwrap()).unwrap();
    assert_kev(back.energy_calibration.as_ref().unwrap());
}

#[test]
fn a_mev_calibration_is_stored_in_kev_in_spc() {
    let back = spc::read(&spc::write(&mev()).unwrap()).unwrap();
    assert_kev(back.energy_calibration.as_ref().unwrap());
}

#[test]
fn a_mev_calibration_is_written_in_kev_to_spe() {
    // SpecUtils only takes a `$MCA_CAL` line ending in keV.
    let text = spe::write(&mev()).unwrap();
    let line = text
        .lines()
        .skip_while(|l| *l != "$MCA_CAL:")
        .nth(2)
        .unwrap();
    assert!(line.ends_with(" keV"), "{line}");
    assert_kev(
        spe::read(&text)
            .unwrap()
            .energy_calibration
            .as_ref()
            .unwrap(),
    );
}

#[test]
fn a_unit_conversion_is_noted_on_save() {
    for format in [
        SpectrumFormat::Chn,
        SpectrumFormat::Spc,
        SpectrumFormat::Spe,
    ] {
        let notes = mantaray_formats::text_notes(&mev(), format);
        assert!(
            notes.iter().any(|n| n.contains("keV") && n.contains("MeV")),
            "{format:?}: {notes:?}"
        );
        assert!(mantaray_formats::text_notes(&sample(), format).is_empty());
    }
}

#[test]
fn units_a_binary_format_cannot_carry_are_refused_by_name() {
    let mut s = sample();
    s.energy_calibration = Some(EnergyCalibration::new([0.0, 2.0, 0.0], "ch"));
    for result in [chn::write(&s), spc::write(&s)] {
        let err = result.unwrap_err().to_string();
        assert!(err.contains("ch") && err.contains("keV"), "{err}");
    }
}

// --- S7: `.Spe` descriptions stay one line of text ---

#[test]
fn a_description_cannot_become_a_spe_keyword() {
    let mut s = sample();
    s.sample_description = "$NOTE:".into();
    let text = spe::write(&s).unwrap();
    // SpecUtils trims each line before it looks for `$`, so the `$` goes.
    assert!(!text.lines().any(|l| l.trim() == "$NOTE:"), "{text}");
    assert_eq!(spe::read(&text).unwrap().sample_description, "NOTE:");
    let notes = mantaray_formats::text_notes(&s, SpectrumFormat::Spe);
    assert!(notes.iter().any(|n| n.contains("'$'")), "{notes:?}");
}

#[test]
fn a_description_with_newlines_stays_on_one_line() {
    let mut s = sample();
    s.sample_description = "soil\r\n$DATA:\nsample 3".into();
    s.detector_description = "HPGe\n$ROI:".into();
    let text = spe::write(&s).unwrap();
    let lines: Vec<&str> = text.lines().collect();
    assert_eq!(
        lines.iter().filter(|l| **l == "$DATA:").count(),
        1,
        "{text}"
    );
    assert!(!lines.contains(&"$ROI:"), "{text}");
    let back = spe::read(&text).unwrap();
    assert_eq!(back.sample_description, "soil  $DATA: sample 3");
    assert_eq!(back.channels.len(), 1024);
}

// --- N6: `.Spc` is written, so it says so ---

#[test]
fn spc_is_listed_as_writable() {
    assert!(SpectrumFormat::Spc.can_write());
}
