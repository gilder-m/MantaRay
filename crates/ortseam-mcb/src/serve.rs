//! The bridge proper: ortseam's dialect in, ORTEC's calls out.
//!
//! ortseam speaks one small ASCII dialect to every instrument it does not hold
//! in process - `SHOW_STATUS`, `SHOW_DATA`, `START`, `SET_PRESET_LIVE 300`. A
//! real ORTEC MCB speaks something else: `SHOW_LIVE` answering `$G0007663632108`,
//! ten digits of twenty-millisecond ticks and a checksum, and it rejects
//! `SET_PRESET_LIVE` outright. Translating between the two is this module's
//! whole job, and it is the reason ortseam needs no special case for hardware:
//! the same [`RemoteMcb`] drives a socket and drives this.
//!
//! Lines arrive on standard input and replies leave on standard output, one
//! apiece, so the transport at the other end is a pipe rather than a socket.
//!
//! [`RemoteMcb`]: https://docs.rs/ortseam-device

use std::io::{BufRead, Write};

use crate::umcbi::{Hdet, Umcbi};

/// Seconds in one of the instrument's clock ticks.
///
/// Checked against hardware rather than assumed: `SHOW_TRUE` answered
/// `$G0007892556117` while MAESTRO displayed 157851.12 s for the same detector,
/// and 7892556 * 0.02 is exactly that.
const TICK: f64 = 0.02;

/// Runs the bridge until standard input closes.
pub fn run(library: &Umcbi, detector: Hdet, number: i32) -> Result<(), String> {
    // Said out loud on standard error, because it is the one thing here that
    // can be quietly wrong: a configuration copied from another machine can put
    // a calibration under the wrong instrument, and a spectrum scaled by
    // somebody else's gain looks perfectly reasonable.
    let mcb = library.mcb_number(detector);
    match calibration_for(mcb) {
        Some((a, b, c)) => eprintln!("calibration from MCBLOC32.INI [M{mcb}S01]: {a} {b} {c}"),
        None => eprintln!("no calibration stored for [M{mcb}S01]; the spectrum is in channels"),
    }
    let input = std::io::stdin();
    let mut output = std::io::stdout();
    let mut session = Session {
        library,
        detector,
        number,
        total: 0,
    };
    for line in input.lock().lines() {
        let line = line.map_err(|error| format!("reading a command: {error}"))?;
        let command = line.trim();
        if command.is_empty() {
            continue;
        }
        let reply = session.handle(command);
        writeln!(output, "{reply}").map_err(|error| format!("writing a reply: {error}"))?;
        output
            .flush()
            .map_err(|error| format!("flushing a reply: {error}"))?;
    }
    Ok(())
}

/// What the bridge remembers between commands.
struct Session<'a> {
    library: &'a Umcbi,
    detector: Hdet,
    number: i32,
    /// Total counts, from the last spectrum read.
    ///
    /// Summing eight thousand channels is the only way to get this number, and
    /// a status poll asks for it twice a second, so it is answered from the
    /// last read rather than by reading again.
    total: u64,
}

impl Session<'_> {
    fn handle(&mut self, command: &str) -> String {
        let (verb, argument) = match command.split_once(' ') {
            Some((verb, rest)) => (verb, rest.trim()),
            None => (command, ""),
        };
        let outcome = match verb {
            "SHOW_CONFIGURATION" => self.configuration(),
            "SHOW_STATUS" => self.status(),
            "SHOW_DATA" => self.data(),
            "START" => self.pass("START"),
            "STOP" => self.pass("STOP"),
            "CLEAR" => self.clear(),
            "SET_PRESET_CLEAR" => self.clear_presets(),
            "SET_PRESET_REAL" => self.preset("SET_TRUE_PRESET", argument),
            "SET_PRESET_LIVE" => self.preset("SET_LIVE_PRESET", argument),
            // Sent verbatim, so that MAESTRO's SEND_MESSAGE window and anything
            // else that knows the real dialect can reach the instrument.
            other if other.starts_with('$') || other.contains('_') => self.pass(command),
            other => Err(format!("{other} is not a command this bridge knows")),
        };
        match outcome {
            Ok(reply) => reply,
            Err(error) => format!("ERR {error}"),
        }
    }

    /// Sends a command through untouched and reports OK.
    fn pass(&mut self, command: &str) -> Result<String, String> {
        self.library.command(self.detector, command)?;
        Ok("OK".into())
    }

    fn clear(&mut self) -> Result<String, String> {
        self.library.command(self.detector, "CLEAR")?;
        self.total = 0;
        Ok("OK".into())
    }

    fn clear_presets(&mut self) -> Result<String, String> {
        self.library.command(self.detector, "SET_TRUE_PRESET 0")?;
        self.library.command(self.detector, "SET_LIVE_PRESET 0")?;
        Ok("OK".into())
    }

    /// A preset in seconds, sent on in the instrument's ticks.
    fn preset(&mut self, verb: &str, argument: &str) -> Result<String, String> {
        let seconds: f64 = argument
            .parse()
            .map_err(|_| format!("{argument:?} is not a number of seconds"))?;
        let ticks = (seconds / TICK).round().max(0.0) as u64;
        self.library
            .command(self.detector, &format!("{verb} {ticks}"))?;
        Ok("OK".into())
    }

    fn configuration(&mut self) -> Result<String, String> {
        let model = self.library.model(self.detector);
        let channels = self.library.length(self.detector);
        let firmware = self
            .library
            .command(self.detector, "SHOW_VERSION")
            .unwrap_or_default();
        // A 926 holds no calibration of its own; MAESTRO keeps it host-side, so
        // the bridge fetches it from there and hands it over rather than
        // leaving every spectrum off this instrument uncalibrated.
        let calibration = calibration_for(self.library.mcb_number(self.detector))
            .map(|(a, b, c)| format!(" CAL={a},{b},{c}"))
            .unwrap_or_default();
        Ok(format!(
            "MODEL={} SERIAL={} FIRMWARE={} CHANNELS={channels}{calibration}",
            if model.is_empty() { "MCB" } else { &model },
            self.number,
            record_text(&firmware),
        ))
    }

    fn status(&mut self) -> Result<String, String> {
        let real = self.clock("SHOW_TRUE")?;
        let live = self.clock("SHOW_LIVE")?;
        // The instrument reports no dead time of its own; it is the difference
        // between the two clocks, which is what dead time means.
        let dead = if real > 0.0 {
            (real - live) / real * 100.0
        } else {
            0.0
        };
        let active = i32::from(self.library.is_counting(self.detector));
        Ok(format!(
            "RT={real:.2} LT={live:.2} DT={dead:.2}% ICR=0 ACTIVE={active} TOTAL={}",
            self.total
        ))
    }

    /// One of the instrument's clocks, in seconds.
    fn clock(&mut self, command: &str) -> Result<f64, String> {
        let reply = self.library.command(self.detector, command)?;
        Ok(record_number(&reply)? * TICK)
    }

    fn data(&mut self) -> Result<String, String> {
        let channels = self.library.length(self.detector);
        let counts = self.library.read(self.detector, 0, channels)?;
        self.total = counts.iter().sum();
        let mut reply = String::with_capacity(counts.len() * 4 + 16);
        reply.push_str("DATA ");
        reply.push_str(&counts.len().to_string());
        for count in &counts {
            reply.push(' ');
            reply.push_str(&count.to_string());
        }
        Ok(reply)
    }
}

