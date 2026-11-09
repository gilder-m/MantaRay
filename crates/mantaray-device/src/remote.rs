//! A real instrument at the far end of a [`Transport`].
//!
//! # The wire dialect
//!
//! One ASCII command out, one line back. This is mantaray's own dialect, not a
//! claim about any manufacturer's: a real ORTEC MCB answers `SHOW_LIVE` with
//! `$G0007663632108` and rejects `SET_PRESET_LIVE` outright. Hardware is
//! reached through `mantaray-mcb`, which translates. What is written here is the
//! contract between MantaRay and whatever is at the far end of a
//! [`Transport`]:
//!
//! | Command | Response |
//! |---|---|
//! | `SHOW_CONFIGURATION` | `MODEL=.. SERIAL=.. FIRMWARE=.. CHANNELS=n [CAL=a,b,c]` |
//! | `SHOW_STATUS` | `RT=.. LT=.. DT=..% ICR=.. ACTIVE=0/1 TOTAL=n` |
//! | `SHOW_DATA` | `DATA n c0 c1 ... c(n-1)` |
//! | `SHOW_PRESETS` | `PRESETS REAL=s LIVE=s PEAK=n INTEG=n` (0 means none set) |
//! | `START` `STOP` `CLEAR` | `OK` |
//! | `SET_PRESET_REAL s` etc. | `OK` |
//! | anything rejected | `ERR reason` |
//!
//! `SHOW_PRESETS` is asked once, on connecting, because an instrument's preset
//! registers outlive the session that wrote them - and an instrument holding a
//! satisfied preset accepts `START` and then does not count. An older bridge
//! that refuses the question simply reports no presets.
//!
//! Everything here is exercised against a scripted transport and against a
//! served [`SimulatedMcb`](crate::SimulatedMcb) in the tests. Real hardware
//! arrives through [`BridgeTransport`](crate::BridgeTransport) speaking the
//! same lines, so this file needs no knowledge of it.

use mantaray_core::{AcquisitionMode, Spectrum};

use crate::courier::{Courier, Fetched};
use crate::error::DeviceError;
use crate::mcb::{
    Admitted, AmplifierSettings, HighVoltageSettings, Mcb, McbIdentity, McbProperties, McbStatus,
    differs, plain_zero,
};
use crate::presets::Presets;
use crate::transport::Transport;

/// How often the remote is asked for fresh numbers, in seconds of poll time.
/// Commands (start, stop, presets) always go straight through.
const POLL_INTERVAL: f64 = 0.5;

/// The road to the instrument.
///
/// Held directly, every exchange happens on the calling thread - which is
/// what a script and a test want: ask, get, assert, in one line of control.
/// Carried by a [`Courier`], the regular fetch of status and data happens on
/// the courier's thread instead, and `poll` only collects what has already
/// come back - which is what an application with frames to draw wants,
/// because a slow instrument then slows nothing but its own numbers.
enum Line {
    Direct(Box<dyn Transport>),
    Carried(Courier),
}

/// An instrument reached over a transport, presented as an [`Mcb`].
pub struct RemoteMcb {
    line: Line,
    identity: McbIdentity,
    properties: McbProperties,
    spectrum: Spectrum,
    status: McbStatus,
    mode: AcquisitionMode,
    locked_by: Option<(String, String)>,
    since_poll: f64,
    /// Whether the latest poll (or command) moved the mirror - what
    /// [`Mcb::poll_freshened`] answers, so preset walks run against new
    /// numbers rather than sixty times a second against the same ones.
    freshened: bool,
    /// Where a fetch's channel counts are parsed before they reach the
    /// mirror - see [`Self::integrate`] for why they do not go straight in.
    scratch: Vec<u64>,
    /// A channel count one fetch declared that the mirror has not believed
    /// yet - see [`Self::believes`].
    pending_length: Option<usize>,
    /// Whether the last fetch was a held one (a new length not yet
    /// believed), so a run of them is set aside once, never forever.
    held_last: bool,
    /// The pause between the status reads that confirm a STOP
    /// ([`CONFIRM_STEP`] unless a test or a slow adapter sets another).
    confirm_step: std::time::Duration,
    /// Why the last status reply was not believed, while it is not: the
    /// display keeps the last good values meanwhile.
    untrusted: Option<String>,
    /// Status readings the polls have worked in so far ([`Mcb::readings`]).
    readings: u64,
    /// An accepted START that no reading has yet shown counting.
    start_unseen: Option<StartUnseen>,
    /// Whether the spectrum's start date is the provisional one set when
    /// START was sent on a fresh count, to be replaced by the first reading
    /// that shows it counting ([`Self::date_start`]).
    provisional_start: bool,
    /// Whether a STOP's confirm reads are under way (they date nothing).
    confirming: bool,
    /// The confirm read that found the detector stopped: its status reply
    /// and when it arrived, for the final fetch ([`Self::final_fetch`]).
    confirm_reply: Option<(String, chrono::NaiveDateTime, std::time::Instant)>,
    /// When the latest fetch's status reply arrived ([`Mcb::reading_at`]).
    reading_at: Option<std::time::Instant>,
    /// When the START behind a provisional date was accepted: no date of
    /// that count is earlier.
    start_sent: Option<chrono::NaiveDateTime>,
}

/// An accepted START waiting for a reading to show the count begin.
///
/// The detector may take a while to raise ACTIVE after it accepts START
/// (nobody has measured the 926's lag). Until a reading shows the count
/// begin, it is treated as counting for the preset rule, so no preset write
/// (and the CLEAR it carries) can land on a count that is about to begin. A
/// START the detector never acts on stops being waited for after
/// [`crate::NEVER_STARTED_READINGS`] believed fetches, the bound a job's WAIT
/// and the batch use to call it never started.
#[derive(Clone, Copy, Debug)]
struct StartUnseen {
    /// The real time of the latest reading (the gate's fresh status read
    /// just before START, to begin with).
    real_time: f64,
    /// Whether a reading since START has shown the clocks run on. A CLEAR
    /// keeps it, as the count watch does: a count that ran is over once its
    /// clocks stand still, cleared or not.
    ran: bool,
    /// Whether a reading has shown the clocks run on since START or since
    /// they were last reset: clocks that run on again after it date the
    /// count.
    ran_since_reset: bool,
    /// The real time of the latest fetch, and when a fetch first read that
    /// value: once the clocks have run, a fetch far enough after that on the
    /// same value says the count is over.
    last_fetch: Option<(f64, std::time::Instant)>,
    /// Believed fetches since then that said idle with the clocks unmoved.
    idle_fetches: u64,
}

/// The most status reads that confirm a STOP before it is called failed.
///
/// The bound is in reads, not in wall time: a budget of time that includes
/// the round trips gave a slow adapter two reads where a fast one got ten,
/// so a detector dropping ACTIVE in 300 ms read as "did not stop" on exactly
/// the hardware that is slow (review of 21df1c3, #4). Ten reads a step apart
/// is the spec's second of confirming plus whatever the wire costs; every
/// read has the transport's own timeout, so it is bounded either way.
const CONFIRM_READS: usize = 10;

/// The default pause between the status reads that confirm a STOP
/// (device-spec 2.5: every 100 ms, for up to a second).
const CONFIRM_STEP: std::time::Duration = std::time::Duration::from_millis(100);

/// How many times the spectrum is asked for after a confirmed STOP before
/// the STOP is called failed ([`RemoteMcb::final_fetch`]).
const FINAL_FETCH_TRIES: usize = 3;

/// One `SHOW_STATUS` reading. Only a reply that carries the counting flag
/// and both clocks is one: the 926's adapter has been seen answering with
/// another command's reply, and a line without the flag says nothing about
/// whether the detector counts.
#[derive(Clone, Copy, Debug, PartialEq)]
struct Status {
    active: bool,
    real_time: f64,
    live_time: f64,
}

/// The reading in a status reply, or `None` when the reply is not a status
/// line (`ACTIVE=`, `RT=` and `LT=` all present). The one parse for the poll
/// and for the gate's question, so they cannot disagree.
fn parse_status(reply: &str) -> Option<Status> {
    Some(Status {
        active: status_number(reply, "ACTIVE")? != 0.0,
        real_time: status_number(reply, "RT")?,
        live_time: status_number(reply, "LT")?,
    })
}

impl RemoteMcb {
    /// Connects: asks the instrument what it is, and sizes the local mirror.
    pub fn connect(
        transport: Box<dyn Transport>,
        number: u16,
        name: &str,
    ) -> Result<Self, DeviceError> {
        Self::connect_expecting(transport, number, name, None)
    }

