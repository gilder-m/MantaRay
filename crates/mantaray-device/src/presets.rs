//! Presets: the conditions that stop an acquisition (MAESTRO §4.2.8).

use crate::error::DeviceError;
use mantaray_core::CalculationSettings;
use mantaray_core::{Roi, Spectrum, mda_currie, peak_info, statistical_uncertainty};
use serde::{Deserialize, Serialize};

/// Which preset stopped a run.
#[derive(Clone, Copy, Debug, PartialEq, Eq, Serialize, Deserialize)]
pub enum PresetKind {
    /// Real (clock) time.
    RealTime,
    /// Live time.
    LiveTime,
    /// Counts in the tallest channel of any marked region.
    RoiPeak,
    /// Sum of the counts in every marked region.
    RoiIntegral,
    /// Statistical uncertainty of a channel range.
    Uncertainty,
    /// Minimum detectable activity of a region.
    Mda,
}

impl PresetKind {
    /// Label for the status sidebar.
    pub fn label(&self) -> &'static str {
        match self {
            Self::RealTime => "Real time",
            Self::LiveTime => "Live time",
            Self::RoiPeak => "ROI peak",
            Self::RoiIntegral => "ROI integral",
            Self::Uncertainty => "Uncertainty",
            Self::Mda => "MDA",
        }
    }
}

/// Stop when the relative uncertainty of a channel range falls below a limit.
#[derive(Clone, Copy, Debug, PartialEq, Serialize, Deserialize)]
pub struct UncertaintyPreset {
    /// Requested one-sigma uncertainty, in percent.
    pub limit_percent: f64,
    /// First channel of the region used.
    pub low_channel: usize,
    /// Last channel of the region used.
    pub high_channel: usize,
}

/// Stop when the minimum detectable activity of a region falls below a target.
#[derive(Clone, Copy, Debug, PartialEq, Serialize, Deserialize)]
pub struct MdaPreset {
    /// Target MDA in becquerel.
    pub target: f64,
    /// Region the MDA is computed for.
    pub region: Roi,
    /// Absolute efficiency at the line of interest, as a fraction.
    pub efficiency: f64,
    /// Yield of the line, as a fraction.
    pub yield_fraction: f64,
}

/// The shortest real- or live-time preset, in seconds: one tick of the
/// instrument's clock (`mantaray-mcb` counts presets in 20 ms ticks). A
/// shorter one would be sent as zero ticks, which means no limit.
pub const PRESET_TICK_SECONDS: f64 = 0.02;

/// The Presets tab of the MCB Properties dialog.
#[derive(Clone, Copy, Debug, Default, PartialEq, Serialize, Deserialize)]
pub struct Presets {
    /// Real-time limit in seconds.
    pub real_time: Option<f64>,
    /// Live-time limit in seconds.
    pub live_time: Option<f64>,
    /// Counts in the tallest marked channel.
    pub roi_peak: Option<u64>,
    /// Sum over every marked channel.
    pub roi_integral: Option<u64>,
    /// Statistical uncertainty limit.
    pub uncertainty: Option<UncertaintyPreset>,
    /// Minimum detectable activity target.
    pub mda: Option<MdaPreset>,
}

impl Presets {
    /// Clears every preset (`SET_PRESET_CLEAR`).
    pub fn clear(&mut self) {
        *self = Self::default();
    }

    /// The presets the instrument itself holds: `SET_PRESET_REAL`,
    /// `SET_PRESET_LIVE`, `SET_PRESET_COUNT` and `SET_PRESET_INTEG`. The
    /// uncertainty and MDA presets are worked out host-side and never leave
    /// this program.
    pub fn held_by_the_instrument(&self) -> Self {
        Self {
            real_time: self.real_time,
            live_time: self.live_time,
            roi_peak: self.roi_peak,
            roi_integral: self.roi_integral,
            ..Self::default()
        }
    }

    /// Whether the presets the instrument holds differ between the two; a
    /// change to the host-side ones alone is not one, and a NaN that is still
    /// NaN is not a change.
    pub fn held_differ(&self, other: &Self) -> bool {
        fn seconds(a: Option<f64>, b: Option<f64>) -> bool {
            match (a, b) {
                (Some(a), Some(b)) => mantaray_core::differs(a, b),
                (None, None) => false,
                _ => true,
            }
        }
        seconds(self.real_time, other.real_time)
            || seconds(self.live_time, other.live_time)
            || self.roi_peak != other.roi_peak
            || self.roi_integral != other.roi_integral
    }

