//! ORTEC `.Spc` spectra.
//!
//! The file is a sequence of 128-byte records. Record 1 is a header of pointers
//! and scalars; the records it points at hold the acquisition information, the
//! descriptions, the channel data and the calibration.
//!
//! ```text
//! record 1, byte offsets (little endian)
//!   0  u16  INFTYP  information type, 1 for a spectrum
//!   2  u16  FILTYP  1 = integer channel data, 2 = floating point
//!   8  u16  ACQIRP  record holding the acquisition information
//!  10  u16  SAMDRP  record holding the sample description
//!  12  u16  DETDRP  record holding the detector description
//!  34  u16  CALRP1  first calibration record
//!  36  u16  CALRP2  second calibration record
//!  40  u16  ROIRP1  first region-of-interest record
//!  54  u16  MAXRCS  records in the file
//!  56  u16  LSTREC  last record used
//!  60  u16  SPCTRP  first record of the channel data
//!  62  u16  SPCRCN  number of channel-data records (32 channels each)
//!  64  u16  SPCCHN  number of channels
//!  66  u16  ABSTCH  absolute start channel
//!  68  f32  ACQTIM  acquisition time
//!  72  f64  ACQTI8  acquisition time, double precision
//!  82  u16  MCANU   detector (MCA) number
//!  84  u16  SEGNUM  segment number
//!  88  u16  CHNSRT  first channel stored
//!  90  f32  RLTMDT  real time in seconds
//!  94  f32  LVTMDT  live time in seconds
//!
//! acquisition information record
//!   0  16 ascii  default file name
//!  16  12 ascii  date, "DD-MMM-YY" plus a century flag ('1' for 2000+)
//!  28  10 ascii  time, "HH:MM:SS"
//!  38  10 ascii  live time in seconds
//!  48  10 ascii  real time in seconds
//!  92  10 ascii  sample collection start date
//! 102   8 ascii  sample collection start time
//! 110  10 ascii  sample collection stop date
//! 120   8 ascii  sample collection stop time
//!
//! first calibration record (CALRP1)
//!  20  3 f32  energy calibration, keV: offset, gain, quadratic
//!  32  3 f32  peak shape (FWHM) coefficients, not decoded here
//! ```
//!
//! Record numbers are one-based, so record `n` starts at byte `(n - 1) * 128`.
//!
//! The record map is documented in ORTEC's *Software File Structure Manual*; the
//! field offsets used here were confirmed against a real `TransSpec` file whose
//! `.Spe` conversion carries the same times, date and calibration. The layout
//! table in LBNL's [becquerel](https://github.com/lbl-anp/becquerel) parser was a
//! useful cross-check. Unlike a fixed record map, this reader follows the header
//! pointers, so files with any channel count work.
//!
//! Regions of interest are not decoded: their record layout is not part of the
//! material this was written from. Save regions to a `.Roi` file instead.

use chrono::{Datelike, NaiveDate, NaiveDateTime, Timelike};
use mantaray_core::{EnergyCalibration, Spectrum};

use crate::{Cursor, FormatError};

/// Size of one record.
pub const RECORD_SIZE: usize = 128;
/// Channels in one channel-data record.
pub const CHANNELS_PER_RECORD: usize = 32;
/// Records of header and descriptions that precede the channel data.
const LEADING_RECORDS: usize = 21;
/// Records that follow the channel data (two reserved, then a copy of the
/// calibration for readers that look there).
const TRAILING_RECORDS: usize = 3;

const MONTHS: [&str; 12] = [
    "JAN", "FEB", "MAR", "APR", "MAY", "JUN", "JUL", "AUG", "SEP", "OCT", "NOV", "DEC",
];

/// Which channel-data representation a file uses.
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub enum ChannelFormat {
    /// 32-bit integers.
    Integer,
    /// 32-bit floats.
    Float,
}

