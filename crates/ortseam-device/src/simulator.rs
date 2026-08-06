//! A physics-based detector simulator.
//!
//! It is a real instrument as far as [`Mcb`] is concerned: it has gain, an ADC
//! with discriminators, presets, dead time, list mode and zero-dead-time modes.
//! Counting statistics come from a seeded generator, so a run is reproducible.
//!
//! The model, per second of live time:
//!
//! * every [`SourceLine`] delivers `cps` interactions, of which
//!   [`SimulatedSource::photopeak_fraction`] land in the full-energy peak and the
//!   rest spread over that line's Compton continuum, up to the Compton edge
//!   `2E^2/(511 + 2E)`;
//! * [`SimulatedSource::continuum_cps`] events form a falling exponential
//!   continuum, standing in for room background and bremsstrahlung;
//! * peak energies are smeared with the detector resolution
//!   `FWHM = a + b*sqrt(E)`, published as the spectrum's shape calibration;
//! * dead time is non-paralysable with a per-event dead time
//!   [`SimulatedSource::dead_time_us`], so throughput saturates at high rates.

use ortseam_core::{AcquisitionMode, EnergyCalibration, ShapeCalibration, Spectrum};
use ortseam_formats::ListModeFile;

use crate::error::DeviceError;
use crate::mcb::{
    AdcSettings, AmplifierSettings, Capabilities, HighVoltageSettings, Mcb, McbIdentity,
    McbProperties, McbStatus, StabilizerSettings,
};
use crate::presets::Presets;
use crate::rng::Rng;

/// One gamma line of the simulated source.
#[derive(Clone, Copy, Debug, PartialEq)]
pub struct SourceLine {
    /// Line energy in keV.
    pub energy: f64,
    /// Interactions per second contributed by this line.
    pub cps: f64,
}

impl SourceLine {
    /// A line at an energy with an interaction rate.
    pub fn new(energy: f64, cps: f64) -> Self {
        Self { energy, cps }
    }
}

/// What the simulated detector is looking at.
#[derive(Clone, Debug, PartialEq)]
pub struct SimulatedSource {
    /// Gamma lines.
    pub lines: Vec<SourceLine>,
    /// Continuum events per second.
    pub continuum_cps: f64,
    /// Decay constant of the continuum shape, in keV.
    pub continuum_decay_kev: f64,
    /// Fraction of a line's interactions that land in the full-energy peak.
    pub photopeak_fraction: f64,
    /// Dead time per event, in microseconds.
    pub dead_time_us: f64,
    /// Resolution constant term, in keV.
    pub resolution_constant_kev: f64,
    /// Resolution square-root term, in keV per sqrt(keV).
    pub resolution_sqrt_kev: f64,
}

impl Default for SimulatedSource {
    fn default() -> Self {
        Self {
            lines: vec![
                SourceLine::new(661.657, 200.0),
                SourceLine::new(1173.228, 90.0),
                SourceLine::new(1332.492, 80.0),
                SourceLine::new(1460.822, 20.0),
            ],
            continuum_cps: 150.0,
            continuum_decay_kev: 350.0,
            photopeak_fraction: 0.4,
            dead_time_us: 5.0,
            resolution_constant_kev: 0.9,
            resolution_sqrt_kev: 0.028,
        }
    }
}

impl SimulatedSource {
    /// Total interaction rate at the input.
    pub fn input_rate(&self) -> f64 {
        self.continuum_cps + self.lines.iter().map(|line| line.cps).sum::<f64>()
    }

    /// Detector resolution in keV at an energy.
    pub fn fwhm_kev(&self, energy: f64) -> f64 {
        self.resolution_constant_kev + self.resolution_sqrt_kev * energy.max(0.0).sqrt()
    }
}

/// Builder for [`SimulatedMcb`].
#[derive(Clone, Debug)]
pub struct SimulatedMcbBuilder {
    number: u16,
    name: String,
    model: String,
    channels: usize,
    gain_kev_per_channel: f64,
    source: SimulatedSource,
    seed: u64,
}

impl SimulatedMcbBuilder {
    /// Memory size in channels.
    pub fn channels(mut self, channels: usize) -> Self {
        self.channels = channels.clamp(64, 65_536);
        self
    }