    /// Whether the presets the program itself holds (uncertainty, MDA)
    /// differ; a NaN that is still NaN is not a change.
    pub fn host_differ(&self, other: &Self) -> bool {
        let uncertainty = match (&self.uncertainty, &other.uncertainty) {
            (Some(a), Some(b)) => {
                mantaray_core::differs(a.limit_percent, b.limit_percent)
                    || a.low_channel != b.low_channel
                    || a.high_channel != b.high_channel
            }
            (None, None) => false,
            _ => true,
        };
        let mda = match (&self.mda, &other.mda) {
            (Some(a), Some(b)) => {
                mantaray_core::differs(a.target, b.target)
                    || a.region != b.region
                    || mantaray_core::differs(a.efficiency, b.efficiency)
                    || mantaray_core::differs(a.yield_fraction, b.yield_fraction)
            }
            (None, None) => false,
            _ => true,
        };
        uncertainty || mda
    }

    /// Whether any preset differs between the two. The one comparison for
    /// "is this edit a preset change": derived `!=` on these f64 fields calls
    /// a NaN that is still NaN a change.
    pub fn differ(&self, other: &Self) -> bool {
        self.held_differ(other) || self.host_differ(other)
    }

    /// The same presets with every zero turned into "not set": MAESTRO's "To
    /// disable a preset, enter a value of zero". A zero time or count, an
    /// uncertainty limit of zero and an MDA target of zero are each `None`, so
    /// a zero is never stored or sent as a limit. Every preset change runs
    /// through this before the gate ([`crate::Mcb::apply_presets`],
    /// [`crate::Mcb::set_presets`]).
    pub fn normalized(self) -> Self {
        let seconds = |value: Option<f64>| value.filter(|seconds| *seconds != 0.0);
        let counts = |value: Option<u64>| value.filter(|counts| *counts != 0);
        Self {
            real_time: seconds(self.real_time),
            live_time: seconds(self.live_time),
            roi_peak: counts(self.roi_peak),
            roi_integral: counts(self.roi_integral),
            uncertainty: self
                .uncertainty
                .filter(|preset| preset.limit_percent != 0.0),
            mda: self.mda.filter(|preset| preset.target != 0.0),
        }
    }

    /// Refuses a preset that is not a usable limit, by name, rounding or
    /// truncating nothing. Run by the gate on the [`Self::normalized`] value
    /// (a zero is already "not set"):
    /// - every number is finite and not negative;
    /// - a time preset is at least one tick of the instrument's clock,
    ///   [`PRESET_TICK_SECONDS`] (shorter would be sent as zero ticks, which
    ///   the instrument reads as no limit at all);
    /// - an uncertainty range runs from low to high (channels are whole by
    ///   type);
    /// - an MDA efficiency and yield are fractions above zero and at most one
    ///   (the MDA divides by both; 85 typed for 85 % would stop at once).
    pub fn validate(&self) -> Result<(), DeviceError> {
        fn refuse(setting: &str, detail: String) -> Result<(), DeviceError> {
            Err(DeviceError::OutOfRange {
                setting: setting.into(),
                detail,
            })
        }
        fn number(setting: &str, value: f64) -> Result<(), DeviceError> {
            if value.is_finite() && value >= 0.0 {
                return Ok(());
            }
            refuse(
                setting,
                format!("{value} is not a valid preset (it must be a number, not negative)"),
            )
        }
        for (setting, seconds) in [
            ("real-time preset", self.real_time),
            ("live-time preset", self.live_time),
        ] {
            if let Some(seconds) = seconds {
                number(setting, seconds)?;
                if seconds > 0.0 && seconds < PRESET_TICK_SECONDS {
                    refuse(
                        setting,
                        format!(
                            "{seconds} s is shorter than the instrument's clock tick of \
                             {PRESET_TICK_SECONDS} s (0 disables the preset)"
                        ),
                    )?;
                }
            }
        }
        if let Some(preset) = &self.uncertainty {
            number("uncertainty preset", preset.limit_percent)?;
            if preset.low_channel > preset.high_channel {
                refuse(
                    "uncertainty preset",
                    format!(
                        "the channel range {} to {} runs backwards (low must not exceed high)",
                        preset.low_channel, preset.high_channel
                    ),
                )?;
            }
        }
        if let Some(preset) = &self.mda {
            number("MDA preset target", preset.target)?;
            for (setting, value) in [
                ("MDA preset efficiency", preset.efficiency),
                ("MDA preset yield", preset.yield_fraction),
            ] {
                number(setting, value)?;
                if value <= 0.0 || value > 1.0 {
                    refuse(
                        setting,
                        format!("{value} is not usable (a fraction above 0 and at most 1)"),
                    )?;
                }
            }
        }
        Ok(())
    }

