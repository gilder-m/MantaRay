//! IAEA / CTBTO ASCII `.Spe` files.
//!
//! The file is a sequence of `$KEYWORD:` blocks, each followed by its lines.
//! mantaray writes and understands:
//!
//! | Keyword | Contents |
//! |---|---|
//! | `$SPEC_ID:` | sample description |
//! | `$SPEC_REM:` | remarks; `DET#`, `DETDESC#` and `AP#` are parsed |
//! | `$DATE_MEA:` | `MM/DD/YYYY HH:MM:SS` start of acquisition |
//! | `$MEAS_TIM:` | live time then real time, in seconds |
//! | `$DATA:` | first and last channel, then one count per line |
//! | `$ROI:` | region count, then `first last` pairs |
//! | `$PRESETS:` | preset mode and values |
//! | `$ENER_FIT:` | zero and gain of the energy calibration |
//! | `$MCA_CAL:` | coefficient count, then the coefficients and units |
//! | `$SHAPE_CAL:` | coefficient count, then the shape coefficients |
//!
//! Unknown keywords are preserved on read only as far as they are needed; they
//! are ignored rather than rejected.

use chrono::{NaiveDate, NaiveDateTime};
use mantaray_core::{EnergyCalibration, Roi, ShapeCalibration, Spectrum};

use crate::{FormatError, MOST_CHANNELS, ansi};

/// Reads a `.Spe` file from its bytes, as stored on disk.
///
/// MAESTRO writes the file in Windows-1252, one byte per character; text that
/// is valid UTF-8 (pure ASCII, or what mantaray 0.2.4 and earlier wrote) is
/// read as UTF-8.
///
/// # Errors
///
/// As [`read()`].
pub fn read_bytes(bytes: &[u8]) -> Result<Spectrum, FormatError> {
    read(&ansi::decode(bytes))
}

/// Writes a `.Spe` file as the bytes to store: [`write()`] in Windows-1252,
/// the code page MAESTRO reads it in. A character it lacks is stored as `?`.
///
/// # Errors
///
/// As [`write()`].
pub fn write_bytes(spectrum: &Spectrum) -> Result<Vec<u8>, FormatError> {
    Ok(ansi::text(&write(spectrum)?).bytes)
}