    /// Energy per channel, which sets the calibration.
    pub fn gain_kev_per_channel(mut self, gain: f64) -> Self {
        if gain > 0.0 {
            self.gain_kev_per_channel = gain;
        }
        self
    }

    /// What the detector is looking at.
    pub fn source(mut self, source: SimulatedSource) -> Self {
        self.source = source;
        self
    }

    /// Model name reported on the About tab.
    pub fn model(mut self, model: &str) -> Self {
        self.model = model.to_string();
        self
    }

    /// Seed of the counting statistics.
    pub fn seed(mut self, seed: u64) -> Self {
        self.seed = seed;
        self
    }

    /// Builds the detector.
    pub fn build(self) -> SimulatedMcb {
        let gain = self.gain_kev_per_channel;
        let mut identity = McbIdentity {
            number: self.number,
            name: self.name,
            model: self.model,
            serial: format!("SIM-{:04}", self.number),
            firmware: concat!("ortseam simulator ", env!("CARGO_PKG_VERSION")).to_string(),
            description: String::new(),
            channels: self.channels,
            capabilities: Capabilities {
                max_channels: self.channels,
                stored_spectra: true,
                ..Capabilities::default()
            },
        };
        identity.description = format!("{} simulated HPGe", identity.model);

        let properties = McbProperties {
            amplifier: AmplifierSettings::default(),
            adc: AdcSettings {
                conversion_gain: self.channels,
                lower_discriminator: 0,
                upper_discriminator: self.channels as u32 - 1,
                ..AdcSettings::default()
            },
            high_voltage: HighVoltageSettings::default(),
            stabilizer: StabilizerSettings::default(),
            presets: Presets::default(),
        };

        let mut spectrum = Spectrum::new(self.channels);
        spectrum.detector_id = self.number;
        spectrum.detector_name = identity.name.clone();
        spectrum.detector_description = identity.description.clone();
        spectrum.energy_calibration = Some(EnergyCalibration::linear(0.0, gain));
        spectrum.shape_calibration = Some(fit_shape(&self.source, gain, self.channels));

        SimulatedMcb {
            identity,
            properties,
            source: self.source,
            gain_kev_per_channel: gain,
            raw: spectrum.clone(),
            corrected: spectrum.clone(),
            error: spectrum,
            list: None,
            mode: AcquisitionMode::Pha,
            active: false,
            real_time: 0.0,
            live_time: 0.0,
            stored_counts: 0,
            rng: Rng::new(self.seed),
            seed: self.seed,
            lock: None,
            sample_ready: true,
            output_high: false,
            stored: Vec::new(),
            tuning: Tuning::Idle,
            // Each simulated preamp has its own right answer, so Optimize has
            // something real to find.
            true_pole_zero: 1600 + (self.seed % 800) as u16,
        }
    }
}

/// Approximates the physical `a + b*sqrt(E)` resolution as the polynomial in
/// channel that instruments store, by fitting three points across the range.
fn fit_shape(source: &SimulatedSource, gain: f64, channels: usize) -> ShapeCalibration {
    let points: Vec<(f64, f64)> = [0.1, 0.5, 1.0]
        .iter()
        .map(|fraction| {
            let channel = fraction * channels as f64;
            let energy = channel * gain;
            (channel, source.fwhm_kev(energy) / gain)
        })
        .collect();
    ShapeCalibration::fit(&points).unwrap_or_else(|_| {
        ShapeCalibration::new([source.resolution_constant_kev / gain, 0.0, 0.0])
    })
}

/// A simulated multichannel buffer.
#[derive(Clone, Debug)]
pub struct SimulatedMcb {
    identity: McbIdentity,
    properties: McbProperties,
    source: SimulatedSource,
    gain_kev_per_channel: f64,
    raw: Spectrum,
    corrected: Spectrum,
    error: Spectrum,
    list: Option<ListModeFile>,
    mode: AcquisitionMode,
    active: bool,
    real_time: f64,
    live_time: f64,
    stored_counts: u64,
    rng: Rng,
    seed: u64,
    lock: Option<(String, String)>,
    sample_ready: bool,
    output_high: bool,
    /// Spectra stored in instrument memory (field mode).
    stored: Vec<Spectrum>,
    /// The automatic routine in progress, if any.
    tuning: Tuning,
    /// Where the pole zero belongs for this detector's (simulated) preamp.
    true_pole_zero: u16,
}

