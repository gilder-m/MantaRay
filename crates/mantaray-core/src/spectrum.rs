//! The spectrum: channel data plus the descriptors saved with it.

use chrono::NaiveDateTime;
use serde::{Deserialize, Serialize};

use crate::calibration::{EnergyCalibration, ShapeCalibration};
use crate::roi::RoiSet;

/// Data-collection mode, shown in the corner of the full spectrum view.
#[derive(Clone, Copy, Debug, Default, PartialEq, Eq, Serialize, Deserialize)]
pub enum AcquisitionMode {
    /// Pulse-height analysis: the normal histogramming mode.
    #[default]
    Pha,
    /// Event-by-event list mode.
    List,
    /// Live-time corrected spectrum.
    Ltc,
    /// Zero-dead-time corrected spectrum.
    Zdt,
    /// Uncertainty ("error") spectrum that accompanies a ZDT spectrum.
    Err,
}

impl AcquisitionMode {
    /// Short label as displayed by MAESTRO.
    pub fn label(&self) -> &'static str {
        match self {
            Self::Pha => "PHA",
            Self::List => "LIST",
            Self::Ltc => "LTC",
            Self::Zdt => "ZDT",
            Self::Err => "ERR",
        }
    }
}

/// A histogram plus everything saved alongside it.
#[derive(Clone, Debug, PartialEq, Serialize, Deserialize)]
pub struct Spectrum {
    /// Channel contents.
    pub channels: Vec<u64>,
    /// Live time in seconds (real time minus dead time).
    pub live_time: f64,
    /// Real (clock) time in seconds.
    pub real_time: f64,
    /// Local wall-clock time at which acquisition started.
    pub start_time: Option<NaiveDateTime>,
    /// Detector number in the detector list (0 means "buffer").
    pub detector_id: u16,
    /// Detector name, e.g. `DET-01`.
    pub detector_name: String,
    /// Free-form detector description saved with the spectrum.
    pub detector_description: String,
    /// Sample description (63 characters in `.Chn`, 128 in `.Spc`).
    pub sample_description: String,
    /// Segment number (multi-segment hardware).
    pub segment: u16,
    /// Channel offset of the first stored channel.
    pub channel_offset: u16,
    /// Collection mode.
    pub mode: AcquisitionMode,
    /// Energy calibration, when calibrated.
    pub energy_calibration: Option<EnergyCalibration>,
    /// Peak-shape calibration, when available.
    pub shape_calibration: Option<ShapeCalibration>,
    /// Marked regions of interest.
    pub rois: RoiSet,
    /// Where the spectrum came from (file path or device label).
    pub source: Option<String>,
}

impl Default for Spectrum {
    fn default() -> Self {
        Self::new(0)
    }
}

impl Spectrum {
    /// A zeroed spectrum of `length` channels.
    pub fn new(length: usize) -> Self {
        Self {
            channels: vec![0; length],
            live_time: 0.0,
            real_time: 0.0,
            start_time: None,
            detector_id: 0,
            detector_name: String::new(),
            detector_description: String::new(),
            sample_description: String::new(),
            segment: 0,
            channel_offset: 0,
            mode: AcquisitionMode::Pha,
            energy_calibration: None,
            shape_calibration: None,
            rois: RoiSet::new(),
            source: None,
        }
    }

    /// A spectrum wrapping existing channel data.
    pub fn from_counts(channels: Vec<u64>) -> Self {
        Self {
            channels,
            ..Self::new(0)
        }
    }

    /// Number of channels.
    pub fn len(&self) -> usize {
        self.channels.len()
    }

    /// True when the spectrum has no channels at all.
    pub fn is_empty(&self) -> bool {
        self.channels.is_empty()
    }

    /// Contents of a channel; out-of-range channels read as zero.
    pub fn counts(&self, channel: usize) -> u64 {
        self.channels.get(channel).copied().unwrap_or(0)
    }

    /// Adds one count to a channel, ignoring out-of-range channels.
    pub fn add_count(&mut self, channel: usize) {
        self.add_counts(channel, 1);
    }

