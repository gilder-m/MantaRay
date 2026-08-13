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
use crate::mcb::{Mcb, McbIdentity, McbProperties, McbStatus};
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
    /// Where a fetch's channel counts are parsed before they reach the
    /// mirror - see [`Self::integrate`] for why they do not go straight in.
    scratch: Vec<u64>,
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
        let configuration = checked(transport.exchange("SHOW_CONFIGURATION")?)?;
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
        let mut remote = Self {
            line: Line::Direct(transport),
            identity,
            properties: McbProperties::default(),
            spectrum,
            status: McbStatus::default(),
            mode: AcquisitionMode::Pha,
            locked_by: None,
            since_poll: f64::MAX,
            scratch: Vec::new(),
        };
        remote.refresh()?;
        remote.read_presets();
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
        let field = |key: &str| -> Option<f64> {
            reply.split_whitespace().find_map(|word| {
                word.strip_prefix(key)
                    .and_then(|rest| rest.strip_prefix('='))
                    .and_then(|value| value.parse::<f64>().ok())
            })
        };
        // Zero is how an instrument says "none set", so it is not a preset of
        // zero - which would mean an acquisition that stops immediately.
        let seconds = |value: Option<f64>| value.filter(|value| *value > 0.0);
        let counts = |value: Option<f64>| {
            value
                .filter(|value| *value >= 1.0)
                .map(|value| value as u64)
        };
        self.properties.presets.real_time = seconds(field("REAL"));
        self.properties.presets.live_time = seconds(field("LIVE"));
        self.properties.presets.roi_peak = counts(field("PEAK"));
        self.properties.presets.roi_integral = counts(field("INTEG"));
    }

    /// One command out, one line back, however the line is held.
    fn raw(&mut self, command: &str) -> Result<String, DeviceError> {
        match &mut self.line {
            Line::Direct(transport) => transport.exchange(command),
            Line::Carried(courier) => courier.exchange(command),
        }
    }

    /// One command out, one checked line back.
    fn command(&mut self, command: &str) -> Result<String, DeviceError> {
        checked(self.raw(command)?)
    }

    /// Pulls fresh status and data from the instrument, on this thread.
    ///
    /// The direct line's whole poll; over a courier this runs only at
    /// connection time, before the courier exists, and [`Self::integrate`]
    /// alone runs afterwards.
    fn refresh(&mut self) -> Result<(), DeviceError> {
        self.since_poll = 0.0;
        let status = self.raw("SHOW_STATUS")?;
        let data = self.raw("SHOW_DATA")?;
        self.integrate(Fetched { status, data })
    }

    /// Works a fetch's lines into the mirror.
    ///
    /// Parsing lives here rather than with the fetch so that a courier can
    /// stay a courier: it moves bytes, and what the bytes mean - including
    /// an `ERR` where an answer should be - is decided on the caller's side.
    fn integrate(&mut self, fetched: Fetched) -> Result<(), DeviceError> {
        let status = checked(fetched.status)?;
        let number = |key: &str| -> Option<f64> {
            status.split_whitespace().find_map(|word| {
                word.strip_prefix(key)
                    .and_then(|rest| rest.strip_prefix('='))
                    .map(|value| value.trim_end_matches('%'))
                    .and_then(|value| value.parse().ok())
            })
        };
        self.status.real_time = number("RT").unwrap_or(self.status.real_time);
        self.status.live_time = number("LT").unwrap_or(self.status.live_time);
        self.status.dead_time_percent = number("DT").unwrap_or(0.0);
        self.status.input_count_rate = number("ICR").unwrap_or(0.0);
        self.status.active = number("ACTIVE").unwrap_or(0.0) != 0.0;
        self.status.total_counts = number("TOTAL").unwrap_or(0.0) as u64;

        let data = checked(fetched.data)?;
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
        if count > 0 {
            if self.spectrum.len() != count {
                // The instrument's conversion gain changed underneath us. The
                // counts are new, but whose spectrum this is - detector,
                // calibration, start date - is not.
                let mut resized = Spectrum::new(count);
                resized.copy_descriptors_from(&self.spectrum);
                self.spectrum = resized;
            }
            self.spectrum.channels.copy_from_slice(&self.scratch);
        }
        self.spectrum.real_time = self.status.real_time;
        self.spectrum.live_time = self.status.live_time;
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
        if self.spectrum.start_time.is_none() && self.status.active && self.status.real_time > 0.0 {
            let counted = chrono::Duration::milliseconds((self.status.real_time * 1000.0) as i64);
            self.spectrum.start_time = chrono::Local::now()
                .naive_local()
                .checked_sub_signed(counted);
        }
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

impl Mcb for RemoteMcb {
    fn identity(&self) -> &McbIdentity {
        &self.identity
    }

    fn properties(&self) -> &McbProperties {
        &self.properties
    }

    fn set_properties(&mut self, properties: McbProperties) -> Result<(), DeviceError> {
        self.ensure_unlocked()?;
        // Push the settings hardware understands; the rest stay in the mirror.
        self.command(&format!(
            "SET_HV {:.0}",
            properties.high_voltage.target_volts
        ))?;
        self.command(&format!(
            "SET_GAIN_COARSE {:.0}",
            properties.amplifier.coarse_gain
        ))?;
        self.set_presets(properties.presets)?;
        self.properties = properties;
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

    fn presets_mut(&mut self) -> &mut Presets {
        &mut self.properties.presets
    }

    fn set_presets(&mut self, presets: Presets) -> Result<(), DeviceError> {
        self.ensure_unlocked()?;
        self.command("SET_PRESET_CLEAR")?;
        if let Some(seconds) = presets.real_time {
            self.command(&format!("SET_PRESET_REAL {seconds}"))?;
        }
        if let Some(seconds) = presets.live_time {
            self.command(&format!("SET_PRESET_LIVE {seconds}"))?;
        }
        if let Some(counts) = presets.roi_peak {
            self.command(&format!("SET_PRESET_COUNT {counts}"))?;
        }
        if let Some(counts) = presets.roi_integral {
            self.command(&format!("SET_PRESET_INTEG {counts}"))?;
        }
        self.properties.presets = presets;
        Ok(())
    }

    fn start(&mut self) -> Result<(), DeviceError> {
        self.ensure_unlocked()?;
        // A preset that is already satisfied is why an instrument can answer
        // START and then not count: the 926 accepts the command, leaves the
        // clocks where they are, and says nothing. Refusing here, by name,
        // beats a Start button that appears to work and does not.
        //
        // Only the four the instrument itself holds are checked. Uncertainty
        // and MDA are worked out host-side, and an acquisition that begins
        // already satisfying one is stopped by `advance` on the next poll,
        // which says so - it is not the silent refusal this guards against.
        let held_by_the_instrument = Presets {
            real_time: self.properties.presets.real_time,
            live_time: self.properties.presets.live_time,
            roi_peak: self.properties.presets.roi_peak,
            roi_integral: self.properties.presets.roi_integral,
            ..Default::default()
        };
        if let Some(kind) = held_by_the_instrument.reached(
            &self.spectrum,
            self.status.real_time,
            self.status.live_time,
        ) {
            return Err(DeviceError::Command {
                command: "START".into(),
                detail: format!(
                    "the {} preset is already reached - clear the spectrum, or change the preset",
                    kind.label()
                ),
            });
        }
        self.command("START")?;
        self.status.active = true;
        // The instrument reports no measurement date of its own, so the moment
        // START was accepted is the record - as the simulator keeps it. A
        // resumed count keeps the original date; Clear resets it.
        if self.spectrum.start_time.is_none() {
            self.spectrum.start_time = Some(chrono::Local::now().naive_local());
        }
        Ok(())
    }

    fn stop(&mut self) -> Result<(), DeviceError> {
        self.command("STOP")?;
        self.status.active = false;
        Ok(())
    }

    fn clear(&mut self) -> Result<(), DeviceError> {
        self.ensure_unlocked()?;
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
                    Some(fetched) => self.integrate(fetched?),
                    None => Ok(()),
                }
            }
        }
    }

    fn set_mode(&mut self, mode: AcquisitionMode) -> Result<(), DeviceError> {
        self.ensure_unlocked()?;
        self.mode = mode;
        Ok(())
    }

    fn mode(&self) -> AcquisitionMode {
        self.mode
    }

    fn send_message(&mut self, command: &str) -> Result<String, DeviceError> {
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
}