    /// Sets the real-time preset; zero disables it, as in MAESTRO ("a zero
    /// value disables a preset"), and is never stored as a limit of zero.
    pub fn set_real_time(&mut self, seconds: f64) {
        self.real_time = (seconds != 0.0).then_some(seconds);
    }

    /// Sets the live-time preset; zero disables it.
    pub fn set_live_time(&mut self, seconds: f64) {
        self.live_time = (seconds != 0.0).then_some(seconds);
    }

    /// Sets the ROI peak-count preset; zero disables it.
    pub fn set_roi_peak(&mut self, counts: u64) {
        self.roi_peak = (counts != 0).then_some(counts);
    }

    /// Sets the ROI integral preset; zero disables it.
    pub fn set_roi_integral(&mut self, counts: u64) {
        self.roi_integral = (counts != 0).then_some(counts);
    }

    /// Sets the uncertainty preset; a zero limit disables it.
    pub fn set_uncertainty(&mut self, limit_percent: f64, low_channel: usize, high_channel: usize) {
        self.uncertainty = (limit_percent != 0.0).then_some(UncertaintyPreset {
            limit_percent,
            low_channel,
            high_channel,
        });
    }

    /// True when nothing will stop the acquisition.
    pub fn is_empty(&self) -> bool {
        *self == Self::default()
    }

    /// A real-time-only preset.
    pub fn real(seconds: f64) -> Self {
        Self {
            real_time: Some(seconds),
            ..Self::default()
        }
    }

    /// A live-time-only preset.
    pub fn live(seconds: f64) -> Self {
        Self {
            live_time: Some(seconds),
            ..Self::default()
        }
    }

    /// Which preset a count that has ended ended on, if any: the batch
    /// queue's "complete or stopped early".
    ///
    /// The instrument decides its own time presets on its own clock, in
    /// [`PRESET_TICK_SECONDS`] ticks, and the clocks it reports can read a
    /// tick short of the preset it stopped on (LT=299.99 on a 300 s preset).
    /// Judged exactly, that count read as stopped early and the batch
    /// stalled (review of 21df1c3, #5). So a held time preset counts as
    /// reached within one tick; everything else as [`Self::reached`] says.
    pub fn ended_on(
        &self,
        spectrum: &Spectrum,
        real_time: f64,
        live_time: f64,
    ) -> Option<PresetKind> {
        self.reached(spectrum, real_time, live_time).or_else(|| {
            self.held_by_the_instrument().reached(
                spectrum,
                real_time + PRESET_TICK_SECONDS,
                live_time + PRESET_TICK_SECONDS,
            )
        })
    }

    /// Which preset, if any, is satisfied.
    ///
    /// Presets are checked in the order MAESTRO lists them on the dialog; the
    /// first one satisfied stops the run.
    pub fn reached(
        &self,
        spectrum: &Spectrum,
        real_time: f64,
        live_time: f64,
    ) -> Option<PresetKind> {
        if let Some(limit) = self.real_time
            && limit > 0.0
            && real_time >= limit
        {
            return Some(PresetKind::RealTime);
        }
        if let Some(limit) = self.live_time
            && limit > 0.0
            && live_time >= limit
        {
            return Some(PresetKind::LiveTime);
        }
        if let Some(limit) = self.roi_peak
            && limit > 0
            && roi_peak_counts(spectrum) >= limit
        {
            return Some(PresetKind::RoiPeak);
        }
        if let Some(limit) = self.roi_integral
            && limit > 0
            && roi_integral_counts(spectrum) >= limit
        {
            return Some(PresetKind::RoiIntegral);
        }
        if let Some(preset) = self.uncertainty
            && preset.limit_percent > 0.0
            && let Some(current) =
                statistical_uncertainty(spectrum, preset.low_channel, preset.high_channel)
            && current <= preset.limit_percent
        {
            return Some(PresetKind::Uncertainty);
        }
        if let Some(preset) = self.mda
            && preset.target > 0.0
            && live_time > 0.0
            && let Some(current) = mda_of(spectrum, &preset, live_time)
            && current <= preset.target
        {
            return Some(PresetKind::Mda);
        }
        None
    }

