//! The multichannel buffer abstraction: what MAESTRO calls a Detector.
//!
//! Everything the user interface needs from an instrument sits behind [`Mcb`], so
//! a simulated detector, a file replay or a real ORTEC unit all look the same.

use mantaray_core::{AcquisitionMode, Spectrum};
use mantaray_formats::ListModeFile;
use serde::{Deserialize, Serialize};

use crate::error::DeviceError;
use crate::presets::{PresetKind, Presets};

/// What an instrument can do; the interface greys out what it cannot.
#[derive(Clone, Copy, Debug, PartialEq, Eq, Serialize, Deserialize)]
pub struct Capabilities {
    /// Event-by-event list mode.
    pub list_mode: bool,
    /// Zero-dead-time / live-time-corrected acquisition.
    pub zdt: bool,
    /// Sample-changer control lines.
    pub sample_changer: bool,
    /// Programmable detector bias.
    pub high_voltage: bool,
    /// Gain/zero stabiliser.
    pub stabilizer: bool,
    /// Reports the input count rate, not just the stored rate.
    pub input_count_rate: bool,
    /// Automatic pole-zero and optimise.
    pub optimize: bool,
    /// Spectra stored inside the instrument (field mode).
    pub stored_spectra: bool,
    /// Largest usable channel count.
    pub max_channels: usize,
}

impl Default for Capabilities {
    fn default() -> Self {
        Self {
            list_mode: true,
            zdt: true,
            sample_changer: true,
            high_voltage: true,
            stabilizer: true,
            input_count_rate: true,
            optimize: true,
            stored_spectra: false,
            max_channels: 16_384,
        }
    }
}

/// Fixed facts about an instrument, shown on the About and Status tabs.
#[derive(Clone, Debug, PartialEq, Eq, Serialize, Deserialize)]
pub struct McbIdentity {
    /// Detector number in the pick list, 1..=999.
    pub number: u16,
    /// Display name.
    pub name: String,
    /// Model, e.g. `DSPEC-50`.
    pub model: String,
    /// Serial number.
    pub serial: String,
    /// Firmware or simulator version.
    pub firmware: String,
    /// Free-form detector description saved with spectra.
    pub description: String,
    /// Configured memory size in channels.
    pub channels: usize,
    /// What the instrument supports.
    pub capabilities: Capabilities,
}

/// Signal polarity of the preamplifier input.
#[derive(Clone, Copy, Debug, Default, PartialEq, Eq, Serialize, Deserialize)]
pub enum Polarity {
    /// Positive-going pulses.
    #[default]
    Positive,
    /// Negative-going pulses.
    Negative,
}

/// Baseline restorer setting.
#[derive(Clone, Copy, Debug, Default, PartialEq, Eq, Serialize, Deserialize)]
pub enum BaselineRestorer {
    /// Chosen by the instrument from the rise time and rate.
    #[default]
    Automatic,
    /// Fixed fast setting.
    Fast,
    /// Fixed slow setting.
    Slow,
}

/// Gating mode of the ADC.
#[derive(Clone, Copy, Debug, Default, PartialEq, Eq, Serialize, Deserialize)]
pub enum GateMode {
    /// No gating.
    #[default]
    Off,
    /// Coincidence gate.
    Coincidence,
    /// Anticoincidence gate.
    Anticoincidence,
}

/// Amplifier tab of the MCB Properties dialog.
#[derive(Clone, Copy, Debug, PartialEq, Serialize, Deserialize)]
pub struct AmplifierSettings {
    /// Coarse gain multiplier.
    pub coarse_gain: f64,
    /// Fine gain, 0.4 to 1.0 on real units; the product is the total gain.
    pub fine_gain: f64,
    /// Shaping rise time in microseconds.
    pub rise_time_us: f64,
    /// Flat-top width in microseconds.
    pub flat_top_us: f64,
    /// Pole-zero setting, 0 to 4095.
    pub pole_zero: u16,
    /// Baseline restorer mode.
    pub baseline_restorer: BaselineRestorer,
    /// Pile-up (pulse-shape) rejection.
    pub pileup_rejection: bool,
    /// Input polarity.
    pub polarity: Polarity,
}

impl AmplifierSettings {
    /// Lowest coarse gain the settings dialog offers.
    pub const COARSE_GAIN_MIN: f64 = 1.0;
    /// Highest coarse gain the settings dialog offers. The range is the
    /// dialog's, not a figure from an instrument's documentation.
    pub const COARSE_GAIN_MAX: f64 = 1000.0;

    /// Whether `gain` can be sent as `SET_GAIN_COARSE`: a whole number, since
    /// the command carries one, within the range above. Every place that takes
    /// a coarse gain asks this, so none sends or keeps a value another refuses.
    pub fn check_coarse_gain(gain: f64) -> Result<(), DeviceError> {
        check_whole(
            "coarse gain",
            gain,
            Self::COARSE_GAIN_MIN,
            Self::COARSE_GAIN_MAX,
            "",
        )
    }
}

// The change rules, one copy, in the core crate (it cannot depend on this one).
// Public here so the dialogs and the instruments reach them by the path they
// always used.
pub use mantaray_core::{differs, plain_zero};

/// `value` must be finite, whole and within `min..=max`.
fn check_whole(
    setting: &str,
    value: f64,
    min: f64,
    max: f64,
    unit: &str,
) -> Result<(), DeviceError> {
    if value.is_finite() && value.fract() == 0.0 && (min..=max).contains(&value) {
        return Ok(());
    }
    Err(DeviceError::OutOfRange {
        setting: setting.into(),
        detail: format!("{value}{unit} is not a whole number from {min} to {max}{unit}"),
    })
}

