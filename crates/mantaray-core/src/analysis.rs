//! Spectrum analysis: peak information, peak search, smoothing and stripping.
//!
//! The area and uncertainty expressions are MAESTRO equations (17)-(21); the
//! activity expression is equation (22) and the smoothing kernel equation (23).

use serde::{Deserialize, Serialize};

use crate::FWHM_PER_SIGMA;
use crate::calibration::{EnergyCalibration, least_squares3};
use crate::error::AnalysisError;
use crate::roi::Roi;
use crate::settings::CalculationSettings;
use crate::spectrum::Spectrum;

/// A Gaussian fitted to the background-subtracted channels of a region.
#[derive(Clone, Copy, Debug, PartialEq, Serialize, Deserialize)]
pub struct GaussianFit {
    /// Peak height above background.
    pub amplitude: f64,
    /// Fitted centroid, in channels.
    pub centroid: f64,
    /// Fitted standard deviation, in channels.
    pub sigma: f64,
}

impl GaussianFit {
    /// Full width at half maximum implied by the fit.
    pub fn fwhm(&self) -> f64 {
        FWHM_PER_SIGMA * self.sigma
    }

    /// Analytic area of the fitted Gaussian.
    pub fn area(&self) -> f64 {
        self.amplitude * self.sigma * (2.0 * std::f64::consts::PI).sqrt()
    }
}

/// Everything MAESTRO's Peak Info reports for one region.
#[derive(Clone, Debug, PartialEq, Serialize, Deserialize)]
pub struct PeakInfo {
    /// The region analysed.
    pub roi: Roi,
    /// Background points used at each end (`n` in the equations).
    pub background_points: usize,
    /// `x` used for the FW(1/x)M report.
    pub fw_x: u32,
    /// Gross area, equation (18).
    pub gross_area: f64,
    /// Adjusted gross area, equation (19).
    pub adjusted_gross_area: f64,
    /// Background area, equation (17).
    pub background: f64,
    /// Net area, equation (20).
    pub net_area: f64,
    /// Uncertainty in the net area, equation (21).
    pub net_area_uncertainty: f64,
    /// Peak centroid in channels (from the Gaussian fit when it succeeded).
    pub centroid: f64,
    /// Full width at half maximum, in channels (0 when not measurable).
    pub fwhm: f64,
    /// Full width at 1/x maximum, in channels (0 when not measurable).
    pub fw_x_m: f64,
    /// The Gaussian fit, when one could be made.
    pub fit: Option<GaussianFit>,
}

impl PeakInfo {
    /// Centroid in calibration units.
    pub fn centroid_energy(&self, cal: &EnergyCalibration) -> f64 {
        cal.energy(self.centroid)
    }

    /// FWHM in calibration units.
    pub fn fwhm_energy(&self, cal: &EnergyCalibration) -> f64 {
        cal.width(self.centroid, self.fwhm)
    }

    /// FW(1/x)M in calibration units.
    pub fn fw_x_m_energy(&self, cal: &EnergyCalibration) -> f64 {
        cal.width(self.centroid, self.fw_x_m)
    }

    /// Gross counts per second over a live time.
    pub fn gross_count_rate(&self, live_time: f64) -> f64 {
        if live_time > 0.0 {
            self.gross_area / live_time
        } else {
            0.0
        }
    }

    /// Net counts per second over a live time.
    pub fn net_count_rate(&self, live_time: f64) -> f64 {
        if live_time > 0.0 {
            self.net_area / live_time
        } else {
            0.0
        }
    }

    /// Net-area uncertainty as a percentage of the net area.
    pub fn net_uncertainty_percent(&self) -> f64 {
        if self.net_area.abs() > 0.0 {
            100.0 * self.net_area_uncertainty / self.net_area.abs()
        } else {
            f64::INFINITY
        }
    }

    /// Counting activity from a library yield, equation (22).
    pub fn activity(&self, yield_percent: f64, live_time: f64) -> f64 {
        counting_activity(self.net_area, yield_percent, live_time)
    }

    /// Uncertainty of the counting activity, scaled from the net-area error.
    pub fn activity_uncertainty(&self, yield_percent: f64, live_time: f64) -> f64 {
        counting_activity(self.net_area_uncertainty, yield_percent, live_time)
    }
}

