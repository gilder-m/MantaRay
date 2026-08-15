//! Spectrum analysis: peak information, peak search, smoothing and stripping.
//!
//! The area and uncertainty expressions are MAESTRO equations (17)-(21); the
//! activity expression is equation (22) and the smoothing kernel equation (23).

use serde::{Deserialize, Serialize};

use crate::FWHM_PER_SIGMA;
use crate::calibration::{EnergyCalibration, least_squares3};
use crate::error::AnalysisError;
use crate::fit::FittedPeak;
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
    /// Every line the region turned out to hold, when it holds more than one.
    ///
    /// Empty for an ordinary single peak, so nothing that reads a region as
    /// one peak has to change. When it is not empty the region is a
    /// multiplet, [`PeakInfo::centroid`] and [`PeakInfo::fit`] describe the
    /// strongest of these lines rather than the midpoint of the bump, and the
    /// per-line areas are here.
    pub multiplet: Vec<FittedPeak>,
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

/// Computes the Peak Info figures for a region, as one peak.
///
/// Cheap and unconditional: this is what the region tables, the reports and
/// the acquisition presets call, sometimes for every region on every frame.
/// It never asks whether the region holds more than one line - see
/// [`resolved_peak_info`] for that, which costs some tens of milliseconds and
/// is meant for one region at a time.
pub fn peak_info(
    spectrum: &Spectrum,
    roi: Roi,
    settings: &CalculationSettings,
) -> Result<PeakInfo, AnalysisError> {
    peak_info_inner(spectrum, roi, settings, false)
}

/// Peak Info for a region, having first asked how many lines are in it.
///
/// Everything [`peak_info`] reports, plus [`PeakInfo::multiplet`] when the
/// counts turn out to hold more than one line - in which case the centroid
/// and the fit describe the strongest of them rather than the middle of the
/// bump. This is what an operator asking about one particular peak should
/// get; it fits the whole region several times over and takes long enough
/// that running it across a table of regions would be felt.
pub fn resolved_peak_info(
    spectrum: &Spectrum,
    roi: Roi,
    settings: &CalculationSettings,
) -> Result<PeakInfo, AnalysisError> {
    peak_info_inner(spectrum, roi, settings, true)
}

fn peak_info_inner(
    spectrum: &Spectrum,
    roi: Roi,
    settings: &CalculationSettings,
    resolve: bool,
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

    let mut fit = fit_gaussian(&net, l);
    let mut centroid = match &fit {
        Some(fit) => fit.centroid,
        None => moment_centroid(&net, l).unwrap_or_else(|| roi.center()),
    };

    // Is this one line or several? The single-Gaussian fit above cannot tell -
    // it is a parabola through one bump's logarithm and will fit the middle
    // of a doublet as confidently as it fits a real peak, reporting a width
    // half again too large and a centroid that belongs to neither line. So
    // the whole region is fitted at once and asked how many peaks it holds.
    // Only a region that turns out to hold more than one is treated
    // differently; everything else keeps the measurement it always had.
    let multiplet = match resolve {
        true => multiplet_in(&spectrum.as_f64(), roi, fwhm, max_net),
        false => Vec::new(),
    };
    if let Some(strongest) = multiplet
        .iter()
        .max_by(|a, b| {
            a.amplitude
                .partial_cmp(&b.amplitude)
                .unwrap_or(std::cmp::Ordering::Equal)
        })
        .filter(|_| multiplet.len() > 1)
    {
        centroid = strongest.centroid;
        fit = Some(GaussianFit {
            amplitude: strongest.amplitude,
            centroid: strongest.centroid,
            sigma: strongest.sigma,
        });
    }

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
        multiplet,
    })
}

/// The lines a region holds, when it holds more than one.
///
/// Empty for a region the joint fit reads as a single peak - which is the
/// common case and the one that must stay cheap and unchanged - and empty
/// when the fit cannot be made at all.
///
/// The width to start from is the region's own measured FWHM. The shape
/// calibration is deliberately not consulted: a file's `$SHAPE_CAL` is
/// written whether or not anyone calibrated shape, and a fit seeded from a
/// wrong width searches the wrong neighbourhood. The measured width is
/// inflated when the region really is a doublet, but the fit shrinks it as it
/// adds peaks, so an inflated start is the right kind of wrong.
fn multiplet_in(counts: &[f64], roi: Roi, measured_fwhm: f64, max_net: f64) -> Vec<FittedPeak> {
    if max_net <= 0.0 {
        return Vec::new();
    }
    let fwhm = if measured_fwhm > 1.0 {
        measured_fwhm
    } else {
        roi.len() as f64 / 4.0
    };
    let fit = crate::fit::fit_multiplet(
        counts,
        roi,
        roi.center(),
        fwhm / FWHM_PER_SIGMA,
        MOST_LINES_IN_A_REGION,
        crate::fit::FitOptions::default(),
    );
    match fit {
        Some(fit) if fit.peaks.len() > 1 && believable_lines(&fit.peaks, fwhm) => fit.peaks,
        _ => Vec::new(),
    }
}