/// Reads a `.Spe` file already decoded to text.
///
/// # Errors
///
/// A missing `$DATA:` block, a channel range that ends before it starts or
/// is longer than any MCB, a count or a time that is not a number.
pub fn read(text: &str) -> Result<Spectrum, FormatError> {
    let blocks = split_blocks(text);
    let data = blocks
        .iter()
        .find(|(keyword, _)| keyword == "$DATA:")
        .ok_or_else(|| FormatError::missing("$DATA: block"))?;

    let mut lines = data.1.iter();
    let range = lines
        .next()
        .ok_or_else(|| FormatError::missing("$DATA: channel range"))?;
    let mut range_parts = range.split_whitespace();
    let first: usize = parse_field(range_parts.next(), "$DATA: first channel")?;
    let mut last: usize = parse_field(range_parts.next(), "$DATA: last channel")?;
    // The range is the first and the last channel, both inclusive. MantaRay
    // 0.2.4 and earlier wrote `offset count-1` instead, so a file it saved at
    // an offset says less than it holds (or ends before it starts) while every
    // count is there. Such a file is read as what it holds: it names this
    // program, starts above 0, and has exactly `last + 1` counts (the block's
    // lines are the range line and then the counts). A file written by the
    // spec cannot match that, because its count is `last - first + 1`.
    if first > 0
        && data.1.len() == last.saturating_add(2)
        && blocks
            .iter()
            .filter(|(keyword, _)| keyword == "$SPEC_REM:")
            .flat_map(|(_, lines)| lines)
            .any(|line| line.starts_with("AP# mantaray"))
    {
        // Checked: a damaged or crafted first channel near `usize::MAX`
        // panicked a test build and wrapped in a release one.
        last = first
            .checked_add(data.1.len() - 2)
            .ok_or_else(|| FormatError::invalid("$DATA: channel range", format!("{first} {last}")))?;
    }
    // The declared range sizes an allocation, so it is not taken on faith: no
    // real MCB reaches a million channels, and an otherwise ordinary file
    // saying `0 900000000` would otherwise force gigabytes and an abort.
    if last < first {
        return Err(FormatError::invalid(
            "$DATA: channel range",
            format!("{first} {last}: the last channel {last} is before the first channel {first}"),
        ));
    }
    // Checked, as 59e3230 had it: `0 18446744073709551615` overflows the
    // `+ 1`, which a release build (no overflow checks) wrapped to 0 channels
    // and opened as an empty spectrum without a word.
    let count = (last - first).checked_add(1).ok_or_else(|| {
        FormatError::invalid(
            "$DATA: channel range",
            format!("{first} {last} is more channels than any MCB has"),
        )
    })?;
    if count > MOST_CHANNELS {
        return Err(FormatError::invalid(
            "$DATA: channel range",
            format!(
                "{first} {last} is {count} channels, more than the {MOST_CHANNELS} any MCB has"
            ),
        ));
    }

    let mut channels = Vec::with_capacity(count);
    for line in lines.take(count) {
        for token in line.split_whitespace() {
            let value: f64 = token
                .parse()
                .map_err(|_| FormatError::invalid("$DATA: channel", token))?;
            channels.push(value.max(0.0).round() as u64);
        }
    }
    channels.resize(count, 0);

    let mut spectrum = Spectrum::from_counts(channels);
    // The offset is sixteen bits everywhere a spectrum carries one; a first
    // channel beyond it is a corrupt range, not an offset to truncate quietly.
    spectrum.channel_offset = u16::try_from(first)
        .map_err(|_| FormatError::invalid("$DATA: first channel", first.to_string()))?;

    if let Some((_, lines)) = blocks.iter().find(|(keyword, _)| keyword == "$SPEC_ID:") {
        spectrum.sample_description = lines.join(" ").trim().to_string();
    }
    if let Some((_, lines)) = blocks.iter().find(|(keyword, _)| keyword == "$SPEC_REM:") {
        for line in lines {
            if let Some(rest) = line.strip_prefix("DET#") {
                spectrum.detector_id = rest.trim().parse().unwrap_or(0);
            } else if let Some(rest) = line.strip_prefix("DETDESC#") {
                spectrum.detector_description = rest.trim().to_string();
            }
        }
    }
    if let Some((_, lines)) = blocks.iter().find(|(keyword, _)| keyword == "$MEAS_TIM:")
        && let Some(line) = lines.first()
    {
        let mut parts = line.split_whitespace();
        spectrum.live_time = parts
            .next()
            .and_then(|value| value.parse().ok())
            .unwrap_or(0.0);
        spectrum.real_time = parts
            .next()
            .and_then(|value| value.parse().ok())
            .unwrap_or(spectrum.live_time);
        // `NaN inf` parses, and was carried into a native file that then
        // never opened again. A time that is not a number of seconds is
        // refused here instead.
        if !spectrum.live_time.is_finite() || !spectrum.real_time.is_finite() {
            return Err(FormatError::invalid(
                "$MEAS_TIM",
                format!(
                    "{:?} gives live time {} s, real time {} s: a time has to be a finite \
                     number of seconds",
                    line.trim(),
                    spectrum.live_time,
                    spectrum.real_time
                ),
            ));
        }
    }
    if let Some((_, lines)) = blocks.iter().find(|(keyword, _)| keyword == "$DATE_MEA:") {
        spectrum.start_time = lines.first().and_then(|line| parse_date(line));
    }
    if let Some((_, lines)) = blocks.iter().find(|(keyword, _)| keyword == "$MCA_CAL:")
        && let Some(coefficients) = lines.get(1)
    {
        let mut values = Vec::new();
        let mut units = "keV".to_string();
        for token in coefficients.split_whitespace() {
            match token.parse::<f64>() {
                Ok(value) => values.push(value),
                Err(_) => units = token.to_string(),
            }
        }
        if values.iter().any(|v| *v != 0.0) {
            spectrum.energy_calibration = Some(EnergyCalibration::new(
                [
                    values.first().copied().unwrap_or(0.0),
                    values.get(1).copied().unwrap_or(0.0),
                    values.get(2).copied().unwrap_or(0.0),
                ],
                &units,
            ));
        }
    }
    if spectrum.energy_calibration.is_none()
        && let Some((_, lines)) = blocks.iter().find(|(keyword, _)| keyword == "$ENER_FIT:")
        && let Some(line) = lines.first()
    {
        let values: Vec<f64> = line
            .split_whitespace()
            .filter_map(|token| token.parse().ok())
            .collect();
        if values.len() >= 2 && values.iter().any(|v| *v != 0.0) {
            spectrum.energy_calibration = Some(EnergyCalibration::linear(values[0], values[1]));
        }
    }
    if let Some((_, lines)) = blocks.iter().find(|(keyword, _)| keyword == "$SHAPE_CAL:")
        && let Some(coefficients) = lines.get(1)
    {
        let values: Vec<f64> = coefficients
            .split_whitespace()
            .filter_map(|token| token.parse().ok())
            .collect();
        if values.iter().any(|v| *v != 0.0) {
            spectrum.shape_calibration = Some(ShapeCalibration::new([
                values.first().copied().unwrap_or(0.0),
                values.get(1).copied().unwrap_or(0.0),
                values.get(2).copied().unwrap_or(0.0),
            ]));
        }
    }
    if let Some((_, lines)) = blocks.iter().find(|(keyword, _)| keyword == "$ROI:") {
        for line in lines.iter().skip(1) {
            let bounds: Vec<usize> = line
                .split_whitespace()
                .filter_map(|token| token.parse().ok())
                .collect();
            if bounds.len() >= 2 {
                spectrum.rois.mark(Roi::new(bounds[0], bounds[1]));
            }
        }
    }
    Ok(spectrum)
}