    /// Fractional progress of the preset closest to stopping the run.
    pub fn progress(&self, spectrum: &Spectrum, real_time: f64, live_time: f64) -> Option<f64> {
        let mut best: Option<f64> = None;
        let mut consider = |value: f64| {
            let value = value.clamp(0.0, 1.0);
            best = Some(best.map_or(value, |current: f64| current.max(value)));
        };
        if let Some(limit) = self.real_time.filter(|l| *l > 0.0) {
            consider(real_time / limit);
        }
        if let Some(limit) = self.live_time.filter(|l| *l > 0.0) {
            consider(live_time / limit);
        }
        if let Some(limit) = self.roi_peak.filter(|l| *l > 0) {
            consider(roi_peak_counts(spectrum) as f64 / limit as f64);
        }
        if let Some(limit) = self.roi_integral.filter(|l| *l > 0) {
            consider(roi_integral_counts(spectrum) as f64 / limit as f64);
        }
        if let Some(preset) = self.uncertainty.filter(|p| p.limit_percent > 0.0) {
            // Uncertainty falls as the square root of the counts, so progress is
            // the ratio of the required counts to the counts collected.
            if let Some(current) =
                statistical_uncertainty(spectrum, preset.low_channel, preset.high_channel)
            {
                consider((preset.limit_percent / current).powi(2));
            } else {
                consider(0.0);
            }
        }
        if let Some(preset) = self.mda.filter(|p| p.target > 0.0) {
            match mda_of(spectrum, &preset, live_time) {
                Some(current) if current > 0.0 => consider(preset.target / current),
                _ => consider(0.0),
            }
        }
        best
    }
}

/// Counts in the tallest channel covered by any region.
fn roi_peak_counts(spectrum: &Spectrum) -> u64 {
    spectrum
        .rois
        .iter()
        .flat_map(|roi| roi.channels())
        .map(|channel| spectrum.counts(channel))
        .max()
        .unwrap_or(0)
}

/// Sum over every channel covered by a region.
fn roi_integral_counts(spectrum: &Spectrum) -> u64 {
    spectrum
        .rois
        .iter()
        .map(|roi| spectrum.sum_range(roi.start, roi.end))
        .sum()
}

/// Current MDA of the preset's region, in becquerel.
fn mda_of(spectrum: &Spectrum, preset: &MdaPreset, live_time: f64) -> Option<f64> {
    if preset.efficiency <= 0.0 || preset.yield_fraction <= 0.0 || live_time <= 0.0 {
        return None;
    }
    let region = preset.region.clamped(spectrum.len())?;
    // Use the region's fitted background when it is wide enough, and its gross
    // counts otherwise; both are conservative.
    let background = match peak_info(spectrum, region, &CalculationSettings::default()) {
        Ok(info) => info.background,
        Err(_) => spectrum.sum_range(region.start, region.end) as f64,
    };
    if background <= 0.0 {
        return None;
    }
    Some(mda_currie(
        background,
        preset.efficiency,
        preset.yield_fraction,
        live_time,
    ))
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn an_empty_preset_never_stops_a_run() {
        let spectrum = Spectrum::from_counts(vec![10; 100]);
        assert!(Presets::default().reached(&spectrum, 1e9, 1e9).is_none());
        assert!(Presets::default().progress(&spectrum, 1.0, 1.0).is_none());
    }

    #[test]
    fn zero_limits_are_treated_as_unset() {
        let spectrum = Spectrum::from_counts(vec![10; 100]);
        let presets = Presets {
            real_time: Some(0.0),
            ..Presets::default()
        };
        assert!(presets.reached(&spectrum, 100.0, 100.0).is_none());
    }

    #[test]
    fn roi_helpers_only_count_marked_channels() {
        let mut spectrum = Spectrum::from_counts(vec![1, 2, 3, 4, 5, 6]);
        assert_eq!(roi_peak_counts(&spectrum), 0);
        spectrum.rois.mark(Roi::new(1, 3));
        assert_eq!(roi_peak_counts(&spectrum), 4);
        assert_eq!(roi_integral_counts(&spectrum), 9);
    }
}
