//! ANSI N42.42 spectra.
//!
//! The standard XML format for radiation measurement data, and what ORTEC's own
//! instruments write when asked for something portable - the Detective, the
//! Micro-Detective and the portal monitors all produce it.
//!
//! Only reading is implemented, and only the part of the standard that carries a
//! spectrum: a `<Spectrum>` element with its live and real times, an energy
//! calibration, and channel counts. The rest of an N42 file - dose rates, GPS
//! tracks, analysis results, per-detector breakdowns - is stepped over. A file
//! holding several spectra gives up the first pulse-height one, which is what a
//! spectroscopy program is being asked for.
//!
//! # The parts that matter
//!
//! ```xml
//! <Spectrum Type="PHA">
//!   <RealTime>PT14S</RealTime>
//!   <LiveTime>PT13.78S</LiveTime>
//!   <Calibration Type="Energy" EnergyUnits="keV">
//!     <Equation Model="Polynomial">
//!       <Coefficients>0.022902 0.36579 3.4231E-08</Coefficients>
//!     </Equation>
//!   </Calibration>
//!   <ChannelData Compression="CountedZeroes"> 0 38 1 1 5 ... </ChannelData>
//! </Spectrum>
//! ```
//!
//! Durations are ISO 8601, which for a measurement always means `PT<number>S`.
//! `CountedZeroes` is the standard's own run-length scheme: a zero is followed
//! by the number of zeros it stands for.
//!
//! The reader is a scanner over elements rather than a full XML parser. That is
//! a deliberate limit, not an oversight: these files are machine-written, deeply
//! nested, and every value wanted here is the text of a named element. What it
//! will not do is pretend to have understood a file it has not - a spectrum
//! without channel data is an error rather than an empty spectrum.

use mantaray_core::{EnergyCalibration, ShapeCalibration, Spectrum};

use crate::FormatError;

/// Reads the first pulse-height spectrum in an N42 document.
pub fn from_str(text: &str) -> Result<Spectrum, FormatError> {
    let body = pha_spectrum(text)
        .ok_or_else(|| FormatError::invalid("N42", "the file holds no spectrum"))?;

    let data = element(body, "ChannelData")
        .ok_or_else(|| FormatError::invalid("N42", "the spectrum has no channel data"))?;
    // The 2011 revision renamed the attribute and some writers wrap the counts
    // in a <Data> element; both say the same thing.
    let compressed = attribute(body, "ChannelData", "Compression")
        .or_else(|| attribute(body, "ChannelData", "compressionCode"))
        .map(|value| value.eq_ignore_ascii_case("CountedZeroes"))
        .unwrap_or(false);
    let inner = inner_data(data);
    let channels = channel_counts(inner.as_deref().unwrap_or(data), compressed)?;
    if channels.is_empty() {
        return Err(FormatError::invalid("N42", "the spectrum has no channels"));
    }

    let mut spectrum = Spectrum::from_counts(channels);
    // The 2011 revision moved the times and the calibration out of <Spectrum>
    // and into the measurement around it, so each is looked for in the spectrum
    // first and in the whole document after.
    let either = |name: &str, other: &str| {
        element(body, name)
            .or_else(|| element(body, other))
            .or_else(|| element(text, name))
            .or_else(|| element(text, other))
    };
    spectrum.real_time = either("RealTime", "RealTimeDuration")
        .and_then(duration)
        .unwrap_or(0.0);
    spectrum.live_time = either("LiveTime", "LiveTimeDuration")
        .and_then(duration)
        .unwrap_or(spectrum.real_time);
    spectrum.start_time = either("StartTime", "StartDateTime").and_then(timestamp);
    spectrum.energy_calibration = calibration(body, "Energy")
        .or_else(|| calibration(text, "Energy"))
        .or_else(|| modern_calibration(text))
        .map(|(coefficients, units)| {
            EnergyCalibration::new(coefficients, units.as_deref().unwrap_or("keV"))
        });
    spectrum.shape_calibration = calibration(body, "FWHM")
        .or_else(|| calibration(text, "FWHM"))
        .map(|(coefficients, _)| ShapeCalibration::new(coefficients));
    Ok(spectrum)
}