/// Whether a set of fitted lines is a resolution of the bump or a decoration
/// of something that was never a peak.
///
/// The same two questions wherever a region is separated, so that
/// double-clicking a bump and searching for it agree about what a line is.
/// Both are asked against the bump itself rather than against the width law,
/// because when a bump really is two lines the law has already been taught
/// that bump's merged width and is the wrong ruler for its parts.
///
/// How much narrower may the parts be? A merged pair reads about twice a
/// single line's width when the two are a width apart, and three times at two
/// widths, so a third of the whole is as narrow as an honest part gets.
/// Narrower is not a resolution but a spike: the overflow wall at the top of
/// one bench background - a jagged pile-up artefact, never a peak - offered
/// three "lines" a quarter the width of the bump holding them. Wider than the
/// bump is not a resolution either, since separating something can only make
/// its parts smaller than the whole; the low-level discriminator's cut-on at
/// the bottom of every spectrum, which is a peak's rising flank with no peak
/// behind it, offered lines half again wider than itself.
///
/// And a line far smaller than its neighbour is much more likely to be the
/// strong peak's own shape than a separate line: a tail not quite caught, a
/// percent of asymmetry. Shape errors are small and a real line has no reason
/// to be. Ba-133's genuine 79.6 and 81.0 keV pair stands at 8% and is kept;
/// the companion the fit offered beside the same source's 356 keV line, which
/// the counts show as a clean single peak, stood at 1.3% and is not.
fn believable_lines(lines: &[FittedPeak], bump_fwhm: f64) -> bool {
    let strongest = lines.iter().map(|line| line.area).fold(0.0f64, f64::max);
    lines.iter().all(|line| {
        let share = line.fwhm() / bump_fwhm.max(f64::EPSILON);
        (SPLIT_WIDTH_BAND.0..=SPLIT_WIDTH_BAND.1).contains(&share)
            && line.area >= MIN_COMPANION_SHARE * strongest
    })
}

/// The half-width of a region drawn around a peak to ask what it holds.
///
/// Three widths each side, and never less than twenty channels. The size is
/// set by what could overlap the peak rather than by where the peak ends,
/// which is why it is so much wider than the peak: a companion has to sit a
/// full width inside the window before the fit will believe in it - against
/// the window's edge a Gaussian is fitted from one side only and can be
/// anything - so a window drawn just wide enough to contain a neighbour puts
/// that neighbour exactly where it will be disbelieved. At two and a half
/// widths a real four-to-one pair one and a fifth widths apart was lost that
/// way on seven noise realisations in sixteen; at three widths, three.
///
/// The floor of twenty channels is what makes the question askable at all on
/// a narrow germanium line, which does not fill enough channels to support a
/// two-Gaussian model inside its own three widths. The extra span is nearly
/// all continuum, which is what the continuum term is for.
///
/// Everything that draws its own region uses this, so that double-clicking a
/// bump and searching for it are asking about the same channels. They are
/// not otherwise: with the search looking at forty channels and the
/// double-click at thirty, the same overflow wall came back as three pieces
/// to one and two to the other.
pub fn resolving_region(centroid: f64, fwhm: f64, length: usize) -> Option<Roi> {
    let half = (3.0 * fwhm).ceil().max(20.0);
    let low = (centroid - half).max(0.0) as usize;
    let high = ((centroid + half) as usize).min(length.saturating_sub(1));
    (high > low).then(|| Roi::new(low, high))
}

/// How many lines one region is allowed to resolve into.
///
/// Three covers every multiplet an operator marks by hand - Eu-152's 1,085
/// and 1,089 keV pair, Ba-133's 79 and 81, a photopeak with a
/// single-escape line leaning on it. Past three, a region wide enough to
/// hold four lines is a region that should have been marked as two.
const MOST_LINES_IN_A_REGION: usize = 3;

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
    /// Expected full width at half maximum at the centroid, in channels -
    /// from the shape calibration when the counts agree with it, otherwise
    /// from the width law the spectrum taught itself.
    pub width: f64,
    /// How firmly the peak is held, in sigmas: the matched filter's
    /// signal-to-noise for a peak the filter found on its own, and the
    /// fitted area over its own uncertainty for one separated out of a
    /// multiplet. Both answer "how sure are we", on the same scale, which is
    /// what the sensitivity setting and the neighbour resolution need of it.
    pub significance: f64,
    /// Rough net area above the local baseline.
    pub net_estimate: f64,
}