/// Reads a `.Spc` file.
pub fn read(bytes: &[u8]) -> Result<Spectrum, FormatError> {
    if bytes.len() < 2 * RECORD_SIZE {
        return Err(FormatError::Truncated {
            expected: 2 * RECORD_SIZE,
            got: bytes.len(),
        });
    }
    let mut header = Cursor::new(bytes);
    let info_type = header.u16()?;
    if info_type != 1 {
        return Err(FormatError::BadMagic {
            expected: ".Spc (information type 1)".into(),
            found: format!("information type {info_type}"),
        });
    }
    let file_type = header.u16()?;
    let channel_format = match file_type {
        1 => ChannelFormat::Integer,
        2 => ChannelFormat::Float,
        other => {
            return Err(FormatError::Unsupported {
                detail: format!(".Spc file type {other}"),
            });
        }
    };

    let word = |index: usize| -> Result<u16, FormatError> {
        let offset = index * 2;
        let slice = bytes
            .get(offset..offset + 2)
            .ok_or(FormatError::Truncated {
                expected: offset + 2,
                got: bytes.len(),
            })?;
        Ok(u16::from_le_bytes([slice[0], slice[1]]))
    };
    let float = |offset: usize| -> Result<f32, FormatError> {
        let slice = bytes
            .get(offset..offset + 4)
            .ok_or(FormatError::Truncated {
                expected: offset + 4,
                got: bytes.len(),
            })?;
        Ok(f32::from_le_bytes([slice[0], slice[1], slice[2], slice[3]]))
    };

    let acquisition_record = word(4)?;
    let sample_record = word(5)?;
    let detector_record = word(6)?;
    let calibration_record_1 = word(17)?;
    let calibration_record_2 = word(18)?;
    let last_record = word(28)?;
    let spectrum_record = word(30)?;
    let spectrum_records = word(31)? as usize;
    let channels = word(32)? as usize;
    let detector_number = word(41)?;
    let segment = word(42)?;
    let channel_start = word(44)?;
    let real_time = float(90)? as f64;
    let live_time = float(94)? as f64;

    if channels == 0 || spectrum_record == 0 {
        return Err(FormatError::invalid(
            ".Spc header",
            format!("{channels} channels at record {spectrum_record}"),
        ));
    }
    let expected_records = spectrum_records.max(channels.div_ceil(CHANNELS_PER_RECORD));
    let data_offset = (spectrum_record as usize - 1) * RECORD_SIZE;
    let data_length = expected_records * RECORD_SIZE;
    if bytes.len() < data_offset + data_length {
        return Err(FormatError::Truncated {
            expected: data_offset + data_length,
            got: bytes.len(),
        });
    }

    let mut data = Cursor::new(&bytes[data_offset..data_offset + data_length]);
    let mut counts = Vec::with_capacity(channels);
    for _ in 0..channels {
        let value = match channel_format {
            ChannelFormat::Integer => data.u32()? as u64,
            ChannelFormat::Float => data.f32()?.max(0.0).round() as u64,
        };
        counts.push(value);
    }

    let mut spectrum = Spectrum::from_counts(counts);
    spectrum.detector_id = detector_number;
    spectrum.segment = segment;
    spectrum.channel_offset = channel_start;
    spectrum.real_time = real_time;
    spectrum.live_time = live_time;

    // Descriptions.
    spectrum.sample_description = record_text(bytes, sample_record).unwrap_or_default();
    spectrum.detector_description = record_text(bytes, detector_record).unwrap_or_default();

    // Acquisition information: date, time, and the ASCII times as a fallback.
    if let Some(record) = record_bytes(bytes, acquisition_record) {
        let date = ascii(&record[16..28]);
        let time = ascii(&record[28..38]);
        spectrum.start_time = parse_datetime(&date, &time);
        if spectrum.live_time <= 0.0 {
            spectrum.live_time = ascii(&record[38..48]).trim().parse().unwrap_or(0.0);
        }
        if spectrum.real_time <= 0.0 {
            spectrum.real_time = ascii(&record[48..58]).trim().parse().unwrap_or(0.0);
        }
    }

    // Energy calibration. GammaVision keeps it in the first calibration
    // record, three f32 coefficients from byte 20; confirmed on a GammaVision
    // 6.09 file against the `.Clb` it was calibrated with. This program's
    // writer puts the same twelve bytes in the last record as well (where
    // earlier versions kept it), so a file whose two copies are bit-identical
    // was written by this program, and what it holds is believed whatever the
    // numbers (a gain of 20000 keV per channel is a calibration somebody wrote)
    // as long as they are finite and energy changes with the channel. Any
    // other file - GammaVision's or anyone's - gets the plausibility filter on
    // every candidate, tried in the order earlier versions did: byte 20 of the
    // first calibration record, byte 0 of the last record, then byte 0 of each
    // pointer. A record inside the channel data is never a calibration: in
    // that GammaVision file the last record *is* channel data, and its counts
    // read as coefficients of 1e-43 put every peak below a thousandth of a keV.
    let data_records = spectrum_record as usize..spectrum_record as usize + expected_records;
    let candidate_triplet = |candidate: u16, at: usize| -> Option<[u8; 12]> {
        if candidate == 0 || data_records.contains(&(candidate as usize)) {
            return None;
        }
        record_bytes(bytes, candidate)?
            .get(at..at + 12)?
            .try_into()
            .ok()
    };
    let triplet = |raw: [u8; 12]| -> [f32; 3] {
        let word = |index: usize| {
            let start = index * 4;
            f32::from_le_bytes([raw[start], raw[start + 1], raw[start + 2], raw[start + 3]])
        };
        [word(0), word(1), word(2)]
    };
    let stated = candidate_triplet(calibration_record_1, CALIBRATION_OFFSET);
    let written_by_us = match (stated, candidate_triplet(last_record, 0)) {
        (Some(first), Some(last)) if first == last => stated_calibration(triplet(first)),
        _ => None,
    };
    if written_by_us.is_some() {
        spectrum.energy_calibration = written_by_us;
    } else {
        for (candidate, at) in [
            (calibration_record_1, CALIBRATION_OFFSET),
            (last_record, 0),
            (calibration_record_2, 0),
            (calibration_record_1, 0),
        ] {
            let Some(raw) = candidate_triplet(candidate, at) else {
                continue;
            };
            if let Some(calibration) = plausible_calibration(triplet(raw), channels) {
                spectrum.energy_calibration = Some(calibration);
                break;
            }
        }
    }

    Ok(spectrum)
}

