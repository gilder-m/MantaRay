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

use crate::{FormatError, MOST_CHANNELS};

/// Reads a `.Spe` file.
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
    let last: usize = parse_field(range_parts.next(), "$DATA: last channel")?;
    // The declared range sizes an allocation, so it is not taken on faith: no
    // real MCB reaches a million channels, and an otherwise ordinary file
    // saying `0 900000000` would otherwise force gigabytes and an abort.
    let count = last
        .checked_sub(first)
        .and_then(|span| span.checked_add(1))
        .filter(|count| *count <= MOST_CHANNELS)
        .ok_or_else(|| FormatError::invalid("$DATA: channel range", format!("{first} {last}")))?;

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

/// Writes a `.Spe` file.
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

    out.push_str("$DATE_MEA:\n");
    match spectrum.start_time {
        Some(start) => out.push_str(&start.format("%m/%d/%Y %H:%M:%S").to_string()),
        None => out.push_str("01/01/1970 00:00:00"),
    }
    out.push('\n');

    out.push_str("$MEAS_TIM:\n");
    out.push_str(&format!(
        "{} {}\n",
        format_seconds(spectrum.live_time),
        format_seconds(spectrum.real_time)
    ));

    out.push_str("$DATA:\n");
    let last = spectrum.len().saturating_sub(1);
    out.push_str(&format!("{} {}\n", spectrum.channel_offset, last));
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
