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
    let level = 0.3 * max;
    // Fit in coordinates local to the tallest channel for conditioning.
    let rows: Vec<([f64; 3], f64)> = net
        .iter()
        .enumerate()
        .filter(|(_, value)| **value >= level && **value > 0.0)
        .map(|(index, value)| {
            let x = index as f64 - peak_index as f64;
            ([1.0, x, x * x], value.ln())
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
    /// Significance of the second-difference response (how many weighted errors).
    pub significance: f64,
    /// Rough net area above the local baseline.
    pub net_estimate: f64,
}

/// Kernel scales (in channels) scanned by the peak search.
///
/// The geometric ladder covers everything from a one-channel spike to the very
/// broad peaks of a scintillator (a NaI(Tl) line can be 100 channels wide),
/// which a single fixed width cannot.
const SEARCH_SCALES: [f64; 9] = [1.0, 1.5, 2.25, 3.375, 5.0, 7.5, 11.25, 17.0, 25.5];

/// Mariscotti-style peak search.
///
/// For every scale the spectrum is convolved with a zero-sum second-difference
/// kernel; a peak is reported where the response exceeds the sensitivity factor
/// times its own weighted error over at least two adjacent channels. Detections
/// from the different scales are then merged, keeping the most significant.
pub fn peak_search(spectrum: &Spectrum, settings: &CalculationSettings) -> Vec<FoundPeak> {
    let data = spectrum.as_f64();
    if data.len() < 9 {
        return Vec::new();
    }
    let threshold = settings.sensitivity.clamp(
        CalculationSettings::MIN_SENSITIVITY,
        CalculationSettings::MAX_SENSITIVITY,
    ) as f64;

    // With a peak-shape calibration in hand, a candidate whose measured width
    // disagrees wildly with the calibrated resolution is not a gamma peak:
    // narrow spikes are electronic artefacts (real files end with them at the
    // top of the ADC range) and very wide ones are continuum structure.
    let shape = spectrum.shape_calibration.filter(|shape| shape.is_usable());

    let mut candidates: Vec<FoundPeak> = Vec::new();
    // One response buffer and one squared-kernel scratch for all nine scales;
    // this used to allocate a spectrum-sized buffer per scale.
    let mut significance = Vec::new();
    let mut squares = Vec::new();
    for sigma in SEARCH_SCALES {
        let kernel = second_difference_kernel(sigma);
        let half = kernel.len() / 2;
        if data.len() <= 2 * half + 2 {
            continue;
        }
        squares.clear();
        squares.extend(kernel.iter().map(|k| k * k));
        response(&data, &kernel, &squares, half, &mut significance);

        let mut index = half;
        while index < data.len() - half {
            if significance[index] <= threshold {
                index += 1;
                continue;
            }
            let start = index;
            while index < data.len() - half && significance[index] > threshold {
                index += 1;
            }
            let end = index - 1;
            if end - start + 1 < 2 {
                continue;
            }
            let apex = (start..=end)
                .max_by(|a, b| {
                    significance[*a]
                        .partial_cmp(&significance[*b])
                        .unwrap_or(std::cmp::Ordering::Equal)
                })
                .unwrap_or(start);
            if let Some(peak) = describe_peak(&data, apex, sigma, significance[apex])
                && shape_consistent(shape.as_ref(), &peak)
            {
                candidates.push(peak);
            }
        }
    }
    merge_candidates(candidates)
}

/// Narrowest fraction of the calibrated FWHM a real peak may measure.
///
/// Deliberately generous: real files often carry a *constant* shape calibration
/// fitted at high energy, so a genuine low-energy peak can be far narrower than
/// it predicts. In one of the test fixtures the shape calibration says 11.4
/// channels everywhere while the 59.5 keV americium peak is a clean 3.3 channels
/// wide. Only spikes an order of magnitude too narrow are rejected.
const MIN_WIDTH_FRACTION: f64 = 0.2;
/// Widest multiple of the calibrated FWHM a real peak may measure.
///
/// A peak cannot be much broader than the detector resolution; anything that is
/// belongs to the continuum (escape structure, Compton edges).
const MAX_WIDTH_FACTOR: f64 = 3.0;

/// True when a candidate's measured width agrees with the shape calibration, or
/// when there is no shape calibration to judge it by.
fn shape_consistent(
    shape: Option<&crate::calibration::ShapeCalibration>,
    peak: &FoundPeak,
) -> bool {
    let Some(shape) = shape else {
        return true;
    };
    let expected = shape.fwhm(peak.centroid);
    if expected <= 0.0 {
        return true;
    }
    let ratio = peak.width / expected;
    (MIN_WIDTH_FRACTION..=MAX_WIDTH_FACTOR).contains(&ratio)
}

/// Zero-sum discrete second-difference kernel of width `sigma`.
///
/// `k(j) = (1 - j^2/sigma^2) * exp(-j^2 / 2 sigma^2)`, shifted so the
/// coefficients sum to exactly zero: a flat or linear background gives no
/// response at all.
fn second_difference_kernel(sigma: f64) -> Vec<f64> {
    let half = (3.0 * sigma).ceil().max(1.0) as isize;
    let mut kernel: Vec<f64> = (-half..=half)
        .map(|j| {
            let x = j as f64 / sigma;
            (1.0 - x * x) * (-0.5 * x * x).exp()
        })
        .collect();
    let mean = kernel.iter().sum::<f64>() / kernel.len() as f64;
    kernel.iter_mut().for_each(|k| *k -= mean);
    kernel
}

/// Response of the kernel divided by its weighted (Poisson) error.
///
/// `squares` is the kernel's coefficients squared, computed once per scale
/// rather than once per tap - the variance sum is the same taps again, and
/// squaring inside the inner loop doubled its multiplies. Written into `out`
/// so the caller can keep one buffer across scales.
fn response(data: &[f64], kernel: &[f64], squares: &[f64], half: usize, out: &mut Vec<f64>) {
    out.clear();
    out.resize(data.len(), 0.0);
    for index in half..data.len().saturating_sub(half) {
        let window = &data[index - half..=index + half];
        let mut sum = 0.0;
        let mut variance = 0.0;
        for ((value, k), k2) in window.iter().zip(kernel).zip(squares) {
            sum += k * value;
            variance += k2 * value.max(1.0);
        }
        out[index] = if variance > 0.0 {
            sum / variance.sqrt()
        } else {
            0.0
        };
    }
}

/// Measures centroid, width and rough area around a detected apex.
fn describe_peak(data: &[f64], apex: usize, sigma: f64, significance: f64) -> Option<FoundPeak> {
    let reach = (2.0 * sigma).ceil().max(2.0) as usize;
    let start = apex.saturating_sub(reach);
    let end = (apex + reach).min(data.len() - 1);
    if end <= start + 2 {
        return None;
    }
    // Local baseline: a line between the mean of the two channels at each edge.
    let y_low = (data[start] + data[start + 1]) / 2.0;
    let y_high = (data[end] + data[end - 1]) / 2.0;
    let x_low = start as f64 + 0.5;
    let x_high = end as f64 - 0.5;
    let slope = (y_high - y_low) / (x_high - x_low);
    let net: Vec<f64> = (start..=end)
        .map(|channel| data[channel] - (y_low + slope * (channel as f64 - x_low)))
        .collect();

    let local_apex = net
        .iter()
        .enumerate()
        .max_by(|a, b| a.1.partial_cmp(b.1).unwrap_or(std::cmp::Ordering::Equal))
        .map(|(index, _)| index)?;
    let max_net = net[local_apex];
    if max_net <= 0.0 {
        return None;
    }
    let mut width = width_at_level(&net, local_apex, max_net / 2.0);
    if width <= 0.0 {
        width = FWHM_PER_SIGMA * sigma;
    }
    let centroid = fit_gaussian(&net, start)
        .map(|fit| fit.centroid)
        .or_else(|| moment_centroid(&net, start))
        .unwrap_or((start + local_apex) as f64);
    let net_estimate = net.iter().filter(|v| **v > 0.0).sum();

    Some(FoundPeak {
        channel: start + local_apex,
        centroid,
        width,
        significance,
        net_estimate,
    })
}

/// Keeps the most significant detection of each peak across scales.
fn merge_candidates(mut candidates: Vec<FoundPeak>) -> Vec<FoundPeak> {
    candidates.sort_by(|a, b| {
        a.centroid
            .partial_cmp(&b.centroid)
            .unwrap_or(std::cmp::Ordering::Equal)
    });
    let mut merged: Vec<FoundPeak> = Vec::new();
    for candidate in candidates {
        match merged.last_mut() {
            Some(previous)
                if (candidate.centroid - previous.centroid).abs()
                    <= previous.width.max(candidate.width).max(1.0) =>
            {
                if candidate.significance > previous.significance {
                    *previous = candidate;
                }
            }
            _ => merged.push(candidate),
        }
    }
    merged.sort_by(|a, b| {
        a.centroid
            .partial_cmp(&b.centroid)
            .unwrap_or(std::cmp::Ordering::Equal)
    });
    merged
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

    #[test]
    fn the_search_kernel_sums_to_zero() {
        for sigma in SEARCH_SCALES {
            let kernel = second_difference_kernel(sigma);
            let sum: f64 = kernel.iter().sum();
            assert!(sum.abs() < 1e-12, "sigma {sigma}: sum {sum}");
            assert!(kernel.len() % 2 == 1, "kernel must be centred");
        }
    }

    /// The squared coefficients `response` expects beside a kernel.
    fn squared(kernel: &[f64]) -> Vec<f64> {
        kernel.iter().map(|k| k * k).collect()
    }

    #[test]
    fn a_flat_background_gives_no_response() {
        let data = vec![250.0; 64];
        let kernel = second_difference_kernel(2.0);
        let half = kernel.len() / 2;
        let mut z = Vec::new();
        response(&data, &kernel, &squared(&kernel), half, &mut z);
        assert!(z[32].abs() < 1e-9, "got {}", z[32]);
    }

    #[test]
    fn a_sloping_background_gives_no_response() {
        let data: Vec<f64> = (0..64).map(|c| 500.0 - 3.0 * c as f64).collect();
        let kernel = second_difference_kernel(2.0);
        let half = kernel.len() / 2;
        let mut z = Vec::new();
        response(&data, &kernel, &squared(&kernel), half, &mut z);
        assert!(z[32].abs() < 1e-6, "got {}", z[32]);
    }
}