/// Computes the Peak Info figures for a region.
pub fn peak_info(
    spectrum: &Spectrum,
    roi: Roi,
    settings: &CalculationSettings,
) -> Result<PeakInfo, AnalysisError> {
    let length = spectrum.len();
    if length == 0 {
        return Err(AnalysisError::EmptySpectrum);
    }
    if roi.end >= length {
        return Err(AnalysisError::RoiOutOfRange {
            start: roi.start,
            end: roi.end,
            length,
        });
    }
    let n = settings.background_points.clamp(
        CalculationSettings::MIN_BACKGROUND_POINTS,
        CalculationSettings::MAX_BACKGROUND_POINTS,
    );
    let needed = 2 * n + 1;
    if roi.len() < needed {
        return Err(AnalysisError::RoiTooNarrow {
            width: roi.len(),
            needed,
            background_points: n,
        });
    }

    let l = roi.start;
    let h = roi.end;
    let counts = |channel: usize| spectrum.counts(channel) as f64;
    let roi_width = (h - l + 1) as f64;
    let nf = n as f64;

    // Equation (17): background area from the mean of the n end channels.
    let sum_low: f64 = (l..l + n).map(counts).sum();
    let sum_high: f64 = (h + 1 - n..=h).map(counts).sum();
    let background = (sum_low + sum_high) * roi_width / (2.0 * nf);

    // Equations (18) and (19).
    let gross_area: f64 = (l..=h).map(counts).sum();
    let adjusted_gross_area: f64 = (l + n..=h - n).map(counts).sum();

    // Equations (20) and (21).
    //
    // The printed equations use `h - l - (n - 1)` for the width of the adjusted
    // region. That disagrees with equation (19), which sums `h - l + 1 - 2n`
    // channels, and it over-subtracts background: with the reference case in
    // `tests/peak_info.rs` (a 370-count peak on a flat background) the printed
    // form gives 340, and a flat region gives a net area of -30 instead of 0.
    // We therefore use the number of channels equation (19) actually covers,
    // which reproduces both known answers exactly. Equation (21) is derived from
    // the same width, so it follows automatically.
    let adjusted_width = roi_width - 2.0 * nf;
    let net_area = adjusted_gross_area - background * adjusted_width / roi_width;
    let variance = adjusted_gross_area
        + background * (adjusted_width / (2.0 * nf)) * (adjusted_width / roi_width);
    let net_area_uncertainty = variance.max(0.0).sqrt();

    // Straight-line background between the two background anchor points, whose
    // channels are the midpoints of the n end channels (§4.3.5.1, Fig. 61).
    let x_low = l as f64 + (nf - 1.0) / 2.0;
    let x_high = h as f64 - (nf - 1.0) / 2.0;
    let y_low = sum_low / nf;
    let y_high = sum_high / nf;
    let slope = if (x_high - x_low).abs() > f64::EPSILON {
        (y_high - y_low) / (x_high - x_low)
    } else {
        0.0
    };
    let net: Vec<f64> = (l..=h)
        .map(|channel| counts(channel) - (y_low + slope * (channel as f64 - x_low)))
        .collect();

    let peak_index = net
        .iter()
        .enumerate()
        .max_by(|a, b| a.1.partial_cmp(b.1).unwrap_or(std::cmp::Ordering::Equal))
        .map(|(index, _)| index)
        .unwrap_or(0);
    let max_net = net[peak_index];

    let (fwhm, fw_x_m) = if max_net > 0.0 {
        (
            width_at_level(&net, peak_index, max_net / 2.0),
            width_at_level(
                &net,
                peak_index,
                max_net
                    / settings
                        .fw_x
                        .clamp(CalculationSettings::MIN_FW_X, CalculationSettings::MAX_FW_X)
                        as f64,
            ),
        )
    } else {
        (0.0, 0.0)
    };

    let fit = fit_gaussian(&net, l);
    let centroid = match &fit {
        Some(fit) => fit.centroid,
        None => moment_centroid(&net, l).unwrap_or_else(|| roi.center()),
    };

    Ok(PeakInfo {
        roi,
        background_points: n,
        fw_x: settings.fw_x,
        gross_area,
        adjusted_gross_area,
        background,
        net_area,
        net_area_uncertainty,
        centroid,
        fwhm,
        fw_x_m,
        fit,
    })
}

/// Width of a peak at an absolute level, by linear interpolation between the
/// background-subtracted channels. Returns 0.0 when the level is not crossed on
/// both sides.
fn width_at_level(net: &[f64], peak_index: usize, level: f64) -> f64 {
    let left = crossing_left(net, peak_index, level);
    let right = crossing_right(net, peak_index, level);
    match (left, right) {
        (Some(a), Some(b)) if b > a => b - a,
        _ => 0.0,
    }
}