/// Writes a `.Spe` file as text; [`write_bytes`] gives the bytes to store.
pub fn write(spectrum: &Spectrum) -> Result<String, FormatError> {
    let mut out = String::new();
    out.push_str("$SPEC_ID:\n");
    let description = if spectrum.sample_description.trim().is_empty() {
        "No sample description was entered."
    } else {
        spectrum.sample_description.trim()
    };
    out.push_str(description);
    out.push('\n');

    out.push_str("$SPEC_REM:\n");
    out.push_str(&format!("DET# {}\n", spectrum.detector_id));
    out.push_str(&format!(
        "DETDESC# {}\n",
        spectrum.detector_description.trim()
    ));
    out.push_str(concat!("AP# mantaray ", env!("CARGO_PKG_VERSION"), "\n"));

    // Written only when there is one. A spectrum whose start was never
    // recorded used to be given `01/01/1970 00:00:00`, which reads back as a
    // measurement made at the Unix epoch - a date no reader can tell from one
    // that was meant, and one that decay correction will happily use, wrong by
    // decades and silently. The `.Chn` writer has always left the field blank
    // in this case; a missing keyword is the same statement here, and the
    // reader above treats its absence as "no date" rather than as an error.
    if let Some(start) = spectrum.start_time {
        out.push_str("$DATE_MEA:\n");
        out.push_str(&start.format("%m/%d/%Y %H:%M:%S").to_string());
        out.push('\n');
    }

    out.push_str("$MEAS_TIM:\n");
    out.push_str(&format!(
        "{} {}\n",
        format_seconds(spectrum.live_time),
        format_seconds(spectrum.real_time)
    ));

    // First and last channel, both inclusive, as MAESTRO writes and reads
    // them: 1024 channels from channel 100 are `100 1123`.
    out.push_str("$DATA:\n");
    let first = usize::from(spectrum.channel_offset);
    let last = (first + spectrum.len()).saturating_sub(1);
    out.push_str(&format!("{first} {last}\n"));
    for count in &spectrum.channels {
        out.push_str(&format!("{count}\n"));
    }

    if !spectrum.rois.is_empty() {
        out.push_str("$ROI:\n");
        out.push_str(&format!("{}\n", spectrum.rois.len()));
        for roi in spectrum.rois.iter() {
            out.push_str(&format!("{} {}\n", roi.start, roi.end));
        }
    }

    out.push_str("$PRESETS:\n");
    out.push_str("None\n0\n0\n");

    let energy = spectrum
        .energy_calibration
        .clone()
        .unwrap_or_else(|| EnergyCalibration::new([0.0, 0.0, 0.0], "keV"));
    out.push_str("$ENER_FIT:\n");
    out.push_str(&format!(
        "{:.6} {:.6}\n",
        energy.coefficients[0], energy.coefficients[1]
    ));
    out.push_str("$MCA_CAL:\n3\n");
    out.push_str(&format!(
        "{:.6E} {:.6E} {:.6E} {}\n",
        energy.coefficients[0], energy.coefficients[1], energy.coefficients[2], energy.units
    ));

    let shape = spectrum.shape_calibration.unwrap_or_default();
    out.push_str("$SHAPE_CAL:\n3\n");
    out.push_str(&format!(
        "{:.6E} {:.6E} {:.6E}\n",
        shape.coefficients[0], shape.coefficients[1], shape.coefficients[2]
    ));
    Ok(out)
}