/// The body of the first `<Spectrum>` that carries pulse-height data.
///
/// An instrument that also records a count-rate trace writes several spectra,
/// and only one of them is what a spectroscopy program means by the word.
fn pha_spectrum(text: &str) -> Option<&str> {
    let mut at = 0;
    let mut first = None;
    while let Some(body) = next_element(text, "Spectrum", &mut at) {
        if first.is_none() && element(body, "ChannelData").is_some() {
            first = Some(body);
        }
        let is_pha = tag_attribute(text, "Spectrum", "Type", body)
            .map(|kind| kind.eq_ignore_ascii_case("PHA"))
            .unwrap_or(false);
        if is_pha && element(body, "ChannelData").is_some() {
            return Some(body);
        }
    }
    // No spectrum said what it was; the first one holding counts is the answer.
    first
}

/// Walks to the next element of a name, returning its body.
fn next_element<'a>(text: &'a str, name: &str, at: &mut usize) -> Option<&'a str> {
    let open = format!("<{name}");
    let close = format!("</{name}>");
    // A loop, deliberately: every `<SpectrumX...` prefix hit is another pass,
    // and recursing here let a sub-megabyte crafted file overflow the stack.
    loop {
        let start = text[*at..].find(&open)? + *at;
        // The tag must end at a delimiter, or <Spectrum> matches <SpectrumFoo>.
        let after_name = start + open.len();
        let delimiter = text.as_bytes().get(after_name)?;
        if !matches!(delimiter, b'>' | b' ' | b'\t' | b'\r' | b'\n' | b'/') {
            *at = after_name;
            continue;
        }
        let body_start = text[start..].find('>')? + start + 1;
        let body_end = text[body_start..].find(&close)? + body_start;
        *at = body_end + close.len();
        return Some(&text[body_start..body_end]);
    }
}

/// The text of the first element of a name, trimmed.
fn element<'a>(text: &'a str, name: &str) -> Option<&'a str> {
    let mut at = 0;
    next_element(text, name, &mut at).map(str::trim)
}

/// An attribute of the first element of a name.
fn attribute<'a>(text: &'a str, name: &str, attribute: &str) -> Option<&'a str> {
    let open = text.find(&format!("<{name}"))?;
    let end = text[open..].find('>')? + open;
    attribute_in(&text[open..end], attribute)
}

/// The same, for an element whose body has already been found.
fn tag_attribute<'a>(text: &'a str, name: &str, attribute: &str, body: &'a str) -> Option<&'a str> {
    // The opening tag is whatever sits before this body in the document.
    let offset = body.as_ptr() as usize - text.as_ptr() as usize;
    let open = text[..offset].rfind(&format!("<{name}"))?;
    let end = text[open..offset].find('>')? + open;
    attribute_in(&text[open..end], attribute)
}

fn attribute_in<'a>(tag: &'a str, attribute: &str) -> Option<&'a str> {
    let at = tag.find(&format!("{attribute}="))? + attribute.len() + 1;
    let rest = tag[at..].trim_start();
    let quote = rest.chars().next()?;
    if quote != '"' && quote != '\'' {
        return None;
    }
    let value = &rest[1..];
    let end = value.find(quote)?;
    Some(&value[..end])
}

/// The counts inside a `<Data>` wrapper, when a writer has used one.
///
/// Several `<Data>` blocks in one `<ChannelData>` are one spectrum split up, so
/// they are joined rather than the first being taken for the whole.
fn inner_data(text: &str) -> Option<String> {
    let mut at = 0;
    let mut joined = String::new();
    while let Some(block) = next_element(text, "Data", &mut at) {
        if !joined.is_empty() {
            joined.push(' ');
        }
        joined.push_str(block.trim());
    }
    (!joined.is_empty()).then_some(joined)
}