/// Byte offset of the energy coefficients inside GammaVision's calibration
/// record. The peak-shape coefficients follow at byte 32.
const CALIBRATION_OFFSET: usize = 20;

/// The coefficients of a calibration this program wrote (the two copies in the
/// file are bit-identical): any finite numbers, as long as energy changes with
/// the channel. A record of zeros is no calibration.
fn stated_calibration(coefficients: [f32; 3]) -> Option<EnergyCalibration> {
    let [offset, gain, quadratic] = coefficients;
    let usable = coefficients.iter().all(|c| c.is_finite()) && (gain != 0.0 || quadratic != 0.0);
    usable.then(|| {
        EnergyCalibration::new(
            [f64::from(offset), f64::from(gain), f64::from(quadratic)],
            "keV",
        )
    })
}

/// Three coefficients that could be an energy calibration in keV, or `None`.
/// For every record that is not a calibration this program wrote; see
/// [`stated_calibration`].
///
/// Every coefficient must be zero or a normal float - counts and text read as
/// floats are mostly subnormal or absurd - the gain must lie between 0.0001
/// and 1000 keV per channel, the offset within 10 MeV of zero, and energy
/// must still rise at the last channel.
fn plausible_calibration(coefficients: [f32; 3], channels: usize) -> Option<EnergyCalibration> {
    if !coefficients.iter().all(|c| *c == 0.0 || c.is_normal()) {
        return None;
    }
    let [offset, gain, quadratic] = coefficients.map(f64::from);
    let last = channels.saturating_sub(1) as f64;
    let plausible =
        (1e-4..1e3).contains(&gain) && offset.abs() < 1e4 && gain + 2.0 * quadratic * last > 0.0;
    plausible.then(|| EnergyCalibration::new([offset, gain, quadratic], "keV"))
}