/// Splits the file into `(keyword, lines)` blocks, tolerating CRLF and blanks.
fn split_blocks(text: &str) -> Vec<(String, Vec<String>)> {
    let mut blocks: Vec<(String, Vec<String>)> = Vec::new();
    for raw in text.lines() {
        let line = raw.trim_end_matches(['\r', '\n']);
        if line.starts_with('$') && line.ends_with(':') {
            blocks.push((line.to_string(), Vec::new()));
        } else if let Some(block) = blocks.last_mut()
            && !line.trim().is_empty()
        {
            block.1.push(line.trim().to_string());
        }
    }
    blocks
}

fn parse_field<T: std::str::FromStr>(value: Option<&str>, what: &str) -> Result<T, FormatError> {
    value
        .ok_or_else(|| FormatError::missing(what))?
        .parse()
        .map_err(|_| FormatError::invalid(what, value.unwrap_or_default()))
}

fn parse_date(line: &str) -> Option<NaiveDateTime> {
    let line = line.trim();
    // The Unix epoch is this program's own old placeholder for "no date", and
    // files carrying it are already out there - the writer above put it in
    // every spectrum whose start was never recorded. Read as the absence it
    // stood for, not as a measurement made in 1970, because a spectrum from
    // before the format existed is not a thing anyone has. A count genuinely
    // begun at that instant loses its date and nothing else.
    if line.starts_with("01/01/1970 00:00:00") {
        return None;
    }
    for format in [
        "%m/%d/%Y %H:%M:%S%.f",
        "%m/%d/%Y %H:%M:%S",
        "%m-%d-%Y %H:%M:%S",
        "%Y-%m-%d %H:%M:%S",
        "%d-%b-%Y %H:%M:%S",
    ] {
        if let Ok(parsed) = NaiveDateTime::parse_from_str(line, format) {
            return Some(parsed);
        }
    }
    // Date only.
    for format in ["%m/%d/%Y", "%Y-%m-%d"] {
        if let Ok(date) = NaiveDate::parse_from_str(line, format) {
            return date.and_hms_opt(0, 0, 0);
        }
    }
    None
}

/// Formats a time without a trailing `.0`, as MAESTRO does for whole seconds.
fn format_seconds(value: f64) -> String {
    if (value - value.round()).abs() < 1e-9 {
        format!("{}", value.round() as i64)
    } else {
        let text = format!("{value:.6}");
        text.trim_end_matches('0').trim_end_matches('.').to_string()
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn whole_seconds_are_written_without_a_decimal_point() {
        assert_eq!(format_seconds(3600.0), "3600");
        assert_eq!(format_seconds(1234.5), "1234.5");
        assert_eq!(format_seconds(0.125), "0.125");
    }

    #[test]
    fn blocks_are_split_on_keywords() {
        let blocks = split_blocks("$A:\n1\n2\n$B:\n3\n");
        assert_eq!(blocks.len(), 2);
        assert_eq!(blocks[0].0, "$A:");
        assert_eq!(blocks[0].1, vec!["1", "2"]);
        assert_eq!(blocks[1].1, vec!["3"]);
    }
}