/// The automatic tuning routines an instrument can be running.
#[derive(Clone, Copy, Debug, PartialEq)]
enum Tuning {
    /// Nothing running.
    Idle,
    /// The full optimisation, with simulated seconds left.
    Optimize { remaining: f64 },
    /// The automatic pole zero, with simulated seconds left.
    PoleZero { remaining: f64 },
}

/// How long the routines take, in simulated seconds - real units take about
/// this long over a minute of pulses.
const OPTIMIZE_SECONDS: f64 = 8.0;
const POLE_ZERO_SECONDS: f64 = 4.0;

impl SimulatedMcb {
    /// Starts a builder for a detector with a number and a name.
    pub fn builder(number: u16, name: &str) -> SimulatedMcbBuilder {
        SimulatedMcbBuilder {
            number,
            name: name.to_string(),
            model: "SIM-HPGe".to_string(),
            channels: 8_192,
            gain_kev_per_channel: 0.5,
            source: SimulatedSource::default(),
            seed: 0x5EED,
        }
    }

    /// A detector with the default source and settings.
    pub fn new(number: u16, name: &str) -> Self {
        Self::builder(number, name).build()
    }

    /// What the detector is looking at.
    pub fn source(&self) -> &SimulatedSource {
        &self.source
    }

    /// Replaces the source, so the interface can offer "change sample".
    pub fn set_source(&mut self, source: SimulatedSource) {
        self.source = source;
        let shape = fit_shape(&self.source, self.gain_kev_per_channel, self.raw.len());
        for spectrum in [&mut self.raw, &mut self.corrected, &mut self.error] {
            spectrum.shape_calibration = Some(shape);
        }
    }

    /// Restarts the random sequence, so a repeated run repeats exactly.
    pub fn reseed(&mut self, seed: u64) {
        self.seed = seed;
        self.rng = Rng::new(seed);
    }

    /// Whether the sample-changer output line is asserted.
    pub fn output_high(&self) -> bool {
        self.output_high
    }

    /// Sets the sample-ready input, as a changer would.
    pub fn set_sample_ready(&mut self, ready: bool) {
        self.sample_ready = ready;
    }

    fn ensure_unlocked(&self) -> Result<(), DeviceError> {
        match &self.lock {
            Some((_, owner)) => Err(DeviceError::Locked {
                owner: owner.clone(),
            }),
            None => Ok(()),
        }
    }

    /// What every road to the presets must check: unlocked, and not counting.
    fn presets_busy_check(&self, command: &str) -> Result<(), DeviceError> {
        self.ensure_unlocked()?;
        if self.active {
            return Err(DeviceError::Busy {
                detail: format!("{command}: presets cannot be changed while counting"),
            });
        }
        Ok(())
    }

    fn dead_fraction(&self) -> f64 {
        let rate = self.source.input_rate();
        let tau = self.source.dead_time_us * 1e-6;
        let product = rate * tau;
        if product <= 0.0 {
            0.0
        } else {
            product / (1.0 + product)
        }
    }

    /// Samples one event's channel, including resolution smearing.
    fn sample_channel(&mut self, energy: f64) -> Option<usize> {
        if energy <= 0.0 {
            return None;
        }
        let centre = energy / self.gain_kev_per_channel;
        let fwhm_channels = self.source.fwhm_kev(energy) / self.gain_kev_per_channel;
        let sigma = (fwhm_channels / ortseam_core::FWHM_PER_SIGMA).max(0.35);
        let smeared = centre + sigma * self.rng.normal();
        if smeared < 0.0 {
            return None;
        }
        Some(smeared.round() as usize)
    }