    /// The same, refusing an instrument that is not the one expected.
    ///
    /// A saved detector says how to reach an instrument - a detector number,
    /// or a position on the USB bus - and neither is an identity. Add a second
    /// adapter, or replug the one there, and the same route can lead to a
    /// different instrument; two of the same model answer identically apart
    /// from their serial, so nothing downstream would notice. Given what the
    /// instrument called itself last time, this compares and refuses rather
    /// than quietly opening the wrong detector and labelling its spectra with
    /// the wrong name.
    ///
    /// `None`, or an empty expectation, learns instead of checking - which is
    /// what the first open of a new entry does.
    pub fn connect_expecting(
        mut transport: Box<dyn Transport>,
        number: u16,
        name: &str,
        expected_serial: Option<&str>,
    ) -> Result<Self, DeviceError> {
        // Timed into the journal, because connecting is the slow thing a
        // journal of the connected life cannot see: everything here runs
        // before the mirror exists to write fetch lines.
        let started = std::time::Instant::now();
        let configuration = checked(transport.exchange("SHOW_CONFIGURATION")?)?;
        if crate::journal::on() {
            crate::journal::line(&format!(
                "connect d{number}: SHOW_CONFIGURATION answered in {:.2}s",
                started.elapsed().as_secs_f64()
            ));
        }
        let field = |key: &str| -> Option<String> {
            configuration.split_whitespace().find_map(|word| {
                word.strip_prefix(key)
                    .and_then(|rest| rest.strip_prefix('='))
                    .map(str::to_string)
            })
        };
        let channels: usize = field("CHANNELS")
            .and_then(|value| value.parse().ok())
            .unwrap_or(1024);
        let reported = field("SERIAL").unwrap_or_default();
        // Compared before anything else is built on it: an instrument that is
        // not the expected one must not become a detector at all.
        if let Some(expected) = expected_serial
            && !expected.trim().is_empty()
            && !reported.trim().is_empty()
            && !expected.trim().eq_ignore_ascii_case(reported.trim())
        {
            // Refusals go in the journal by their reason: a connect that
            // silently fails and is clicked again reads, in a journal without
            // this line, as nine unexplained seconds between two handshakes.
            if crate::journal::on() {
                crate::journal::line(&format!(
                    "connect d{number}: refused - reports {reported:?}, expected {expected:?}"
                ));
            }
            return Err(DeviceError::Connection {
                address: transport.peer(),
                detail: format!(
                    "this is instrument {reported:?}, not {expected:?} - the adapters may have \
                     been swapped or replugged. Rescan to pick it up under its own entry."
                ),
            });
        }
        let identity = McbIdentity {
            number,
            name: name.to_string(),
            model: field("MODEL").unwrap_or_else(|| "remote MCB".into()),
            serial: reported,
            firmware: field("FIRMWARE").unwrap_or_default(),
            description: format!("network instrument at {}", transport.peer()),
            channels,
            capabilities: Default::default(),
        };
        // An instrument that keeps its calibration host-side can hand it over
        // here; ORTEC's do, through the bridge. Without it the spectrum is
        // honestly uncalibrated rather than wrongly scaled.
        let calibration = field("CAL").and_then(|value| {
            let numbers: Vec<f64> = value
                .split(',')
                .filter_map(|number| number.parse().ok())
                .collect();
            match numbers[..] {
                [a, b, c] => Some(mantaray_core::EnergyCalibration::new([a, b, c], "keV")),
                [a, b] => Some(mantaray_core::EnergyCalibration::linear(a, b)),
                _ => None,
            }
        });
        let mut spectrum = Spectrum::new(channels);
        spectrum.energy_calibration = calibration;
        // The instrument's place in the pick list travels with its spectrum,
        // so a file saved from this window says which detector it came from
        // rather than claiming to be a buffer.
        spectrum.detector_id = number;
        spectrum.detector_name = name.to_string();
        spectrum.detector_description = name.to_string();
        // The upper discriminator starts at the last channel this instrument
        // has. Left at the default's u32::MAX, the ADC tab clamped it the
        // moment it was drawn, and that clamp counted as an edit.
        let mut properties = McbProperties::default();
        properties.adc.upper_discriminator = u32::try_from(channels)
            .unwrap_or(u32::MAX)
            .saturating_sub(1);
        let mut remote = Self {
            line: Line::Direct(transport),
            identity,
            properties,
            spectrum,
            status: McbStatus::default(),
            mode: AcquisitionMode::Pha,
            locked_by: None,
            since_poll: f64::MAX,
            freshened: true,
            scratch: Vec::new(),
            pending_length: None,
            held_last: false,
            confirm_step: CONFIRM_STEP,
            untrusted: None,
            readings: 0,
            start_unseen: None,
            provisional_start: false,
            confirming: false,
            confirm_reply: None,
            reading_at: None,
            start_sent: None,
        };
        remote.refresh()?;
        // A first status that is not a status line leaves nothing to show
        // but the mirror's zeros, and those are not a reading: an adapter
        // that cannot give one does not become a detector (review of
        // 21df1c3, #7). Later odd lines keep the last good reading instead.
        if let Some(reason) = remote.untrusted.take() {
            return Err(DeviceError::Connection {
                address: remote.identity.description.clone(),
                detail: format!("the instrument's status cannot be read: {reason}"),
            });
        }
        remote.read_presets();
        if crate::journal::on() {
            crate::journal::line(&format!(
                "connect d{number}: {} serial={:?} {} channels, ready in {:.2}s",
                remote.identity.model,
                remote.identity.serial,
                remote.identity.channels,
                started.elapsed().as_secs_f64()
            ));
        }
        Ok(remote)
    }

    /// Puts the line in a courier's hands, for callers with frames to draw.
    ///
    /// From here `poll` never touches the wire: it collects whatever fetch
    /// the courier has finished and asks for the next one, both the work of
    /// microseconds however slow the instrument is. On the bench that
    /// prompted this, the fetch took 325 ms and ran on the interface's own
    /// thread twice a second - a frozen frame per fetch, read by an operator
    /// as a broken program. Commands still wait for their answers.
    ///
    /// The command line and the tests stay on the direct line instead: a
    /// script has no frames to hold, and a `WAIT` that reads the clocks right
    /// after polling wants them already fresh.
    pub fn with_courier(mut self) -> Self {
        self.line = match self.line {
            Line::Direct(transport) => Line::Carried(Courier::carry(transport)),
            carried => carried,
        };
        self
    }

    /// Asks the instrument what presets it is already holding.
    ///
    /// An instrument's preset registers outlive the session that wrote them.
    /// Until this was asked, mantaray only ever wrote presets, so a restarted
    /// application showed an empty Presets tab while the instrument held, say,
    /// a three-hundred-second live preset - and then answered START without
    /// counting, because that preset was already satisfied. Seen on the bench
    /// 926 on 2026-08-06, and silent from the operator's side.
    ///
    /// Deliberately not an error: a bridge or instrument that does not know
    /// the question leaves the presets empty, exactly as every version before
    /// this one did, so an older helper still connects.
    fn read_presets(&mut self) {
        let Ok(reply) = self.command("SHOW_PRESETS") else {
            return;
        };
        let held = parse_presets(&reply).unwrap_or_default();
        self.properties.presets.real_time = held.real_time;
        self.properties.presets.live_time = held.live_time;
        self.properties.presets.roi_peak = held.roi_peak;
        self.properties.presets.roi_integral = held.roi_integral;
    }

    /// Sets the pause between the status reads that confirm a STOP (100 ms
    /// by default; `CONFIRM_READS` reads at most). Tests set zero.
    pub fn set_confirm_step(&mut self, step: std::time::Duration) {
        self.confirm_step = step;
    }

    /// One command out, one line back, however the line is held. Over a
    /// courier, a fetch it had already finished is thrown away: it describes
    /// the instrument from before this command.
    fn raw(&mut self, command: &str) -> Result<String, DeviceError> {
        let answer = match &mut self.line {
            Line::Direct(transport) => transport.exchange(command),
            Line::Carried(courier) => courier.exchange(command),
        };
        if crate::journal::on() {
            // Digested: a SHOW_DATA answer is a whole spectrum, and the
            // journal wants its shape, not its channels - integrate writes
            // those as a sum. Each line names its detector: the first bench
            // journal interleaved three instruments' fetches with no way to
            // tell whose was whose, and attribution was the reading's whole
            // point.
            let number = self.identity.number;
            match &answer {
                Ok(reply) => {
                    let shown: String = reply.chars().take(64).collect();
                    let more = if reply.len() > shown.len() { "…" } else { "" };
                    crate::journal::line(&format!("d{number}: {command} -> {shown}{more}"));
                }
                Err(error) => {
                    crate::journal::line(&format!("d{number}: {command} -> ERROR {error}"));
                }
            }
        }
        answer
    }