/// The energy calibration MAESTRO stores for an instrument.
///
/// `MCBLOC32.INI` keeps one section per instrument, `[M<mcb>S01]`, and the
/// `EnergyCalibration` line in it holds the three polynomial coefficients. The
/// file's location is the local layer's to decide, so it is asked.
pub fn calibration_for(mcb: i32) -> Option<(f64, f64, f64)> {
    let path = crate::umcbi::local_paths()
        .into_iter()
        .find(|(label, _)| *label == "MCBLOC32.INI")
        .map(|(_, path)| path)?;
    let text = std::fs::read_to_string(path).ok()?;
    let wanted = format!("[M{mcb}S01]");
    let mut inside = false;
    for line in text.lines() {
        let line = line.trim();
        if line.starts_with('[') {
            inside = line.eq_ignore_ascii_case(&wanted);
            continue;
        }
        if !inside {
            continue;
        }
        if let Some(values) = line.strip_prefix("EnergyCalibration=") {
            let numbers: Vec<f64> = values
                .split_whitespace()
                .filter_map(|value| value.parse().ok())
                .collect();
            if let [a, b, c] = numbers[..] {
                // An all-zero line means "never calibrated", not "energy is
                // always zero", so it is left alone.
                if b != 0.0 || c != 0.0 {
                    return Some((a, b, c));
                }
            }
            return None;
        }
    }
    None
}

/// The number carried by a `$C`/`$G` record, without its checksum.
///
/// A record is `$`, a letter, fixed-width decimal fields, then three digits of
/// checksum. Only the leading field is wanted here, and every record this is
/// used on carries exactly one.
fn record_number(reply: &str) -> Result<f64, String> {
    let digits: String = reply.chars().filter(char::is_ascii_digit).collect();
    if digits.len() <= 3 {
        return Err(format!("{reply:?} carries no number"));
    }
    digits[..digits.len() - 3]
        .parse()
        .map_err(|_| format!("{reply:?} is not a number"))
}

/// The text of an `$F` record, which carries no checksum to strip.
fn record_text(reply: &str) -> String {
    let text = reply.trim().trim_start_matches('$');
    let text = text.strip_prefix('F').unwrap_or(text);
    if text.is_empty() {
        "unknown".into()
    } else {
        text.replace(' ', "_")
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn a_clock_record_loses_its_checksum() {
        // The reply the 926 gave for SHOW_TRUE while MAESTRO showed 157851.12 s.
        assert_eq!(record_number("$G0007892556117").unwrap(), 7_892_556.0);
        assert!((7_892_556.0 * TICK - 157_851.12).abs() < 1e-6);
    }

    #[test]
    fn a_conversion_gain_record_reads_as_its_channel_count() {
        assert_eq!(record_number("$C08192107").unwrap(), 8192.0);
    }

    #[test]
    fn a_record_with_nothing_in_it_is_refused() {
        assert!(record_number("$F0926-001").is_err() || record_number("$C").is_err());
        assert!(record_number("").is_err());
    }

    #[test]
    fn a_version_record_keeps_its_text() {
        assert_eq!(record_text("$F0926-001"), "0926-001");
        assert_eq!(record_text(""), "unknown");
    }
}