    /// True when the channel passes the discriminators and fits in memory.
    fn accepts(&self, channel: usize) -> bool {
        let adc = &self.properties.adc;
        let limit = self.raw.len().min(adc.conversion_gain);
        channel < limit
            && channel as u32 >= adc.lower_discriminator
            && channel as u32 <= adc.upper_discriminator
    }

    /// Compton edge of a full-energy line.
    fn compton_edge(energy: f64) -> f64 {
        2.0 * energy * energy / (511.0 + 2.0 * energy)
    }

    fn resize_spectra(&mut self, channels: usize) {
        for spectrum in [&mut self.raw, &mut self.corrected, &mut self.error] {
            spectrum.resize(channels);
        }
        if let Some(list) = &mut self.list {
            list.channel_count = channels;
        }
    }

    fn active_spectrum(&self) -> &Spectrum {
        match self.mode {
            AcquisitionMode::Zdt | AcquisitionMode::Ltc => &self.corrected,
            _ => &self.raw,
        }
    }
}

impl Mcb for SimulatedMcb {
    fn identity(&self) -> &McbIdentity {
        &self.identity
    }

    fn set_name(&mut self, name: &str) {
        self.identity.name = name.to_string();
        // Every spectrum this instrument holds carries the detector's name
        // into a saved file, so all three follow the rename.
        for spectrum in [&mut self.raw, &mut self.corrected, &mut self.error] {
            spectrum.detector_name = name.to_string();
            spectrum.detector_description = name.to_string();
        }
    }

    fn properties(&self) -> &McbProperties {
        &self.properties
    }

    fn set_properties(&mut self, properties: McbProperties) -> Result<(), DeviceError> {
        self.ensure_unlocked()?;
        if properties.adc.conversion_gain > self.identity.channels {
            return Err(DeviceError::OutOfRange {
                setting: "conversion gain".into(),
                detail: format!(
                    "{} channels exceeds the {} channel memory",
                    properties.adc.conversion_gain, self.identity.channels
                ),
            });
        }
        if properties.adc.conversion_gain < 64 {
            return Err(DeviceError::OutOfRange {
                setting: "conversion gain".into(),
                detail: "at least 64 channels are needed".into(),
            });
        }
        if self.active && properties.presets != self.properties.presets {
            return Err(DeviceError::Busy {
                detail: "presets cannot be changed while counting".into(),
            });
        }
        let resize = properties.adc.conversion_gain != self.properties.adc.conversion_gain;
        let gain = properties.adc.conversion_gain;
        self.properties = properties;
        if resize {
            self.resize_spectra(gain);
        }
        Ok(())
    }

    fn presets_mut(&mut self) -> &mut Presets {
        &mut self.properties.presets
    }

    fn set_presets(&mut self, presets: Presets) -> Result<(), DeviceError> {
        self.ensure_unlocked()?;
        if self.active {
            return Err(DeviceError::Busy {
                detail: "presets cannot be changed while counting".into(),
            });
        }
        self.properties.presets = presets;
        Ok(())
    }

    fn start(&mut self) -> Result<(), DeviceError> {
        self.ensure_unlocked()?;
        self.active = true;
        if self.raw.start_time.is_none() {
            self.raw.start_time = Some(chrono::Local::now().naive_local());
            self.corrected.start_time = self.raw.start_time;
            self.error.start_time = self.raw.start_time;
        }
        Ok(())
    }

    fn stop(&mut self) -> Result<(), DeviceError> {
        self.ensure_unlocked()?;
        self.active = false;
        Ok(())
    }

    fn clear(&mut self) -> Result<(), DeviceError> {
        self.ensure_unlocked()?;
        self.raw.clear();
        self.corrected.clear();
        self.error.clear();
        self.real_time = 0.0;
        self.live_time = 0.0;
        self.stored_counts = 0;
        if let Some(list) = &mut self.list {
            // Clearing empties the events, not the identity: the description
            // and calibration set_mode gave the file must survive, as they do
            // on the histogram side.
            let mut fresh = ListModeFile::new(list.channel_count);
            fresh.detector_id = self.identity.number;
            fresh.detector_description = self.identity.description.clone();
            fresh.energy_calibration = self.raw.energy_calibration.clone();
            self.list = Some(fresh);
        }
        Ok(())
    }