/// Matched-filter peak search.
///
/// One zero-sum kernel whose width follows the detector's resolution is laid
/// over every channel; a peak is a local maximum of the resulting
/// signal-to-noise map that also passes a width gate. The method comes from
/// the Becquerel project, rewritten in Rust in the `matched` module, where
/// it is described. It
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
    // The cap is a runaway guard, not a quota: the bench background carries
    // 38 genuine lines and lost eight of them to a cap of 40, each pushed
    // out by a stronger real line. A hundred covers the busiest natural
    // spectrum with room to spare.
    let found: Vec<FoundPeak> =
        crate::matched::find(&data, &widths, min_snr, crate::matched::width_gate(), 100)
            .into_iter()
            .map(|found| FoundPeak {
                channel: found.centroid.round().max(0.0) as usize,
                centroid: found.centroid,
                width: found.fwhm,
                significance: found.snr,
                net_estimate: found.area,
            })
            .collect();
    resolve_multiplets(&data, found)
}

/// The smallest share of a region's strongest line that a companion may hold.
///
/// See [`believable_lines`].
const MIN_COMPANION_SHARE: f64 = 0.05;

/// How wide a split's parts may be, as a share of the bump they came from.
///
/// See [`believable_lines`] for what each end is defending against.
const SPLIT_WIDTH_BAND: (f64, f64) = (0.33, 1.15);

/// Splits any peak the filter merged, by fitting its region.
///
/// The matched filter answers with one maximum per bump, and two lines closer
/// than about one and a fifth of a width make one bump: the kernel is as wide
/// as a peak, so it smooths a pair that close into a single response. Worse,
/// the merge is self-sustaining - the width the bump measures is half again a
/// real peak's, the spectrum's width law learns that width, and a kernel
/// built to the learned width is now far too wide to ever separate the pair
/// that taught it. Two lines a full width apart were merged into one 27
/// channels wide where the truth is 14.
///
/// Fitting the region settles it, because a fit is not smoothing anything:
/// two Gaussians and one continuum either account for the counts better than
/// one Gaussian or they do not. Only where the fit is sure - see
/// [`crate::fit`] for what "sure" costs - is the peak replaced by its parts.
fn resolve_multiplets(counts: &[f64], found: Vec<FoundPeak>) -> Vec<FoundPeak> {
    let mut resolved: Vec<FoundPeak> = Vec::with_capacity(found.len());
    for peak in found {
        let mut parts = split_of(counts, &peak);
        match parts.len() {
            0 => resolved.push(peak),
            _ => resolved.append(&mut parts),
        }
    }
    // Neighbouring regions overlap, so the same line can be found twice -
    // once on its own account and once inside its neighbour's region. The
    // stronger word wins, exactly as it does in the search itself.
    resolved.sort_by(|a, b| {
        b.significance
            .partial_cmp(&a.significance)
            .unwrap_or(std::cmp::Ordering::Equal)
    });
    let mut kept: Vec<FoundPeak> = Vec::with_capacity(resolved.len());
    for peak in resolved {
        let separation = (peak.width * 0.5).max(2.0);
        if kept
            .iter()
            .all(|held| (held.centroid - peak.centroid).abs() >= separation)
        {
            kept.push(peak);
        }
    }
    kept.sort_by(|a, b| {
        a.centroid
            .partial_cmp(&b.centroid)
            .unwrap_or(std::cmp::Ordering::Equal)
    });
    kept
}

/// The lines one found peak turns out to be, or empty if it is just itself.
fn split_of(counts: &[f64], peak: &FoundPeak) -> Vec<FoundPeak> {
    let Some(region) = resolving_region(peak.centroid, peak.width, counts.len()) else {
        return Vec::new();
    };
    let fit = crate::fit::fit_multiplet(
        counts,
        region,
        peak.centroid,
        peak.width / FWHM_PER_SIGMA,
        MOST_LINES_IN_A_REGION,
        crate::fit::FitOptions::default(),
    );
    let Some(fit) = fit.filter(|fit| fit.peaks.len() > 1) else {
        return Vec::new();
    };
    if !believable_lines(&fit.peaks, peak.width) {
        return Vec::new();
    }
    fit.peaks
        .iter()
        .map(|line| FoundPeak {
            channel: line.centroid.round().max(0.0) as usize,
            centroid: line.centroid,
            width: line.fwhm(),
            significance: line.significance(),
            net_estimate: line.area,
        })
        .collect()
}

