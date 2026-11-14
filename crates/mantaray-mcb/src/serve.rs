//! The bridge proper: mantaray's dialect in, ORTEC's calls out.
//!
//! mantaray speaks one small ASCII dialect to every instrument it does not hold
//! in process - `SHOW_STATUS`, `SHOW_DATA`, `START`, `SET_PRESET_LIVE 300`. A
//! real ORTEC MCB speaks something else: `SHOW_LIVE` answering `$G0007663632108`,
//! ten digits of twenty-millisecond ticks and a checksum, and it rejects
//! `SET_PRESET_LIVE` outright. Translating between the two is this module's
//! whole job, and it is the reason mantaray needs no special case for hardware:
//! the same [`RemoteMcb`] drives a socket and drives this.
//!
//! Lines arrive on standard input and replies leave on standard output, one
//! apiece, so the transport at the other end is a pipe rather than a socket.
//!
//! [`RemoteMcb`]: https://docs.rs/mantaray-device

use std::io::{BufRead, Write};

#[cfg(windows)]
use crate::umcbi::{Hdet, Umcbi};

/// Seconds in one of the instrument's clock ticks.
///
/// Checked against hardware rather than assumed: `SHOW_TRUE` answered
/// `$G0007892556117` while MAESTRO displayed 157851.12 s for the same detector,
/// and 7892556 * 0.02 is exactly that.
pub(crate) const TICK: f64 = 0.02;

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
    /// Reads every channel the instrument is set to use. An instrument whose
    /// channel count can move between two calls reads both in one.
    fn read_all(&self) -> Result<Vec<u64>, String> {
        self.read(0, self.channels())
    }
    /// Whether the ADC is acquiring, or why that cannot be told. A failed
    /// read is never "not acquiring": one false idle mid-count ended a job's
    /// wait and saved a partial spectrum (release review S1).
    fn is_counting(&self) -> Result<bool, String>;
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
        // One command, one line. Passthrough relays the instrument's own
        // words, and a reply is only cut at a carriage return or a NUL - a
        // lone line feed in it would arrive as two answers and pair every
        // later question with the wrong one, for as long as the connection
        // lasts. The instrument's text is not this program's to trust.
        let reply = one_line(&reply);
        writeln!(output, "{reply}").map_err(|error| format!("writing a reply: {error}"))?;
        output
            .flush()
            .map_err(|error| format!("flushing a reply: {error}"))?;
    }
    Ok(())
}

/// One opened instrument and what the hub remembers about it.
struct HubEntry<'a> {
    instrument: Box<dyn Instrument + 'a>,
    /// [`Session::total`], carried between commands the way `run`'s single
    /// session carries it for one instrument.
    total: u64,
}

/// Runs the hub: every local detector through one process, one command at a
/// time.
///
/// The bench that forced this ran one bridge process per detector entry, and
/// ORTEC's configuration on that machine resolved three entries to the same
/// physical instrument - so three processes transacted with one mailbox at
/// once, their replies crossed, and the spectrum on screen flashed between
/// three interleaved snapshots until the adapter wedged outright. A single
/// process answering one command before reading the next cannot interleave
/// anything: the serialisation is the loop.
///
/// The dialect grows one routing mark: `@<n> <command>`, where `n` is the
/// detector number the entry names. An instrument is opened the first time
/// its number is asked for - `open` decides how, trying the same roads
/// `serve <n>` would - and an entry that cannot be opened answers `ERR`
/// with the reason, each time, rather than poisoning the others.
// Only the Windows bridge runs a hub today: the libusb road's adapters are
// claimed exclusively, so its per-adapter processes cannot converge on one
// instrument the way `serve N`'s lone-adapter forgiveness let ORTEC entries
// do. The routing itself is road-agnostic and tested everywhere.
#[cfg_attr(not(windows), allow(dead_code))]
#[allow(clippy::type_complexity)]
pub fn run_hub<'a>(
    open: &mut dyn FnMut(i32) -> Result<Box<dyn Instrument + 'a>, String>,
) -> Result<(), String> {
    use std::io::{BufRead, Write};
    eprintln!("hub: serving local detectors as they are asked for");
    let input = std::io::stdin();
    let mut output = std::io::stdout();
    let mut entries: std::collections::BTreeMap<i32, HubEntry<'a>> = Default::default();
    for line in input.lock().lines() {
        let line = line.map_err(|error| format!("reading a command: {error}"))?;
        if line.trim().is_empty() {
            continue;
        }
        // Bound before `one_line` sees it, not passed as a temporary: the
        // sibling branch teaches `one_line` to borrow its input when there is
        // nothing to change, and a borrow of a temporary is the build break
        // git's auto-merge would hand whoever merged second.
        let reply = hub_line(line.trim(), &mut entries, open);
        let reply = one_line(&reply);
        writeln!(output, "{reply}").map_err(|error| format!("writing a reply: {error}"))?;
        output
            .flush()
            .map_err(|error| format!("flushing a reply: {error}"))?;
    }
    Ok(())
}

/// One hub command handled: route, opening the detector if this is its first.
///
/// Split from [`run_hub`]'s pipe loop so the routing can be tested without a
/// process around it.
fn hub_line<'a>(
    line: &str,
    entries: &mut std::collections::BTreeMap<i32, HubEntry<'a>>,
    open: &mut dyn FnMut(i32) -> Result<Box<dyn Instrument + 'a>, String>,
) -> String {
    let Some((number, command)) = parse_hub_line(line) else {
        return format!("ERR the hub routes by detector: @<n> <command>, not {line:?}");
    };
    let entry = match entries.entry(number) {
        std::collections::btree_map::Entry::Occupied(held) => held.into_mut(),
        std::collections::btree_map::Entry::Vacant(empty) => match open(number) {
            Ok(instrument) => empty.insert(HubEntry {
                instrument,
                total: 0,
            }),
            Err(error) => return format!("ERR opening detector {number}: {error}"),
        },
    };
    // A session per command, around the entry's carried state: `Session`
    // stays exactly what `run` uses, so the two roads cannot drift apart.
    let mut session = Session {
        instrument: entry.instrument.as_ref(),
        total: entry.total,
    };
    let reply = session.handle(command);
    entry.total = session.total;
    reply
}

/// Reads `@<n> <command>` into its number and command.
fn parse_hub_line(line: &str) -> Option<(i32, &str)> {
    let (mark, command) = line.strip_prefix('@')?.split_once(' ')?;
    let command = command.trim();
    if command.is_empty() {
        return None;
    }
    Some((mark.parse().ok()?, command))
}