    fn is_active(&self) -> bool {
        self.active
    }

    fn status(&self) -> McbStatus {
        let dead = self.dead_fraction() * 100.0;
        McbStatus {
            real_time: self.real_time,
            live_time: self.live_time,
            dead_time_percent: if self.real_time > 0.0 { dead } else { 0.0 },
            input_count_rate: self.source.input_rate(),
            total_count_rate: if self.real_time > 0.0 {
                self.stored_counts as f64 / self.real_time
            } else {
                0.0
            },
            active: self.active,
            high_voltage: if self.properties.high_voltage.enabled {
                self.properties.high_voltage.target_volts
            } else {
                0.0
            },
            sample_ready: self.sample_ready,
            total_counts: self.stored_counts,
        }
    }

    fn spectrum(&self) -> &Spectrum {
        self.active_spectrum()
    }

    fn spectrum_mut(&mut self) -> &mut Spectrum {
        match self.mode {
            AcquisitionMode::Zdt | AcquisitionMode::Ltc => &mut self.corrected,
            _ => &mut self.raw,
        }
    }

    fn poll(&mut self, elapsed_seconds: f64) -> Result<(), DeviceError> {
        // The tuning routines run on the instrument's clock whether or not it
        // is counting.
        if elapsed_seconds > 0.0 {
            match &mut self.tuning {
                Tuning::Idle => {}
                Tuning::Optimize { remaining } => {
                    *remaining -= elapsed_seconds;
                    if *remaining <= 0.0 {
                        self.properties.amplifier.pole_zero = self.true_pole_zero;
                        self.tuning = Tuning::Idle;
                    }
                }
                Tuning::PoleZero { remaining } => {
                    *remaining -= elapsed_seconds;
                    if *remaining <= 0.0 {
                        self.properties.amplifier.pole_zero = self.true_pole_zero;
                        self.tuning = Tuning::Idle;
                    }
                }
            }
        }
        if !self.active || elapsed_seconds <= 0.0 {
            return Ok(());
        }
        let dead_fraction = self.dead_fraction();
        let live_increment = elapsed_seconds * (1.0 - dead_fraction);
        let correction = if dead_fraction < 1.0 {
            1.0 / (1.0 - dead_fraction)
        } else {
            1.0
        };

        // Collect this poll's events, then apply them in time order so list-mode
        // data is monotonic.
        let mut events: Vec<(f64, usize)> = Vec::new();
        let start_time = self.real_time;

        let lines = self.source.lines.clone();
        let photopeak = self.source.photopeak_fraction.clamp(0.0, 1.0);
        for line in &lines {
            let total = self.rng.poisson(line.cps * live_increment);
            for _ in 0..total {
                let in_peak = self.rng.uniform() < photopeak;
                let energy = if in_peak {
                    line.energy
                } else {
                    let edge = Self::compton_edge(line.energy);
                    self.rng
                        .range(0.02 * line.energy, edge.max(0.03 * line.energy))
                };
                if let Some(channel) = self.sample_channel(energy)
                    && self.accepts(channel)
                {
                    let offset = self.rng.uniform() * elapsed_seconds;
                    events.push((start_time + offset, channel));
                }
            }
        }

        let continuum = self.rng.poisson(self.source.continuum_cps * live_increment);
        let decay = self.source.continuum_decay_kev.max(1.0);
        let top_energy = self.raw.len() as f64 * self.gain_kev_per_channel;
        for _ in 0..continuum {
            // Exponential continuum, truncated to the spectrum's energy range.
            let u = self.rng.uniform().clamp(1e-12, 1.0 - 1e-12);
            let energy = (-decay * u.ln()).min(top_energy);
            if let Some(channel) = self.sample_channel(energy)
                && self.accepts(channel)
            {
                let offset = self.rng.uniform() * elapsed_seconds;
                events.push((start_time + offset, channel));
            }
        }

        events.sort_by(|a, b| a.0.partial_cmp(&b.0).unwrap_or(std::cmp::Ordering::Equal));

        let zdt = matches!(self.mode, AcquisitionMode::Zdt | AcquisitionMode::Ltc);
        for (time, channel) in &events {
            self.raw.add_count(*channel);
            self.stored_counts += 1;
            if zdt {
                // Zero-dead-time weighting: each event stands for 1/live-fraction
                // events, and the error spectrum accumulates the variance.
                let weight = correction;
                let whole = weight.floor();
                let fraction = weight - whole;
                let mut counts = whole as u64;
                if self.rng.uniform() < fraction {
                    counts += 1;
                }
                self.corrected.add_counts(*channel, counts);
                let variance = (weight * weight).round().max(1.0) as u64;
                self.error.add_counts(*channel, variance);
            }
            if let Some(list) = &mut self.list {
                list.push(*time, *channel as u32);
            }
        }

        self.real_time += elapsed_seconds;
        self.live_time += live_increment;
        for spectrum in [&mut self.raw, &mut self.corrected, &mut self.error] {
            spectrum.real_time = self.real_time;
            spectrum.live_time = self.live_time;
        }
        if let Some(list) = &mut self.list {
            list.real_time = self.real_time;
            list.live_time = self.live_time;
        }
        Ok(())
    }