/// The 2011 revision's calibration: `<EnergyCalibration><CoefficientValues>`.
fn modern_calibration(text: &str) -> Option<([f64; 3], Option<String>)> {
    let block = element(text, "EnergyCalibration")?;
    let values = element(block, "CoefficientValues")?;
    let mut coefficients = [0.0; 3];
    for (slot, value) in coefficients.iter_mut().zip(values.split_whitespace()) {
        *slot = value.parse().ok()?;
    }
    Some((coefficients, None))
}

/// A `<Calibration Type="...">` block's polynomial coefficients and units.
fn calibration(body: &str, kind: &str) -> Option<([f64; 3], Option<String>)> {
    let mut at = 0;
    while let Some(block) = next_element(body, "Calibration", &mut at) {
        let is_wanted = tag_attribute(body, "Calibration", "Type", block)
            .map(|value| value.eq_ignore_ascii_case(kind))
            .unwrap_or(false);
        if !is_wanted {
            continue;
        }
        let text = element(block, "Coefficients")?;
        let mut coefficients = [0.0; 3];
        for (slot, value) in coefficients.iter_mut().zip(text.split_whitespace()) {
            *slot = value.parse().ok()?;
        }
        let units = tag_attribute(body, "Calibration", "EnergyUnits", block)
            .or_else(|| tag_attribute(body, "Calibration", "FWHMUnits", block))
            .map(str::to_string);
        return Some((coefficients, units));
    }
    None
}

/// An ISO 8601 duration, which for a measurement is always seconds.
///
/// `PT14S`, `PT13.78S`, and the occasional bare number that instruments write
/// when they should not.
fn duration(text: &str) -> Option<f64> {
    let trimmed = text.trim();
    let digits = trimmed
        .trim_start_matches(['P', 'p'])
        .trim_start_matches(['T', 't'])
        .trim_end_matches(['S', 's']);
    digits.parse().ok()
}

/// An ISO 8601 timestamp, keeping the local wall clock rather than shifting it.
fn timestamp(text: &str) -> Option<chrono::NaiveDateTime> {
    let trimmed = text.trim();
    chrono::DateTime::parse_from_rfc3339(trimmed)
        .map(|parsed| parsed.naive_local())
        .ok()
        .or_else(|| chrono::NaiveDateTime::parse_from_str(trimmed, "%Y-%m-%dT%H:%M:%S").ok())
}

/// Channel counts, expanding the standard's run-length scheme when it is used.
///
/// Under `CountedZeroes` a zero is followed by how many zeros it stands for, so
/// `0 38` is thirty-eight empty channels. A trailing lone zero is a file that
/// was cut off, and says so rather than silently dropping a channel.
fn channel_counts(text: &str, compressed: bool) -> Result<Vec<u64>, FormatError> {
    let mut counts = Vec::new();
    let mut values = text.split_whitespace();
    while let Some(value) = values.next() {
        let count: f64 = value.parse().map_err(|_| {
            FormatError::invalid("N42", format!("{value:?} is not a channel count"))
        })?;
        let count = count.max(0.0).round() as u64;
        if compressed && count == 0 {
            let run = values.next().ok_or_else(|| {
                FormatError::invalid("N42", "the channel data ends in an unfinished run of zeros")
            })?;
            let run: usize = run
                .parse::<f64>()
                .map(|value| value.max(0.0) as usize)
                .map_err(|_| {
                    FormatError::invalid("N42", format!("{run:?} is not a number of zeros"))
                })?;
            // A run long enough to exhaust memory is a corrupt file, not a
            // sixty-million-channel spectrum - and the runs accumulate, so
            // the total is what the cap must hold: sixty-four repeats of a
            // capped run in a kilobyte of XML used to add up to gigabytes.
            if run > crate::MOST_CHANNELS || counts.len() + run > crate::MOST_CHANNELS {
                return Err(FormatError::invalid(
                    "N42",
                    "the channel data claims an impossible run of zeros",
                ));
            }
            counts.resize(counts.len() + run, 0);
        } else {
            if counts.len() >= crate::MOST_CHANNELS {
                return Err(FormatError::invalid("N42", "the channel data does not end"));
            }
            counts.push(count);
        }
    }
    Ok(counts)
}