    /// Adds `n` counts to a channel, ignoring out-of-range channels.
    pub fn add_counts(&mut self, channel: usize, n: u64) {
        if let Some(slot) = self.channels.get_mut(channel) {
            *slot = slot.saturating_add(n);
        }
    }

    /// Sum of every channel.
    pub fn total_counts(&self) -> u64 {
        self.channels.iter().sum()
    }

    /// Largest channel content.
    pub fn max_count(&self) -> u64 {
        self.channels.iter().copied().max().unwrap_or(0)
    }

    /// Channel holding the largest content.
    pub fn max_channel(&self) -> Option<usize> {
        if self.channels.is_empty() {
            return None;
        }
        let mut best = 0usize;
        for (index, value) in self.channels.iter().enumerate() {
            if *value > self.channels[best] {
                best = index;
            }
        }
        Some(best)
    }

    /// Inclusive sum over a channel range; bounds are normalised and clamped.
    pub fn sum_range(&self, a: usize, b: usize) -> u64 {
        if self.channels.is_empty() {
            return 0;
        }
        let last = self.channels.len() - 1;
        let low = a.min(b).min(last);
        let high = a.max(b).min(last);
        self.channels[low..=high].iter().sum()
    }

    /// Erases the data, real time, live time and start time. Presets, ROI marks
    /// and calibrations are left alone, as MAESTRO's Acquire/Clear does.
    pub fn clear(&mut self) {
        self.channels.iter_mut().for_each(|c| *c = 0);
        self.live_time = 0.0;
        self.real_time = 0.0;
        self.start_time = None;
    }

    /// Dead time as a percentage of the real time, when both times are known.
    pub fn dead_time_percent(&self) -> Option<f64> {
        if self.real_time <= 0.0 {
            return None;
        }
        Some((1.0 - self.live_time / self.real_time) * 100.0)
    }

    /// Total count rate over the live time, in counts per second.
    pub fn count_rate(&self) -> Option<f64> {
        (self.live_time > 0.0).then(|| self.total_counts() as f64 / self.live_time)
    }

    /// True when an energy calibration is present.
    pub fn is_calibrated(&self) -> bool {
        self.energy_calibration.is_some()
    }

    /// Energy of a channel, when calibrated.
    pub fn energy_of(&self, channel: f64) -> Option<f64> {
        self.energy_calibration.as_ref().map(|c| c.energy(channel))
    }

    /// Channel of an energy, when calibrated and invertible.
    pub fn channel_of(&self, energy: f64) -> Option<f64> {
        self.energy_calibration
            .as_ref()
            .and_then(|c| c.channel(energy))
    }

    /// Calibration units, or `"ch"` when uncalibrated.
    pub fn units(&self) -> &str {
        match &self.energy_calibration {
            Some(cal) => &cal.units,
            None => "ch",
        }
    }

    /// Predicted FWHM in channels from the shape calibration.
    pub fn fwhm_at(&self, channel: f64) -> Option<f64> {
        self.shape_calibration
            .as_ref()
            .filter(|s| s.is_usable())
            .map(|s| s.fwhm(channel))
            .filter(|w| *w > 0.0)
    }

    /// Grows or shrinks the spectrum, keeping the leading channels.
    pub fn resize(&mut self, length: usize) {
        self.channels.resize(length, 0);
        self.rois.clamp_to(length);
    }

    /// Copies the descriptors (times, calibration, descriptions) from another
    /// spectrum without touching the channel data.
    pub fn copy_descriptors_from(&mut self, other: &Spectrum) {
        self.live_time = other.live_time;
        self.real_time = other.real_time;
        self.start_time = other.start_time;
        self.detector_id = other.detector_id;
        self.detector_name = other.detector_name.clone();
        self.detector_description = other.detector_description.clone();
        self.sample_description = other.sample_description.clone();
        self.segment = other.segment;
        self.channel_offset = other.channel_offset;
        self.mode = other.mode;
        self.energy_calibration = other.energy_calibration.clone();
        self.shape_calibration = other.shape_calibration;
    }

    /// Channel contents as floating point, for analysis routines.
    pub fn as_f64(&self) -> Vec<f64> {
        self.channels.iter().map(|c| *c as f64).collect()
    }
}