    fn set_mode(&mut self, mode: AcquisitionMode) -> Result<(), DeviceError> {
        self.ensure_unlocked()?;
        match mode {
            AcquisitionMode::List if !self.identity.capabilities.list_mode => {
                return Err(DeviceError::NotSupported {
                    feature: "list mode".into(),
                });
            }
            AcquisitionMode::Zdt | AcquisitionMode::Ltc | AcquisitionMode::Err
                if !self.identity.capabilities.zdt =>
            {
                return Err(DeviceError::NotSupported {
                    feature: "zero dead time".into(),
                });
            }
            _ => {}
        }
        self.mode = mode;
        if mode == AcquisitionMode::List {
            let mut list = ListModeFile::new(self.raw.len());
            list.detector_id = self.identity.number;
            list.detector_description = self.identity.description.clone();
            list.energy_calibration = self.raw.energy_calibration.clone();
            list.real_time = self.real_time;
            list.live_time = self.live_time;
            self.list = Some(list);
        } else {
            self.list = None;
        }
        for spectrum in [&mut self.raw, &mut self.corrected, &mut self.error] {
            spectrum.mode = mode;
        }
        self.error.mode = AcquisitionMode::Err;
        Ok(())
    }

    fn mode(&self) -> AcquisitionMode {
        self.mode
    }

    fn list_data(&self) -> Option<&ListModeFile> {
        self.list.as_ref()
    }

    fn uncorrected_spectrum(&self) -> &Spectrum {
        &self.raw
    }

    fn error_spectrum(&self) -> Option<&Spectrum> {
        matches!(self.mode, AcquisitionMode::Zdt | AcquisitionMode::Ltc).then_some(&self.error)
    }

    fn start_optimize(&mut self) -> Result<(), DeviceError> {
        self.ensure_unlocked()?;
        if self.active {
            return Err(DeviceError::Busy {
                detail: "stop the acquisition before optimising".into(),
            });
        }
        self.tuning = Tuning::Optimize {
            remaining: OPTIMIZE_SECONDS,
        };
        Ok(())
    }

    fn optimizing(&self) -> bool {
        matches!(self.tuning, Tuning::Optimize { .. })
    }

    fn set_pole_zero_running(&mut self, run: bool) -> Result<(), DeviceError> {
        self.ensure_unlocked()?;
        if run {
            if self.active {
                return Err(DeviceError::Busy {
                    detail: "stop the acquisition before the pole zero".into(),
                });
            }
            self.tuning = Tuning::PoleZero {
                remaining: POLE_ZERO_SECONDS,
            };
        } else if matches!(self.tuning, Tuning::PoleZero { .. }) {
            self.tuning = Tuning::Idle;
        }
        Ok(())
    }

    fn pole_zeroing(&self) -> bool {
        matches!(self.tuning, Tuning::PoleZero { .. })
    }

    fn pole_zero_error(&self) -> f64 {
        (self.properties.amplifier.pole_zero as f64 - self.true_pole_zero as f64) / 4096.0
    }

