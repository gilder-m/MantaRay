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

#[cfg(windows)]
use crate::umcbi::{Hdet, Umcbi};

/// Seconds in one of the instrument's clock ticks.
///
/// Checked against hardware rather than assumed: `SHOW_TRUE` answered
/// `$G0007892556117` while MAESTRO displayed 157851.12 s for the same detector,
/// and 7892556 * 0.02 is exactly that.
const TICK: f64 = 0.02;

/// An instrument the bridge can drive, however it is reached.
///
/// There are two ways in, and the dialect above them is identical: ORTEC's
/// library, and this project's own USB. The second needs none of ORTEC's
/// user-mode software, so it is what a machine with only the driver installed
/// can use - which is the whole reason this is a trait rather than a struct.
pub trait Instrument {
    /// Sends a command in the instrument's own dialect and returns its answer.
    fn command(&self, text: &str) -> Result<String, String>;
    /// How many channels the instrument is set to use.
    fn channels(&self) -> usize;
    /// Reads channel counts.
    fn read(&self, start: usize, count: usize) -> Result<Vec<u64>, String>;
    /// Whether the ADC is acquiring.
    fn is_counting(&self) -> bool;
    /// Model name, empty when the instrument does not offer one.
    fn model(&self) -> String;
    /// What to report as the serial: a detector number or an adapter serial.
    fn identity(&self) -> String;
    /// The energy calibration, when the host has one stored.
    fn calibration(&self) -> Option<(f64, f64, f64)>;
    /// One line for the log, saying how the instrument is being reached.
    fn route(&self) -> String;
}