fn crossing_left(net: &[f64], peak_index: usize, level: f64) -> Option<f64> {
    let mut index = peak_index;
    while index > 0 {
        let (y0, y1) = (net[index - 1], net[index]);
        if y0 <= level {
            if (y1 - y0).abs() < f64::EPSILON {
                return Some(index as f64);
            }
            return Some((index - 1) as f64 + (level - y0) / (y1 - y0));
        }
        index -= 1;
    }
    None
}

fn crossing_right(net: &[f64], peak_index: usize, level: f64) -> Option<f64> {
    let mut index = peak_index;
    while index + 1 < net.len() {
        let (y0, y1) = (net[index], net[index + 1]);
        if y1 <= level {
            if (y0 - y1).abs() < f64::EPSILON {
                return Some(index as f64);
            }
            return Some(index as f64 + (y0 - level) / (y0 - y1));
        }
        index += 1;
    }
    None
}

/// Net-weighted first moment, in absolute channels.
fn moment_centroid(net: &[f64], offset: usize) -> Option<f64> {
    let max = net.iter().copied().fold(f64::MIN, f64::max);
    if max <= 0.0 {
        return None;
    }
    let level = 0.25 * max;
    let (mut weight, mut moment) = (0.0f64, 0.0f64);
    for (index, value) in net.iter().enumerate() {
        if *value >= level {
            weight += *value;
            moment += *value * (index + offset) as f64;
        }
    }
    (weight > 0.0).then(|| moment / weight)
}

/// Fits a Gaussian to the background-subtracted channels by a parabola through
/// the logarithms of the channels above 30 % of the peak height.
fn fit_gaussian(net: &[f64], offset: usize) -> Option<GaussianFit> {
    let (peak_index, max) = net
        .iter()
        .enumerate()
        .max_by(|a, b| a.1.partial_cmp(b.1).unwrap_or(std::cmp::Ordering::Equal))
        .map(|(index, value)| (index, *value))?;
    if max <= 0.0 {
        return None;
    }
    // Down to a twentieth of the peak, not the thirty percent this used to
    // keep: a Gaussian is told from its neighbours by its wings, and a fit
    // that never sees them reads a real peak's width off its cap alone -
    // which on bench spectra sat visibly narrower than the data. The wings
    // are safe to admit because of the weighting below; without it they were
    // the channels that hurt most.
    let level = 0.05 * max;
    // Fit in coordinates local to the tallest channel for conditioning.
    //
    // Each row is scaled by its own count, which turns the plain least
    // squares below into the count-squared-weighted fit of the logarithm.
    // Taking logarithms makes a parabola of a Gaussian, but it also inflates
    // the noise of the small channels: a wing channel of a few counts swings
    // its logarithm by whole units where the cap's channels move by parts in
    // a thousand, so an unweighted fit is steered by exactly the channels
    // that know the least. Weighting by the count undoes the inflation -
    // the variance of ln(y) is about 1/y for counting data - and the wings
    // then inform the width without deciding it.
    let rows: Vec<([f64; 3], f64)> = net
        .iter()
        .enumerate()
        .filter(|(_, value)| **value >= level && **value > 0.0)
        .map(|(index, value)| {
            let x = index as f64 - peak_index as f64;
            ([*value, value * x, value * x * x], value * value.ln())
        })
        .collect();
    if rows.len() < 3 {
        return None;
    }
    let [a, b, c] = least_squares3(&rows)?;
    if c >= 0.0 {
        return None;
    }
    let local_centroid = -b / (2.0 * c);
    let sigma = (-1.0 / (2.0 * c)).sqrt();
    let amplitude = (a - b * b / (4.0 * c)).exp();
    if !sigma.is_finite() || sigma <= 0.0 || !local_centroid.is_finite() || !amplitude.is_finite() {
        return None;
    }
    Some(GaussianFit {
        amplitude,
        centroid: local_centroid + (peak_index + offset) as f64,
        sigma,
    })
}

/// A peak located by [`peak_search`].
#[derive(Clone, Copy, Debug, PartialEq, Serialize, Deserialize)]
pub struct FoundPeak {
    /// Channel of the tallest channel of the peak.
    pub channel: usize,
    /// Interpolated centroid, in channels.
    pub centroid: f64,
    /// Measured full width at half maximum, in channels.
    pub width: f64,
    /// Signal-to-noise of the matched-filter response, in Poisson sigmas.
    pub significance: f64,
    /// Rough net area above the local baseline.
    pub net_estimate: f64,
}