/// Runs a peak search and marks every peak found as a region of interest.
///
/// The region spans three times the width the search assigned the peak -
/// its audited shape calibration or its learned width law, whichever the
/// search itself trusted. Reading the raw shape calibration here instead
/// would resurrect exactly the claims the audit overruled: the bench
/// background's stale constant sized every region 1.6 channels wide.
/// MAESTRO's manual says an uncalibrated search marks "the width of the
/// peak as determined by Peak Search"; we widen that to three FWHM as well,
/// because the background points of equations (17)-(21) must fall outside
/// the peak to give a correct net area.
///
/// The `n` background channels at each end are then given their own room
/// outside that span, rather than being taken out of it. Three FWHM is
/// where the peak ends; a region exactly that wide puts its innermost
/// background channel around two sigma from the centroid, where a Gaussian
/// still stands at a tenth of its height. The background is read off the
/// peak's own shoulders, comes out too high, and equation (20) then returns
/// a net area a tenth to a seventh short - measured against a generous
/// hand-drawn region on the bench corpus, Co-60's 1,332 keV line lost 14%
/// and Cs-137's 662 keV line 10%. The fit felt it too, reading every line
/// narrower than the counts do. With the background points outside, the
/// same lines come back within a few percent and the fitted width matches
/// the measured one. The region is never narrower than `2n + 1` channels,
/// the minimum the background model needs. Existing regions are kept.
///
/// Returns the number of peaks marked.
pub fn mark_peaks(spectrum: &mut Spectrum, settings: &CalculationSettings) -> usize {
    let peaks = peak_search(spectrum, settings);
    let length = spectrum.len();
    let n = settings.background_points.clamp(
        CalculationSettings::MIN_BACKGROUND_POINTS,
        CalculationSettings::MAX_BACKGROUND_POINTS,
    );
    let minimum = (2 * n + 1) as f64;
    let mut marked = 0;
    for peak in &peaks {
        let width = (3.0 * peak.width + 2.0 * n as f64).max(minimum);
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

    /// Counts holding Gaussians on a sloping continuum.
    fn region_counts(peaks: &[(f64, f64, f64)]) -> Vec<f64> {
        (0..400)
            .map(|channel| {
                let x = channel as f64;
                3000.0 - 4.0 * x
                    + peaks
                        .iter()
                        .map(|(centre, sigma, area)| {
                            let z = (x - centre) / sigma;
                            area / (sigma * (2.0 * std::f64::consts::PI).sqrt())
                                * (-0.5 * z * z).exp()
                        })
                        .sum::<f64>()
            })
            .collect()
    }

    fn bump(centroid: f64, width: f64) -> FoundPeak {
        FoundPeak {
            channel: centroid.round() as usize,
            centroid,
            width,
            significance: 50.0,
            net_estimate: 0.0,
        }
    }

    /// A whisper beside a shout is the shout's own shape, not a second line.
    ///
    /// This is Ba-133's 356 keV line as the bench corpus has it: a clean
    /// single peak that a fit will nonetheless improve on by adding a
    /// companion a fiftieth its size, because no Gaussian is exactly the
    /// shape of a real peak. A companion has to hold a real share of the
    /// region before it is a line.
    #[test]
    fn a_companion_far_smaller_than_its_neighbour_is_not_a_line() {
        let sigma = 6.0;
        let fwhm = FWHM_PER_SIGMA * sigma;
        let counts = region_counts(&[(200.0, sigma, 400_000.0), (200.0 + fwhm, sigma, 8_000.0)]);
        // Two percent of its neighbour: under the share guard, not a line.
        let parts = split_of(&counts, &bump(200.0, fwhm));
        assert!(
            parts.is_empty(),
            "a 2% companion was reported as a line: {:?}",
            parts.iter().map(|p| p.centroid).collect::<Vec<_>>()
        );
        // Ten percent of it, and the same fit is believed - so the guard is
        // about the share, not about refusing to split at all.
        let counts = region_counts(&[(200.0, sigma, 400_000.0), (200.0 + fwhm, sigma, 40_000.0)]);
        let parts = split_of(&counts, &bump(200.0, fwhm));
        assert_eq!(parts.len(), 2, "a 10% companion is a line: {parts:?}");
    }

    /// The rule that decides whether a set of fitted lines is a resolution.
    ///
    /// Stated directly because the two ends of it defend against different
    /// real things, and both were found on real files rather than reasoned
    /// out. Below a third of the bump's width the "lines" are the
    /// one-channel structure of an overflow artefact; above the bump's own
    /// width they are decoration on an edge - the discriminator cut-on at the
    /// bottom of every spectrum came back as two lines and then three, each
    /// wider than the bump containing them, which cannot be what separating
    /// something does.
    #[test]
    fn a_resolution_makes_parts_smaller_than_the_whole() {
        let line = |centroid: f64, fwhm: f64, area: f64| FittedPeak {
            centroid,
            sigma: fwhm / FWHM_PER_SIGMA,
            amplitude: area / (fwhm / FWHM_PER_SIGMA * (2.0 * std::f64::consts::PI).sqrt()),
            area,
            area_uncertainty: area / 50.0,
        };
        // Two honest halves of a ten-channel bump.
        assert!(believable_lines(
            &[line(100.0, 7.0, 5_000.0), line(107.0, 7.0, 5_000.0)],
            10.0
        ));
        // Wider than the bump they came out of: an edge, not a pair.
        assert!(!believable_lines(
            &[line(100.0, 13.0, 5_000.0), line(107.0, 13.0, 5_000.0)],
            10.0
        ));
        // A quarter of the bump's width: spikes inside an artefact.
        assert!(!believable_lines(
            &[line(100.0, 2.5, 5_000.0), line(107.0, 2.5, 5_000.0)],
            10.0
        ));
        // A companion a fiftieth of its neighbour: the strong line's shape.
        assert!(!believable_lines(
            &[line(100.0, 7.0, 5_000.0), line(107.0, 7.0, 100.0)],
            10.0
        ));
        // A companion a tenth of it, which Ba-133's 79.6 keV line is: a line.
        assert!(believable_lines(
            &[line(100.0, 7.0, 5_000.0), line(107.0, 7.0, 500.0)],
            10.0
        ));
    }

    /// A step with nothing behind it is not a line, however it is decorated.
    ///
    /// Every spectrum begins with one: below the low-level discriminator the
    /// counts are exactly zero, and at it they step to the continuum inside a
    /// channel. That edge is a peak's rising flank with no peak behind it, and
    /// the fit will put lines on it - wider ones than the bump it was asked
    /// about, which is the tell, because separating something can only make
    /// its parts smaller than the whole. On one bench Cs-137 spectrum this
    /// edge came back as two lines and then three.
    #[test]
    fn a_step_with_nothing_behind_it_is_not_a_line() {
        // Nothing at all, then a flat continuum: the discriminator's cut-on.
        let counts: Vec<f64> = (0..400)
            .map(|channel| if channel < 190 { 0.0 } else { 150.0 })
            .collect();
        // What the search reports there: a narrow bump on the edge itself.
        let parts = split_of(&counts, &bump(192.0, 2.2));
        assert!(
            parts.is_empty(),
            "the cut-on was decorated with lines: {:?}",
            parts
                .iter()
                .map(|p| (p.centroid, p.width))
                .collect::<Vec<_>>()
        );
    }

    /// Pieces far narrower than the bump they came from are not lines either.
    ///
    /// The overflow wall at the top of a spectrum is a jagged pile-up
    /// artefact several channels wide with one-channel structure in it; asked
    /// to resolve it, the fit will happily offer three "lines" a quarter the
    /// width of anything the detector can produce there.
    #[test]
    fn pieces_much_narrower_than_the_bump_are_not_lines() {
        let mut counts = region_counts(&[]);
        // Three narrow spikes inside what a search would call one wide bump.
        for (centre, area) in [(196.0, 60_000.0), (200.0, 60_000.0), (204.0, 60_000.0)] {
            for (channel, value) in counts.iter_mut().enumerate() {
                let z = (channel as f64 - centre) / 0.8;
                *value += area / (0.8 * (2.0 * std::f64::consts::PI).sqrt()) * (-0.5 * z * z).exp();
            }
        }
        // The search would report one bump about 14 channels wide here; its
        // parts measure under two, which is a fifth of that.
        let parts = split_of(&counts, &bump(200.0, 14.0));
        assert!(
            parts.is_empty(),
            "spikes a fifth the bump's width were called lines: {:?}",
            parts
                .iter()
                .map(|p| (p.centroid, p.width))
                .collect::<Vec<_>>()
        );
    }
}
