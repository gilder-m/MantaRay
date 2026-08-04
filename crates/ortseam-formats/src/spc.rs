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
//!  56  u16  LSTREC  last record used; holds the energy calibration
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
//! last record: three f32 energy calibration coefficients
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
use ortseam_core::{EnergyCalibration, Spectrum};

use crate::{Cursor, FormatError};

/// Size of one record.
pub const RECORD_SIZE: usize = 128;
/// Channels in one channel-data record.
pub const CHANNELS_PER_RECORD: usize = 32;
/// Records of header and descriptions that precede the channel data.
const LEADING_RECORDS: usize = 21;
/// Records that follow the channel data (two reserved, then the calibration).
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

    // Energy calibration: the last record, falling back to the pointers.
    for candidate in [last_record, calibration_record_2, calibration_record_1] {
        if candidate == 0 {
            continue;
        }
        let Some(record) = record_bytes(bytes, candidate) else {
            continue;
        };
        let coefficients = [
            f32::from_le_bytes([record[0], record[1], record[2], record[3]]) as f64,
            f32::from_le_bytes([record[4], record[5], record[6], record[7]]) as f64,
            f32::from_le_bytes([record[8], record[9], record[10], record[11]]) as f64,
        ];
        // A usable calibration has a sane, non-zero gain.
        if coefficients.iter().all(|c| c.is_finite())
            && coefficients[1] > 0.0
            && coefficients[1] < 1e4
        {
            spectrum.energy_calibration = Some(EnergyCalibration::new(coefficients, "keV"));
            break;
        }
    }

    Ok(spectrum)
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
        write_ascii(&mut out[base..], 0, 16, "ortseam.SPC");
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

    // Last record: the energy calibration.
    if let Some(calibration) = &spectrum.energy_calibration {
        let base = (last_record - 1) * RECORD_SIZE;
        for (index, coefficient) in calibration.coefficients.iter().enumerate() {
            let offset = base + index * 4;
            out[offset..offset + 4].copy_from_slice(&(*coefficient as f32).to_le_bytes());
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
}