    /// One command out, one checked line back.
    fn command(&mut self, command: &str) -> Result<String, DeviceError> {
        checked(self.raw(command)?)
    }

    /// Sends `steps` in order. The mirror follows each step the moment the
    /// instrument accepted it, so after a failure it still says what the
    /// instrument was told. `applied` is what already took effect without a
    /// command (settings kept only in this program).
    ///
    /// On a failure the error names what took effect and every step not sent;
    /// with nothing applied and nothing sent it is the instrument's own error.
    fn run_steps(&mut self, steps: Vec<Step>, mut applied: Vec<String>) -> Result<(), DeviceError> {
        let mut steps = steps.into_iter();
        while let Some(step) = steps.next() {
            if let Err(error) = self.command(&step.command) {
                let not_sent: Vec<String> = steps.map(|s| s.name.to_string()).collect();
                return Err(partly_applied(applied, step.name, &not_sent, error));
            }
            step.mirror.apply(&mut self.properties);
            applied.push(step.done);
        }
        Ok(())
    }

    /// Reads the status until it says the detector has stopped: a STOP
    /// finishes only when the detector agrees. At most `CONFIRM_READS`
    /// reads, [`Self::confirm_step`] apart: a bound inside one command, never
    /// a wait across frames, and the same number of reads however slow the
    /// wire. A read that cannot tell (a failed status read, or the bridge's
    /// ERR for a flag it could not read) uses up one of those reads and the
    /// next one is tried: one hiccup on a stopped detector no longer fails
    /// the STOP of a job's START / WAIT / STOP / SAVE (release review of
    /// 4597899). Only when no read could tell is the STOP called sent, but
    /// not confirmed, with the last read's reason.
    fn confirm_stopped(&mut self) -> Result<bool, DeviceError> {
        self.confirming = true;
        self.confirm_reply = None;
        let confirmed = self.confirm_reads();
        self.confirming = false;
        confirmed
    }

    /// The reads behind [`Self::confirm_stopped`].
    fn confirm_reads(&mut self) -> Result<bool, DeviceError> {
        let mut told = false;
        let mut unknown = None;
        for read in 0..CONFIRM_READS {
            if read > 0 && !self.confirm_step.is_zero() {
                std::thread::sleep(self.confirm_step);
            }
            match self.read_counting() {
                Ok(false) => return Ok(true),
                Ok(true) => told = true,
                Err(DeviceError::CountingUnknown { reason }) => unknown = Some(reason),
                Err(error) => return Err(error),
            }
        }
        // A read that told said it still counts: it did not stop.
        match unknown {
            Some(reason) if !told => Err(DeviceError::Command {
                command: "STOP".into(),
                detail: format!("sent, but could not confirm: {reason}"),
            }),
            _ => Ok(false),
        }
    }

    /// Pulls fresh status and data from the instrument, on this thread.
    ///
    /// The direct line's whole poll; over a courier this runs only at
    /// connection time, before the courier exists, and [`Self::integrate`]
    /// alone runs afterwards.
    fn refresh(&mut self) -> Result<(), DeviceError> {
        self.since_poll = 0.0;
        let status = self.raw("SHOW_STATUS")?;
        let read_instant = std::time::Instant::now();
        let read_at = chrono::Local::now().naive_local();
        let data = self.raw("SHOW_DATA")?;
        self.integrate(Fetched {
            status,
            data,
            read_at,
            read_instant,
        })
        .map(|_| ())
    }

    /// Reads the spectrum the detector stopped at, after a confirmed STOP:
    /// one SHOW_DATA beside the confirming status read (taken after the
    /// stop, so its clocks are the final ones), tried up to
    /// [`FINAL_FETCH_TRIES`] times.
    ///
    /// The confirm read only the status, so the mirror kept the channels and
    /// clocks of the last regular fetch, and a SAVE straight after STOP - a
    /// job's STOP / SAVE, MAESTRO's own pattern - wrote that: 1.54 s / 154
    /// counts of a detector stopped at 2.00 s / 200 (deep review F2). A
    /// spectrum that cannot be read fails the STOP, saying the detector did
    /// stop, and leaves the mirror marked untrusted, never quietly short.
    fn final_fetch(&mut self) -> Result<(), DeviceError> {
        let Some((status, read_at, read_instant)) = self.confirm_reply.take() else {
            return Ok(());
        };
        let mut reason = None;
        for _ in 0..FINAL_FETCH_TRIES {
            let landed = self.raw("SHOW_DATA").and_then(|data| {
                self.integrate(Fetched {
                    status: status.clone(),
                    data,
                    read_at,
                    read_instant,
                })
            });
            match landed {
                Ok(true) => return Ok(()),
                Ok(false) => reason.clone_from(&self.untrusted),
                Err(error) => reason = Some(error.to_string()),
            }
        }
        let reason = reason.unwrap_or_else(|| "its channel count did not hold still".into());
        self.untrusted = Some(format!("the final spectrum could not be read: {reason}"));
        Err(DeviceError::Command {
            command: "STOP".into(),
            detail: format!(
                "the detector stopped, but its final spectrum could not be read \
                 ({FINAL_FETCH_TRIES} tries): {reason}; the spectrum shown is from \
                 before the stop until a reading lands"
            ),
        })
    }

    /// Works a believed reading, taken at `read_at` (`read_instant` on the
    /// monotonic clock, which spaces readings), into the start date and an
    /// accepted START still waiting to be seen ([`StartUnseen`]). Only a
    /// fetch (`fetch`) counts towards giving up on it, never the gate's own
    /// status reads.
    fn see_start(
        &mut self,
        reading: Status,
        read_at: chrono::NaiveDateTime,
        read_instant: std::time::Instant,
        fetch: bool,
    ) {
        // A STOP's confirm reads date nothing: a flag that trails the stop
        // beside clocks frozen at it would date the count late.
        if reading.active && !self.confirming {
            self.date_start(reading.real_time, read_at);
        }
        let Some(unseen) = self.start_unseen.as_mut() else {
            return;
        };
        if reading.active {
            self.start_unseen = None;
            return;
        }
        // Clocks that went back were cleared (by whoever): the readings
        // after decide afresh, as the count watch does. That they had run
        // is kept.
        if reading.real_time < unseen.real_time {
            unseen.real_time = reading.real_time;
            unseen.last_fetch = None;
            unseen.ran_since_reset = false;
        }
        // How long the clocks have read this value, on fetches: a fetch on a
        // new value starts the span afresh.
        let mut stood_still = false;
        if fetch {
            match unseen.last_fetch {
                Some((value, since)) if value == reading.real_time => {
                    stood_still =
                        read_instant.saturating_duration_since(since) >= crate::STOOD_STILL_SPACING;
                }
                _ => unseen.last_fetch = Some((reading.real_time, read_instant)),
            }
        }
        if reading.real_time > unseen.real_time {
            // Idle, with the clocks run on: it has begun, and the flag may
            // just trail the clocks (the bridge also reads the flag before
            // the clocks), so this decides nothing and the hold stays. Clocks
            // that run on again after an earlier such reading say it counts
            // now, and date it (still provisionally: a reading that shows it
            // counting dates it better).
            let again = unseen.ran_since_reset;
            unseen.real_time = reading.real_time;
            unseen.ran = true;
            unseen.ran_since_reset = true;
            if again {
                self.date_provisionally(reading.real_time, read_at);
            }
        } else if unseen.ran {
            // Fetches far enough apart on the same clocks, after they ran:
            // it counted and is over between readings. When it began cannot
            // be read back, so the date stays as it is, for good (a resume's
            // clock would include this run). A fetch the gate forced out
            // right behind another decides nothing.
            if stood_still {
                self.start_unseen = None;
                self.provisional_start = false;
            }
        } else if fetch {
            unseen.idle_fetches += 1;
            if unseen.idle_fetches >= crate::NEVER_STARTED_READINGS {
                self.start_unseen = None;
            }
        }
    }

    /// Dates the count from a reading that shows it counting, `real_time`
    /// into it at `read_at`, when its date is missing or only provisional.
    /// The real-time clock runs only while it counts, so the count began
    /// that long before the reading: the real start, however late after
    /// START the detector raised its flag. A date this process saw happen
    /// otherwise is never overwritten.
    fn date_start(&mut self, real_time: f64, read_at: chrono::NaiveDateTime) {
        self.date_from(real_time, read_at, false);
    }

    /// As [`Self::date_start`], from an idle reading whose clocks ran on
    /// twice: the date stays provisional, for a reading that shows the
    /// count running to replace.
    fn date_provisionally(&mut self, real_time: f64, read_at: chrono::NaiveDateTime) {
        self.date_from(real_time, read_at, true);
    }