/// A reply reduced to the one line the protocol allows.
///
/// Control characters become spaces rather than disappearing, so that a reply
/// that arrives mangled still reads as mangled instead of silently closing up
/// into something plausible.
fn one_line(reply: &str) -> std::borrow::Cow<'_, str> {
    let trimmed = reply.trim();
    if !trimmed.chars().any(char::is_control) {
        // The ordinary case - and the DATA line is the largest string in the
        // program, so rebuilding it twice here to change nothing was the
        // most expensive way to leave it alone.
        return std::borrow::Cow::Borrowed(trimmed);
    }
    std::borrow::Cow::Owned(
        trimmed
            .chars()
            .map(|c| if c.is_control() { ' ' } else { c })
            .collect::<String>()
            .trim()
            .to_string(),
    )
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
            "SHOW_PRESETS" => self.presets(),
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
        let reply = match outcome {
            Ok(reply) => reply,
            Err(error) => format!("ERR {error}"),
        };
        if crate::journal::on() {
            // Digested: a DATA reply is a whole spectrum, and its journal
            // line is its shape - data() writes the sum separately.
            let shown: String = reply.chars().take(48).collect();
            let more = if reply.len() > shown.len() { "…" } else { "" };
            crate::journal::line(&format!("{command} -> {shown}{more}"));
        }
        reply
    }

    /// Sends a command through untouched and relays what came back.
    ///
    /// A SHOW sent through SEND_MESSAGE deserves its answer, not a flat OK
    /// that hides it. A valid SET answers an empty reply (indistinguishable
    /// from an unknown verb - the 926 will not say), and the dialect's answer
    /// for that is OK.
    fn pass(&mut self, command: &str) -> Result<String, String> {
        let reply = self.instrument.command(command)?;
        let reply = reply.trim();
        if reply.is_empty() {
            Ok("OK".into())
        } else {
            Ok(reply.to_string())
        }
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

    /// What the instrument is holding in its own preset registers.
    ///
    /// Presets outlive the session that set them: a live preset set yesterday
    /// still stops a count today, whether or not anything on the host knows
    /// about it. Until this could be asked, mantaray could only write presets,
    /// so a restarted application showed none while the instrument quietly
    /// refused to count - it answers START with its usual empty reply and
    /// then does not run. Time presets are read in ticks and reported in
    /// seconds; count presets are counts either way. Zero means none set,
    /// which is how the instrument itself says it.
    fn presets(&mut self) -> Result<String, String> {
        let real = self.register("SHOW_TRUE_PRESET") * TICK;
        let live = self.register("SHOW_LIVE_PRESET") * TICK;
        let peak = self.register("SHOW_PEAK_PRESET") as u64;
        let integral = self.register("SHOW_INTEGRAL_PRESET") as u64;
        Ok(format!(
            "PRESETS REAL={real:.2} LIVE={live:.2} PEAK={peak} INTEG={integral}"
        ))
    }

    /// One preset register, or zero when the instrument will not say.
    ///
    /// An instrument that does not know the question is one with no preset
    /// set as far as anything here can tell, which is the honest answer and
    /// the same one it gave before the question existed.
    fn register(&self, verb: &str) -> f64 {
        self.instrument
            .command(verb)
            .ok()
            .and_then(|reply| record_number(&reply, 'G').ok())
            .unwrap_or(0.0)
            .max(0.0)
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
        // Space-delimited fields carry no spaces: a UMCBI model name like
        // "DSPEC 50" would otherwise shift every later field.
        let model = if model.is_empty() {
            "MCB".to_string()
        } else {
            model.replace(' ', "_")
        };
        Ok(format!(
            "MODEL={model} SERIAL={} FIRMWARE={} CHANNELS={channels}{calibration}",
            self.instrument.identity().replace(' ', "_"),
            record_text(&firmware),
        ))
    }

    fn status(&mut self) -> Result<String, String> {
        // The counting flag before the clocks. The three are separate
        // commands, a round trip apart each, and an instrument counting out a
        // preset can stop *between* them: flag last, the reply said "stopped,
        // LT=0.98" - clocks older than the stop they arrived beside, and a
        // preset the instrument honoured to the tick looked unreached. Flag
        // first, a reply that says stopped carries clocks read after the
        // stop, which are the settled ones; the harmless converse - counting
        // beside clocks that have run on a little - is the ordinary state of
        // any status read from a live instrument. Caught by the bench 926
        // counting out a one-second live preset.
        //
        // A flag that cannot be read makes the whole status an ERR, which the
        // far side takes as "cannot tell": it keeps the last good reading and
        // refuses changes. Printed as ACTIVE=0 beside good clocks, it read as
        // a count that had ended.
        let active = self
            .instrument
            .is_counting()
            .map(i32::from)
            .map_err(|error| format!("the counting flag cannot be read: {error}"))?;
        // Live first, then real. The two clocks are separate commands, so they
        // are sampled a round trip apart, and whichever is read second has run
        // on: reading real time second keeps it the larger of the two, which is
        // the direction the arithmetic below can survive.
        let live = self.clock("SHOW_LIVE")?;
        let real = self.clock("SHOW_TRUE")?;
        // The instrument reports no dead time of its own; it is the difference
        // between the two clocks, which is what dead time means. Clamped,
        // because the two readings are not simultaneous: on the bench 926 at a
        // low count rate this printed DT=-2.00% from RT=1.00 LT=1.02, and dead
        // time below zero is not a measurement, it is a sampling artefact.
        let dead = if real > 0.0 {
            ((real - live) / real * 100.0).clamp(0.0, 100.0)
        } else {
            0.0
        };
        Ok(format!(
            "RT={real:.2} LT={live:.2} DT={dead:.2}% ICR=0 ACTIVE={active} TOTAL={}",
            self.total
        ))
    }

    /// One of the instrument's clocks, in seconds.
    fn clock(&mut self, command: &str) -> Result<f64, String> {
        let reply = self.instrument.command(command)?;
        Ok(record_number(&reply, 'G')? * TICK)
    }

    fn data(&mut self) -> Result<String, String> {
        use std::fmt::Write as _;
        let counts = self.instrument.read_all()?;
        self.total = counts.iter().sum();
        if crate::journal::on() {
            crate::journal::line(&format!(
                "read: returned={} sum={}",
                counts.len(),
                self.total
            ));
        }
        // Eight bytes a channel holds any count to seven digits with its
        // space; the old four under-sized the moment counts passed 999 and
        // paid a mid-build reallocation of the largest string in the program.
        let mut reply = String::with_capacity(counts.len() * 8 + 16);
        reply.push_str("DATA ");
        // Formatted in place: `to_string` per channel was a heap allocation
        // and free for every one of sixteen thousand channels, twice a
        // second. Writing to a String cannot fail.
        let _ = write!(reply, "{}", counts.len());
        for count in &counts {
            let _ = write!(reply, " {count}");
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
    fn is_counting(&self) -> Result<bool, String> {
        // `MIOIsActive` has no failure of its own to report.
        Ok(self.library.is_counting(self.detector))
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
        let (channels, model) = {
            let memory = crate::dpm::Dpm::new(&device);
            identify(&mut |text: &str| memory.command(text), &mut || {
                device.settle()
            })?
        };
        Ok(Self {
            device,
            channels,
            model,
            serial,
        })
    }

    /// The device path this instrument was opened by.
    pub fn path(&self) -> &str {
        self.device.path()
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
    fn is_counting(&self) -> Result<bool, String> {
        active_record(self.command("SHOW_ACTIVE"))
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

#[cfg(windows)]
use crate::pick::serial_of;

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
        let (channels, model) = {
            let memory = crate::dpm::Dpm::new(&device);
            identify(&mut |text: &str| memory.command(text), &mut || {
                device.settle()
            })?
        };
        Ok(Self {
            device,
            channels,
            model,
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
    fn is_counting(&self) -> Result<bool, String> {
        active_record(self.command("SHOW_ACTIVE"))
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

/// One input of a Model 927, reached over libusb.
///
/// A 927 has no ASCII dialect on the wire - ORTEC's own library turns the
/// commands into register operations on the host - so this does the same for
/// the handful of commands [`Session`] sends, and answers them in the record
/// format `Session` reads. Every other command is refused by name. That is
/// deliberate: a 926 answers a command it does not know with the same empty
/// reply as one it obeyed, and an `OK` for a high voltage this box does not
/// have would be a lie the operator could not see.
///
/// The box is held for one of these calls at a time and let go in between,
/// which is what lets a second process serve the other input.
#[cfg(not(windows))]
pub struct Via927<B: crate::mcb927::Shared> {
    device: B,
    serial: String,
    input: crate::mcb927::Input,
    /// The gain as found at start-up; only the answer of last resort.
    channels: usize,
}

#[cfg(not(windows))]
impl<B: crate::mcb927::Shared> Via927<B> {
    /// Opens one input, loading the FPGA first if it has no configuration.
    ///
    /// `image` is asked for only when a load is needed, so a box that is
    /// already configured needs no image at all - and is left exactly as it
    /// was found, discriminators included. The check and the load happen
    /// under one hold, so two processes opening the two inputs of a box just
    /// powered up load it once between them.
    pub fn open(
        device: B,
        serial: &str,
        input: crate::mcb927::Input,
        image: &dyn Fn() -> Result<Vec<u8>, String>,
    ) -> Result<Self, String> {
        use crate::mcb927::{Fpga, Mcb927, OPEN_PATIENCE};
        let channels = device.hold(OPEN_PATIENCE, |wire| {
            let mcb = Mcb927::new(wire);
            if mcb.fpga_status()? == Fpga::NeedsConfiguration {
                let image = image()?;
                mcb.configure(&image)?;
                eprintln!(
                    "loaded the 927's FPGA ({} bytes) and set both inputs' discriminators to \
                     {} and {}",
                    image.len(),
                    crate::mcb927::DEFAULT_LOWER_DISCRIMINATOR,
                    crate::mcb927::DEFAULT_UPPER_DISCRIMINATOR,
                );
            }
            Ok(mcb.channels(input)?)
        })?;
        Ok(Self {
            device,
            serial: serial.to_string(),
            input,
            channels,
        })
    }

    /// Runs one operation with the box held.
    fn held<T>(
        &self,
        operation: impl FnOnce(&crate::mcb927::Mcb927<'_>) -> Result<T, crate::mcb927::Error>,
    ) -> Result<T, String> {
        self.device.hold(crate::mcb927::PATIENCE, |wire| {
            operation(&crate::mcb927::Mcb927::new(wire)).map_err(String::from)
        })
    }
}

/// The preset an ORTEC preset verb is about.
#[cfg(not(windows))]
fn preset_of(verb: &str) -> Option<crate::mcb927::Preset> {
    use crate::mcb927::Preset;
    match verb.split_once('_')?.1 {
        "TRUE_PRESET" => Some(Preset::Real),
        "LIVE_PRESET" => Some(Preset::Live),
        "PEAK_PRESET" => Some(Preset::RoiPeak),
        "INTEGRAL_PRESET" => Some(Preset::RoiIntegral),
        _ => None,
    }
}

#[cfg(not(windows))]
impl<B: crate::mcb927::Shared> Instrument for Via927<B> {
    fn command(&self, text: &str) -> Result<String, String> {
        let (verb, argument) = match text.trim().split_once(' ') {
            Some((verb, rest)) => (verb, rest.trim()),
            None => (text.trim(), ""),
        };
        let input = self.input;
        let bare = || {
            if argument.is_empty() {
                Ok(())
            } else {
                Err(format!(
                    "{verb} takes no argument on a Model 927, and was given {argument:?}"
                ))
            }
        };
        match verb {
            "SHOW_VERSION" => {
                bare()?;
                let version = self.held(|mcb| mcb.firmware_version())?;
                Ok(format!("$F{version}"))
            }
            "SHOW_LIVE" => {
                bare()?;
                Ok(record('G', self.held(|mcb| mcb.live_ticks(input))?))
            }
            "SHOW_TRUE" => {
                bare()?;
                Ok(record('G', self.held(|mcb| mcb.real_ticks(input))?))
            }
            "SHOW_TRUE_PRESET"
            | "SHOW_LIVE_PRESET"
            | "SHOW_PEAK_PRESET"
            | "SHOW_INTEGRAL_PRESET" => {
                bare()?;
                let preset = preset_of(verb).expect("matched above");
                Ok(record('G', self.held(|mcb| mcb.preset(input, preset))?))
            }
            "SET_TRUE_PRESET" | "SET_LIVE_PRESET" | "SET_PEAK_PRESET" | "SET_INTEGRAL_PRESET" => {
                let preset = preset_of(verb).expect("matched above");
                let value: u32 = argument.parse().map_err(|_| {
                    format!(
                        "{verb} wants a whole number from 0 to {}, not {argument:?}",
                        u32::MAX
                    )
                })?;
                self.held(|mcb| mcb.set_preset(input, preset, value))?;
                Ok(String::new())
            }
            "START" => {
                bare()?;
                self.held(|mcb| mcb.start(input))?;
                Ok(String::new())
            }
            "STOP" => {
                bare()?;
                self.held(|mcb| mcb.stop(input))?;
                Ok(String::new())
            }
            // Counts and clocks both, as the 926's CLEAR does and as the
            // application expects of it. Not quite all the counts: ORCA's
            // frame stops one word short, so channel 16383 may survive -
            // see `Mcb927::clear`.
            "CLEAR" => {
                bare()?;
                self.held(|mcb| {
                    mcb.clear(input)?;
                    mcb.clear_clocks(input)
                })?;
                Ok(String::new())
            }
            other => Err(format!(
                "{other} is not a command this bridge can carry out on a Model 927"
            )),
        }
    }
    fn channels(&self) -> usize {
        // The gain can move under us (another process, a reloaded FPGA), so
        // it is asked again; the value from opening stands in if the box
        // cannot be asked.
        let input = self.input;
        self.held(|mcb| mcb.channels(input))
            .unwrap_or(self.channels)
    }
    fn read(&self, start: usize, count: usize) -> Result<Vec<u64>, String> {
        // The whole spectrum from channel zero, which is the read ORCA does
        // and the bench has seen; a partial read is a slice of it.
        Ok(self
            .read_all()?
            .into_iter()
            .skip(start)
            .take(count)
            .collect())
    }
    fn read_all(&self) -> Result<Vec<u64>, String> {
        // The length comes from the same hold as the counts, so a gain moved
        // in between cannot cut the spectrum to the old one.
        let input = self.input;
        let counts = self.held(|mcb| {
            let channels = mcb.channels(input)?;
            mcb.read_spectrum(input, 0, channels)
        })?;
        Ok(counts.into_iter().map(u64::from).collect())
    }
    fn is_counting(&self) -> Result<bool, String> {
        let input = self.input;
        self.held(|mcb| mcb.is_counting(input))
    }
    fn model(&self) -> String {
        "927".into()
    }
    fn identity(&self) -> String {
        format!("{}-{}", self.serial, self.input.number())
    }
    fn calibration(&self) -> Option<(f64, f64, f64)> {
        // There is no MAESTRO on this machine to have stored one.
        None
    }
    fn route(&self) -> String {
        format!(
            "input {} of Model 927 {} over libusb, with no vendor driver",
            self.input.number(),
            self.serial
        )
    }
}

/// A `$G` or `$C` record carrying one number, checksum and all.
///
/// The inverse of [`record_number`]: the width is the letter's, and the
/// checksum is the sum of every byte before it, modulo 256, in three digits.
#[cfg(not(windows))]
fn record(letter: char, value: u32) -> String {
    let width = if letter == 'C' { 5 } else { 10 };
    let body = format!("${letter}{value:0width$}");
    let sum = body.bytes().map(u32::from).sum::<u32>() % 256;
    format!("{body}{sum:03}")
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
/// used on carries exactly one. The letter is checked: under a reply desync a
/// version record's digits would otherwise parse as a plausible clock.
/// The counting flag in a `SHOW_ACTIVE` reply: `$C00001088` when the ADC is
/// acquiring, `$C00000087` when it is not. A failed read, or a reply that is
/// not a `$C` record (one out of step), says nothing either way.
pub(crate) fn active_record(reply: Result<String, String>) -> Result<bool, String> {
    let reply = reply.map_err(|error| format!("SHOW_ACTIVE failed: {error}"))?;
    Ok(record_number(&reply, 'C')? != 0.0)
}

pub(crate) fn record_number(reply: &str, letter: char) -> Result<f64, String> {
    let body = reply
        .trim()
        .strip_prefix('$')
        .and_then(|body| body.strip_prefix(letter))
        .ok_or_else(|| format!("{reply:?} is not a ${letter} record"))?;
    let digits: String = body.chars().filter(char::is_ascii_digit).collect();
    if digits.len() <= 3 || digits.len() != body.len() {
        return Err(format!("{reply:?} does not carry a ${letter} number"));
    }
    digits[..digits.len() - 3]
        .parse()
        .map_err(|_| format!("{reply:?} is not a number"))
}

/// The text of an `$F` record, which carries no checksum to strip.
pub(crate) fn record_text(reply: &str) -> String {
    let text = reply.trim().trim_start_matches('$');
    let text = text.strip_prefix('F').unwrap_or(text);
    if text.is_empty() {
        "unknown".into()
    } else {
        text.replace(' ', "_")
    }
}

/// How many times opening clears the pipes and asks again before giving up.
pub(crate) const OPEN_ATTEMPTS: usize = 3;

/// Whether a reply is the `$F` record `SHOW_VERSION` answers with.
fn is_version_record(reply: &str) -> bool {
    reply
        .trim()
        .strip_prefix("$F")
        .is_some_and(|rest| !rest.is_empty())
}

/// Asks a freshly opened adapter what it is, and makes sure it is in step.
///
/// An adapter left out of step answers each question with the reply to the one
/// before, so every answer looks right but belongs to the wrong question: a
/// channel count read off a version string, or a version read off a status.
/// Each reply is therefore checked for the record letter its question gets
/// (`$F` for `SHOW_VERSION`, `$C` for `SHOW_GAIN_CONVERSION`), and the version
/// is asked again last, so a reply one behind shows up as the wrong letter.
/// On any mismatch the pipes are settled, as a timed-out request settles them,
/// and the whole exchange starts over, up to [`OPEN_ATTEMPTS`] times.
///
/// Returns the conversion gain in channels and the model ("" when unknown).
pub(crate) fn identify(
    ask: &mut dyn FnMut(&str) -> Result<String, String>,
    settle: &mut dyn FnMut(),
) -> Result<(usize, String), String> {
    let mut last = String::new();
    for attempt in 0..OPEN_ATTEMPTS {
        if attempt > 0 {
            settle();
        }
        let version = match ask("SHOW_VERSION") {
            Ok(reply) if is_version_record(&reply) => reply,
            Ok(reply) => {
                last = format!("SHOW_VERSION answered {reply:?}, not a $F record");
                continue;
            }
            Err(error) => {
                last = format!("SHOW_VERSION: {error}");
                continue;
            }
        };
        let gain = match ask("SHOW_GAIN_CONVERSION") {
            Ok(reply) => reply,
            Err(error) => {
                last = format!("SHOW_GAIN_CONVERSION: {error}");
                continue;
            }
        };
        let Ok(channels) = record_number(&gain, 'C') else {
            last = format!("SHOW_GAIN_CONVERSION answered {gain:?}, not a $C record");
            continue;
        };
        match ask("SHOW_VERSION") {
            Ok(reply) if is_version_record(&reply) => {}
            Ok(reply) => {
                last = format!("SHOW_VERSION asked again answered {reply:?}");
                continue;
            }
            Err(error) => {
                last = format!("SHOW_VERSION asked again: {error}");
                continue;
            }
        }
        // In step and well formed, so a nonsense gain is the instrument's own
        // answer, not a stale one: there is no point asking again.
        if !(1.0..=f64::from(1_u32 << 20)).contains(&channels) {
            return Err(format!("{channels} channels is not a conversion gain"));
        }
        #[allow(clippy::cast_possible_truncation, clippy::cast_sign_loss)]
        let channels = channels as usize;
        let model = record_text(&version);
        let model = if model == "unknown" {
            String::new()
        } else {
            model
        };
        return Ok((channels, model));
    }
    Err(format!(
        "the adapter is not answering in step ({last}) after {OPEN_ATTEMPTS} tries with the \
         pipes cleared between them. Close MantaRay and run `mantaray-mcb usbtalk SHOW_VERSION` \
         twice; a healthy adapter answers $F0926-001 both times"
    ))
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn a_clock_record_loses_its_checksum() {
        // The reply the 926 gave for SHOW_TRUE while MAESTRO showed 157851.12 s.
        assert_eq!(record_number("$G0007892556117", 'G').unwrap(), 7_892_556.0);
        assert!((7_892_556.0 * TICK - 157_851.12).abs() < 1e-6);
    }

    #[test]
    fn a_conversion_gain_record_reads_as_its_channel_count() {
        assert_eq!(record_number("$C08192107", 'C').unwrap(), 8192.0);
    }

    #[test]
    fn the_wrong_record_letter_is_refused_not_read_as_a_number() {
        // Under a reply desync, a version record's digits would parse as a
        // plausible clock - "$F0926-001" carries 0926001, which after the
        // checksum strip reads as 926. The letter check is what refuses it.
        assert!(record_number("$F0926-001", 'G').is_err());
        assert!(record_number("$G0007892556117", 'C').is_err());
        assert!(record_number("$C", 'C').is_err());
        assert!(record_number("", 'G').is_err());
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
        /// Answers for the coming flag reads, oldest first; counting once
        /// they run out.
        flag: std::cell::RefCell<std::collections::VecDeque<Result<bool, String>>>,
    }

    impl Bench {
        fn new() -> Self {
            Self {
                sent: std::cell::RefCell::new(Vec::new()),
                flag: std::cell::RefCell::new(std::collections::VecDeque::new()),
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
        fn is_counting(&self) -> Result<bool, String> {
            self.flag.borrow_mut().pop_front().unwrap_or(Ok(true))
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

    /// Release review S1: a flag read that fails (a SHOW_ACTIVE that timed
    /// out, or came back out of step) once printed ACTIVE=0 beside good
    /// clocks, and one such line ended a job's WAIT mid-count. It says ERR,
    /// which the far side already takes as "cannot tell".
    #[test]
    fn a_counting_flag_that_cannot_be_read_is_an_err_never_idle() {
        let bench = Bench::new();
        bench
            .flag
            .borrow_mut()
            .push_back(Err("SHOW_ACTIVE timed out".into()));
        let mut session = Session {
            instrument: &bench,
            total: 0,
        };
        let reply = session.handle("SHOW_STATUS");
        assert!(reply.starts_with("ERR "), "{reply}");
        assert!(!reply.contains("ACTIVE="), "{reply}");
        assert!(reply.contains("SHOW_ACTIVE timed out"), "{reply}");
    }

    /// One failed flag read changes nothing after it: the next status is the
    /// ordinary line, word for word.
    #[test]
    fn one_failed_flag_read_leaves_the_next_status_as_it_was() {
        let bench = Bench::new();
        bench.flag.borrow_mut().push_back(Err("timed out".into()));
        let mut session = Session {
            instrument: &bench,
            total: 0,
        };
        assert!(session.handle("SHOW_STATUS").starts_with("ERR "));
        assert_eq!(
            session.handle("SHOW_STATUS"),
            "RT=157851.12 LT=153272.64 DT=2.90% ICR=0 ACTIVE=1 TOTAL=0"
        );
    }

    /// The USB roads' flag: a `$C` record, or no answer at all.
    #[test]
    fn the_flag_record_is_read_or_refused() {
        assert_eq!(active_record(Ok("$C00001088".into())), Ok(true));
        assert_eq!(active_record(Ok("$C00000087".into())), Ok(false));
        assert!(active_record(Err("timed out".into())).is_err());
        // One behind: SHOW_LIVE's reply where the flag's should be.
        assert!(active_record(Ok("$G0007663632108".into())).is_err());
        assert!(active_record(Ok(String::new())).is_err());
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
        // The instrument's own answer comes back - a SHOW through
        // SEND_MESSAGE deserves its reply, not a flat OK over it.
        assert_eq!(session.handle("SET_GAIN_CONVERSION 8192"), "%000000069");
        assert_eq!(session.handle("SHOW_TRUE"), "$G0007892556117");
        assert_eq!(
            bench.sent.borrow()[..],
            ["SET_GAIN_CONVERSION 8192", "SHOW_TRUE"]
        );
    }

    #[test]
    fn an_empty_reply_from_a_valid_set_reads_as_ok() {
        // The 926 answers a valid SET with an empty reply; the dialect's
        // answer for "it worked, nothing to say" is OK, not a blank line.
        struct Silent;
        impl Instrument for Silent {
            fn command(&self, _text: &str) -> Result<String, String> {
                Ok(String::new())
            }
            fn channels(&self) -> usize {
                4
            }
            fn read(&self, _start: usize, _count: usize) -> Result<Vec<u64>, String> {
                Ok(vec![0; 4])
            }
            fn is_counting(&self) -> Result<bool, String> {
                Ok(false)
            }
            fn model(&self) -> String {
                String::new()
            }
            fn identity(&self) -> String {
                "0".into()
            }
            fn calibration(&self) -> Option<(f64, f64, f64)> {
                None
            }
            fn route(&self) -> String {
                "a silent double".into()
            }
        }
        let silent = Silent;
        let mut session = Session {
            instrument: &silent,
            total: 0,
        };
        assert_eq!(session.handle("SET_GAIN_STABILIZER 1"), "OK");
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
    fn dead_time_never_reads_below_zero() {
        // Measured on the bench 926: the two clocks are separate commands, so
        // at a low count rate the live clock read a round trip later can come
        // back past the real clock - RT=1.00 LT=1.02 printed DT=-2.00%, which
        // is not a dead time any instrument can have.
        struct Skewed;
        impl Instrument for Skewed {
            fn command(&self, text: &str) -> Result<String, String> {
                Ok(match text {
                    "SHOW_TRUE" => "$G0000000050000".into(), // 50 ticks = 1.00 s
                    "SHOW_LIVE" => "$G0000000051000".into(), // 51 ticks = 1.02 s
                    _ => String::new(),
                })
            }
            fn channels(&self) -> usize {
                1
            }
            fn read(&self, _start: usize, _count: usize) -> Result<Vec<u64>, String> {
                Ok(vec![0])
            }
            fn is_counting(&self) -> Result<bool, String> {
                Ok(true)
            }
            fn model(&self) -> String {
                String::new()
            }
            fn identity(&self) -> String {
                "0".into()
            }
            fn calibration(&self) -> Option<(f64, f64, f64)> {
                None
            }
            fn route(&self) -> String {
                "a skewed double".into()
            }
        }
        let skewed = Skewed;
        let mut session = Session {
            instrument: &skewed,
            total: 0,
        };
        let status = session.handle("SHOW_STATUS");
        assert!(status.contains("DT=0.00%"), "{status}");
        assert!(!status.contains("DT=-"), "{status}");
    }

    #[test]
    fn a_reply_is_reduced_to_one_line_before_it_is_written() {
        // A reply is only cut at a carriage return or a NUL, so a lone line
        // feed reaches here. Two lines for one command would pair every later
        // question with the wrong answer, permanently.
        assert_eq!(one_line("$G0007892556117"), "$G0007892556117");
        assert_eq!(one_line("DATA 2 1\n2"), "DATA 2 1 2");
        assert_eq!(one_line("ERR bad\nreply\ttext"), "ERR bad reply text");
        assert!(!one_line("\n\n").contains('\n'));
    }

    #[test]
    fn the_presets_the_instrument_holds_can_be_read_back() {
        // The bench 926 on 2026-08-06 answered SHOW_LIVE_PRESET with
        // $G0000015000081 - 15000 ticks, a 300-second live preset it had been
        // holding since the session before. Reading them back is what lets
        // the application show a preset it did not set itself.
        struct Holding;
        impl Instrument for Holding {
            fn command(&self, text: &str) -> Result<String, String> {
                Ok(match text {
                    "SHOW_TRUE_PRESET" => "$G0000000000075".into(),
                    "SHOW_LIVE_PRESET" => "$G0000015000081".into(),
                    "SHOW_PEAK_PRESET" => "$G0000000123081".into(),
                    // The verb this instrument does not know.
                    "SHOW_INTEGRAL_PRESET" => return Err("unknown verb".into()),
                    _ => "%000000069".into(),
                })
            }
            fn channels(&self) -> usize {
                4
            }
            fn read(&self, _start: usize, _count: usize) -> Result<Vec<u64>, String> {
                Ok(vec![0; 4])
            }
            fn is_counting(&self) -> Result<bool, String> {
                Ok(false)
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
                "an instrument holding a preset".into()
            }
        }
        let holding = Holding;
        let mut session = Session {
            instrument: &holding,
            total: 0,
        };
        // Ticks out, seconds in; counts stay counts; a register the
        // instrument will not report reads as none set, not as an error.
        assert_eq!(
            session.handle("SHOW_PRESETS"),
            "PRESETS REAL=0.00 LIVE=300.00 PEAK=123 INTEG=0"
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

    #[test]
    fn a_hub_line_reads_its_number_and_command() {
        assert_eq!(parse_hub_line("@3 SHOW_STATUS"), Some((3, "SHOW_STATUS")));
        assert_eq!(
            parse_hub_line("@12 SET_PRESET_LIVE 300"),
            Some((12, "SET_PRESET_LIVE 300"))
        );
        // Unrouted, empty, and unnumbered lines are not hub lines.
        assert_eq!(parse_hub_line("SHOW_STATUS"), None);
        assert_eq!(parse_hub_line("@3 "), None);
        assert_eq!(parse_hub_line("@x SHOW_STATUS"), None);
    }

    /// The hub opens each detector once, keeps their sessions apart, and an
    /// entry that cannot open answers `ERR` without poisoning the rest.
    #[test]
    fn the_hub_routes_by_detector_and_opens_each_once() {
        let opens = std::cell::RefCell::new(Vec::new());
        let mut open = |number: i32| -> Result<Box<dyn Instrument>, String> {
            opens.borrow_mut().push(number);
            match number {
                9 => Err("nothing at 9".into()),
                _ => Ok(Box::new(Bench::new())),
            }
        };
        let mut entries = std::collections::BTreeMap::new();

        // Two detectors, each opened on first sight and only then.
        assert!(hub_line("@1 SHOW_DATA", &mut entries, &mut open).starts_with("DATA 4 "));
        assert!(hub_line("@2 SHOW_DATA", &mut entries, &mut open).starts_with("DATA 4 "));
        assert!(hub_line("@1 SHOW_STATUS", &mut entries, &mut open).contains("TOTAL=60"));
        assert_eq!(*opens.borrow(), vec![1, 2], "each detector opens once");

        // A detector that opens nowhere reports so every time it is asked,
        // and the ones that opened go on answering.
        assert!(hub_line("@9 SHOW_STATUS", &mut entries, &mut open).starts_with("ERR "));
        assert!(hub_line("@9 SHOW_STATUS", &mut entries, &mut open).starts_with("ERR "));
        assert_eq!(
            *opens.borrow(),
            vec![1, 2, 9, 9],
            "a failed open is retried"
        );
        assert!(hub_line("@2 SHOW_STATUS", &mut entries, &mut open).contains("TOTAL=60"));

        // A line without a route is refused as such.
        assert!(hub_line("SHOW_STATUS", &mut entries, &mut open).starts_with("ERR "));
    }

    /// A scripted adapter: answers in step, or one reply behind until settled
    /// often enough.
    struct Scripted {
        behind: Option<String>,
        settles_needed: usize,
        settles: usize,
        asked: usize,
        version: String,
        gain: String,
    }

    impl Scripted {
        fn new(stale: Option<&str>, settles_needed: usize) -> Self {
            Self {
                behind: stale.map(str::to_string),
                settles_needed,
                settles: 0,
                asked: 0,
                version: "$F0926-001".into(),
                gain: "$C04096087".into(),
            }
        }

        fn answer(&mut self, text: &str) -> Result<String, String> {
            self.asked += 1;
            let fresh = match text {
                "SHOW_VERSION" => self.version.clone(),
                "SHOW_GAIN_CONVERSION" => self.gain.clone(),
                other => return Err(format!("unscripted {other}")),
            };
            if self.settles >= self.settles_needed {
                return Ok(fresh);
            }
            // Out of step: this question gets the previous one's reply.
            Ok(self.behind.replace(fresh).unwrap_or_default())
        }

        fn identify(&mut self) -> Result<(usize, String), String> {
            let cell = std::cell::RefCell::new(self);
            identify(
                &mut |text: &str| cell.borrow_mut().answer(text),
                &mut || {
                    let mut adapter = cell.borrow_mut();
                    adapter.settles += 1;
                    if adapter.settles >= adapter.settles_needed {
                        adapter.behind = None;
                    }
                },
            )
        }
    }

    #[test]
    fn an_adapter_in_step_opens_without_settling() {
        let mut adapter = Scripted::new(None, 0);
        assert_eq!(adapter.identify(), Ok((4096, "0926-001".into())));
        assert_eq!(adapter.settles, 0);
        assert_eq!(adapter.asked, 3);
    }

    #[test]
    fn an_adapter_one_behind_is_settled_and_asked_again() {
        // A status reply left over from the last session answers the first
        // question; before, the gain was asked first and read off the version
        // that came back for it.
        let mut adapter = Scripted::new(Some("$M000372850500037299040000000000081"), 1);
        assert_eq!(adapter.identify(), Ok((4096, "0926-001".into())));
        assert_eq!(adapter.settles, 1);
    }

    #[test]
    fn a_stale_version_reply_is_caught_by_the_gain_letter() {
        // The stale reply is itself a $F, so the first check passes; the gain
        // question then gets the version and must be refused, not read.
        let mut adapter = Scripted::new(Some("$F0926-001"), 1);
        assert_eq!(adapter.identify(), Ok((4096, "0926-001".into())));
        assert_eq!(adapter.settles, 1);
    }

    #[test]
    fn a_stale_gain_reply_is_caught_by_the_second_version() {
        // Stale replies that happen to carry the right letters for the first
        // two questions: only asking the version again shows the third one is
        // not a version, and the adapter is refused rather than believed.
        let mut asked = 0;
        let replies = ["$F0926-001", "$C04096087", "$M0000"];
        let error = identify(
            &mut |_: &str| {
                asked += 1;
                Ok(replies[(asked - 1) % 3].to_string())
            },
            &mut || {},
        )
        .unwrap_err();
        assert!(error.contains("asked again"), "{error}");
        assert_eq!(asked, 3 * OPEN_ATTEMPTS);
    }

    #[test]
    fn an_adapter_that_never_comes_into_step_is_refused_clearly() {
        let mut adapter = Scripted::new(Some("$M0000"), usize::MAX);
        let error = adapter.identify().unwrap_err();
        assert_eq!(adapter.settles, OPEN_ATTEMPTS - 1);
        assert!(error.contains("not answering in step"), "{error}");
        assert!(error.contains("usbtalk SHOW_VERSION"), "{error}");
    }

    #[test]
    fn a_failed_request_on_open_is_retried_after_settling() {
        let mut failures = 1;
        let mut settles = 0;
        let mut adapter = Scripted::new(None, 0);
        let result = identify(
            &mut |text: &str| {
                if failures > 0 {
                    failures -= 1;
                    return Err("the instrument did not answer within 5000 ms".into());
                }
                adapter.answer(text)
            },
            &mut || settles += 1,
        );
        assert_eq!(result, Ok((4096, "0926-001".into())));
        assert_eq!(settles, 1);
    }

    #[test]
    fn a_malformed_gain_is_never_taken_as_a_channel_count() {
        let mut adapter = Scripted::new(None, 0);
        adapter.gain = "$CX".into();
        let error = adapter.identify().unwrap_err();
        assert!(error.contains("not a $C record"), "{error}");
        let mut adapter = Scripted::new(None, 0);
        adapter.gain = "$C00000087".into();
        let error = adapter.identify().unwrap_err();
        assert!(error.contains("not a conversion gain"), "{error}");
    }

    #[test]
    fn a_bare_f_is_not_a_version() {
        assert!(!is_version_record("$F"));
        assert!(!is_version_record("$C04096087"));
        assert!(is_version_record("$F0926-001\r"));
    }
}

/// The 927 behind the same `Session` the 926 is served through.
#[cfg(all(test, not(windows)))]
mod tests_927 {
    use super::*;
    use crate::mcb927::double::Fake927;
    use crate::mcb927::{Input, Register};

    /// A made-up image: not ORTEC's.
    const IMAGE: [u8; 120] = [0x5A; 120];

    fn no_image() -> Result<Vec<u8>, String> {
        panic!("a configured 927 must not ask for an image");
    }

    /// Input 2 of a configured box, which is the one whose addresses differ.
    fn second_input(fake: &Fake927) -> Via927<&Fake927> {
        Via927::open(fake, "9049204", Input::Two, &no_image).expect("open")
    }

    fn session<'a>(instrument: &'a dyn Instrument) -> Session<'a> {
        Session {
            instrument,
            total: 0,
        }
    }

    impl crate::mcb927::Shared for &Fake927 {
        fn hold<T>(
            &self,
            patience: std::time::Duration,
            operation: impl FnOnce(&dyn crate::dpm::BulkDevice) -> Result<T, String>,
        ) -> Result<T, String> {
            (**self).hold(patience, operation)
        }
    }

    #[test]
    fn a_record_is_built_the_way_the_926_builds_them() {
        // The two records the 926 itself gave, checksums included.
        assert_eq!(record('G', 7_892_556), "$G0007892556117");
        assert_eq!(record('C', 8192), "$C08192107");
        assert_eq!(
            record_number(&record('G', u32::MAX), 'G').unwrap(),
            f64::from(u32::MAX)
        );
    }

    #[test]
    fn a_configuration_names_the_input_and_reads_its_gain() {
        let fake = Fake927::configured();
        fake.set(Input::Two, Register::ConversionGain, 2);
        let instrument = second_input(&fake);
        assert_eq!(
            session(&instrument).handle("SHOW_CONFIGURATION"),
            "MODEL=927 SERIAL=9049204-2 FIRMWARE=2 CHANNELS=4096"
        );
        assert_eq!(
            instrument.route(),
            "input 2 of Model 927 9049204 over libusb, with no vendor driver"
        );
    }

    #[test]
    fn a_gain_changed_after_opening_is_the_gain_read() {
        let fake = Fake927::configured();
        fake.set(Input::Two, Register::ConversionGain, 2);
        let instrument = second_input(&fake);
        // Another process, or a reloaded FPGA, moves the gain under us.
        fake.set(Input::Two, Register::ConversionGain, 1);
        assert_eq!(instrument.channels(), 8192);
        assert_eq!(instrument.read(0, 16_384).unwrap().len(), 8192);
        assert_eq!(
            session(&instrument).handle("SHOW_CONFIGURATION"),
            "MODEL=927 SERIAL=9049204-2 FIRMWARE=2 CHANNELS=8192"
        );
    }

    #[test]
    fn a_data_read_takes_its_length_and_its_counts_under_one_hold() {
        // Between two holds the other process, or a reloaded FPGA, can move
        // the gain, and the spectrum would be cut to the old length.
        let fake = Fake927::configured();
        fake.set(Input::Two, Register::ConversionGain, 2);
        let instrument = second_input(&fake);
        let mut session = session(&instrument);
        let before = fake.holds.get();
        assert!(session.handle("SHOW_DATA").starts_with("DATA 4096 "));
        assert_eq!(fake.holds.get() - before, 1);
    }

    #[test]
    fn a_status_reads_this_inputs_flag_and_clocks() {
        let fake = Fake927::configured();
        fake.set(Input::Two, Register::AcquisitionStatus, 1);
        fake.set(Input::Two, Register::LiveTime, 500);
        fake.set(Input::Two, Register::RealTime, 600);
        // Input 1 holds other numbers, which must not be the ones reported.
        fake.set(Input::One, Register::LiveTime, 9999);
        let instrument = second_input(&fake);
        assert_eq!(
            session(&instrument).handle("SHOW_STATUS"),
            "RT=12.00 LT=10.00 DT=16.67% ICR=0 ACTIVE=1 TOTAL=0"
        );
        let first = Via927::open(&fake, "9049204", Input::One, &no_image).unwrap();
        assert_eq!(
            session(&first).handle("SHOW_STATUS"),
            "RT=0.00 LT=199.98 DT=0.00% ICR=0 ACTIVE=0 TOTAL=0"
        );
    }

    #[test]
    fn a_data_read_is_this_inputs_memory_as_plain_counts() {
        let fake = Fake927::configured();
        fake.set(Input::Two, Register::ConversionGain, 6);
        fake.memory.borrow_mut()[0x8000] = 3;
        fake.memory.borrow_mut()[0x8000 + 255] = 4;
        fake.memory.borrow_mut()[0] = 1000;
        let instrument = second_input(&fake);
        let mut session = session(&instrument);
        let data = session.handle("SHOW_DATA");
        assert!(data.starts_with("DATA 256 3 0 0 "), "{data}");
        assert!(data.ends_with(" 0 4"), "{data}");
        assert!(session.handle("SHOW_STATUS").ends_with("TOTAL=7"));
        // And the read after a spectrum is still in step.
        assert!(session.handle("SHOW_STATUS").starts_with("RT="));
    }

    #[test]
    fn a_preset_in_seconds_lands_in_the_register_in_ticks_and_is_armed() {
        let fake = Fake927::configured();
        let instrument = second_input(&fake);
        let mut session = session(&instrument);
        assert_eq!(session.handle("SET_PRESET_LIVE 300"), "OK");
        assert_eq!(fake.get(Input::Two, Register::LivePreset), 15_000);
        assert_eq!(fake.get(Input::Two, Register::PresetControl), 1 << 3);
        assert_eq!(session.handle("SET_PRESET_COUNT 100"), "OK");
        assert_eq!(session.handle("SET_PRESET_INTEG 5000"), "OK");
        assert_eq!(session.handle("SET_PRESET_REAL 2"), "OK");
        assert_eq!(
            session.handle("SHOW_PRESETS"),
            "PRESETS REAL=2.00 LIVE=300.00 PEAK=100 INTEG=5000"
        );
        assert_eq!(session.handle("SET_PRESET_CLEAR"), "OK");
        assert_eq!(
            session.handle("SHOW_PRESETS"),
            "PRESETS REAL=0.00 LIVE=0.00 PEAK=0 INTEG=0"
        );
        assert_eq!(fake.get(Input::Two, Register::PresetControl), 0);
        assert_eq!(fake.get(Input::One, Register::LivePreset), 0, "input 1");
    }

    #[test]
    fn a_start_and_a_stop_touch_nothing_but_the_run() {
        let fake = Fake927::configured();
        fake.set(Input::Two, Register::ConversionGain, 1);
        fake.set(Input::Two, Register::LivePreset, 15_000);
        fake.set(Input::Two, Register::PresetControl, 1 << 3);
        fake.set(Input::Two, Register::Control, 0x20);
        let instrument = second_input(&fake);
        let mut session = session(&instrument);

        assert_eq!(session.handle("START"), "OK");
        assert!(session.handle("SHOW_STATUS").contains("ACTIVE=1"));
        assert_eq!(*fake.slow_ops.borrow(), [0x06, 0x10], "one reset MDA");
        assert_eq!(session.handle("STOP"), "OK");
        assert!(session.handle("SHOW_STATUS").contains("ACTIVE=0"));

        // Only the control and stop registers of input 2 were written.
        let writes = fake.writes.borrow();
        assert_eq!(
            *writes,
            [
                (0x20, 0x20),
                (0x20, 0x21),
                (0x3F, 1),
                (0x3F, 0),
                (0x20, 0x20)
            ]
        );
        assert_eq!(fake.get(Input::Two, Register::ConversionGain), 1);
        assert_eq!(fake.get(Input::Two, Register::LivePreset), 15_000);
        assert_eq!(fake.get(Input::Two, Register::PresetControl), 1 << 3);
        assert!(fake.discriminators.borrow().is_empty());
    }

    #[test]
    fn a_clear_zeroes_this_inputs_counts_and_clocks_and_not_the_others() {
        let fake = Fake927::configured();
        for (channel, count) in [(10, 55), (16_382, 8), (16_383, 9)] {
            fake.memory.borrow_mut()[0x8000 + channel] = count;
        }
        fake.memory.borrow_mut()[10] = 66;
        fake.set(Input::Two, Register::LiveTime, 500);
        fake.set(Input::Two, Register::RealTime, 600);
        fake.set(Input::One, Register::RealTime, 700);
        let instrument = second_input(&fake);
        let mut session = session(&instrument);
        session.handle("SHOW_DATA");
        assert_eq!(session.handle("CLEAR"), "OK");
        // Up to the last channel the frame reaches.
        assert_eq!(fake.memory.borrow()[0x8000 + 10], 0);
        assert_eq!(fake.memory.borrow()[0x8000 + 16_382], 0);
        assert_eq!(fake.memory.borrow()[10], 66);
        assert_eq!(fake.get(Input::Two, Register::LiveTime), 0);
        assert_eq!(fake.get(Input::Two, Register::RealTime), 0);
        assert_eq!(fake.get(Input::One, Register::RealTime), 700);
        // `Session` zeroes its own total on a clear, whatever the box did.
        assert!(session.handle("SHOW_STATUS").ends_with("TOTAL=0"));
        // ORCA's frame is 16383 words, so channel 16383 is left as it was,
        // and a read of the box after the clear says so. Whether the real
        // box clears it anyway is not yet measured; the fake does what the
        // frame says.
        assert_eq!(fake.memory.borrow()[0x8000 + 16_383], 9);
        assert!(session.handle("SHOW_DATA").ends_with(" 0 9"));
        assert!(session.handle("SHOW_STATUS").ends_with("TOTAL=9"));
    }

    #[test]
    fn every_other_verb_is_refused_by_name_and_never_reaches_the_box() {
        let fake = Fake927::configured();
        let instrument = second_input(&fake);
        let mut session = session(&instrument);
        let holds = fake.holds.get();
        for command in [
            "SET_HV 1000",
            "SET_GAIN_COARSE 2",
            "SET_GAIN_CONVERSION 8192",
            "SHOW_ACTIVE",
            "ENABLE_HV",
            "$X",
            "SHOW_LIVE now",
        ] {
            let verb = command.split(' ').next().unwrap();
            let reply = session.handle(command);
            assert!(reply.starts_with("ERR "), "{command}: {reply}");
            assert!(reply.contains(verb), "{command}: {reply}");
        }
        // `Session` sends START bare whatever came after it; asked directly,
        // an argument this layer would have to ignore is refused instead.
        let error = instrument.command("START 1").unwrap_err();
        assert!(error.contains("START takes no argument"), "{error}");
        assert_eq!(session.handle("SET_LIVE_PRESET many"), {
            "ERR SET_LIVE_PRESET wants a whole number from 0 to 4294967295, not \"many\""
        });
        assert_eq!(fake.holds.get(), holds, "the box was never asked");
        assert!(fake.writes.borrow().is_empty());
    }

    #[test]
    fn the_real_dialects_own_verbs_pass_through_to_the_same_registers() {
        let fake = Fake927::configured();
        fake.set(Input::Two, Register::RealTime, 7_892_556);
        let instrument = second_input(&fake);
        let mut session = session(&instrument);
        assert_eq!(session.handle("SHOW_TRUE"), "$G0007892556117");
        assert_eq!(session.handle("SET_LIVE_PRESET 15000"), "OK");
        assert_eq!(session.handle("SHOW_LIVE_PRESET"), "$G0000015000081");
    }

    #[test]
    fn the_box_is_held_once_for_each_thing_asked_of_it() {
        let fake = Fake927::configured();
        let instrument = second_input(&fake);
        assert_eq!(fake.holds.get(), 1, "opening");
        instrument.is_counting().unwrap();
        instrument.command("START").unwrap();
        instrument.read(0, 16_384).unwrap();
        assert_eq!(fake.holds.get(), 4);
    }

    #[test]
    fn a_box_held_elsewhere_is_an_err_and_never_idle() {
        let fake = Fake927::configured();
        fake.set(Input::Two, Register::AcquisitionStatus, 1);
        let instrument = second_input(&fake);
        fake.busy.set(true);
        let reply = session(&instrument).handle("SHOW_STATUS");
        assert!(reply.starts_with("ERR "), "{reply}");
        assert!(!reply.contains("ACTIVE="), "{reply}");
        assert!(session(&instrument).handle("START").starts_with("ERR "));
        fake.busy.set(false);
        assert!(
            session(&instrument)
                .handle("SHOW_STATUS")
                .contains("ACTIVE=1")
        );
    }

    #[test]
    fn a_box_that_needs_its_fpga_is_loaded_when_an_image_is_given() {
        let fake = Fake927::unconfigured();
        let instrument =
            Via927::open(&fake, "9049204", Input::One, &|| Ok(IMAGE.to_vec())).expect("open");
        assert!(fake.configured.get());
        assert_eq!(*fake.loaded.borrow(), IMAGE);
        // Both inputs' discriminators, since the load reset the whole box
        // and the process serving the other input will not set them.
        assert_eq!(
            *fake.discriminators.borrow(),
            [(1, 164), (0, 16_383), (3, 164), (2, 16_383)]
        );
        assert_eq!(fake.holds.get(), 1, "checked and loaded under one hold");
        assert_eq!(instrument.channels(), 16_384);
    }

    #[test]
    fn a_box_already_configured_is_left_exactly_as_found() {
        let fake = Fake927::configured();
        let _ = second_input(&fake);
        assert_eq!(*fake.slow_ops.borrow(), [0x06], "the test and nothing else");
        assert!(fake.discriminators.borrow().is_empty());
        assert!(fake.writes.borrow().is_empty());
    }

    #[test]
    fn a_box_that_needs_its_fpga_and_has_no_image_says_how_to_give_one() {
        let fake = Fake927::unconfigured();
        let error = Via927::open(&fake, "9049204", Input::One, &|| {
            crate::mcb927::firmware_image(None)
        })
        .err()
        .expect("must refuse");
        assert!(error.contains("--firmware PATH"), "{error}");
        assert!(error.contains("MANTARAY_M927_RBF"), "{error}");
        // Asking is all that happened: nothing was sent that changes it.
        assert_eq!(*fake.slow_ops.borrow(), [0x06]);
    }

    #[test]
    fn a_load_the_box_refuses_fails_the_open() {
        let fake = Fake927::unconfigured();
        let error = Via927::open(&fake, "9049204", Input::One, &|| Ok(Vec::new()))
            .err()
            .expect("must refuse");
        assert!(error.contains("empty"), "{error}");
    }
}
