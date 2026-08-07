//! ORTEC `.Chn` integer spectrum files.
//!
//! The layout is taken from the reader published in Appendix B of the MAESTRO
//! v7 manual. All fields are little endian.
//!
//! ```text
//! 0   i16    file type, always -1
//! 2   i16    MCA (detector) number
//! 4   i16    segment number
//! 6   2 char seconds of the start time, ASCII
//! 8   i32    real time in 20 ms ticks
//! 12  i32    live time in 20 ms ticks
//! 16  8 char start date: DD MMM YY plus a century flag ('1' for 2000+, ' ' for 1900s)
//! 24  4 char start time: HHMM
//! 28  u16    channel offset of the first stored channel
//! 30  u16    number of channels
//! 32  4*n    i32 channel data
//! ```
//!
//! A 512-byte trailer follows the data:
//!
//! ```text
//! 0   i16     -102 (with a quadratic term) or -101
//! 2   i16     reserved
//! 4   3 f32   energy calibration: zero, gain, quadratic
//! 16  3 f32   peak-shape calibration: zero, gain, quadratic
//! 28  228     reserved
//! 256 1 + 63  detector description: length byte then text
//! 320 1 + 63  sample description: length byte then text
//! 384 128     reserved
//! ```

use chrono::{Datelike, NaiveDate, NaiveDateTime, Timelike};
use mantaray_core::{EnergyCalibration, ShapeCalibration, Spectrum};

use crate::{Cursor, FormatError};

/// Duration of one tick of the stored times, in seconds.
pub const TICK_SECONDS: f64 = 0.02;

/// Size of the fixed header, in bytes.
pub const HEADER_SIZE: usize = 32;

const DESCRIPTION_LEN: usize = 63;
const MONTHS: [&str; 12] = [
    "JAN", "FEB", "MAR", "APR", "MAY", "JUN", "JUL", "AUG", "SEP", "OCT", "NOV", "DEC",
];

/// Reads a `.Chn` file.
pub fn read(bytes: &[u8]) -> Result<Spectrum, FormatError> {
    let mut cursor = Cursor::new(bytes);
    let file_type = cursor.i16()?;
    if file_type != -1 {
        return Err(FormatError::BadMagic {
            expected: ".Chn (file type -1)".into(),
            found: format!("file type {file_type}"),
        });
    }
    let detector_id = cursor.i16()? as u16;
    let segment = cursor.i16()? as u16;
    let seconds_text = String::from_utf8_lossy(cursor.take(2)?).to_string();
    let real_ticks = cursor.i32()?;
    let live_ticks = cursor.i32()?;
    let date_text = String::from_utf8_lossy(cursor.take(8)?).to_string();
    let time_text = String::from_utf8_lossy(cursor.take(4)?).to_string();
    let channel_offset = cursor.u16()?;
    let channel_count = cursor.u16()? as usize;

    let mut channels = Vec::with_capacity(channel_count);
    for _ in 0..channel_count {
        channels.push(cursor.i32()?.max(0) as u64);
    }

    let mut spectrum = Spectrum::from_counts(channels);
    spectrum.detector_id = detector_id;
    spectrum.segment = segment;
    spectrum.channel_offset = channel_offset;
    spectrum.real_time = real_ticks as f64 * TICK_SECONDS;
    spectrum.live_time = live_ticks as f64 * TICK_SECONDS;
    spectrum.start_time = parse_start(&date_text, &time_text, &seconds_text);

    // The trailer is optional: some tools write only header and data.
    if cursor.remaining() >= 28 {
        let footer_type = cursor.i16()?;
        cursor.skip(2)?;
        let energy = [
            cursor.f32()? as f64,
            cursor.f32()? as f64,
            cursor.f32()? as f64,
        ];
        let shape = [
            cursor.f32()? as f64,
            cursor.f32()? as f64,
            cursor.f32()? as f64,
        ];
        // -101 marks a calibration without the quadratic term.
        let energy = if footer_type == -101 {
            [energy[0], energy[1], 0.0]
        } else {
            energy
        };
        if energy.iter().any(|c| *c != 0.0) {
            spectrum.energy_calibration = Some(EnergyCalibration::new(energy, "keV"));
        }
        if shape.iter().any(|c| *c != 0.0) {
            spectrum.shape_calibration = Some(ShapeCalibration::new(shape));
        }
        if cursor.remaining() >= 228 + 2 * (1 + DESCRIPTION_LEN) {
            cursor.skip(228)?;
            spectrum.detector_description = read_description(&mut cursor)?;
            spectrum.sample_description = read_description(&mut cursor)?;
        }
    }
    Ok(spectrum)
}