/// Runs the bridge until standard input closes.
pub fn run(instrument: &dyn Instrument) -> Result<(), String> {
    // Said out loud on standard error, because these are the two things here
    // that can be quietly wrong: a configuration copied from another machine
    // can put a calibration under the wrong instrument, and a spectrum scaled
    // by somebody else's gain looks perfectly reasonable.
    eprintln!("{}", instrument.route());
    match instrument.calibration() {
        Some((a, b, c)) => eprintln!("energy calibration: {a} {b} {c}"),
        None => eprintln!("no stored calibration; the spectrum is in channels"),
    }
    let input = std::io::stdin();
    let mut output = std::io::stdout();
    let mut session = Session {
        instrument,
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
    instrument: &'a dyn Instrument,
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
            // Counts, not seconds, so no tick conversion. The verbs were
            // established on the bench: SET_PEAK_PRESET 100 reads back as
            // $G0000000100076 from SHOW_PEAK_PRESET.
            "SET_PRESET_COUNT" => self.count_preset("SET_PEAK_PRESET", argument),
            "SET_PRESET_INTEG" => self.count_preset("SET_INTEGRAL_PRESET", argument),
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
        self.instrument.command(command)?;
        Ok("OK".into())
    }

    fn clear(&mut self) -> Result<String, String> {
        self.instrument.command("CLEAR")?;
        self.total = 0;
        Ok("OK".into())
    }

    fn clear_presets(&mut self) -> Result<String, String> {
        self.instrument.command("SET_TRUE_PRESET 0")?;
        self.instrument.command("SET_LIVE_PRESET 0")?;
        self.instrument.command("SET_PEAK_PRESET 0")?;
        self.instrument.command("SET_INTEGRAL_PRESET 0")?;
        Ok("OK".into())
    }

    /// A count preset, passed on as the whole number it is.
    fn count_preset(&mut self, verb: &str, argument: &str) -> Result<String, String> {
        let counts: u64 = argument
            .parse()
            .map_err(|_| format!("{argument:?} is not a number of counts"))?;
        self.instrument.command(&format!("{verb} {counts}"))?;
        Ok("OK".into())
    }

    /// A preset in seconds, sent on in the instrument's ticks.
    fn preset(&mut self, verb: &str, argument: &str) -> Result<String, String> {
        let seconds: f64 = argument
            .parse()
            .map_err(|_| format!("{argument:?} is not a number of seconds"))?;
        let ticks = (seconds / TICK).round().max(0.0) as u64;
        self.instrument.command(&format!("{verb} {ticks}"))?;
        Ok("OK".into())
    }

    fn configuration(&mut self) -> Result<String, String> {
        let model = self.instrument.model();
        let channels = self.instrument.channels();
        let firmware = self.instrument.command("SHOW_VERSION").unwrap_or_default();
        // A 926 holds no calibration of its own; MAESTRO keeps it host-side, so
        // the bridge fetches it from there and hands it over rather than
        // leaving every spectrum off this instrument uncalibrated.
        let calibration = self
            .instrument
            .calibration()
            .map(|(a, b, c)| format!(" CAL={a},{b},{c}"))
            .unwrap_or_default();
        Ok(format!(
            "MODEL={} SERIAL={} FIRMWARE={} CHANNELS={channels}{calibration}",
            if model.is_empty() { "MCB" } else { &model },
            self.instrument.identity(),
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
        let active = i32::from(self.instrument.is_counting());
        Ok(format!(
            "RT={real:.2} LT={live:.2} DT={dead:.2}% ICR=0 ACTIVE={active} TOTAL={}",
            self.total
        ))
    }

    /// One of the instrument's clocks, in seconds.
    fn clock(&mut self, command: &str) -> Result<f64, String> {
        let reply = self.instrument.command(command)?;
        Ok(record_number(&reply)? * TICK)
    }

    fn data(&mut self) -> Result<String, String> {
        let channels = self.instrument.channels();
        let counts = self.instrument.read(0, channels)?;
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

/// An instrument reached through ORTEC's library.
#[cfg(windows)]
pub struct ViaUmcbi<'a> {
    pub library: &'a Umcbi,
    pub detector: Hdet,
    pub number: i32,
}

#[cfg(windows)]
impl Instrument for ViaUmcbi<'_> {
    fn command(&self, text: &str) -> Result<String, String> {
        self.library.command(self.detector, text)
    }
    fn channels(&self) -> usize {
        usize::from(self.library.length(self.detector))
    }
    fn read(&self, start: usize, count: usize) -> Result<Vec<u64>, String> {
        // ORTEC's call counts channels in sixteen bits, which is every channel
        // any of these instruments has.
        let start = u16::try_from(start).map_err(|_| format!("channel {start} is out of range"))?;
        let count = u16::try_from(count).map_err(|_| format!("{count} channels is too many"))?;
        self.library.read(self.detector, start, count)
    }
    fn is_counting(&self) -> bool {
        self.library.is_counting(self.detector)
    }
    fn model(&self) -> String {
        self.library.model(self.detector)
    }
    fn identity(&self) -> String {
        self.number.to_string()
    }
    fn calibration(&self) -> Option<(f64, f64, f64)> {
        calibration_for(self.library.mcb_number(self.detector))
    }
    fn route(&self) -> String {
        let mcb = self.library.mcb_number(self.detector);
        format!(
            "detector {} (MCB {mcb}) through ORTEC's library",
            self.number
        )
    }
}

/// An instrument reached over USB, with none of ORTEC's user-mode software.
///
/// Only the kernel driver is involved. Everything the bridge needs is asked of
/// the instrument in its own dialect, so a machine that has never had MAESTRO
/// on it drives the hardware exactly as one that has.
#[cfg(windows)]
pub struct ViaUsb {
    device: crate::usb::Device,
    /// Asked once at start-up: the conversion gain is how many channels the
    /// instrument is actually set to use, and it is not always all of them.
    channels: usize,
    model: String,
    serial: String,
}

#[cfg(windows)]
impl ViaUsb {
    /// Opens an instrument and asks it what it is.
    pub fn open(device: crate::usb::Device) -> Result<Self, String> {
        let serial = serial_of(device.path());
        let memory = crate::dpm::Dpm::new(&device);
        let gain = memory.command("SHOW_GAIN_CONVERSION")?;
        let channels = record_number(&gain)
            .map_err(|_| format!("SHOW_GAIN_CONVERSION answered {gain:?}"))?
            as usize;
        if channels == 0 || channels > 1 << 20 {
            return Err(format!("{channels} channels is not a conversion gain"));
        }
        let model = record_text(&memory.command("SHOW_VERSION").unwrap_or_default());
        Ok(Self {
            device,
            channels,
            model: if model == "unknown" {
                String::new()
            } else {
                model
            },
            serial,
        })
    }
}

#[cfg(windows)]
impl Instrument for ViaUsb {
    fn command(&self, text: &str) -> Result<String, String> {
        crate::dpm::Dpm::new(&self.device).command(text)
    }
    fn channels(&self) -> usize {
        self.channels
    }
    fn read(&self, start: usize, count: usize) -> Result<Vec<u64>, String> {
        // The instrument hands over the whole histogram in one frame, so a
        // partial read is a slice of it rather than a different request.
        let (counts, _regions) = crate::dpm::Dpm::new(&self.device).read_spectrum(self.channels)?;
        Ok(counts
            .into_iter()
            .skip(start)
            .take(count)
            .map(u64::from)
            .collect())
    }
    fn is_counting(&self) -> bool {
        // `$C00001088` when the ADC is acquiring, `$C00000087` when it is not.
        self.command("SHOW_ACTIVE")
            .ok()
            .and_then(|reply| record_number(&reply).ok())
            .is_some_and(|active| active != 0.0)
    }
    fn model(&self) -> String {
        self.model.clone()
    }
    fn identity(&self) -> String {
        self.serial.clone()
    }
    fn calibration(&self) -> Option<(f64, f64, f64)> {
        // A 926 holds no calibration of its own. If this machine happens to
        // have MAESTRO's file, use it; a machine without one is not worse off
        // than it was, it simply reports channels.
        None
    }
    fn route(&self) -> String {
        format!(
            "adapter {} over USB, with none of ORTEC's user-mode software",
            self.serial
        )
    }
}

/// The adapter serial out of a device path.
///
/// A path reads `\\?\USB#VID_0A2D&PID_0016#08134079#{guid}`, and the serial is
/// the field before the interface GUID.
#[cfg(windows)]
fn serial_of(path: &str) -> String {
    path.split('#')
        .nth(2)
        .filter(|field| !field.is_empty())
        .unwrap_or(path)
        .to_string()
}

/// An instrument reached over libusb, with no vendor driver at all.
///
/// The non-Windows twin of `ViaUsb` - not a link, because that type exists
/// only on Windows and rustdoc runs here too. The same questions are asked of
/// the instrument in its own dialect; only the road the frames travel differs.
/// Nothing host-side is consulted, because away from Windows there is no
/// MAESTRO installation to consult.
#[cfg(not(windows))]
pub struct ViaDirect {
    device: crate::direct::Device,
    /// Asked once at start-up: the conversion gain is how many channels the
    /// instrument is actually set to use, and it is not always all of them.
    channels: usize,
    model: String,
}

#[cfg(not(windows))]
impl ViaDirect {
    /// Opens an instrument and asks it what it is.
    pub fn open(device: crate::direct::Device) -> Result<Self, String> {
        let memory = crate::dpm::Dpm::new(&device);
        let gain = memory.command("SHOW_GAIN_CONVERSION")?;
        let channels = record_number(&gain)
            .map_err(|_| format!("SHOW_GAIN_CONVERSION answered {gain:?}"))?
            as usize;
        if channels == 0 || channels > 1 << 20 {
            return Err(format!("{channels} channels is not a conversion gain"));
        }
        let model = record_text(&memory.command("SHOW_VERSION").unwrap_or_default());
        Ok(Self {
            device,
            channels,
            model: if model == "unknown" {
                String::new()
            } else {
                model
            },
        })
    }
}

#[cfg(not(windows))]
impl Instrument for ViaDirect {
    fn command(&self, text: &str) -> Result<String, String> {
        crate::dpm::Dpm::new(&self.device).command(text)
    }
    fn channels(&self) -> usize {
        self.channels
    }
    fn read(&self, start: usize, count: usize) -> Result<Vec<u64>, String> {
        // The instrument hands over the whole histogram in one frame, so a
        // partial read is a slice of it rather than a different request.
        let (counts, _regions) = crate::dpm::Dpm::new(&self.device).read_spectrum(self.channels)?;
        Ok(counts
            .into_iter()
            .skip(start)
            .take(count)
            .map(u64::from)
            .collect())
    }
    fn is_counting(&self) -> bool {
        // `$C00001088` when the ADC is acquiring, `$C00000087` when it is not.
        self.command("SHOW_ACTIVE")
            .ok()
            .and_then(|reply| record_number(&reply).ok())
            .is_some_and(|active| active != 0.0)
    }
    fn model(&self) -> String {
        self.model.clone()
    }
    fn identity(&self) -> String {
        self.device.serial().to_string()
    }
    fn calibration(&self) -> Option<(f64, f64, f64)> {
        // There is no MAESTRO on this machine to have stored one.
        None
    }
    fn route(&self) -> String {
        format!(
            "adapter {} over libusb, with no vendor driver",
            self.device.serial()
        )
    }
}

/// The energy calibration MAESTRO stores for an instrument.
///
/// `MCBLOC32.INI` keeps one section per instrument, `[M<mcb>S01]`, and the
/// `EnergyCalibration` line in it holds the three polynomial coefficients. The
/// file's location is the local layer's to decide, so it is asked.
#[cfg(windows)]
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

    /// A bench double answering as the 926 on the bench answered, so the
    /// translation layer is pinned on every platform, not only where the
    /// hardware is.
    struct Bench {
        sent: std::cell::RefCell<Vec<String>>,
    }

    impl Bench {
        fn new() -> Self {
            Self {
                sent: std::cell::RefCell::new(Vec::new()),
            }
        }
    }

    impl Instrument for Bench {
        fn command(&self, text: &str) -> Result<String, String> {
            self.sent.borrow_mut().push(text.to_string());
            Ok(match text {
                // The replies the real instrument gave for these.
                "SHOW_TRUE" => "$G0007892556117".into(),
                "SHOW_LIVE" => "$G0007663632108".into(),
                "SHOW_VERSION" => "$F0926-001".into(),
                _ => "%000000069".into(),
            })
        }
        fn channels(&self) -> usize {
            4
        }
        fn read(&self, start: usize, count: usize) -> Result<Vec<u64>, String> {
            Ok((start..start + count)
                .map(|channel| channel as u64 * 10)
                .collect())
        }
        fn is_counting(&self) -> bool {
            true
        }
        fn model(&self) -> String {
            "0926-001".into()
        }
        fn identity(&self) -> String {
            "08134079".into()
        }
        fn calibration(&self) -> Option<(f64, f64, f64)> {
            None
        }
        fn route(&self) -> String {
            "a bench double".into()
        }
    }

    #[test]
    fn a_status_reads_the_clocks_in_seconds_and_dead_time_as_their_difference() {
        let bench = Bench::new();
        let mut session = Session {
            instrument: &bench,
            total: 0,
        };
        let reply = session.handle("SHOW_STATUS");
        assert_eq!(
            reply,
            "RT=157851.12 LT=153272.64 DT=2.90% ICR=0 ACTIVE=1 TOTAL=0"
        );
    }

    #[test]
    fn a_preset_in_seconds_reaches_the_instrument_in_ticks() {
        let bench = Bench::new();
        let mut session = Session {
            instrument: &bench,
            total: 0,
        };
        assert_eq!(session.handle("SET_PRESET_LIVE 300"), "OK");
        assert_eq!(bench.sent.borrow()[..], ["SET_LIVE_PRESET 15000"]);
    }

    #[test]
    fn a_count_preset_reaches_the_instrument_as_counts_not_ticks() {
        let bench = Bench::new();
        let mut session = Session {
            instrument: &bench,
            total: 0,
        };
        assert_eq!(session.handle("SET_PRESET_COUNT 100"), "OK");
        assert_eq!(session.handle("SET_PRESET_INTEG 5000"), "OK");
        assert_eq!(
            bench.sent.borrow()[..],
            ["SET_PEAK_PRESET 100", "SET_INTEGRAL_PRESET 5000"]
        );
    }

    #[test]
    fn clearing_presets_zeroes_every_register_the_instrument_has() {
        let bench = Bench::new();
        let mut session = Session {
            instrument: &bench,
            total: 0,
        };
        assert_eq!(session.handle("SET_PRESET_CLEAR"), "OK");
        assert_eq!(
            bench.sent.borrow()[..],
            [
                "SET_TRUE_PRESET 0",
                "SET_LIVE_PRESET 0",
                "SET_PEAK_PRESET 0",
                "SET_INTEGRAL_PRESET 0"
            ]
        );
    }

    #[test]
    fn a_data_read_reports_every_channel_and_feeds_the_status_total() {
        let bench = Bench::new();
        let mut session = Session {
            instrument: &bench,
            total: 0,
        };
        assert_eq!(session.handle("SHOW_DATA"), "DATA 4 0 10 20 30");
        let status = session.handle("SHOW_STATUS");
        assert!(status.ends_with("TOTAL=60"), "{status}");
    }

    #[test]
    fn the_instruments_own_dialect_passes_through_untouched() {
        let bench = Bench::new();
        let mut session = Session {
            instrument: &bench,
            total: 0,
        };
        assert_eq!(session.handle("SET_GAIN_CONVERSION 8192"), "OK");
        assert_eq!(bench.sent.borrow()[..], ["SET_GAIN_CONVERSION 8192"]);
    }

    #[test]
    fn a_configuration_names_the_instrument() {
        let bench = Bench::new();
        let mut session = Session {
            instrument: &bench,
            total: 0,
        };
        assert_eq!(
            session.handle("SHOW_CONFIGURATION"),
            "MODEL=0926-001 SERIAL=08134079 FIRMWARE=0926-001 CHANNELS=4"
        );
    }

    #[test]
    fn a_verb_the_bridge_does_not_know_is_refused_by_name() {
        let bench = Bench::new();
        let mut session = Session {
            instrument: &bench,
            total: 0,
        };
        assert!(session.handle("FROBNICATE").starts_with("ERR "));
        assert!(bench.sent.borrow().is_empty());
    }
}