    fn stored_spectra(&self) -> usize {
        self.stored.len()
    }

    fn stored_spectrum(&self, index: usize) -> Option<&Spectrum> {
        self.stored.get(index)
    }

    fn store_spectrum(&mut self) -> Result<usize, DeviceError> {
        if self.is_active() {
            return Err(DeviceError::Busy {
                detail: "stop the acquisition before storing".into(),
            });
        }
        // The clear at the end refuses a locked instrument; refusing here
        // keeps store-and-clear one action, not a store that half happened.
        self.ensure_unlocked()?;
        let mut snapshot = self.spectrum().clone();
        snapshot.real_time = self.real_time;
        snapshot.live_time = self.live_time;
        self.stored.push(snapshot);
        self.clear()?;
        Ok(self.stored.len())
    }

    fn erase_stored(&mut self) {
        self.stored.clear();
    }

    fn send_message(&mut self, command: &str) -> Result<String, DeviceError> {
        let trimmed = command.trim();
        let (verb, argument) = match trimmed.split_once(char::is_whitespace) {
            Some((verb, rest)) => (verb.to_ascii_uppercase(), rest.trim().to_string()),
            None => (trimmed.to_ascii_uppercase(), String::new()),
        };
        let number = || -> Result<f64, DeviceError> {
            argument.parse::<f64>().map_err(|_| DeviceError::Command {
                command: trimmed.to_string(),
                detail: format!("{argument:?} is not a number"),
            })
        };
        match verb.as_str() {
            "SHOW_STATUS" => {
                let status = self.status();
                Ok(format!(
                    "RT={:.2} LT={:.2} DT={:.1}% ICR={:.0} ACTIVE={} TOTAL={}",
                    status.real_time,
                    status.live_time,
                    status.dead_time_percent,
                    status.input_count_rate,
                    status.active as u8,
                    status.total_counts
                ))
            }
            "SHOW_VERSION" => Ok(self.identity.firmware.clone()),
            // The presets this instrument is holding, in the same shape the
            // bridge reports a real one's registers. Zero means none set.
            // Uncertainty and MDA are not here: no instrument carries them,
            // they are worked out host-side from the spectrum.
            "SHOW_PRESETS" => {
                let presets = &self.properties.presets;
                Ok(format!(
                    "PRESETS REAL={:.2} LIVE={:.2} PEAK={} INTEG={}",
                    presets.real_time.unwrap_or(0.0),
                    presets.live_time.unwrap_or(0.0),
                    presets.roi_peak.unwrap_or(0),
                    presets.roi_integral.unwrap_or(0)
                ))
            }
            "SHOW_CONFIGURATION" => Ok(format!(
                "MODEL={} SERIAL={} FIRMWARE={} CHANNELS={}",
                self.identity.model.replace(' ', "_"),
                self.identity.serial.replace(' ', "_"),
                self.identity.firmware.replace(' ', "_"),
                self.identity.channels
            )),
            "SHOW_DATA" => {
                // The whole histogram on one line: `DATA n c0 c1 ...`.
                let spectrum = self.spectrum();
                let mut line = format!("DATA {}", spectrum.len());
                for count in &spectrum.channels {
                    line.push(' ');
                    line.push_str(&count.to_string());
                }
                Ok(line)
            }
            "START" => self.start().map(|_| "OK".to_string()),
            "STOP" => self.stop().map(|_| "OK".to_string()),
            "CLEAR" => self.clear().map(|_| "OK".to_string()),
            "STORE" => self
                .store_spectrum()
                .map(|count| format!("OK {count} stored")),
            "SHOW_STORED" => Ok(format!("STORED={}", self.stored.len())),
            "START_OPTIMIZE" => self.start_optimize().map(|_| "OK".to_string()),
            "START_PZ" => self.set_pole_zero_running(true).map(|_| "OK".to_string()),
            "STOP_PZ" => self.set_pole_zero_running(false).map(|_| "OK".to_string()),
            "SHOW_PZ" => Ok(format!(
                "PZ={} RUNNING={}",
                self.properties.amplifier.pole_zero,
                (self.pole_zeroing() || self.optimizing()) as u8
            )),
            "SET_GAIN_FINE" => {
                // Real units take 0..4095 across the fine-gain range.
                self.ensure_unlocked()?;
                self.properties.amplifier.fine_gain = number()? / 4096.0;
                Ok("OK".to_string())
            }
            "SET_GAIN_COARSE" => {
                self.ensure_unlocked()?;
                self.properties.amplifier.coarse_gain = number()?;
                Ok("OK".to_string())
            }
            // Through `presets_busy_check`, so the wire cannot do what
            // `set_presets` refuses: changing presets mid-count.
            "SET_PRESET_REAL" => {
                self.presets_busy_check(trimmed)?;
                self.properties.presets.real_time = Some(number()?);
                Ok("OK".to_string())
            }
            "SET_PRESET_LIVE" => {
                self.presets_busy_check(trimmed)?;
                self.properties.presets.live_time = Some(number()?);
                Ok("OK".to_string())
            }
            "SET_PRESET_COUNT" => {
                self.presets_busy_check(trimmed)?;
                self.properties.presets.roi_peak = Some(number()? as u64);
                Ok("OK".to_string())
            }
            "SET_PRESET_INTEG" | "SET_PRESET_INTEGRAL" => {
                self.presets_busy_check(trimmed)?;
                self.properties.presets.roi_integral = Some(number()? as u64);
                Ok("OK".to_string())
            }
            "SET_PRESET_CLEAR" => {
                self.presets_busy_check(trimmed)?;
                self.properties.presets.clear();
                Ok("OK".to_string())
            }
            "SET_OUTPUT_HIGH" => {
                self.output_high = true;
                Ok("OK".to_string())
            }
            "SET_OUTPUT_LOW" => {
                self.output_high = false;
                Ok("OK".to_string())
            }
            "SET_HV" => {
                self.ensure_unlocked()?;
                self.properties.high_voltage.target_volts = number()?;
                Ok("OK".to_string())
            }
            other => Err(DeviceError::Command {
                command: trimmed.to_string(),
                detail: format!("{other} is not a command this instrument knows"),
            }),
        }
    }

