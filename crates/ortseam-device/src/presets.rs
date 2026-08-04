//! Presets: the conditions that stop an acquisition (MAESTRO §4.2.8).

use ortseam_core::CalculationSettings;
use ortseam_core::{Roi, Spectrum, mda_currie, peak_info, statistical_uncertainty};
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
