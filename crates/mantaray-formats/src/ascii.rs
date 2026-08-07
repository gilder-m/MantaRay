//! Plain ASCII channel dumps, compatible with the text files ORTEC's TRANSLT
//! utility reads and writes (§7.3 of the MAESTRO manual).
//!
//! The header carries the real and live time in seconds:
//!
//! ```text
//! Real Time   240
//! Live Time   120
//!      0:      10      12      13      11      9
//!      5:      14      16      12      10      8
//! ```

use mantaray_core::Spectrum;

use crate::FormatError;

/// Formatting options, mirroring TRANSLT's command-line switches.
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub struct AsciiOptions {
    /// Data columns per line (`-col n`), default 5.
    pub columns: usize,
    /// Print the channel number at the start of each line (off with `-nc`).
    pub channel_numbers: bool,
    /// Write the real/live time header (off with `-nh`).
    pub header: bool,
    /// Write the acquisition and analysis information (off with `-ni`).
    pub acquisition_info: bool,
}

impl Default for AsciiOptions {
    fn default() -> Self {
        Self {
            columns: 5,
            channel_numbers: true,
            header: true,
            acquisition_info: true,
        }
    }
}

/// Writes an ASCII dump.
pub fn write(spectrum: &Spectrum, options: &AsciiOptions) -> String {
    let columns = options.columns.max(1);
    let mut out = String::new();
    if options.header {
        out.push_str(&format!(
            "Real Time   {}\n",
            trim_number(spectrum.real_time)
        ));
        out.push_str(&format!(
            "Live Time   {}\n",
            trim_number(spectrum.live_time)
        ));
    }
    if options.acquisition_info {
        if let Some(start) = spectrum.start_time {
            out.push_str(&format!(
                "Start Time  {}\n",
                start.format("%d-%b-%Y %H%M%S")
            ));
        }
        if !spectrum.detector_description.trim().is_empty() {
            out.push_str(&format!(
                "Detector    {}\n",
                spectrum.detector_description.trim()
            ));
        }
        if !spectrum.sample_description.trim().is_empty() {
            out.push_str(&format!(
                "Sample      {}\n",
                spectrum.sample_description.trim()
            ));
        }
        if let Some(cal) = &spectrum.energy_calibration {
            out.push_str(&format!(
                "Calibration {:.6} {:.6} {:.6E} {}\n",
                cal.coefficients[0], cal.coefficients[1], cal.coefficients[2], cal.units
            ));
        }
    }
    for (index, chunk) in spectrum.channels.chunks(columns).enumerate() {
        if options.channel_numbers {
            out.push_str(&format!("{:6}:", index * columns));
        }
        for (position, value) in chunk.iter().enumerate() {
            if options.channel_numbers || position > 0 {
                out.push(' ');
            }
            out.push_str(&value.to_string());
        }
        out.push('\n');
    }
    out
}

/// Reads an ASCII dump.
///
/// Any line whose first token is not a number is treated as a header line; the
/// `Real Time` and `Live Time` keywords are recognised. Channel labels of the
/// form `123:` are ignored.
pub fn read(text: &str) -> Result<Spectrum, FormatError> {
    let mut channels: Vec<u64> = Vec::new();
    let mut live_time = 0.0f64;
    let mut real_time = 0.0f64;
    let mut calibration: Option<Vec<f64>> = None;
    let mut units = "keV".to_string();

    for raw in text.lines() {
        let line = raw.trim();
        if line.is_empty() || line.starts_with('#') {
            continue;
        }
        let lower = line.to_ascii_lowercase();
        if let Some(rest) = lower.strip_prefix("real time") {
            real_time = parse_first_number(rest).unwrap_or(real_time);
            continue;
        }
        if let Some(rest) = lower.strip_prefix("live time") {
            live_time = parse_first_number(rest).unwrap_or(live_time);
            continue;
        }
        if lower.starts_with("calibration") {
            let values: Vec<f64> = line
                .split_whitespace()
                .skip(1)
                .filter_map(|token| match token.parse::<f64>() {
                    Ok(value) => Some(value),
                    Err(_) => {
                        units = token.to_string();
                        None
                    }
                })
                .collect();
            if !values.is_empty() {
                calibration = Some(values);
            }
            continue;
        }
        // Strip a leading channel label.
        let body = match line.split_once(':') {
            Some((label, rest)) if label.trim().parse::<usize>().is_ok() => rest,
            _ => line,
        };
        let mut parsed_any = false;
        for token in body.split_whitespace() {
            match token.parse::<f64>() {
                Ok(value) => {
                    channels.push(value.max(0.0).round() as u64);
                    parsed_any = true;
                }
                Err(_) => {
                    // A non-numeric token means this was a header line after all.
                    if !parsed_any {
                        break;
                    }
                }
            }
        }
    }

    if channels.is_empty() {
        return Err(FormatError::missing("channel data"));
    }
    let mut spectrum = Spectrum::from_counts(channels);
    spectrum.live_time = live_time;
    spectrum.real_time = if real_time > 0.0 {
        real_time
    } else {
        live_time
    };
    if let Some(values) = calibration
        && values.iter().any(|v| *v != 0.0)
    {
        spectrum.energy_calibration = Some(mantaray_core::EnergyCalibration::new(
            [
                values.first().copied().unwrap_or(0.0),
                values.get(1).copied().unwrap_or(0.0),
                values.get(2).copied().unwrap_or(0.0),
            ],
            &units,
        ));
    }
    Ok(spectrum)
}

fn parse_first_number(text: &str) -> Option<f64> {
    text.split_whitespace()
        .find_map(|token| token.parse::<f64>().ok())
}

fn trim_number(value: f64) -> String {
    if (value - value.round()).abs() < 1e-9 {
        format!("{}", value.round() as i64)
    } else {
        let text = format!("{value:.6}");
        text.trim_end_matches('0').trim_end_matches('.').to_string()
    }
}