    fn date_from(&mut self, real_time: f64, read_at: chrono::NaiveDateTime, provisional: bool) {
        if self.spectrum.start_time.is_some() && !self.provisional_start {
            return;
        }
        let counted = chrono::Duration::milliseconds((real_time.max(0.0) * 1000.0) as i64);
        if let Some(start) = read_at.checked_sub_signed(counted) {
            // A count this process started began no earlier than its START.
            let start = match self.start_sent {
                Some(sent) if self.provisional_start => start.max(sent),
                _ => start,
            };
            if crate::journal::on() {
                crate::journal::line(&format!(
                    "d{}: mirror: start dated {real_time:.2}s back",
                    self.identity.number
                ));
            }
            self.spectrum.start_time = Some(start);
            self.provisional_start = provisional;
        }
    }

    /// Whether a fetch's channel count is believed enough to rebuild for.
    ///
    /// Rebuilding the mirror for a new length starts it from zeros, so a
    /// count that flaps clears the operator's spectrum from the screen with
    /// every flip. And flap it does: on the Windows road, ORTEC's library is
    /// asked the detector's length on every read and truncates the data to
    /// however many channels it felt like returning, and a busy instrument
    /// mid-acquisition answered short often enough that the window flashed
    /// empty and full, over and over - read from the bench as the program
    /// clearing the count. The libusb road never showed it, because there
    /// the size is asked once and every read is a whole frame.
    ///
    /// So belief is earned, at the only price a real gain change can always
    /// pay: an *empty* mirror adopts a new length at once, because rebuilding
    /// zeros as zeros loses nothing - which is what lets connection and the
    /// fetch after CLEAR size themselves immediately. A mirror holding counts
    /// adopts a new length only when two fetches in a row agree on it; a
    /// one-fetch flap keeps the spectrum on screen and costs that one fetch
    /// (see [`Self::integrate`]).
    fn believes(&mut self, count: usize) -> bool {
        self.spectrum.total_counts() == 0 || self.pending_length.replace(count) == Some(count)
    }

    /// Works a fetch's lines into the mirror.
    ///
    /// Parsing lives here rather than with the fetch so that a courier can
    /// stay a courier: it moves bytes, and what the bytes mean - including
    /// an `ERR` where an answer should be - is decided on the caller's side.
    ///
    /// `Ok(true)` when the fetch landed whole: its channels with its clocks,
    /// on a believed status.
    fn integrate(&mut self, fetched: Fetched) -> Result<bool, DeviceError> {
        // The spectrum first, though its errors are said after the status
        // is worked in. A fetch whose channels the mirror holds back (a new
        // length, not yet believed) is set aside whole: status, clocks and
        // all. Taking its clocks without its channels left the spectrum's
        // counts half a second older than the live time beside them, and its
        // idle status ended a WAIT on exactly that spectrum, which a job then
        // saved (release review S2). The next fetch decides. Only the first
        // of a run of them, though: lengths that never agree twice must not
        // stop the readings. The rest count as readings that are not
        // believed (below), status and all: worked in, the idle status of a
        // second held fetch in a row ended a WAIT on the channels and clocks
        // of the fetch before, which a job saved as the finished count (deep
        // review N1). Lengths that never settle end a WAIT as unreadable
        // after UNREADABLE_READINGS fetches, never as over.
        let parsed = self.parse_data(&fetched.data);
        let held = matches!(parsed, Ok(count)
            if count > 0 && self.spectrum.len() != count && !self.believes(count));
        if held && !self.held_last {
            self.held_last = true;
            if crate::journal::on() {
                crate::journal::line(&format!(
                    "d{}: fetch: count={} | mirror len={} sum={} | set aside: one fetch of a \
                     new length",
                    self.identity.number,
                    self.scratch.len(),
                    self.spectrum.len(),
                    self.spectrum.total_counts(),
                ));
            }
            return Ok(false);
        }
        self.held_last = held;

        // A status reply that is not a status line with the flag and both
        // clocks (an ERR, another command's reply, a slipped line) is not
        // believed: nothing is invented, the display keeps the last good
        // values, the reading is marked untrusted, and the spectrum fetch
        // goes on. Changes stay refused, because each asks the detector
        // afresh (rule 4).
        //
        // A fetch whose spectrum could not be read (SHOW_DATA answered ERR:
        // on the 926 the 32 KB histogram read is the fragile one, the status
        // reads are small) is set aside the same way, status and all. Its
        // idle status was worked in beside the spectrum of the fetch before,
        // which ended a job's WAIT, and SAVE wrote 2 s of a 3 s count as the
        // finished one (deep review F1). A count is over only on a fetch
        // whose channels landed with its clocks; marked untrusted, a WAIT
        // goes on over the failed reads (each poll tries again) and gives up
        // after UNREADABLE_READINGS of them, saying why.
        let status = fetched.status;
        self.readings += 1;
        self.reading_at = Some(fetched.read_instant);
        let reading = match &parsed {
            Err(error) => Err(format!("the spectrum could not be read: {error}")),
            Ok(count) if held => Err(format!(
                "the spectrum's channel count does not hold still (this read {count}, \
                 the spectrum {})",
                self.spectrum.len()
            )),
            Ok(_) => parse_status(&status)
                .ok_or_else(|| format!("the status reply is not a status line: {status:.60?}")),
        };
        match reading {
            Ok(reading) => {
                let number = |key: &str| status_number(&status, key);
                self.status.real_time = reading.real_time;
                self.status.live_time = reading.live_time;
                self.status.active = reading.active;
                self.status.dead_time_percent = number("DT").unwrap_or(0.0);
                self.status.input_count_rate = number("ICR").unwrap_or(0.0);
                self.status.total_counts = number("TOTAL").unwrap_or(0.0) as u64;
                self.untrusted = None;
                self.see_start(reading, fetched.read_at, fetched.read_instant, true);
            }
            Err(reason) => {
                self.untrusted = Some(reason);
                // An unbelievable fetch is still a fetch towards the bound
                // on an unseen START: the hold never outlasts the wait a
                // WAIT gives up on (UNREADABLE_READINGS, the same count).
                if let Some(unseen) = self.start_unseen.as_mut() {
                    unseen.idle_fetches += 1;
                    if unseen.idle_fetches >= crate::NEVER_STARTED_READINGS {
                        self.start_unseen = None;
                    }
                }
            }
        }

        let count = parsed?;
        // What the mirror did with this fetch, for the journal: the bench
        // that misbehaves is the only machine that can say what its fetches
        // actually carried, and this line is how it says it.
        let mut verdict = "no channels";
        if count > 0 {
            verdict = "landed";
            if held {
                verdict = "held: another fetch of a new length";
            } else if self.spectrum.len() != count {
                // The instrument's conversion gain changed underneath us.
                // The counts are new, but whose spectrum this is -
                // detector, calibration, start date - is not.
                let mut resized = Spectrum::new(count);
                resized.copy_descriptors_from(&self.spectrum);
                self.spectrum = resized;
                verdict = "adopted a new length";
            }
            // Only a fetch the mirror's length agrees with lands; a held
            // fetch's channels are dropped.
            if self.spectrum.len() == count {
                self.spectrum.channels.copy_from_slice(&self.scratch);
                self.pending_length = None;
            }
        }
        if crate::journal::on() {
            let sum: u64 = self.scratch.iter().sum();
            crate::journal::line(&format!(
                "d{}: fetch: count={count} sum={sum} rt={:.2} lt={:.2} active={} total={} | \
                 mirror len={} sum={} | {verdict}",
                self.identity.number,
                self.status.real_time,
                self.status.live_time,
                self.status.active,
                self.status.total_counts,
                self.spectrum.len(),
                self.spectrum.total_counts(),
            ));
        }
        // The spectrum's clocks go with its channels: a held fetch's clocks
        // are not the ones its counts were read at.
        if !held {
            self.spectrum.real_time = self.status.real_time;
            self.spectrum.live_time = self.status.live_time;
        }
        // A count already running when this connected has no start to read.
        // The instrument keeps no measurement date of its own, and the one
        // call that would give it - `MIOGetStartTime` - exists only in ORTEC's
        // Windows library, so nothing on this road can be asked. Closing the
        // window and opening it again therefore lost the date outright, and
        // every file written afterwards carried none.
        //
        // The real-time clock is what survives, and it advances only while the
        // run does, so the run began that many seconds ago. Two limits, both
        // deliberate. Only while it is still counting: an idle instrument
        // holding this morning's spectrum would otherwise be dated to this
        // minute, which is a worse answer than none. And only into a gap - a
        // start this process watched happen is the real one and is never
        // overwritten. A run stopped and resumed reads late by however long it
        // stood paused, which is the residual error and is the reason this is
        // a reconstruction rather than a reading.
        //
        // The same rule dates a count this process started: START gives a
        // provisional date, and the first reading that shows it counting
        // replaces it, so a detector slow to begin is dated when it began.
        // Both are [`Self::date_start`], called on the believed status above.
        self.freshened = true;
        Ok(!held && self.untrusted.is_none())
    }