/// Matched-filter peak search.
///
/// One zero-sum kernel whose width follows the detector's resolution is laid
/// over every channel; a peak is a local maximum of the resulting
/// signal-to-noise map that also passes a width gate. The method is
/// Becquerel's peak finder, reimplemented in the `matched` module, where its
/// provenance and this program's departures from it are written down. It
/// replaced a nine-scale second-difference ladder that had no idea what a
/// peak *should* look like at a given channel - on the bench Cs-137 spectrum
/// that decided the matter, the ladder reported twenty-four peaks of which
/// about half were one-channel statistical spikes; the matched filter
/// reported the seven that are physics.
///
/// The width at each channel comes from the spectrum's own shape calibration
/// when it holds a usable one, and is otherwise learned from the spectrum
/// itself by a bootstrap pass. The sensitivity setting is the signal-to-noise
/// a peak must reach - the same Poisson sigmas the ladder thresholded on, so
/// the dial keeps its meaning.
pub fn peak_search(spectrum: &Spectrum, settings: &CalculationSettings) -> Vec<FoundPeak> {
    let data = spectrum.as_f64();
    if data.len() < 16 {
        return Vec::new();
    }
    let min_snr = settings.sensitivity.clamp(
        CalculationSettings::MIN_SENSITIVITY,
        CalculationSettings::MAX_SENSITIVITY,
    ) as f64;
    let Some(widths) = crate::matched::widths_for(spectrum, &data, min_snr.min(3.0)) else {
        return Vec::new();
    };
    crate::matched::find(&data, &widths, min_snr, crate::matched::width_gate(), 40)
        .into_iter()
        .map(|found| FoundPeak {
            channel: found.centroid.round().max(0.0) as usize,
            centroid: found.centroid,
            width: found.fwhm,
            significance: found.snr,
            net_estimate: found.area,
        })
        .collect()
}

/// Runs a peak search and marks every peak found as a region of interest.
///
/// The region spans three times the FWHM - the calculated width when a
/// peak-shape calibration exists, the measured width otherwise. MAESTRO's manual
/// says an uncalibrated search marks "the width of the peak as determined by
/// Peak Search"; we widen that to three FWHM as well, because the background
/// points of equations (17)-(21) must fall outside the peak to give a correct
/// net area. The region is never narrower than `2n + 1` channels, the minimum
/// the background model needs. Existing regions are kept.
///
/// Returns the number of peaks marked.
pub fn mark_peaks(spectrum: &mut Spectrum, settings: &CalculationSettings) -> usize {
    let peaks = peak_search(spectrum, settings);
    let length = spectrum.len();
    let minimum = (2 * settings.background_points + 1) as f64;
    let mut marked = 0;
    for peak in &peaks {
        let fwhm = spectrum.fwhm_at(peak.centroid).unwrap_or(peak.width);
        let width = (3.0 * fwhm).max(minimum);
        let half = width / 2.0;
        let start = (peak.centroid - half).round().max(0.0) as usize;
        let end = ((peak.centroid + half).round().max(0.0) as usize).min(length.saturating_sub(1));
        if end <= start {
            continue;
        }
        spectrum.rois.mark(Roi::new(start, end));
        marked += 1;
    }
    marked
}

/// Five-point area-preserving binomial smoothing, equation (23).
pub fn smoothed(channels: &[u64]) -> Vec<u64> {
    let get = |index: isize| -> u64 {
        if index < 0 || index as usize >= channels.len() {
            0
        } else {
            channels[index as usize]
        }
    };
    (0..channels.len())
        .map(|index| {
            let i = index as isize;
            let sum = get(i - 2) + 4 * get(i - 1) + 6 * get(i) + 4 * get(i + 1) + get(i + 2);
            (sum + 8) / 16
        })
        .collect()
}

/// Smooths a spectrum in place. Times, calibration and regions are untouched.
pub fn smooth(spectrum: &mut Spectrum) {
    spectrum.channels = smoothed(&spectrum.channels);
}

/// How much of the disk spectrum to subtract in a strip operation.
#[derive(Clone, Copy, Debug, PartialEq, Serialize, Deserialize)]
pub enum StripFactor {
    /// An explicit factor; a negative factor adds the spectra.
    Fixed(f64),
    /// The ratio of the two live times (buffer / disk).
    LiveTimeRatio,
}

/// Subtracts `factor * source` from `target`, channel by channel.
///
/// Channel contents are clamped at zero; the times of `target` are unchanged.
pub fn strip(
    target: &mut Spectrum,
    source: &Spectrum,
    factor: StripFactor,
) -> Result<(), AnalysisError> {
    if target.len() != source.len() {
        return Err(AnalysisError::LengthMismatch {
            left: target.len(),
            right: source.len(),
        });
    }
    let factor = match factor {
        StripFactor::Fixed(value) => value,
        StripFactor::LiveTimeRatio => {
            if target.live_time <= 0.0 || source.live_time <= 0.0 {
                return Err(AnalysisError::NoLiveTime);
            }
            target.live_time / source.live_time
        }
    };
    for (index, value) in target.channels.iter_mut().enumerate() {
        let stripped = *value as f64 - factor * source.channels[index] as f64;
        *value = if stripped <= 0.0 {
            0
        } else {
            stripped.round() as u64
        };
    }
    Ok(())
}

