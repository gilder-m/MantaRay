//! The multichannel buffer abstraction: what MAESTRO calls a Detector.
//!
//! Everything the user interface needs from an instrument sits behind [`Mcb`], so
//! a simulated detector, a file replay or a real ORTEC unit all look the same.

use ortseam_core::{AcquisitionMode, Spectrum};
use ortseam_formats::ListModeFile;
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

/// A multichannel buffer: an instrument that collects a spectrum.
pub trait Mcb: Send {
    /// Fixed facts about the instrument.
    fn identity(&self) -> &McbIdentity;

    /// Current settings.
    fn properties(&self) -> &McbProperties;

    /// Applies settings. Fails while locked, or on out-of-range values.
    fn set_properties(&mut self, properties: McbProperties) -> Result<(), DeviceError>;

    /// Current presets.
    fn presets(&self) -> &Presets {
        &self.properties().presets
    }

    /// Direct access to the presets, for setup before a run.
    fn presets_mut(&mut self) -> &mut Presets;

    /// Applies presets. Fails while the instrument is counting.
    fn set_presets(&mut self, presets: Presets) -> Result<(), DeviceError>;

    /// Starts data collection.
    fn start(&mut self) -> Result<(), DeviceError>;

    /// Stops data collection.
    fn stop(&mut self) -> Result<(), DeviceError>;

    /// Erases the data, real time and live time.
    fn clear(&mut self) -> Result<(), DeviceError>;

    /// Whether data collection is running.
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

    /// Switches collection mode.
    fn set_mode(&mut self, mode: AcquisitionMode) -> Result<(), DeviceError>;

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
        Err(DeviceError::NotSupported {
            feature: "automatic optimisation".into(),
        })
    }

    /// Whether the optimisation routine is still running.
    fn optimizing(&self) -> bool {
        false
    }

    /// Starts (or stops) the automatic pole-zero routine.
    fn set_pole_zero_running(&mut self, _run: bool) -> Result<(), DeviceError> {
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
        Err(DeviceError::NotSupported {
            feature: "stored spectra".into(),
        })
    }

    /// Erases the instrument's stored spectra (after a download).
    fn erase_stored(&mut self) {}

    /// Sends a raw hardware command, as the `SEND_MESSAGE` job command does.
    fn send_message(&mut self, command: &str) -> Result<String, DeviceError>;

    /// Locks the instrument against changes.
    fn lock(&mut self, password: &str, owner: &str) -> Result<(), DeviceError>;

    /// Unlocks the instrument.
    fn unlock(&mut self, password: &str) -> Result<(), DeviceError>;

    /// Whether the instrument is locked.
    fn is_locked(&self) -> bool;

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