    /// Parses a `DATA <n> <count>...` reply into the scratch; the channel
    /// count it declares.
    fn parse_data(&mut self, data: &str) -> Result<usize, DeviceError> {
        let data = checked(data.to_string())?;
        let mut words = data.split_whitespace();
        if words.next() != Some("DATA") {
            return Err(DeviceError::Communication {
                detail: format!("expected DATA, got {data:.40?}"),
            });
        }
        // The count word arrives over a wire and is not to be trusted with an
        // allocation: the largest real MCB memory is 16k channels, so anything
        // past this is a corrupt line, not an instrument. One garbled reply
        // must be an error the caller can show, never an allocation abort and
        // never counts quietly read as zero.
        const MOST_CHANNELS: usize = 65536;
        let count: usize = words
            .next()
            .and_then(|word| word.parse().ok())
            .filter(|count| *count <= MOST_CHANNELS)
            .ok_or_else(|| DeviceError::Communication {
                detail: format!("DATA does not declare a believable channel count: {data:.60?}"),
            })?;
        // Parsed into the scratch first, not straight into the mirror: the
        // parse must be all-or-nothing, because a garbled word halfway along
        // must leave the spectrum as it was, not half old and half new. The
        // scratch is a field so that two fetches a second do not allocate a
        // sixteen-thousand-channel buffer each, forever.
        self.scratch.clear();
        for word in words.take(count) {
            self.scratch.push(
                word.parse::<u64>()
                    .map_err(|_| DeviceError::Communication {
                        detail: format!("a channel in DATA is not a count: {word:.40?}"),
                    })?,
            );
        }
        if self.scratch.len() != count {
            return Err(DeviceError::Communication {
                detail: format!(
                    "DATA declares {count} channels but carries {}",
                    self.scratch.len()
                ),
            });
        }
        Ok(count)
    }
}

/// The four held presets in a `SHOW_PRESETS` reply; `None` when the reply
/// names none of them (a bridge that answers something else is not telling us
/// what the instrument holds).
fn parse_presets(reply: &str) -> Option<Presets> {
    let field = |key: &str| -> Option<f64> {
        reply.split_whitespace().find_map(|word| {
            word.strip_prefix(key)
                .and_then(|rest| rest.strip_prefix('='))
                .and_then(|value| value.parse::<f64>().ok())
        })
    };
    let (real, live, peak, integral) =
        (field("REAL"), field("LIVE"), field("PEAK"), field("INTEG"));
    if real.is_none() && live.is_none() && peak.is_none() && integral.is_none() {
        return None;
    }
    // Zero is how an instrument says "none set", so it is not a preset of
    // zero - which would mean an acquisition that stops immediately.
    let seconds = |value: Option<f64>| value.filter(|value| value.is_finite() && *value > 0.0);
    let counts = |value: Option<f64>| {
        value
            .filter(|value| value.is_finite() && *value >= 1.0)
            .map(|value| value as u64)
    };
    Some(Presets {
        real_time: seconds(real),
        live_time: seconds(live),
        roi_peak: counts(peak),
        roi_integral: counts(integral),
        ..Presets::default()
    })
}

/// One `KEY=value` figure of a `SHOW_STATUS` reply (a trailing `%` allowed).
fn status_number(status: &str, key: &str) -> Option<f64> {
    status.split_whitespace().find_map(|word| {
        word.strip_prefix(key)
            .and_then(|rest| rest.strip_prefix('='))
            .map(|value| value.trim_end_matches('%'))
            .and_then(|value| value.parse().ok())
    })
}

/// Turns an `ERR reason` line into the error it is.
fn checked(response: String) -> Result<String, DeviceError> {
    match response.strip_prefix("ERR") {
        Some(reason) => Err(DeviceError::Command {
            command: String::new(),
            detail: reason.trim().to_string(),
        }),
        None => Ok(response),
    }
}

/// One thing a change sends to the instrument: what it is called in an error,
/// the command, what to say once it has gone through, and how the mirror
/// follows it. Every change is a list of these, run in order by
/// [`RemoteMcb::run_steps`], so the account of what happened and what did not
/// comes from the one list and cannot drift from what was sent.
struct Step {
    name: &'static str,
    command: String,
    done: String,
    mirror: Mirrored,
}

/// How the mirror follows a [`Step`] once the instrument accepted it.
enum Mirrored {
    Bias(f64),
    Gain(f64),
    PresetsCleared,
    RealTime(f64),
    LiveTime(f64),
    RoiPeak(u64),
    RoiIntegral(u64),
}

impl Mirrored {
    fn apply(&self, properties: &mut McbProperties) {
        match *self {
            Self::Bias(volts) => properties.high_voltage.target_volts = volts,
            Self::Gain(gain) => properties.amplifier.coarse_gain = gain,
            Self::PresetsCleared => {
                let presets = &mut properties.presets;
                presets.real_time = None;
                presets.live_time = None;
                presets.roi_peak = None;
                presets.roi_integral = None;
            }
            Self::RealTime(seconds) => properties.presets.real_time = Some(seconds),
            Self::LiveTime(seconds) => properties.presets.live_time = Some(seconds),
            Self::RoiPeak(counts) => properties.presets.roi_peak = Some(counts),
            Self::RoiIntegral(counts) => properties.presets.roi_integral = Some(counts),
        }
    }
}

/// The steps that set the instrument's own presets to `wanted`: a clear and
/// then each preset wanted (the uncertainty and MDA presets are the host's
/// and never cause a command). The one place that decides what a preset
/// action sends, for `set_presets` and `set_properties` alike. It always
/// returns the full list: an explicit action reaches the instrument, and the
/// implicit caller (`set_properties`) decides whether anything changed.
fn preset_steps(wanted: &Presets) -> Vec<Step> {
    let mut steps = vec![Step {
        name: "preset clear",
        command: "SET_PRESET_CLEAR".into(),
        done: "the instrument's presets cleared".into(),
        mirror: Mirrored::PresetsCleared,
    }];
    if let Some(seconds) = wanted.real_time {
        steps.push(Step {
            name: "real-time preset",
            command: format!("SET_PRESET_REAL {seconds}"),
            done: format!("real-time preset set to {seconds} s"),
            mirror: Mirrored::RealTime(seconds),
        });
    }
    if let Some(seconds) = wanted.live_time {
        steps.push(Step {
            name: "live-time preset",
            command: format!("SET_PRESET_LIVE {seconds}"),
            done: format!("live-time preset set to {seconds} s"),
            mirror: Mirrored::LiveTime(seconds),
        });
    }
    if let Some(counts) = wanted.roi_peak {
        steps.push(Step {
            name: "ROI peak preset",
            command: format!("SET_PRESET_COUNT {counts}"),
            done: format!("ROI peak preset set to {counts}"),
            mirror: Mirrored::RoiPeak(counts),
        });
    }
    if let Some(counts) = wanted.roi_integral {
        steps.push(Step {
            name: "ROI integral preset",
            command: format!("SET_PRESET_INTEG {counts}"),
            done: format!("ROI integral preset set to {counts}"),
            mirror: Mirrored::RoiIntegral(counts),
        });
    }
    steps
}

/// The error for a change that stopped part way: what took effect before the
/// failure (`applied`, which includes settings kept only in this program),
/// the step that failed and why, and every step after it that was therefore
/// not sent. With nothing applied before it, the instrument's own error
/// stands.
fn partly_applied(
    applied: Vec<String>,
    failed_step: &str,
    not_sent: &[String],
    error: DeviceError,
) -> DeviceError {
    if applied.is_empty() {
        return error;
    }
    let failed = format!("{failed_step} not set: {error}");
    let failed = if not_sent.is_empty() {
        failed
    } else {
        format!("{failed}; not sent: {}", not_sent.join(", "))
    };
    DeviceError::PartlyApplied {
        applied: applied.join(", "),
        failed,
    }
}

impl Mcb for RemoteMcb {
    fn identity(&self) -> &McbIdentity {
        &self.identity
    }

    fn properties(&self) -> &McbProperties {
        &self.properties
    }