    fn lock(&mut self, password: &str, owner: &str) -> Result<(), DeviceError> {
        self.ensure_unlocked()?;
        self.lock = Some((password.to_string(), owner.to_string()));
        Ok(())
    }

    fn unlock(&mut self, password: &str) -> Result<(), DeviceError> {
        match &self.lock {
            Some((stored, _)) if stored == password => {
                self.lock = None;
                Ok(())
            }
            Some((_, owner)) => Err(DeviceError::Locked {
                owner: owner.clone(),
            }),
            None => Ok(()),
        }
    }

    fn is_locked(&self) -> bool {
        self.lock.is_some()
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn the_compton_edge_is_below_the_full_energy() {
        // 662 keV: the Compton edge sits at 478 keV.
        assert!((SimulatedMcb::compton_edge(661.657) - 477.6).abs() < 1.0);
        assert!(SimulatedMcb::compton_edge(1332.492) < 1332.492);
    }

    #[test]
    fn the_published_shape_calibration_tracks_the_physical_resolution() {
        let source = SimulatedSource::default();
        let gain = 0.5;
        let shape = fit_shape(&source, gain, 8192);
        for channel in [500.0, 2000.0, 6000.0] {
            let energy = channel * gain;
            let physical = source.fwhm_kev(energy) / gain;
            assert!(
                (shape.fwhm(channel) - physical).abs() < 0.1 * physical,
                "channel {channel}: {} against {physical}",
                shape.fwhm(channel)
            );
        }
    }

    #[test]
    fn smeared_channels_land_near_the_calibrated_position() {
        let mut mcb = SimulatedMcb::builder(1, "t")
            .gain_kev_per_channel(0.5)
            .build();
        // 500 keV at 0.5 keV per channel is channel 1000, plus resolution.
        for _ in 0..200 {
            let channel = mcb.sample_channel(500.0).expect("a channel");
            assert!(
                (channel as i64 - 1000).abs() < 30,
                "channel {channel} is too far from 1000"
            );
        }
        assert!(mcb.sample_channel(0.0).is_none());
    }
}