#[cfg(test)]
mod tests {
    use super::*;

    const SAMPLE: &str = r#"<?xml version="1.0"?>
<N42InstrumentData>
<Measurement>
<Spectrum Type="PHA">
<StartTime>2014-11-11T21:25:40+00:00</StartTime>
<RealTime>PT14S</RealTime>
<LiveTime>PT13.78S</LiveTime>
<Calibration Type="Energy" EnergyUnits="keV">
<Equation Model="Polynomial">
<Coefficients>0.022902 0.36579 3.4231E-08</Coefficients>
</Equation>
</Calibration>
<Calibration Type="FWHM" FWHMUnits="Channels">
<Equation Model="Polynomial">
<Coefficients>3.3125 0.0006469 -2.192E-08</Coefficients>
</Equation>
</Calibration>
<ChannelData Compression="CountedZeroes">0 3 7 4 0 2 9</ChannelData>
</Spectrum>
</Measurement>
</N42InstrumentData>"#;

    #[test]
    fn a_counted_zeroes_spectrum_expands_to_its_channels() {
        let spectrum = from_str(SAMPLE).expect("it should read");
        assert_eq!(spectrum.channels, vec![0, 0, 0, 7, 4, 0, 0, 9]);
        assert_eq!(spectrum.len(), 8);
    }

    #[test]
    fn the_times_come_through_as_seconds() {
        let spectrum = from_str(SAMPLE).expect("it should read");
        assert!((spectrum.real_time - 14.0).abs() < 1e-9);
        assert!((spectrum.live_time - 13.78).abs() < 1e-9);
    }

    #[test]
    fn the_energy_calibration_is_the_one_marked_energy() {
        // Two calibrations are present and they must not be confused: picking
        // the FWHM one would put the first channel at 3.3 keV instead of 0.02.
        let spectrum = from_str(SAMPLE).expect("it should read");
        let calibration = spectrum.energy_calibration.expect("an energy calibration");
        assert_eq!(calibration.units, "keV");
        assert!((calibration.coefficients[0] - 0.022902).abs() < 1e-9);
        assert!((calibration.coefficients[1] - 0.36579).abs() < 1e-9);
        assert!((calibration.coefficients[2] - 3.4231e-08).abs() < 1e-15);
        let shape = spectrum.shape_calibration.expect("a shape calibration");
        assert!((shape.coefficients[0] - 3.3125).abs() < 1e-9);
    }

    #[test]
    fn uncompressed_channel_data_is_taken_as_it_stands() {
        let text = SAMPLE.replace(" Compression=\"CountedZeroes\"", "");
        let spectrum = from_str(&text).expect("it should read");
        assert_eq!(spectrum.channels, vec![0, 3, 7, 4, 0, 2, 9]);
    }

    #[test]
    fn a_pulse_height_spectrum_is_preferred_over_any_other() {
        let text = SAMPLE.replace(
            "<Spectrum Type=\"PHA\">",
            "<Spectrum Type=\"MCS\"><ChannelData>1 2 3</ChannelData></Spectrum>\n<Spectrum Type=\"PHA\">",
        );
        let spectrum = from_str(&text).expect("it should read");
        assert_eq!(
            spectrum.channels,
            vec![0, 0, 0, 7, 4, 0, 0, 9],
            "the count-rate trace should not be mistaken for the spectrum"
        );
    }

    #[test]
    fn a_file_with_no_spectrum_says_so() {
        assert!(from_str("<N42InstrumentData></N42InstrumentData>").is_err());
    }

    #[test]
    fn a_run_of_zeros_that_never_ends_is_refused() {
        let text = SAMPLE.replace("0 3 7 4 0 2 9", "7 4 0");
        assert!(
            from_str(&text).is_err(),
            "channel data ending mid-run is a truncated file"
        );
    }
}