    fn do_set_properties(
        &mut self,
        properties: McbProperties,
        _admitted: Admitted,
    ) -> Result<(), DeviceError> {
        // The gate has checked the lock and `set_properties` that the presets
        // are unchanged: this is bias, gain and the settings kept here, and
        // it asks the instrument nothing.
        let mut properties = properties;
        properties.amplifier.coarse_gain = plain_zero(properties.amplifier.coarse_gain);
        properties.high_voltage.target_volts = plain_zero(properties.high_voltage.target_volts);
        // The commands carry whole numbers, so only whole numbers are taken:
        // the mirror then holds exactly what went out. Sent as `{:.0}` while
        // the mirror held the fraction, the dialog showed a gain, or a bias,
        // the instrument never got. Only what changed is checked, and before
        // anything is sent: a value the mirror already holds, strange as it
        // is, must not block an edit of something else.
        let bias_changed = differs(
            properties.high_voltage.target_volts,
            self.properties.high_voltage.target_volts,
        );
        let gain_changed = differs(
            properties.amplifier.coarse_gain,
            self.properties.amplifier.coarse_gain,
        );
        if bias_changed {
            HighVoltageSettings::check_target(properties.high_voltage.target_volts)?;
        }
        if gain_changed {
            AmplifierSettings::check_coarse_gain(properties.amplifier.coarse_gain)?;
        }
        // Only what changed goes on the wire. The mirror starts from defaults,
        // not from the instrument, so sending everything on every edit put a
        // 3000 V bias nobody chose on the detector whenever any other setting
        // moved - opening the ADC tab was enough, through its discriminator
        // clamp - and cleared the presets of a count in progress. The settings
        // with no command here live only in the mirror, as before.
        if properties.high_voltage.enabled != self.properties.high_voltage.enabled {
            // No on/off command has been proven against an instrument, so the
            // checkbox must not look as if it worked.
            return Err(DeviceError::NotSupported {
                feature: "switching the high voltage on or off from here - \
                          use the instrument's own control"
                    .into(),
            });
        }
        // What lives only in the mirror is committed first, and the fields the
        // instrument holds are mirrored one by one, each the moment its
        // command succeeds: the mirror says what the instrument was told, so
        // a failure part way leaves it true. The instrument is not rolled
        // back; the error says what it now holds.
        let wanted = properties;
        let mut steps = Vec::new();
        if bias_changed {
            let volts = wanted.high_voltage.target_volts;
            steps.push(Step {
                name: "bias",
                command: format!("SET_HV {volts:.0}"),
                done: format!("bias set to {volts:.0} V"),
                mirror: Mirrored::Bias(volts),
            });
        }
        if gain_changed {
            let gain = wanted.amplifier.coarse_gain;
            steps.push(Step {
                name: "coarse gain",
                command: format!("SET_GAIN_COARSE {gain:.0}"),
                done: format!("coarse gain set to {gain:.0}"),
                mirror: Mirrored::Gain(gain),
            });
        }
        // What has no command is committed now, and the error says so whenever
        // a step fails, so "nothing applied" is never read when it was.
        let mut kept = Vec::new();
        let current = &self.properties;
        let mut same_amplifier = wanted.amplifier;
        same_amplifier.coarse_gain = current.amplifier.coarse_gain;
        if same_amplifier != current.amplifier {
            kept.push("amplifier");
        }
        if wanted.adc != current.adc {
            kept.push("ADC");
        }
        let mut same_bias = wanted.high_voltage;
        same_bias.target_volts = current.high_voltage.target_volts;
        if same_bias != current.high_voltage {
            kept.push("high-voltage");
        }
        if wanted.stabilizer != current.stabilizer {
            kept.push("stabilizer");
        }
        let mut applied = Vec::new();
        if !kept.is_empty() {
            applied.push(format!(
                "{} settings kept in this program only (the instrument has no command for them)",
                kept.join(", ")
            ));
        }
        let mut next = wanted.clone();
        next.high_voltage.target_volts = current.high_voltage.target_volts;
        next.amplifier.coarse_gain = current.amplifier.coarse_gain;
        next.presets = current.presets;
        self.properties = next;

        self.run_steps(steps, applied)
    }

    fn do_set_high_voltage(&mut self, volts: f64, _admitted: Admitted) -> Result<(), DeviceError> {
        // Checked by `set_high_voltage`. Sent whatever the mirror says: its
        // 3000 V is a default, not a reading, so an operator applying 3000 V
        // must still reach the wire.
        self.command(&format!("SET_HV {volts:.0}"))?;
        self.properties.high_voltage.target_volts = volts;
        Ok(())
    }

    fn do_set_coarse_gain(&mut self, gain: f64, _admitted: Admitted) -> Result<(), DeviceError> {
        // Sent whatever the mirror says, like the bias: the mirror's 20 is a
        // default, not a reading, so applying 20 must still reach the wire.
        self.command(&format!("SET_GAIN_COARSE {gain:.0}"))?;
        self.properties.amplifier.coarse_gain = gain;
        Ok(())
    }

    fn set_name(&mut self, name: &str) {
        self.identity.name = name.to_string();
        // The spectrum carries the detector's name into every file saved from
        // this window, so a rename has to reach it too or the next save still
        // says what the scan called it.
        self.spectrum.detector_name = name.to_string();
        self.spectrum.detector_description = name.to_string();
    }

    fn read_counting(&mut self) -> Result<bool, DeviceError> {
        // One SHOW_STATUS, on the plain exchange (over a courier a finished
        // fetch is discarded, not worked in: it is older than this answer).
        // Only a status line is an answer; anything else - a failed read, an
        // ERR, another command's reply - refuses whatever needed it.
        let reply = self
            .raw("SHOW_STATUS")
            .map_err(|error| DeviceError::CountingUnknown {
                reason: format!("the status read failed: {error}"),
            })?;
        let Some(reading) = parse_status(&reply) else {
            let reason = format!("the status reply is not a status line: {reply:.60?}");
            self.untrusted = Some(reason.clone());
            return Err(DeviceError::CountingUnknown { reason });
        };
        self.untrusted = None;
        // The reading is what the mirror now says. It is not a fetch: no
        // spectrum moves and the poll is not fresh, but the next poll fetches
        // at once.
        self.status.active = reading.active;
        self.status.real_time = reading.real_time;
        self.status.live_time = reading.live_time;
        self.since_poll = POLL_INTERVAL;
        // The mirror's clocks are now this read's, so the time of the
        // reading they are judged by is this read's too.
        let read_instant = std::time::Instant::now();
        let read_at = chrono::Local::now().naive_local();
        self.reading_at = Some(read_instant);
        if self.confirming {
            self.confirm_reply = (!reading.active).then(|| (reply.clone(), read_at, read_instant));
        }
        self.see_start(reading, read_at, read_instant, false);
        // An accepted START no reading has shown begin yet counts as
        // counting: the detector may be about to raise its flag.
        Ok(reading.active || self.start_unseen.is_some())
    }

    fn do_write_presets(
        &mut self,
        presets: Presets,
        _admitted: Admitted,
    ) -> Result<(), DeviceError> {
        // An explicit action always reaches the instrument, like
        // `set_high_voltage` and `set_coarse_gain`: a clear and every held
        // preset are sent. The host-side presets never cause a command and
        // reach the mirror only once the instrument has accepted everything.
        self.run_steps(preset_steps(&presets), Vec::new())?;
        self.properties.presets.uncertainty = presets.uncertainty;
        self.properties.presets.mda = presets.mda;
        Ok(())
    }

    fn do_store_host_presets(
        &mut self,
        presets: Presets,
        _admitted: Admitted,
    ) -> Result<(), DeviceError> {
        self.properties.presets.uncertainty = presets.uncertainty;
        self.properties.presets.mda = presets.mda;
        Ok(())
    }

    fn ensure_unlocked(&self) -> Result<(), DeviceError> {
        match &self.locked_by {
            Some((_, owner)) => Err(DeviceError::Locked {
                owner: owner.clone(),
            }),
            None => Ok(()),
        }
    }