/// Writes an integer `.Spc` file.
pub fn write(spectrum: &Spectrum) -> Result<Vec<u8>, FormatError> {
    let channels = spectrum.len();
    if channels == 0 {
        return Err(FormatError::invalid(".Spc", "an empty spectrum"));
    }
    if channels > u16::MAX as usize {
        return Err(FormatError::Unsupported {
            detail: format!(".Spc holds at most {} channels", u16::MAX),
        });
    }
    let spectrum_records = channels.div_ceil(CHANNELS_PER_RECORD);
    let total_records = LEADING_RECORDS + spectrum_records + TRAILING_RECORDS;
    let spectrum_record = LEADING_RECORDS + 1;
    let last_record = total_records;
    let mut out = vec![0u8; total_records * RECORD_SIZE];

    {
        // Record 1: the header.
        let put_word = |out: &mut [u8], index: usize, value: u16| {
            let offset = index * 2;
            out[offset..offset + 2].copy_from_slice(&value.to_le_bytes());
        };
        let put_f32 = |out: &mut [u8], offset: usize, value: f32| {
            out[offset..offset + 4].copy_from_slice(&value.to_le_bytes());
        };
        let put_f64 = |out: &mut [u8], offset: usize, value: f64| {
            out[offset..offset + 8].copy_from_slice(&value.to_le_bytes());
        };
        put_word(&mut out, 0, 1); // INFTYP
        put_word(&mut out, 1, 1); // FILTYP: integer
        put_word(&mut out, 4, 3); // ACQIRP
        put_word(&mut out, 5, 4); // SAMDRP
        put_word(&mut out, 6, 5); // DETDRP
        put_word(&mut out, 17, 6); // CALRP1
        put_word(&mut out, 18, 7); // CALRP2
        put_word(&mut out, 20, 0); // ROIRP1: regions are not written
        put_word(&mut out, 27, total_records as u16); // MAXRCS
        put_word(&mut out, 28, last_record as u16); // LSTREC
        put_word(&mut out, 30, spectrum_record as u16); // SPCTRP
        put_word(&mut out, 31, spectrum_records as u16); // SPCRCN
        put_word(&mut out, 32, channels as u16); // SPCCHN
        put_word(&mut out, 33, 0); // ABSTCH
        put_f32(&mut out, 68, spectrum.real_time as f32); // ACQTIM
        put_f64(&mut out, 72, spectrum.real_time); // ACQTI8
        put_word(&mut out, 41, spectrum.detector_id); // MCANU
        put_word(&mut out, 42, spectrum.segment); // SEGNUM
        put_word(&mut out, 44, spectrum.channel_offset); // CHNSRT
        put_f32(&mut out, 90, spectrum.real_time as f32); // RLTMDT
        put_f32(&mut out, 94, spectrum.live_time as f32); // LVTMDT
    }

    // Record 3: acquisition information.
    {
        let base = 2 * RECORD_SIZE;
        let (date, time) = match spectrum.start_time {
            Some(start) => (
                format!(
                    "{:02}-{}-{:02}{}",
                    start.day(),
                    MONTHS[start.month0() as usize],
                    start.year().rem_euclid(100),
                    if start.year() >= 2000 { '1' } else { ' ' }
                ),
                format!(
                    "{:02}:{:02}:{:02}",
                    start.hour(),
                    start.minute(),
                    start.second()
                ),
            ),
            None => (String::new(), String::new()),
        };
        write_ascii(&mut out[base..], 0, 16, "mantaray.SPC");
        write_ascii(&mut out[base..], 16, 12, &date);
        write_ascii(&mut out[base..], 28, 10, &time);
        write_ascii(
            &mut out[base..],
            38,
            10,
            &format!("{:>10.0}", spectrum.live_time),
        );
        write_ascii(
            &mut out[base..],
            48,
            10,
            &format!("{:>10.0}", spectrum.real_time),
        );
        write_ascii(&mut out[base..], 92, 10, &date);
        write_ascii(&mut out[base..], 102, 8, &time);
        write_ascii(&mut out[base..], 110, 10, &date);
        write_ascii(&mut out[base..], 120, 8, &time);
    }

    // Records 4 and 5: descriptions.
    write_ascii(
        &mut out[3 * RECORD_SIZE..],
        0,
        RECORD_SIZE,
        &spectrum.sample_description,
    );
    write_ascii(
        &mut out[4 * RECORD_SIZE..],
        0,
        RECORD_SIZE,
        &spectrum.detector_description,
    );

    // Channel data.
    let data_offset = (spectrum_record - 1) * RECORD_SIZE;
    for (index, count) in spectrum.channels.iter().enumerate() {
        let offset = data_offset + index * 4;
        let value = (*count).min(u32::MAX as u64) as u32;
        out[offset..offset + 4].copy_from_slice(&value.to_le_bytes());
    }

    // The energy calibration, where GammaVision keeps it (record 6, the first
    // calibration record) and in the last record, where earlier versions of
    // this program look for it. The file stores f32, so a coefficient that
    // is not zero and does not come out of the conversion as a normal float
    // (it underflows to zero, lands in the subnormals where digits are lost,
    // overflows to infinity, or is NaN) cannot be stored as it is. It is
    // refused by name instead of written as something else.
    if let Some(calibration) = &spectrum.energy_calibration {
        for (name, coefficient) in ["offset", "gain", "quadratic term"]
            .into_iter()
            .zip(calibration.coefficients)
        {
            if coefficient != 0.0 && !(coefficient as f32).is_normal() {
                return Err(FormatError::invalid(
                    ".Spc",
                    format!(
                        "the energy calibration {name} {coefficient:e} cannot be stored in a \
                         32-bit float"
                    ),
                ));
            }
        }
        for base in [
            5 * RECORD_SIZE + CALIBRATION_OFFSET,
            (last_record - 1) * RECORD_SIZE,
        ] {
            for (index, coefficient) in calibration.coefficients.iter().enumerate() {
                let offset = base + index * 4;
                out[offset..offset + 4].copy_from_slice(&(*coefficient as f32).to_le_bytes());
            }
        }
    }

    Ok(out)
}