impl Default for AmplifierSettings {
    fn default() -> Self {
        Self {
            coarse_gain: 20.0,
            fine_gain: 0.5,
            rise_time_us: 12.0,
            flat_top_us: 1.0,
            pole_zero: 2048,
            baseline_restorer: BaselineRestorer::Automatic,
            pileup_rejection: true,
            polarity: Polarity::Positive,
        }
    }
}

impl AmplifierSettings {
    /// Total gain: coarse times fine.
    pub fn gain(&self) -> f64 {
        self.coarse_gain * self.fine_gain
    }
}

/// ADC tab of the MCB Properties dialog.
#[derive(Clone, Copy, Debug, PartialEq, Eq, Serialize, Deserialize)]
pub struct AdcSettings {
    /// Channels the ADC converts into; at most the memory size.
    pub conversion_gain: usize,
    /// Lower level discriminator, in channels.
    pub lower_discriminator: u32,
    /// Upper level discriminator, in channels.
    pub upper_discriminator: u32,
    /// Gating mode.
    pub gate: GateMode,
}

impl Default for AdcSettings {
    fn default() -> Self {
        Self {
            conversion_gain: 8_192,
            lower_discriminator: 0,
            upper_discriminator: u32::MAX,
            gate: GateMode::Off,
        }
    }
}

/// What to do when the bias supply faults.
#[derive(Clone, Copy, Debug, Default, PartialEq, Eq, Serialize, Deserialize)]
pub enum HvShutdown {
    /// Shut down on the detector's own signal.
    #[default]
    Detector,
    /// Shut down on an over-temperature signal.
    Temperature,
    /// No automatic shutdown.
    None,
}

/// High-voltage tab of the MCB Properties dialog.
#[derive(Clone, Copy, Debug, PartialEq, Serialize, Deserialize)]
pub struct HighVoltageSettings {
    /// Requested bias in volts.
    pub target_volts: f64,
    /// Whether the supply is enabled.
    pub enabled: bool,
    /// Shutdown source.
    pub shutdown: HvShutdown,
}

impl HighVoltageSettings {
    /// Highest bias target the settings dialog offers. The range is the
    /// dialog's, not a figure from an instrument's documentation.
    pub const TARGET_MAX_VOLTS: f64 = 5_000.0;

    /// Whether `volts` can be sent as `SET_HV`: a whole number of volts
    /// (the command carries one, so a fraction would be sent as a different
    /// number than the one kept) from 0 to [`Self::TARGET_MAX_VOLTS`].
    pub fn check_target(volts: f64) -> Result<(), DeviceError> {
        check_whole(
            "high voltage target",
            volts,
            0.0,
            Self::TARGET_MAX_VOLTS,
            " V",
        )
    }
}

impl Default for HighVoltageSettings {
    fn default() -> Self {
        Self {
            target_volts: 3_000.0,
            enabled: true,
            shutdown: HvShutdown::Detector,
        }
    }
}

/// Stabiliser tab of the MCB Properties dialog.
#[derive(Clone, Copy, Debug, Default, PartialEq, Eq, Serialize, Deserialize)]
pub struct StabilizerSettings {
    /// Correct gain drift.
    pub gain_enabled: bool,
    /// Correct zero drift.
    pub zero_enabled: bool,
    /// Centre channel of the gain reference peak.
    pub gain_channel: usize,
    /// Width in channels of the gain reference window.
    pub gain_width: usize,
    /// Centre channel of the zero reference peak.
    pub zero_channel: usize,
    /// Width in channels of the zero reference window.
    pub zero_width: usize,
}

/// Everything on the MCB Properties dialog.
#[derive(Clone, Debug, Default, PartialEq, Serialize, Deserialize)]
pub struct McbProperties {
    /// Amplifier tab.
    pub amplifier: AmplifierSettings,
    /// ADC tab.
    pub adc: AdcSettings,
    /// High-voltage tab.
    pub high_voltage: HighVoltageSettings,
    /// Stabiliser tab.
    pub stabilizer: StabilizerSettings,
    /// Presets tab.
    pub presets: Presets,
}

/// A snapshot of what an instrument is doing.
#[derive(Clone, Copy, Debug, Default, PartialEq, Serialize, Deserialize)]
pub struct McbStatus {
    /// Real (clock) time in seconds.
    pub real_time: f64,
    /// Live time in seconds.
    pub live_time: f64,
    /// Dead time as a percentage of the real time.
    pub dead_time_percent: f64,
    /// Events arriving at the input, per second.
    pub input_count_rate: f64,
    /// Events stored per second of real time (throughput).
    pub total_count_rate: f64,
    /// Whether data collection is running.
    pub active: bool,
    /// Measured detector bias in volts.
    pub high_voltage: f64,
    /// Whether the sample-ready line is asserted.
    pub sample_ready: bool,
    /// Total stored counts.
    pub total_counts: u64,
}