    fn do_start(&mut self, _admitted: Admitted) -> Result<(), DeviceError> {
        // Rule 9: the gate has already refused a start that a held preset
        // satisfies, on a fresh reading. START is accepted or refused on its
        // own reply: an ERR fails at once, whatever the detector happens to
        // be doing, and an accepted START is a start even when the count is
        // over before any read could see it. Whether it counts is for the
        // next reading to say - the mirror's flag is only ever a reading - so
        // the next poll fetches at once.
        // Taken before START goes out: no count it begins starts earlier.
        let sending = chrono::Local::now().naive_local();
        self.command("START")?;
        self.since_poll = POLL_INTERVAL;
        // The instrument reports no measurement date on this road (only
        // ORTEC's Windows library has MIOGetStartTime), so the moment START
        // was accepted is a provisional date, replaced by the real start -
        // the first reading that shows it counting, less its real time - so
        // a detector slow to begin is dated when it began. A resumed count
        // keeps the original date (a provisional one stays provisional, so a
        // START stopped before it was seen counting is dated by the count
        // that follows); Clear resets it.
        if self.spectrum.start_time.is_none() {
            self.spectrum.start_time = Some(sending);
            // Provisional only on a fresh count: the clocks read zero on the
            // gate's status read just before START. An undated count resumed
            // without Clear (one held when this connected) cannot be dated
            // from a clock that includes the earlier run, so it keeps the
            // moment START was sent, as before.
            self.provisional_start = self.status.real_time == 0.0;
            self.start_sent = self.spectrum.start_time;
        }
        // A second START on one still unseen keeps the first one's bound, so
        // the hold ends when a WAIT on the first would call it never started.
        if self.start_unseen.is_none() {
            self.start_unseen = Some(StartUnseen {
                real_time: self.status.real_time,
                ran: false,
                ran_since_reset: false,
                last_fetch: None,
                idle_fetches: 0,
            });
        }
        Ok(())
    }

    fn do_stop(&mut self, _admitted: Admitted) -> Result<(), DeviceError> {
        // What STOP is answered does not decide anything: an idle detector is
        // already stopped (MAESTRO: "no operation occurs") even if it answers
        // ERR, and a reply can be another command's. The stop is the detector
        // saying it no longer counts.
        let reply = self.raw("STOP")?;
        // The START is ended, seen to count or not: the confirm asks the
        // detector alone.
        self.start_unseen = None;
        let stopped = self.confirm_stopped();
        // A provisional date whose count ran (the clocks moved) but was never
        // seen running stays the moment START was sent: a resume's clock
        // would include this run. One that never ran stays provisional.
        if self.provisional_start && self.status.real_time > 0.0 {
            self.provisional_start = false;
        }
        if stopped? {
            // Stopped: the count it stopped at is what the mirror holds next.
            return self.final_fetch();
        }
        Err(match checked(reply) {
            Err(DeviceError::Command { detail, .. }) => DeviceError::Command {
                command: "STOP".into(),
                detail,
            },
            _ => DeviceError::Command {
                command: "STOP".into(),
                detail: "the detector did not stop: it still reports counting".into(),
            },
        })
    }

    fn do_clear(&mut self, _admitted: Admitted) -> Result<(), DeviceError> {
        self.command("CLEAR")?;
        self.spectrum.clear();
        self.status.total_counts = 0;
        // CLEAR resets the instrument's clocks, so the mirror's must go with
        // them rather than waiting for the next poll to notice. Anything that
        // reads the clocks before then reads the count that has just been
        // thrown away: with a live preset held, START is refused as already
        // reached - telling the operator to clear the spectrum they have this
        // moment cleared - and a job's CLEAR / START does not start at all.
        self.status.real_time = 0.0;
        self.status.live_time = 0.0;
        // A START still waiting to be seen now waits from zeroed clocks, and
        // the first reading that shows it counting dates the count afresh
        // (the date went with the spectrum).
        if let Some(unseen) = self.start_unseen.as_mut() {
            unseen.real_time = 0.0;
            unseen.last_fetch = None;
            // The clocks run again from zero: dating needs two readings
            // afresh. That they ran is kept (a count that ran out unseen is
            // over once they stand still), as the count watch keeps it.
            unseen.ran_since_reset = false;
        }
        self.provisional_start = false;
        self.start_sent = None;
        self.freshened = true;
        Ok(())
    }

    fn is_active(&self) -> bool {
        self.status.active
    }

    fn status(&self) -> McbStatus {
        self.status
    }

    fn spectrum(&self) -> &Spectrum {
        &self.spectrum
    }

    fn spectrum_mut(&mut self) -> &mut Spectrum {
        &mut self.spectrum
    }

    fn poll(&mut self, elapsed_seconds: f64) -> Result<(), DeviceError> {
        self.freshened = false;
        self.since_poll += elapsed_seconds;
        match &mut self.line {
            // The direct line fetches here and now, on the calling thread -
            // the command line's WAIT reads the clocks straight after
            // polling, and a test asserts on the very next line.
            Line::Direct(_) => {
                if self.since_poll >= POLL_INTERVAL {
                    self.refresh()?;
                }
                Ok(())
            }
            // Over a courier, a poll only collects and asks: whatever fetch
            // has come back is worked into the mirror, and the next one is
            // requested when it is due. Neither step touches the wire, so
            // the frame this runs inside is never the instrument's hostage.
            Line::Carried(courier) => {
                let brought = courier.collect();
                if self.since_poll >= POLL_INTERVAL {
                    courier.request_fetch();
                    self.since_poll = 0.0;
                }
                match brought {
                    Some(fetched) => self.integrate(fetched?).map(|_| ()),
                    None => Ok(()),
                }
            }
        }
    }

    fn poll_freshened(&self) -> bool {
        self.freshened
    }

    fn do_set_mode(
        &mut self,
        mode: AcquisitionMode,
        _admitted: Admitted,
    ) -> Result<(), DeviceError> {
        self.mode = mode;
        Ok(())
    }

    fn mode(&self) -> AcquisitionMode {
        self.mode
    }

    fn do_send_message(
        &mut self,
        command: &str,
        _admitted: Admitted,
    ) -> Result<String, DeviceError> {
        self.command(command)
    }

    fn lock(&mut self, password: &str, owner: &str) -> Result<(), DeviceError> {
        self.locked_by = Some((password.to_string(), owner.to_string()));
        Ok(())
    }

    fn unlock(&mut self, password: &str) -> Result<(), DeviceError> {
        match &self.locked_by {
            Some((held, _)) if held == password => {
                self.locked_by = None;
                Ok(())
            }
            Some((_, owner)) => Err(DeviceError::Locked {
                owner: owner.clone(),
            }),
            None => Ok(()),
        }
    }

    fn is_locked(&self) -> bool {
        self.locked_by.is_some()
    }

    fn reading_untrusted(&self) -> Option<&str> {
        self.untrusted.as_deref()
    }

    fn readings(&self) -> u64 {
        self.readings
    }

    fn reading_at(&self) -> Option<std::time::Instant> {
        self.reading_at
    }
}

#[cfg(test)]
mod tests {
    use std::sync::{Arc, Mutex};

    use super::*;

    /// A line that answers like a quiet eight-channel instrument and logs
    /// every command sent.
    struct Recording {
        log: Arc<Mutex<Vec<String>>>,
    }

    impl Transport for Recording {
        fn exchange(&mut self, command: &str) -> Result<String, DeviceError> {
            self.log.lock().expect("log").push(command.to_string());
            Ok(match command {
                "SHOW_CONFIGURATION" => "MODEL=DSPEC-50 SERIAL=1 FIRMWARE=1 CHANNELS=8".into(),
                "SHOW_STATUS" => "RT=0.00 LT=0.00 DT=0.0% ICR=0 ACTIVE=0 TOTAL=0".into(),
                "SHOW_DATA" => "DATA 8 0 0 0 0 0 0 0 0".into(),
                "SHOW_PRESETS" => "PRESETS REAL=0.00 LIVE=0.00 PEAK=0 INTEG=0".into(),
                _ => "OK".into(),
            })
        }

        fn peer(&self) -> String {
            "recording".into()
        }
    }

    #[test]
    fn a_strange_value_the_mirror_already_holds_does_not_block_or_resend_on_an_unrelated_edit() {
        let log = Arc::new(Mutex::new(Vec::new()));
        let mut remote =
            RemoteMcb::connect(Box::new(Recording { log: log.clone() }), 1, "t").expect("connect");
        remote.properties.amplifier.coarse_gain = 20.4;
        remote.properties.high_voltage.target_volts = f64::NAN;
        log.lock().expect("log").clear();

        let mut properties = remote.properties().clone();
        properties.adc.lower_discriminator = 10;
        remote
            .set_properties(properties)
            .expect("an unrelated edit");
        assert_eq!(remote.properties().adc.lower_discriminator, 10);
        assert_eq!(
            *log.lock().expect("log"),
            Vec::<String>::new(),
            "nothing was sent for a value nobody changed"
        );

        // Changing it to a value that cannot be sent is still refused.
        let mut properties = remote.properties().clone();
        properties.amplifier.coarse_gain = 20.5;
        assert!(remote.set_properties(properties).is_err());
        assert!(remote.set_coarse_gain(20.4).is_err());
        assert!(remote.set_high_voltage(f64::NAN).is_err());
        assert_eq!(*log.lock().expect("log"), Vec::<String>::new());
    }
}