/// Sum of the channels in an inclusive range (the Sum command).
pub fn sum_channels(spectrum: &Spectrum, low: usize, high: usize) -> u64 {
    spectrum.sum_range(low, high)
}

/// Counting activity, equation (22): `cA = (100 / percent) * (net / live)`.
pub fn counting_activity(net_counts: f64, yield_percent: f64, live_time: f64) -> f64 {
    if yield_percent <= 0.0 || live_time <= 0.0 {
        return 0.0;
    }
    (100.0 / yield_percent) * (net_counts / live_time)
}

/// Relative statistical (1 sigma) uncertainty, in percent, of the net peak
/// area in a channel range. This drives the uncertainty preset on instruments
/// that support it: acquisition stops once the value falls below the requested
/// limit. The manual defines the preset as "percent uncertainty at 1 sigma of
/// the net peak area", "calculated in the same manner as for the Peak Info
/// command" - so this is Peak Info's equation (21) over its equation (20).
///
/// `None` when the range holds no net peak (pure continuum, or too narrow for
/// the background points): no uncertainty target can have been reached when
/// there is nothing to measure.
pub fn statistical_uncertainty(spectrum: &Spectrum, low: usize, high: usize) -> Option<f64> {
    let settings = CalculationSettings::default();
    let info = peak_info(spectrum, Roi::new(low, high), &settings).ok()?;
    (info.net_area > 0.0).then(|| 100.0 * info.net_area_uncertainty / info.net_area)
}

/// Currie minimum detectable activity: `(2.71 + 4.65*sqrt(B)) / (eff * yield * live)`.
///
/// `efficiency` and `yield_fraction` are fractions, not percentages. Returns
/// infinity when the denominator vanishes.
pub fn mda_currie(
    background_counts: f64,
    efficiency: f64,
    yield_fraction: f64,
    live_time: f64,
) -> f64 {
    let denominator = efficiency * yield_fraction * live_time;
    let numerator = 2.71 + 4.65 * background_counts.max(0.0).sqrt();
    if denominator <= 0.0 {
        return f64::INFINITY;
    }
    numerator / denominator
}

#[cfg(test)]
mod tests {
    use super::*;

    /// A Gaussian on a small pedestal, as a net-counts slice.
    fn gaussian_net(amplitude: f64, centre: f64, sigma: f64, length: usize) -> Vec<f64> {
        (0..length)
            .map(|channel| {
                let x = (channel as f64 - centre) / sigma;
                amplitude * (-0.5 * x * x).exp()
            })
            .collect()
    }

    /// A clean Gaussian fits back to its own parameters.
    #[test]
    fn the_gaussian_fit_recovers_a_clean_peak() {
        let net = gaussian_net(1_000.0, 40.0, 4.0, 81);
        let fit = fit_gaussian(&net, 100).expect("a clear peak fits");
        assert!((fit.centroid - 140.0).abs() < 0.01, "{}", fit.centroid);
        assert!((fit.sigma - 4.0).abs() < 0.02, "{}", fit.sigma);
        assert!((fit.amplitude - 1_000.0).abs() < 5.0, "{}", fit.amplitude);
    }

    /// Noisy wings inform the fit without deciding it.
    ///
    /// The fit reaches down to a twentieth of the peak so a real peak's width
    /// is read from its wings and not its cap alone - and the wing channels
    /// are the noisiest ones there are, which is why each row is weighted by
    /// its own count. This doubles and halves alternate far-wing channels, a
    /// caricature of counting noise, and the fit must hold to the truth.
    #[test]
    fn noisy_wings_do_not_steer_the_gaussian_fit() {
        let mut net = gaussian_net(1_000.0, 40.0, 4.0, 81);
        let cap = 1_000.0;
        let mut flip = false;
        for value in net.iter_mut() {
            if *value < 0.1 * cap && *value > 0.0 {
                *value *= if flip { 2.0 } else { 0.5 };
                flip = !flip;
            }
        }
        let fit = fit_gaussian(&net, 0).expect("still a peak");
        assert!(
            (fit.centroid - 40.0).abs() < 0.2,
            "the centroid moved to the noise: {}",
            fit.centroid
        );
        assert!(
            (fit.sigma - 4.0).abs() < 0.2,
            "the width was read off the noise: {}",
            fit.sigma
        );
    }
}