/// What a command through the gate ([`admit`]) is about to do.
enum Op<'a> {
    /// START.
    Start,
    /// STOP, asked for by the operator or a job.
    Stop,
    /// CLEAR (the spectrum and clocks; presets are untouched).
    Clear,
    /// Any change to the presets, the host-side ones included.
    Presets(&'a Presets),
    /// Bias, gain and every other setting, the tuning routines and the
    /// instrument's own spectrum store.
    Setting,
    /// A raw command (`SEND_MESSAGE`).
    Raw(&'a str),
}

/// Proof that a command passed the gate. Its field is private to this
/// module, and only [`admit`] and [`Mcb::stop_at_preset`] make one, so the
/// `do_*` methods that take it cannot be reached without the gate.
///
/// ```compile_fail
/// let _ = mantaray_device::mcb::Admitted(());
/// ```
///
/// ```compile_fail
/// use mantaray_device::{Mcb, Presets, SimulatedMcb};
/// let mut detector = SimulatedMcb::new(1, "SIM");
/// detector.presets_mut().live_time = Some(1.0);
/// ```
///
/// ```compile_fail
/// use mantaray_device::{Mcb, Presets, SimulatedMcb};
/// let mut detector = SimulatedMcb::new(1, "SIM");
/// // The arguments are right; only the token cannot be made.
/// detector
///     .do_write_presets(Presets::live(1.0), mantaray_device::mcb::Admitted(()))
///     .unwrap();
/// ```
///
/// What the ones above try by the back door, the front door does:
///
/// ```
/// use mantaray_device::{Mcb, Presets, SimulatedMcb};
/// let mut detector = SimulatedMcb::new(1, "SIM");
/// detector.apply_presets(Presets::live(1.0)).unwrap();
/// ```
pub struct Admitted(());

/// The one gate every changing command passes, in this order:
/// 1. the lock: a locked detector refuses everything but a raw read
///    (`SHOW_...`), and nothing is sent, not even a status read;
/// 2. for a preset change only, whether the detector is counting, asked of
///    the instrument right now ([`Mcb::read_counting`]): an answer that
///    cannot be had refuses ([`DeviceError::CountingUnknown`]), and counting
///    refuses ([`DeviceError::PresetsWhileCounting`]);
/// 3. for a preset change only, the values ([`Presets::validate`]); for a
///    START, whether a held preset is already reached, on a fresh reading.
///
/// A raw message is refused before all of that when it is more than one
/// line or a preset command.
///
/// Private, and the only maker of [`Admitted`] besides the preset stop: both
/// backends get the same refusal, in the same order, from this one function.
fn admit<M: Mcb + ?Sized>(mcb: &mut M, op: Op<'_>) -> Result<Admitted, DeviceError> {
    if let Op::Raw(command) = op {
        raw_allowed(command)?;
    }
    let reads_only = matches!(op, Op::Raw(command) if verb(command).starts_with("SHOW_"));
    if !reads_only {
        mcb.ensure_unlocked()?;
    }
    match op {
        Op::Presets(presets) => {
            if mcb.read_counting()? {
                return Err(DeviceError::PresetsWhileCounting);
            }
            presets.validate()?;
        }
        Op::Start => not_already_reached(mcb)?,
        Op::Raw(command) if verb(command) == "START" => not_already_reached(mcb)?,
        _ => {}
    }
    Ok(Admitted(()))
}

/// Rule 9: a START that a held preset already satisfies is refused before
/// it is sent, in MAESTRO's words. The clocks are the detector's own, read
/// now ([`Mcb::read_counting`] refreshes them): the count may have gone on
/// elsewhere since the last poll. Only the four presets the instrument holds
/// are compared; the host-side ones are stopped by [`crate::advance`].
fn not_already_reached<M: Mcb + ?Sized>(mcb: &mut M) -> Result<(), DeviceError> {
    mcb.read_counting()?;
    let status = mcb.status();
    let held = mcb.presets().held_by_the_instrument();
    match held.reached(mcb.spectrum(), status.real_time, status.live_time) {
        Some(kind) => Err(DeviceError::Command {
            command: "START".into(),
            detail: format!(
                "Preset already reached (the {} preset) - clear the spectrum, or change the preset",
                kind.label()
            ),
        }),
        None => Ok(()),
    }
}

/// A raw message's verb, upper case.
fn verb(command: &str) -> String {
    command
        .split_whitespace()
        .next()
        .unwrap_or("")
        .to_ascii_uppercase()
}

/// What a raw message may be: exactly one line. A second line would be a
/// second command the gate never saw, and would put the replies out of step.
fn raw_allowed(command: &str) -> Result<(), DeviceError> {
    if command.contains(['\r', '\n']) {
        return Err(DeviceError::Command {
            command: command.trim().to_string(),
            detail: "a raw message is one line, and this one holds a line break; nothing was sent"
                .into(),
        });
    }
    Ok(())
}

/// A raw preset command (`SET_PRESET_CLEAR`, `SET_PRESET_REAL s`,
/// `SET_PRESET_LIVE s`, `SET_PRESET_COUNT n`, `SET_PRESET_INTEG n`) as the
/// whole preset set it asks for, starting from `current`; `None` for a
/// message that is not one. A raw preset is a preset change like any other
/// (amended rule, raw messages): it goes through the preset rules, so it
/// becomes a [`Mcb::set_presets`] and is never sent round them. This is also
/// how `mantaray serve` hands a client's presets to the instrument it serves.
fn raw_presets(current: &Presets, command: &str) -> Result<Option<Presets>, DeviceError> {
    let verb = verb(command);
    if !verb.starts_with("SET_PRESET") {
        return Ok(None);
    }
    let argument = command.split_whitespace().nth(1);
    let refuse = |detail: String| DeviceError::Command {
        command: command.trim().to_string(),
        detail,
    };
    let number = || -> Result<f64, DeviceError> {
        argument
            .and_then(|word| word.parse::<f64>().ok())
            .ok_or_else(|| refuse("the value is not a number; nothing was sent".into()))
    };
    // Counts are whole: a fraction is refused, never truncated (rule 6).
    let count = |setting: &str| -> Result<Option<u64>, DeviceError> {
        let value = number()?;
        if !value.is_finite() || value < 0.0 || value.fract() != 0.0 || value >= u64::MAX as f64 {
            return Err(DeviceError::OutOfRange {
                setting: setting.into(),
                detail: format!("{value} is not a whole number of counts"),
            });
        }
        Ok(Some(value as u64))
    };
    let mut presets = *current;
    match verb.as_str() {
        "SET_PRESET_CLEAR" => presets = Presets::default(),
        "SET_PRESET_REAL" => presets.real_time = Some(number()?),
        "SET_PRESET_LIVE" => presets.live_time = Some(number()?),
        "SET_PRESET_COUNT" => presets.roi_peak = count("ROI peak preset")?,
        "SET_PRESET_INTEG" => presets.roi_integral = count("ROI integral preset")?,
        _ => {
            return Err(DeviceError::NotSupported {
                feature: format!(
                    "{verb} sent raw - use the job's own preset commands; nothing was sent"
                ),
            });
        }
    }
    Ok(Some(presets))
}

/// A multichannel buffer: an instrument that collects a spectrum.
///
/// Every command that changes the detector is a provided method here that
/// passes the gate ([`admit`]) and then calls the backend's `do_*` method with
/// the [`Admitted`] it got. A backend implements the `do_*` methods and the
/// accessors; it never overrides a provided command method (a test reads the
/// sources to hold that).
pub trait Mcb: Send {
    /// Fixed facts about the instrument.
    fn identity(&self) -> &McbIdentity;

    /// Renames the instrument as the operator sees it.
    ///
    /// The name is the application's, not the instrument's: it is what the
    /// pick list, the window title and a saved spectrum's detector fields
    /// say. "Bench HPGe" is worth more to whoever reads the file next year
    /// than the model and serial the scan happened to print.
    fn set_name(&mut self, name: &str);

    /// Current settings.
    fn properties(&self) -> &McbProperties;

    /// Applies settings: bias, gain and everything kept only in this
    /// program. Never the presets: an edit whose presets differ from what the
    /// detector holds is refused, and presets change through
    /// [`Self::apply_presets`]. Fails while locked, or on out-of-range values.
    fn set_properties(&mut self, mut properties: McbProperties) -> Result<(), DeviceError> {
        let admitted = admit(self, Op::Setting)?;
        // Zero disables a preset, so a zero is the same preset as none.
        properties.presets = properties.presets.normalized();
        if properties.presets.differ(self.presets()) {
            return Err(DeviceError::NotSupported {
                feature: "changing presets with the other settings - presets are changed \
                          with Apply presets"
                    .into(),
            });
        }
        self.do_set_properties(properties, admitted)
    }

    /// The settings change behind [`Self::set_properties`], presets already
    /// known to be unchanged.
    fn do_set_properties(
        &mut self,
        properties: McbProperties,
        admitted: Admitted,
    ) -> Result<(), DeviceError>;

    /// Sets the high-voltage target, sent even when it equals what the
    /// settings already say.
    ///
    /// The settings an instrument starts from need not be the instrument's
    /// own - a network instrument's are defaults - so a bias the operator
    /// applies must go out whatever the mirror holds.
    fn set_high_voltage(&mut self, volts: f64) -> Result<(), DeviceError> {
        let admitted = admit(self, Op::Setting)?;
        // What is applied is always checked, whether or not it changes the
        // settings: `set_properties` only checks what changed.
        HighVoltageSettings::check_target(volts)?;
        self.do_set_high_voltage(plain_zero(volts), admitted)
    }

    /// The send behind [`Self::set_high_voltage`], the value already checked.
    /// By default a settings change; a network instrument sends `SET_HV`.
    fn do_set_high_voltage(&mut self, volts: f64, admitted: Admitted) -> Result<(), DeviceError> {
        let mut properties = self.properties().clone();
        properties.high_voltage.target_volts = volts;
        self.do_set_properties(properties, admitted)
    }

    /// Sets the coarse gain, sent even when it equals what the settings
    /// already say, for the same reason as [`Self::set_high_voltage`].
    fn set_coarse_gain(&mut self, gain: f64) -> Result<(), DeviceError> {
        let admitted = admit(self, Op::Setting)?;
        AmplifierSettings::check_coarse_gain(gain)?;
        self.do_set_coarse_gain(plain_zero(gain), admitted)
    }

    /// The send behind [`Self::set_coarse_gain`], the value already checked.
    fn do_set_coarse_gain(&mut self, gain: f64, admitted: Admitted) -> Result<(), DeviceError> {
        let mut properties = self.properties().clone();
        properties.amplifier.coarse_gain = gain;
        self.do_set_properties(properties, admitted)
    }

    /// Current presets.
    fn presets(&self) -> &Presets {
        &self.properties().presets
    }

    /// Sends these presets exactly: a clear and every held preset, whatever
    /// the instrument already holds, and keeps the host-side ones. The
    /// explicit primitive, for "Clear presets" and the batch queue. Refused by
    /// the gate like every preset change: locked, counting (asked of the
    /// instrument), invalid values. Zero is "not set" ([`Presets::normalized`]).
    fn set_presets(&mut self, presets: Presets) -> Result<(), DeviceError> {
        let presets = presets.normalized();
        let admitted = admit(self, Op::Presets(&presets))?;
        self.do_write_presets(presets, admitted)
    }

    /// The one road for every preset edit the operator or a job makes.
    ///
    /// 1. The gate: locked, counting (asked of the instrument, once) and
    ///    invalid values are refused before anything is sent.
    /// 2. If only the uncertainty or MDA presets (the ones this program
    ///    holds) changed, they are stored and nothing is sent.
    /// 3. In every other case, a held preset changed or nothing changed at
    ///    all (the operator re-applies), a clear and every held preset are
    ///    sent.
    fn apply_presets(&mut self, presets: Presets) -> Result<(), DeviceError> {
        let presets = presets.normalized();
        let admitted = admit(self, Op::Presets(&presets))?;
        let mirror = *self.presets();
        if presets.host_differ(&mirror) && !presets.held_differ(&mirror) {
            return self.do_store_host_presets(presets, admitted);
        }
        self.do_write_presets(presets, admitted)
    }

    /// Writes presets: a clear and every held preset, then the host-side
    /// ones once the instrument has accepted everything.
    fn do_write_presets(&mut self, presets: Presets, admitted: Admitted)
    -> Result<(), DeviceError>;

    /// Keeps the host-side presets (uncertainty, MDA) of `presets`; nothing is
    /// sent.
    fn do_store_host_presets(
        &mut self,
        presets: Presets,
        admitted: Admitted,
    ) -> Result<(), DeviceError>;

    /// Whether the detector is counting, asked of the instrument right now:
    /// the only input the gate takes for "is it counting". A network
    /// instrument reads its status; an answer that cannot be had, or is not
    /// a status line with the counting flag, is
    /// [`DeviceError::CountingUnknown`]. The reading also becomes what
    /// [`Self::is_active`] says.
    fn read_counting(&mut self) -> Result<bool, DeviceError>;

    /// Fails with [`DeviceError::Locked`] while another owner holds the
    /// instrument. The gate's first question.
    fn ensure_unlocked(&self) -> Result<(), DeviceError>;

    /// Starts data collection. Refused before anything but one status read is
    /// sent when a held preset is already reached; otherwise it succeeds when
    /// the instrument accepts START, and an ERR fails it at once. Whether it
    /// then counts is for the next reading to say.
    fn start(&mut self) -> Result<(), DeviceError> {
        let admitted = admit(self, Op::Start)?;
        self.do_start(admitted)
    }

    /// The START behind [`Self::start`].
    fn do_start(&mut self, admitted: Admitted) -> Result<(), DeviceError>;

    /// Stops data collection; succeeds only when the detector reports it is
    /// not counting (an idle detector is already stopped).
    fn stop(&mut self) -> Result<(), DeviceError> {
        let admitted = admit(self, Op::Stop)?;
        self.do_stop(admitted)
    }

    /// Stops a count whose preset is reached, locked or not (a lock guards
    /// against the operator's commands, and only someone who could change the
    /// presets set this one). Refused when no preset is reached, so it is no
    /// way around the lock. Called by [`crate::advance`].
    fn stop_at_preset(&mut self) -> Result<(), DeviceError> {
        if self.preset_reached().is_none() {
            return Err(DeviceError::Command {
                command: "STOP".into(),
                detail: "no preset is reached".into(),
            });
        }
        self.do_stop(Admitted(()))
    }

    /// The STOP behind [`Self::stop`] and [`Self::stop_at_preset`].
    fn do_stop(&mut self, admitted: Admitted) -> Result<(), DeviceError>;

    /// Erases the data, real time and live time.
    fn clear(&mut self) -> Result<(), DeviceError> {
        let admitted = admit(self, Op::Clear)?;
        self.do_clear(admitted)
    }

    /// The CLEAR behind [`Self::clear`].
    fn do_clear(&mut self, admitted: Admitted) -> Result<(), DeviceError>;

    /// Whether data collection is running, as last read from the detector.
    /// For display and for [`crate::advance`]; never an input to the gate.
    fn is_active(&self) -> bool;

    /// A status snapshot.
    fn status(&self) -> McbStatus;

    /// The spectrum currently being displayed for this instrument.
    fn spectrum(&self) -> &Spectrum;

    /// Mutable spectrum, for marking regions on the live data.
    ///
    /// Instrument memory is never written back to: undoing a detector command
    /// recovers the snapshot into a buffer window instead, exactly as recalling a
    /// file does.
    fn spectrum_mut(&mut self) -> &mut Spectrum;

    /// Advances the instrument by `elapsed_seconds` of real time.
    fn poll(&mut self, elapsed_seconds: f64) -> Result<(), DeviceError>;

    /// Whether the last [`poll`](Self::poll) changed what [`status`](Self::status)
    /// and [`spectrum`](Self::spectrum) answer.
    ///
    /// The default says yes, which is right for an instrument that counts on
    /// every poll - the simulator does. A mirror over a wire says yes only for
    /// the poll that integrated a fetch, so work keyed to the data - preset
    /// walks over marked channels, an MDA fit - runs when the numbers can
    /// have moved and not sixty times a second because frames were drawn.
    fn poll_freshened(&self) -> bool {
        true
    }

    /// Switches collection mode.
    fn set_mode(&mut self, mode: AcquisitionMode) -> Result<(), DeviceError> {
        let admitted = admit(self, Op::Setting)?;
        self.do_set_mode(mode, admitted)
    }

    /// The change behind [`Self::set_mode`].
    fn do_set_mode(&mut self, mode: AcquisitionMode, admitted: Admitted)
    -> Result<(), DeviceError>;

    /// Current collection mode.
    fn mode(&self) -> AcquisitionMode;

    /// Recorded list-mode events, when the instrument is in list mode.
    fn list_data(&self) -> Option<&ListModeFile> {
        None
    }

    /// The raw, uncorrected spectrum (equal to [`Mcb::spectrum`] unless a
    /// zero-dead-time mode is running).
    fn uncorrected_spectrum(&self) -> &Spectrum {
        self.spectrum()
    }

    /// The uncertainty spectrum that accompanies a zero-dead-time spectrum.
    fn error_spectrum(&self) -> Option<&Spectrum> {
        None
    }

    /// Begins the automatic optimisation routine (MCB Properties, Optimize).
    fn start_optimize(&mut self) -> Result<(), DeviceError> {
        let admitted = admit(self, Op::Setting)?;
        self.do_start_optimize(admitted)
    }

    /// The routine behind [`Self::start_optimize`]; not every instrument has
    /// one.
    fn do_start_optimize(&mut self, _admitted: Admitted) -> Result<(), DeviceError> {
        Err(DeviceError::NotSupported {
            feature: "automatic optimisation".into(),
        })
    }

    /// Whether the optimisation routine is still running.
    fn optimizing(&self) -> bool {
        false
    }

    /// Starts (or stops) the automatic pole-zero routine.
    fn set_pole_zero_running(&mut self, run: bool) -> Result<(), DeviceError> {
        let admitted = admit(self, Op::Setting)?;
        self.do_set_pole_zero_running(run, admitted)
    }

    /// The routine behind [`Self::set_pole_zero_running`].
    fn do_set_pole_zero_running(
        &mut self,
        _run: bool,
        _admitted: Admitted,
    ) -> Result<(), DeviceError> {
        Err(DeviceError::NotSupported {
            feature: "automatic pole zero".into(),
        })
    }

    /// Whether the pole-zero routine is still running.
    fn pole_zeroing(&self) -> bool {
        false
    }

    /// How far the pole-zero setting sits from where this instrument's preamp
    /// needs it, as a signed fraction of full scale. Zero when unknown.
    ///
    /// This is what an oscilloscope on the amplifier output shows as the tail
    /// after each pulse; InSight draws it.
    fn pole_zero_error(&self) -> f64 {
        0.0
    }

    /// How many spectra the instrument holds in its own memory (field mode).
    fn stored_spectra(&self) -> usize {
        0
    }

    /// A stored spectrum, by zero-based index.
    fn stored_spectrum(&self, _index: usize) -> Option<&Spectrum> {
        None
    }

    /// Snapshots the live spectrum into instrument memory and clears the live
    /// data for the next sample, returning how many are now stored.
    fn store_spectrum(&mut self) -> Result<usize, DeviceError> {
        let admitted = admit(self, Op::Setting)?;
        self.do_store_spectrum(admitted)
    }

    /// The store behind [`Self::store_spectrum`].
    fn do_store_spectrum(&mut self, _admitted: Admitted) -> Result<usize, DeviceError> {
        Err(DeviceError::NotSupported {
            feature: "stored spectra".into(),
        })
    }

    /// Erases the instrument's stored spectra (after a download).
    fn erase_stored(&mut self) -> Result<(), DeviceError> {
        let admitted = admit(self, Op::Setting)?;
        self.do_erase_stored(admitted)
    }

    /// The erase behind [`Self::erase_stored`].
    fn do_erase_stored(&mut self, _admitted: Admitted) -> Result<(), DeviceError> {
        Ok(())
    }

    /// Sends a raw hardware command, as the `SEND_MESSAGE` job command does.
    /// A read (`SHOW_...`) passes a lock; anything else needs the detector
    /// unlocked.
    ///
    /// A preset command is a preset change, through [`Self::set_presets`]:
    /// locked, counting and bad values are refused as for any other, and the
    /// mirror follows what was sent.
    fn send_message(&mut self, command: &str) -> Result<String, DeviceError> {
        raw_allowed(command)?;
        // The lock is the gate's first question, before the value is read.
        let word = verb(command);
        if word.starts_with("SET_PRESET") || word == "SET_HV" || word == "SET_GAIN_COARSE" {
            self.ensure_unlocked()?;
        }
        if let Some(presets) = raw_presets(self.presets(), command)? {
            self.set_presets(presets)?;
            return Ok("OK".into());
        }
        // The other commands the typed methods own take the same door, so a
        // raw one is checked, confirmed and mirrored like its typed twin.
        let words: Vec<&str> = command.split_whitespace().collect();
        let value = || -> Result<f64, DeviceError> {
            match words.as_slice() {
                [_, word] => word.parse::<f64>().ok(),
                _ => None,
            }
            .ok_or_else(|| DeviceError::Command {
                command: command.trim().to_string(),
                detail: "the value is not a number; nothing was sent".into(),
            })
        };
        let typed = match (verb(command).as_str(), words.len()) {
            ("START", 1) => Some(self.start()),
            ("STOP", 1) => Some(self.stop()),
            ("CLEAR", 1) => Some(self.clear()),
            ("SET_HV", _) => Some(self.set_high_voltage(value()?)),
            ("SET_GAIN_COARSE", _) => Some(self.set_coarse_gain(value()?)),
            _ => None,
        };
        if let Some(outcome) = typed {
            return outcome.map(|()| "OK".into());
        }
        let admitted = admit(self, Op::Raw(command))?;
        self.do_send_message(command, admitted)
    }

    /// The exchange behind [`Self::send_message`].
    fn do_send_message(&mut self, command: &str, admitted: Admitted)
    -> Result<String, DeviceError>;

    /// Locks the instrument against changes.
    fn lock(&mut self, password: &str, owner: &str) -> Result<(), DeviceError>;

    /// Unlocks the instrument.
    fn unlock(&mut self, password: &str) -> Result<(), DeviceError>;

    /// Whether the instrument is locked.
    fn is_locked(&self) -> bool;

    /// Why the last status reading is not believed (a reply that was not a
    /// status line with ACTIVE, RT and LT), or `None` when it is. The display
    /// keeps the last good values meanwhile; changes are refused by the gate
    /// on their own fresh read.
    fn reading_untrusted(&self) -> Option<&str> {
        None
    }

    /// How many status readings the mirror's polls have worked in so far,
    /// believed or not. It only grows. A count's end is judged on a reading
    /// taken after the START ([`crate::CountWatch`]), and this is how a
    /// watcher tells one from a reading older than its own question. The
    /// gate's own status reads are not counted: they come in bursts (a STOP
    /// confirmation, a refused preset), and the watch's bounds are in polls,
    /// which come at the fetch pace.
    fn readings(&self) -> u64;

    /// Which preset, if any, has been satisfied.
    fn preset_reached(&self) -> Option<PresetKind> {
        let status = self.status();
        self.presets()
            .reached(self.spectrum(), status.real_time, status.live_time)
    }

    /// How close the nearest preset is to stopping the run, 0.0 to 1.0.
    fn preset_progress(&self) -> Option<f64> {
        let status = self.status();
        self.presets()
            .progress(self.spectrum(), status.real_time, status.live_time)
    }
}

#[cfg(test)]
mod seal_tests {
    use std::path::{Path, PathBuf};

    /// The command methods every backend inherits and none may override: an
    /// override would be a road around the gate.
    const GATED: [&str; 15] = [
        "start",
        "stop",
        "stop_at_preset",
        "clear",
        "apply_presets",
        "set_presets",
        "set_properties",
        "set_high_voltage",
        "set_coarse_gain",
        "set_mode",
        "start_optimize",
        "set_pole_zero_running",
        "store_spectrum",
        "erase_stored",
        "send_message",
    ];

    /// Every Rust source in the workspace's crates, tests included.
    fn workspace_sources() -> Vec<PathBuf> {
        fn walk(dir: &Path, out: &mut Vec<PathBuf>) {
            let Ok(entries) = std::fs::read_dir(dir) else {
                return;
            };
            for entry in entries.flatten() {
                let path = entry.path();
                if path.is_dir() {
                    if path.file_name().is_some_and(|name| name != "target") {
                        walk(&path, out);
                    }
                } else if path.extension().is_some_and(|ext| ext == "rs") {
                    out.push(path);
                }
            }
        }
        let crates = Path::new(env!("CARGO_MANIFEST_DIR"))
            .parent()
            .expect("the crates directory")
            .to_path_buf();
        let mut out = Vec::new();
        walk(&crates, &mut out);
        out
    }

    /// The source with comments, string literals and character literals
    /// blanked, so that only code is searched (lengths are kept).
    fn code_only(source: &str) -> String {
        let chars: Vec<char> = source.chars().collect();
        let mut out = String::with_capacity(source.len());
        let mut i = 0;
        while i < chars.len() {
            let c = chars[i];
            let next = chars.get(i + 1).copied();
            if c == '/' && next == Some('/') {
                while i < chars.len() && chars[i] != '\n' {
                    out.push(' ');
                    i += 1;
                }
            } else if c == '/' && next == Some('*') {
                let mut depth = 0;
                while i < chars.len() {
                    if chars[i] == '/' && chars.get(i + 1) == Some(&'*') {
                        depth += 1;
                        out.push_str("  ");
                        i += 2;
                    } else if chars[i] == '*' && chars.get(i + 1) == Some(&'/') {
                        depth -= 1;
                        out.push_str("  ");
                        i += 2;
                        if depth == 0 {
                            break;
                        }
                    } else {
                        out.push(' ');
                        i += 1;
                    }
                }
            } else if c == 'r' && (next == Some('"') || next == Some('#')) {
                // A raw string: r"...", r#"..."#.
                let mut j = i + 1;
                let mut hashes = 0;
                while chars.get(j) == Some(&'#') {
                    hashes += 1;
                    j += 1;
                }
                if chars.get(j) != Some(&'"') {
                    out.push(c);
                    i += 1;
                    continue;
                }
                j += 1;
                loop {
                    match chars.get(j) {
                        None => break,
                        Some('"') if (1..=hashes).all(|k| chars.get(j + k) == Some(&'#')) => {
                            j += 1 + hashes;
                            break;
                        }
                        _ => j += 1,
                    }
                }
                out.push_str(&" ".repeat(j - i));
                i = j;
            } else if c == '"' {
                out.push(' ');
                i += 1;
                while i < chars.len() && chars[i] != '"' {
                    let skip = if chars[i] == '\\' { 2 } else { 1 };
                    out.push_str(&" ".repeat(skip));
                    i += skip;
                }
                out.push(' ');
                i += 1;
            } else if c == '\'' && (chars.get(i + 2) == Some(&'\'') || next == Some('\\')) {
                // A character literal ('{', '\n'); a lifetime has no closing
                // quote two characters on.
                let mut j = i + 1;
                if chars.get(j) == Some(&'\\') {
                    j += 2;
                }
                while j < chars.len() && chars[j] != '\'' {
                    j += 1;
                }
                out.push_str(&" ".repeat(j + 1 - i));
                i = j + 1;
            } else {
                out.push(c);
                i += 1;
            }
        }
        out
    }

    /// The bodies of every `impl ... Mcb for ...` block in `code`.
    fn mcb_impl_bodies(code: &str) -> Vec<String> {
        let chars: Vec<char> = code.chars().collect();
        let word_at = |at: usize, word: &str| {
            let is_ident = |c: char| c.is_alphanumeric() || c == '_';
            code_at(&chars, at, word)
                && (at == 0 || !is_ident(chars[at - 1]))
                && chars.get(at + word.len()).is_none_or(|c| !is_ident(*c))
        };
        let mut bodies = Vec::new();
        for at in 0..chars.len() {
            if !word_at(at, "impl") {
                continue;
            }
            let Some(open) = (at..chars.len()).find(|&k| chars[k] == '{' || chars[k] == ';') else {
                continue;
            };
            if chars[open] != '{' {
                continue;
            }
            let header: String = chars[at..open].iter().collect();
            let words: Vec<&str> = header.split_whitespace().collect();
            let implements_mcb = words
                .windows(2)
                .any(|pair| (pair[0] == "Mcb" || pair[0].ends_with("::Mcb")) && pair[1] == "for");
            if !implements_mcb {
                continue;
            }
            let mut depth = 0;
            let mut close = open;
            for (k, c) in chars.iter().enumerate().skip(open) {
                match c {
                    '{' => depth += 1,
                    '}' => {
                        depth -= 1;
                        if depth == 0 {
                            close = k;
                            break;
                        }
                    }
                    _ => {}
                }
            }
            bodies.push(chars[open..=close].iter().collect());
        }
        bodies
    }

    fn code_at(chars: &[char], at: usize, word: &str) -> bool {
        word.chars()
            .enumerate()
            .all(|(k, c)| chars.get(at + k) == Some(&c))
    }

    /// The methods an impl body defines: every identifier after `fn`,
    /// however the code is laid out.
    fn defined(body: &str) -> Vec<String> {
        let words: Vec<&str> = body
            .split(|c: char| !(c.is_alphanumeric() || c == '_'))
            .filter(|word| !word.is_empty())
            .collect();
        words
            .windows(2)
            .filter(|pair| pair[0] == "fn")
            .map(|pair| pair[1].to_string())
            .collect()
    }

    /// T28: no implementation of [`super::Mcb`] anywhere in the workspace
    /// defines a gated method, so every changing command passes `admit`.
    /// Read structurally (every `impl Mcb for` block, every `fn` in it, any
    /// layout), not by a line pattern in a fixed list of files.
    #[test]
    fn no_implementation_overrides_a_gated_method() {
        let mut implementations = 0;
        for path in workspace_sources() {
            let source = std::fs::read_to_string(&path).expect("a source file");
            for body in mcb_impl_bodies(&code_only(&source)) {
                implementations += 1;
                for method in defined(&body) {
                    assert!(
                        !GATED.contains(&method.as_str()),
                        "{} defines `{method}` in an `impl Mcb`, a road around the gate",
                        path.display()
                    );
                }
            }
        }
        assert!(
            implementations >= 3,
            "the simulator, the network instrument and AnyMcb were found: {implementations}"
        );
    }

    /// The reader itself: an override laid out oddly, behind a path, or
    /// after a brace in a string is still found; one in a comment is not.
    #[test]
    fn the_reader_finds_an_override_however_it_is_written() {
        let odd = "impl<T> crate::mcb::Mcb\n for Odd<T> where T: Send {\n  \
                   fn  name(&self) { let _ = \"}\"; let _ = '{'; }\n  \
                   // fn clear(\n  \
                   fn\n   start\n (&mut self) {}\n}";
        let bodies = mcb_impl_bodies(&code_only(odd));
        assert_eq!(bodies.len(), 1);
        assert_eq!(defined(&bodies[0]), vec!["name", "start"]);
        assert!(mcb_impl_bodies(&code_only("impl Other for Mcb {}")).is_empty());
    }

    /// T28: the token is made in exactly three places in this file - its
    /// definition, the gate, the preset stop. Its field is private to this
    /// module, so no other module can make one at all (the compiler holds
    /// that; see the `compile_fail` examples on [`super::Admitted`]).
    #[test]
    fn only_the_gate_and_the_preset_stop_make_the_token() {
        let source = include_str!("mcb.rs");
        let (code, _tests) = source.split_once("mod seal_tests").expect("this module");
        let squeezed: String = code_only(code)
            .chars()
            .filter(|c| !c.is_whitespace())
            .collect();
        assert_eq!(
            squeezed.matches("Admitted(())").count(),
            3,
            "the token's definition, the gate, the preset stop"
        );
    }
}