fn record_bytes(bytes: &[u8], record: u16) -> Option<&[u8]> {
    if record == 0 {
        return None;
    }
    let offset = (record as usize - 1) * RECORD_SIZE;
    bytes.get(offset..offset + RECORD_SIZE)
}

fn record_text(bytes: &[u8], record: u16) -> Option<String> {
    let record = record_bytes(bytes, record)?;
    let text = ascii(record);
    Some(text.trim().to_string())
}

/// Reads a fixed-width ASCII field, dropping the padding ORTEC leaves behind.
fn ascii(bytes: &[u8]) -> String {
    String::from_utf8_lossy(bytes)
        .chars()
        .map(|c| if c == '\0' || c == '\u{1}' { ' ' } else { c })
        .collect::<String>()
        .trim_end()
        .to_string()
}

fn write_ascii(out: &mut [u8], offset: usize, width: usize, text: &str) {
    let bytes: Vec<u8> = text
        .bytes()
        .filter(|b| b.is_ascii() && *b >= 0x20)
        .take(width)
        .collect();
    out[offset..offset + bytes.len()].copy_from_slice(&bytes);
    for byte in &mut out[offset + bytes.len()..offset + width] {
        *byte = b' ';
    }
}

/// Parses `"DD-MMM-YY"` plus a century flag, with `"HH:MM:SS"`.
fn parse_datetime(date: &str, time: &str) -> Option<NaiveDateTime> {
    let date = date.trim();
    let time = time.trim();
    if date.len() < 9 {
        return None;
    }
    let day: u32 = date.get(0..2)?.trim().parse().ok()?;
    let month_text = date.get(3..6)?.to_ascii_uppercase();
    let month = MONTHS.iter().position(|m| *m == month_text)? as u32 + 1;
    let year_text = date.get(7..9)?;
    let two_digit: i32 = year_text.trim().parse().ok()?;
    let mut year = 1900 + two_digit;
    if let Some(flag) = date.as_bytes().get(9)
        && flag.is_ascii_digit()
    {
        year += (flag - b'0') as i32 * 100;
    }
    let mut parts = time.split(':');
    let hour: u32 = parts.next()?.trim().parse().ok()?;
    let minute: u32 = parts.next().unwrap_or("0").trim().parse().unwrap_or(0);
    let second: f64 = parts.next().unwrap_or("0").trim().parse().unwrap_or(0.0);
    NaiveDate::from_ymd_opt(year, month, day)?.and_hms_opt(hour, minute, second as u32)
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn a_round_trip_preserves_the_data_the_format_carries() {
        let mut original = Spectrum::from_counts((0..1024u64).map(|c| c * 5 + 1).collect());
        original.live_time = 900.0;
        original.real_time = 905.42;
        original.detector_id = 3;
        original.segment = 1;
        original.sample_description = "Alcatraz14".into();
        original.detector_description = "Transpec MCB129".into();
        original.energy_calibration = Some(EnergyCalibration::new(
            [0.5783, 0.374436, 2.985859e-7],
            "keV",
        ));
        original.start_time = NaiveDate::from_ymd_opt(2012, 9, 17)
            .unwrap()
            .and_hms_opt(13, 41, 7);

        let bytes = write(&original).unwrap();
        assert_eq!(bytes.len() % RECORD_SIZE, 0);
        let back = read(&bytes).unwrap();

        assert_eq!(back.channels, original.channels);
        assert_eq!(back.detector_id, 3);
        assert_eq!(back.segment, 1);
        assert_eq!(back.sample_description, "Alcatraz14");
        assert_eq!(back.detector_description, "Transpec MCB129");
        assert!((back.live_time - 900.0).abs() < 0.01);
        assert!((back.real_time - 905.42).abs() < 0.01);
        assert_eq!(back.start_time, original.start_time);
        let cal = back.energy_calibration.unwrap();
        assert!((cal.coefficients[1] - 0.374436).abs() < 1e-6);
        assert!((cal.coefficients[2] - 2.985859e-7).abs() < 1e-12);
    }

    #[test]
    fn the_record_map_matches_the_reference_file() {
        // 8192 channels give 256 data records and 280 records in total, which is
        // what the reference TransSpec file contains.
        let spectrum = Spectrum::from_counts(vec![0; 8192]);
        let bytes = write(&spectrum).unwrap();
        assert_eq!(bytes.len() / RECORD_SIZE, 280);
        assert_eq!(u16::from_le_bytes([bytes[60], bytes[61]]), 22, "SPCTRP");
        assert_eq!(u16::from_le_bytes([bytes[62], bytes[63]]), 256, "SPCRCN");
        assert_eq!(u16::from_le_bytes([bytes[64], bytes[65]]), 8192, "SPCCHN");
        assert_eq!(u16::from_le_bytes([bytes[56], bytes[57]]), 280, "LSTREC");
    }

    #[test]
    fn dates_carry_a_century_flag() {
        assert_eq!(
            parse_datetime("17-SEP-121", "13:41:07"),
            NaiveDate::from_ymd_opt(2012, 9, 17)
                .unwrap()
                .and_hms_opt(13, 41, 7)
        );
        assert_eq!(
            parse_datetime("05-MAR-98 ", "08:07:06"),
            NaiveDate::from_ymd_opt(1998, 3, 5)
                .unwrap()
                .and_hms_opt(8, 7, 6)
        );
        assert!(parse_datetime("", "").is_none());
    }

    #[test]
    fn a_foreign_file_is_rejected() {
        let mut bytes = vec![0u8; 4 * RECORD_SIZE];
        bytes[0] = 9;
        assert!(matches!(read(&bytes), Err(FormatError::BadMagic { .. })));
    }

    #[test]
    fn a_truncated_file_is_rejected() {
        let bytes = write(&Spectrum::from_counts(vec![1; 512])).unwrap();
        assert!(matches!(
            read(&bytes[..RECORD_SIZE * 4]),
            Err(FormatError::Truncated { .. })
        ));
    }

    /// A file laid out the way GammaVision 6.09 writes one: no trailing
    /// records, so the last record used is the last record of channel data,
    /// and the calibration in record 6 from byte 20. The coefficients are the
    /// ones such a file carried, matching the `.Clb` it was calibrated with.
    fn gammavision_layout(calibration: Option<[f32; 3]>) -> Vec<u8> {
        let counts: Vec<u64> = (0..1024u64).map(|c| 100 + c % 60).collect();
        let mut bytes = write(&Spectrum::from_counts(counts)).unwrap();
        let data_records = 1024 / CHANNELS_PER_RECORD;
        let last = LEADING_RECORDS + data_records;
        bytes.truncate(last * RECORD_SIZE);
        bytes[56..58].copy_from_slice(&(last as u16).to_le_bytes()); // LSTREC
        bytes[54..56].copy_from_slice(&(last as u16).to_le_bytes()); // MAXRCS
        if let Some(coefficients) = calibration {
            for (index, coefficient) in coefficients.iter().enumerate() {
                let at = 5 * RECORD_SIZE + 20 + index * 4;
                bytes[at..at + 4].copy_from_slice(&coefficient.to_le_bytes());
            }
        }
        bytes
    }

    #[test]
    fn a_gammavision_file_reads_its_calibration_from_the_calibration_record() {
        // Before, the last record - channel data here - was read first, and
        // counts of 100-160 passed as a gain of 2e-43 keV per channel.
        let bytes = gammavision_layout(Some([14.340785, 0.3175845, -3.695523e-8]));
        let calibration = read(&bytes)
            .unwrap()
            .energy_calibration
            .expect("calibrated");
        assert!((calibration.coefficients[0] - 14.340785).abs() < 1e-5);
        assert!((calibration.coefficients[1] - 0.3175845).abs() < 1e-7);
        assert!((calibration.coefficients[2] + 3.695523e-8).abs() < 1e-13);
    }

    #[test]
    fn channel_data_is_never_taken_for_a_calibration() {
        // With no calibration in the file, the counts in the last record must
        // leave the spectrum uncalibrated, not scaled to nothing.
        let back = read(&gammavision_layout(None)).unwrap();
        assert_eq!(back.energy_calibration, None);
    }

    #[test]
    fn written_files_put_the_calibration_where_gammavision_keeps_it() {
        let mut spectrum = Spectrum::from_counts(vec![1; 64]);
        spectrum.energy_calibration = Some(EnergyCalibration::new([1.5, 0.25, 0.0], "keV"));
        let bytes = write(&spectrum).unwrap();
        let at = 5 * RECORD_SIZE + 20;
        let gain = f32::from_le_bytes(bytes[at + 4..at + 8].try_into().unwrap());
        assert_eq!(gain, 0.25);
    }

    #[test]
    fn a_file_from_an_earlier_version_still_reads_its_calibration() {
        // Earlier versions wrote the calibration only into the last record.
        let mut spectrum = Spectrum::from_counts(vec![1; 64]);
        spectrum.energy_calibration = Some(EnergyCalibration::new([1.5, 0.25, 0.0], "keV"));
        let mut bytes = write(&spectrum).unwrap();
        let at = 5 * RECORD_SIZE + 20;
        bytes[at..at + 12].fill(0);
        let calibration = read(&bytes)
            .unwrap()
            .energy_calibration
            .expect("calibrated");
        assert_eq!(calibration.coefficients, [1.5, 0.25, 0.0]);
    }

    #[test]
    fn implausible_coefficients_are_refused() {
        let tiny = f32::from_bits(127); // a count of 127, read as a float
        for coefficients in [
            [0.0, tiny, 0.0],
            [0.0, 0.0, 0.0],
            [0.0, -0.3, 0.0],
            [0.0, 5e3, 0.0],
            [5e4, 0.3, 0.0],
            [0.0, 0.3, -1e-3], // energy falling by the last channel
            [f32::NAN, 0.3, 0.0],
            [1e4, 0.3, 0.0], // the offset bound is exclusive
        ] {
            assert_eq!(
                plausible_calibration(coefficients, 1024),
                None,
                "{coefficients:?}"
            );
        }
        // Energy flat at the last channel is not rising.
        assert_eq!(plausible_calibration([0.0, 0.5, -0.25], 2), None);
        assert!(plausible_calibration([14.34, 0.3176, -3.7e-8], 8192).is_some());
    }

    #[test]
    fn a_falling_quadratic_in_a_candidate_that_is_not_ours_is_rejected() {
        // [0, 0.3, -1e-3] at 1024 channels has energy falling before the last
        // channel. In a file whose two copies differ nothing vouches for it.
        let mut spectrum = Spectrum::from_counts(vec![1; 1024]);
        spectrum.energy_calibration = Some(EnergyCalibration::new([0.0, 0.3, -1e-3], "keV"));
        let mut bytes = write(&spectrum).unwrap();
        let at = 5 * RECORD_SIZE + CALIBRATION_OFFSET;
        bytes[at..at + 4].copy_from_slice(&1.0f32.to_le_bytes()); // copies now differ
        bytes[at + 4..at + 8].copy_from_slice(&0.3f32.to_le_bytes());
        bytes[at + 8..at + 12].copy_from_slice(&(-1e-3f32).to_le_bytes());
        assert_eq!(read(&bytes).unwrap().energy_calibration, None);
    }

    /// What `read` gives for a calibration written by `write`.
    fn through_the_file(coefficients: [f64; 3]) -> Option<[f64; 3]> {
        let mut spectrum = Spectrum::from_counts(vec![1; 64]);
        spectrum.energy_calibration = Some(EnergyCalibration::new(coefficients, "keV"));
        read(&write(&spectrum).unwrap())
            .unwrap()
            .energy_calibration
            .map(|calibration| calibration.coefficients)
    }

    #[test]
    fn the_reader_accepts_every_calibration_the_writer_writes() {
        // The file stores f32. Whatever the writer can emit comes back as the
        // f32 it stored - including the extremes a plausibility filter on the
        // reader's side would refuse: a gain of 1000 keV per channel or more,
        // an offset of 10 MeV or more, and a falling energy.
        let offsets = [0.0, 0.5783, -14.34, 9_999.0, 1e4, -1e4, 5e5, -3e6];
        let gains = [
            1e-4, 0.0123, 0.374436, 1.0, 999.9, 1e3, 2_500.0, 1e6, 1e-7, -0.3,
        ];
        let quadratics = [
            0.0,
            2.985859e-7,
            -3.695523e-8,
            1e-30, // small, but a normal f32
            -1e-37,
            1e-3,
            -1e-3, // energy falling by the last channel
        ];
        let mut checked = 0;
        for offset in offsets {
            for gain in gains {
                for quadratic in quadratics {
                    let wrote = [offset, gain, quadratic];
                    let back = through_the_file(wrote)
                        .unwrap_or_else(|| panic!("{wrote:?} was written and not read"));
                    for (got, put) in back.iter().zip(wrote) {
                        assert_eq!(*got, f64::from(put as f32), "{wrote:?} came back {back:?}");
                    }
                    checked += 1;
                }
            }
        }
        assert_eq!(checked, 8 * 10 * 7);
    }

    #[test]
    fn a_calibration_that_does_not_fit_the_file_is_refused_not_dropped() {
        // 1e300 is infinity as an f32: the file would carry no calibration and
        // say nothing.
        for coefficients in [[1e300, 0.3, 0.0], [0.0, f64::NAN, 0.0], [0.0, 0.3, 1e39]] {
            let mut spectrum = Spectrum::from_counts(vec![1; 64]);
            spectrum.energy_calibration = Some(EnergyCalibration::new(coefficients, "keV"));
            assert!(
                matches!(write(&spectrum), Err(FormatError::Invalid { .. })),
                "{coefficients:?}"
            );
        }
    }

    #[test]
    fn a_coefficient_an_f32_cannot_carry_is_refused_by_name() {
        // Underflow to zero, a subnormal that keeps few digits, overflow, NaN:
        // none may be written as something else without a word.
        for (coefficients, name) in [
            ([0.0, 1e-46, 0.0], "gain"),
            ([0.0, 0.3, 1e-46], "quadratic term"),
            ([1e-46, 0.3, 0.0], "offset"),
            ([0.0, 1e-40, 0.0], "gain"),
            ([0.0, 0.3, -1e-42], "quadratic term"),
            ([0.0, 1e39, 0.0], "gain"),
            ([f64::NAN, 0.3, 0.0], "offset"),
        ] {
            let mut spectrum = Spectrum::from_counts(vec![1; 64]);
            spectrum.energy_calibration = Some(EnergyCalibration::new(coefficients, "keV"));
            let error = write(&spectrum).expect_err("refused").to_string();
            assert!(error.contains(name), "{coefficients:?}: {error}");
        }
        // Zero is zero, and ordinary calibrations are unaffected.
        for coefficients in [[0.0, 0.5, 0.0], [-0.0, 0.5, -0.0], [14.34, 0.3176, -3.7e-8]] {
            let mut spectrum = Spectrum::from_counts(vec![1; 64]);
            spectrum.energy_calibration = Some(EnergyCalibration::new(coefficients, "keV"));
            assert!(write(&spectrum).is_ok(), "{coefficients:?}");
        }
    }

    #[test]
    fn junk_in_the_calibration_record_does_not_hide_a_real_calibration_elsewhere() {
        // A third-party file with finite but absurd numbers at record 6 byte
        // 20 and a real calibration at byte 0 of the last record: the copies
        // differ, so nothing is taken on trust and the plausible one wins.
        let mut spectrum = Spectrum::from_counts(vec![1; 64]);
        spectrum.energy_calibration = Some(EnergyCalibration::new([1.5, 0.25, 0.0], "keV"));
        let mut bytes = write(&spectrum).unwrap();
        let at = 5 * RECORD_SIZE + CALIBRATION_OFFSET;
        for (index, junk) in [0.0f32, 5_000.0, 0.0].iter().enumerate() {
            bytes[at + index * 4..at + index * 4 + 4].copy_from_slice(&junk.to_le_bytes());
        }
        let calibration = read(&bytes)
            .unwrap()
            .energy_calibration
            .expect("the real one");
        assert_eq!(calibration.coefficients, [1.5, 0.25, 0.0]);
    }

    #[test]
    fn a_calibration_only_one_record_carries_must_look_like_one() {
        // No second copy to agree with it: an extreme gain is not believed.
        let mut spectrum = Spectrum::from_counts(vec![1; 64]);
        spectrum.energy_calibration = Some(EnergyCalibration::new([0.0, 20_000.0, 0.0], "keV"));
        let mut bytes = write(&spectrum).unwrap();
        let last = bytes.len() - RECORD_SIZE;
        bytes[last..last + 12].fill(0);
        assert_eq!(read(&bytes).unwrap().energy_calibration, None);
    }

    #[test]
    fn a_file_with_no_calibration_reads_with_none() {
        // The calibration record the writer always fills is zero: that is no
        // calibration, not a gain of nothing.
        let spectrum = Spectrum::from_counts(vec![1; 64]);
        assert_eq!(
            read(&write(&spectrum).unwrap()).unwrap().energy_calibration,
            None
        );
    }

    #[test]
    fn float_channel_data_is_never_taken_for_a_calibration() {
        // Counts stored as floats read as perfectly plausible coefficients -
        // 132, 133, 134 - so only knowing where the channel data lies keeps
        // them out.
        let mut bytes = gammavision_layout(None);
        bytes[2..4].copy_from_slice(&2u16.to_le_bytes()); // FILTYP: floats
        let data = LEADING_RECORDS * RECORD_SIZE;
        for channel in 0..1024 {
            let value = (100 + channel % 60) as f32;
            let at = data + channel * 4;
            bytes[at..at + 4].copy_from_slice(&value.to_le_bytes());
        }
        let back = read(&bytes).unwrap();
        assert_eq!(back.channels[992], 132, "the data reads as floats");
        assert_eq!(back.energy_calibration, None);
    }
}