/// Writes a `.Chn` file.
pub fn write(spectrum: &Spectrum) -> Result<Vec<u8>, FormatError> {
    if spectrum.len() > u16::MAX as usize {
        return Err(FormatError::Unsupported {
            detail: format!(
                ".Chn holds at most {} channels, this spectrum has {}",
                u16::MAX,
                spectrum.len()
            ),
        });
    }
    let mut out = Vec::with_capacity(HEADER_SIZE + 4 * spectrum.len() + 512);
    out.extend_from_slice(&(-1i16).to_le_bytes());
    out.extend_from_slice(&(spectrum.detector_id as i16).to_le_bytes());
    out.extend_from_slice(&(spectrum.segment as i16).to_le_bytes());

    let (date_field, time_field, seconds_field) = match spectrum.start_time {
        Some(start) => (
            format!(
                "{:02}{}{:02}{}",
                start.day(),
                MONTHS[(start.month0()) as usize],
                start.year().rem_euclid(100),
                if start.year() >= 2000 { '1' } else { ' ' }
            ),
            format!("{:02}{:02}", start.hour(), start.minute()),
            format!("{:02}", start.second()),
        ),
        None => ("        ".to_string(), "    ".to_string(), "  ".to_string()),
    };
    out.extend_from_slice(seconds_field.as_bytes());
    let real_ticks = (spectrum.real_time / TICK_SECONDS)
        .round()
        .clamp(0.0, i32::MAX as f64) as i32;
    let live_ticks = (spectrum.live_time / TICK_SECONDS)
        .round()
        .clamp(0.0, i32::MAX as f64) as i32;
    out.extend_from_slice(&real_ticks.to_le_bytes());
    out.extend_from_slice(&live_ticks.to_le_bytes());
    out.extend_from_slice(date_field.as_bytes());
    out.extend_from_slice(time_field.as_bytes());
    out.extend_from_slice(&spectrum.channel_offset.to_le_bytes());
    out.extend_from_slice(&(spectrum.len() as u16).to_le_bytes());

    for count in &spectrum.channels {
        let value = (*count).min(i32::MAX as u64) as i32;
        out.extend_from_slice(&value.to_le_bytes());
    }

    // Trailer.
    out.extend_from_slice(&(-102i16).to_le_bytes());
    out.extend_from_slice(&0i16.to_le_bytes());
    let energy = spectrum
        .energy_calibration
        .as_ref()
        .map(|cal| cal.coefficients)
        .unwrap_or([0.0; 3]);
    let shape = spectrum
        .shape_calibration
        .map(|cal| cal.coefficients)
        .unwrap_or([0.0; 3]);
    for value in energy.iter().chain(shape.iter()) {
        out.extend_from_slice(&(*value as f32).to_le_bytes());
    }
    out.extend_from_slice(&[0u8; 228]);
    write_description(&mut out, &spectrum.detector_description);
    write_description(&mut out, &spectrum.sample_description);
    out.extend_from_slice(&[0u8; 128]);
    Ok(out)
}

fn read_description(cursor: &mut Cursor<'_>) -> Result<String, FormatError> {
    let length = cursor.take(1)?[0] as usize;
    let text = cursor.take(DESCRIPTION_LEN)?;
    let length = length.min(DESCRIPTION_LEN);
    Ok(String::from_utf8_lossy(&text[..length])
        .trim_end_matches(['\0', ' '])
        .to_string())
}

fn write_description(out: &mut Vec<u8>, text: &str) {
    let bytes: Vec<u8> = text
        .bytes()
        .filter(|b| *b >= 0x20)
        .take(DESCRIPTION_LEN)
        .collect();
    out.push(bytes.len() as u8);
    out.extend_from_slice(&bytes);
    out.extend(std::iter::repeat_n(0u8, DESCRIPTION_LEN - bytes.len()));
}

/// Parses the split date and time fields; any malformed part yields `None`.
fn parse_start(date: &str, time: &str, seconds: &str) -> Option<NaiveDateTime> {
    if date.len() < 8 || time.len() < 4 {
        return None;
    }
    let day: u32 = date.get(0..2)?.trim().parse().ok()?;
    let month_text = date.get(2..5)?.to_ascii_uppercase();
    let month = MONTHS.iter().position(|m| *m == month_text)? as u32 + 1;
    let two_digit: i32 = date.get(5..7)?.trim().parse().ok()?;
    let century_flag = date.as_bytes()[7];
    let mut year = 1900 + two_digit;
    if century_flag.is_ascii_digit() {
        year += (century_flag - b'0') as i32 * 100;
    }
    let hour: u32 = time.get(0..2)?.trim().parse().ok()?;
    let minute: u32 = time.get(2..4)?.trim().parse().ok()?;
    let second: u32 = seconds.trim().parse().unwrap_or(0);
    NaiveDate::from_ymd_opt(year, month, day)?.and_hms_opt(hour, minute, second.min(59))
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn header_is_exactly_thirty_two_bytes() {
        let bytes = write(&Spectrum::from_counts(vec![1, 2, 3, 4])).unwrap();
        assert_eq!(&bytes[HEADER_SIZE..HEADER_SIZE + 4], &1i32.to_le_bytes());
        // header + 4 channels + 512 byte trailer
        assert_eq!(bytes.len(), HEADER_SIZE + 16 + 512);
    }

    #[test]
    fn a_blank_date_field_is_not_a_time() {
        assert!(parse_start("        ", "    ", "  ").is_none());
    }
}