#[cfg(test)]
mod preset_road_tests {
    use std::sync::{Arc, Mutex};

    use super::*;
    use crate::presets::{MdaPreset, UncertaintyPreset};

    /// What the instrument says and has been sent, shared with the test.
    struct Bench {
        log: Vec<String>,
        /// The `SHOW_PRESETS` answer; `None` is a bridge that refuses it.
        shown: Option<String>,
    }

    struct Line(Arc<Mutex<Bench>>);

    impl Transport for Line {
        fn exchange(&mut self, command: &str) -> Result<String, DeviceError> {
            let mut bench = self.0.lock().expect("bench");
            bench.log.push(command.to_string());
            Ok(match command {
                "SHOW_CONFIGURATION" => "MODEL=DSPEC-50 SERIAL=1 FIRMWARE=1 CHANNELS=8".into(),
                "SHOW_STATUS" => "RT=0.00 LT=0.00 DT=0.0% ICR=0 ACTIVE=0 TOTAL=0".into(),
                "SHOW_DATA" => "DATA 8 0 0 0 0 0 0 0 0".into(),
                "SHOW_PRESETS" => match &bench.shown {
                    Some(text) => text.clone(),
                    None => "ERR unknown command".into(),
                },
                _ => "OK".into(),
            })
        }

        fn peer(&self) -> String {
            "bench".into()
        }
    }

    /// Connected with the instrument holding `live` seconds; log cleared.
    fn connected(live: &str) -> (RemoteMcb, Arc<Mutex<Bench>>) {
        let bench = Arc::new(Mutex::new(Bench {
            log: Vec::new(),
            shown: Some(format!("PRESETS REAL=0.00 LIVE={live} PEAK=0 INTEG=0")),
        }));
        let remote = RemoteMcb::connect(Box::new(Line(bench.clone())), 1, "t").expect("connect");
        bench.lock().expect("bench").log.clear();
        (remote, bench)
    }

    fn sent(bench: &Arc<Mutex<Bench>>) -> Vec<String> {
        bench
            .lock()
            .expect("bench")
            .log
            .iter()
            .filter(|line| line.starts_with("SET_"))
            .cloned()
            .collect()
    }

    fn uncertainty() -> UncertaintyPreset {
        UncertaintyPreset {
            limit_percent: 1.0,
            low_channel: 1,
            high_channel: 4,
        }
    }

    #[test]
    fn a_host_side_edit_sends_nothing_and_reaches_the_mirror() {
        let (mut remote, bench) = connected("300.00");
        let mut edited = *remote.presets();
        edited.uncertainty = Some(uncertainty());
        remote.apply_presets(edited).expect("applied");
        assert_eq!(sent(&bench), Vec::<String>::new());
        assert_eq!(remote.presets().uncertainty, edited.uncertainty);
        assert_eq!(remote.presets().live_time, Some(300.0));
    }

    #[test]
    fn a_locked_instrument_refuses_even_a_host_side_edit() {
        let (mut remote, bench) = connected("300.00");
        remote.lock("pw", "owner").expect("lock");
        let mut edited = *remote.presets();
        edited.mda = Some(MdaPreset {
            target: 1.0,
            region: mantaray_core::Roi::new(1, 4),
            efficiency: 0.1,
            yield_fraction: 0.9,
        });
        let error = remote.apply_presets(edited).expect_err("locked");
        assert!(matches!(error, DeviceError::Locked { .. }), "{error}");
        assert!(remote.presets().mda.is_none());
        assert_eq!(sent(&bench), Vec::<String>::new());
    }

    #[test]
    fn applying_unchanged_presets_re_arms_with_a_clear_and_every_held_preset() {
        // Nothing changed and the operator presses Apply: the explicit action
        // reaches the instrument, and the instrument is never asked first.
        let (mut remote, bench) = connected("300.00");
        let shown = *remote.presets();
        remote.apply_presets(shown).expect("applied");
        assert_eq!(
            sent(&bench),
            vec![
                "SET_PRESET_CLEAR".to_string(),
                "SET_PRESET_LIVE 300".to_string()
            ]
        );
        assert!(!asked_for_presets(&bench), "applying never reads");
    }

    #[test]
    fn a_changed_live_preset_sends_a_clear_and_the_held_presets() {
        let (mut remote, bench) = connected("300.00");
        let edited = Presets {
            live_time: Some(60.0),
            ..*remote.presets()
        };
        remote.apply_presets(edited).expect("applied");
        assert_eq!(
            sent(&bench),
            vec![
                "SET_PRESET_CLEAR".to_string(),
                "SET_PRESET_LIVE 60".to_string()
            ]
        );
        assert_eq!(remote.presets().live_time, Some(60.0));
    }

    fn asked_for_presets(bench: &Arc<Mutex<Bench>>) -> bool {
        bench
            .lock()
            .expect("bench")
            .log
            .iter()
            .any(|line| line == "SHOW_PRESETS")
    }

    /// Connected to an old bridge: SHOW_PRESETS is refused from the first
    /// question, so the mirror starts with no presets; log cleared.
    fn connected_to_an_old_bridge() -> (RemoteMcb, Arc<Mutex<Bench>>) {
        let bench = Arc::new(Mutex::new(Bench {
            log: Vec::new(),
            shown: None,
        }));
        let remote = RemoteMcb::connect(Box::new(Line(bench.clone())), 1, "t").expect("connect");
        assert!(remote.presets().is_empty(), "the bridge could not say");
        bench.lock().expect("bench").log.clear();
        (remote, bench)
    }

    #[test]
    fn against_an_old_bridge_a_host_side_edit_sends_nothing() {
        let (mut remote, bench) = connected_to_an_old_bridge();
        let mut edited = *remote.presets();
        edited.uncertainty = Some(uncertainty());
        remote.apply_presets(edited).expect("applied");
        assert_eq!(sent(&bench), Vec::<String>::new());
        assert_eq!(remote.presets().uncertainty, edited.uncertainty);
        assert!(!asked_for_presets(&bench), "applying never asks");
    }

    #[test]
    fn against_an_old_bridge_an_unchanged_apply_sends_a_clear_and_the_held_presets() {
        let (mut remote, bench) = connected_to_an_old_bridge();
        // A preset the operator set this session is the only one held.
        remote.apply_presets(Presets::live(120.0)).expect("set");
        bench.lock().expect("bench").log.clear();
        let same = *remote.presets();
        remote.apply_presets(same).expect("re-applied");
        assert_eq!(
            sent(&bench),
            vec![
                "SET_PRESET_CLEAR".to_string(),
                "SET_PRESET_LIVE 120".to_string()
            ]
        );
        assert!(!asked_for_presets(&bench), "applying never asks");
    }

    #[test]
    fn a_preset_change_reads_the_status_once_and_never_the_presets() {
        let (mut remote, bench) = connected("300.00");
        let edited = Presets {
            live_time: Some(60.0),
            ..*remote.presets()
        };
        remote.apply_presets(edited).expect("applied");
        let log = bench.lock().expect("bench").log.clone();
        assert_eq!(
            log,
            vec![
                "SHOW_STATUS".to_string(),
                "SET_PRESET_CLEAR".to_string(),
                "SET_PRESET_LIVE 60".to_string()
            ]
        );
    }

    #[test]
    fn clear_presets_always_sends() {
        let (mut remote, bench) = connected("0.00");
        remote.set_presets(Presets::default()).expect("cleared");
        assert_eq!(sent(&bench), vec!["SET_PRESET_CLEAR".to_string()]);
    }

    #[test]
    fn properties_carrying_an_unchanged_preset_send_nothing_even_when_it_cannot_be_read() {
        let (mut remote, bench) = connected("300.00");
        bench.lock().expect("bench").shown = None;
        let mut properties = remote.properties().clone();
        properties.adc.lower_discriminator = 10;
        remote.set_properties(properties).expect("edit");
        assert_eq!(bench.lock().expect("bench").log, Vec::<String>::new());
    }

    #[test]
    fn the_status_parse_takes_only_a_status_line() {
        let reading =
            parse_status("RT=2.50 LT=2.00 DT=1.0% ICR=5 ACTIVE=1 TOTAL=9").expect("a status line");
        assert_eq!(
            reading,
            Status {
                active: true,
                real_time: 2.5,
                live_time: 2.0
            }
        );
        for reply in [
            "RT=1 LT=1",
            "ACTIVE=1 LT=1",
            "ACTIVE=1 RT=1",
            "OK",
            "$C04096106",
        ] {
            assert_eq!(parse_status(reply), None, "{reply}");
        }
    }
}
