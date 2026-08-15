//! Fitting a whole region at once: several Gaussians on a shared continuum.
//!
//! A region is not always one peak. Two lines a fifth of a keV apart on a
//! germanium detector, or Ba-133's 79 and 81 keV pair on anything, arrive as
//! one bump with a flat top, and every measurement taken from that bump as if
//! it were a single Gaussian is wrong in the same direction: the width comes
//! back half again too wide, the centroid lands between two lines that are
//! both somewhere else, and the area belongs to neither.
//!
//! So the region is fitted as a whole - `n` Gaussians sharing one width and
//! one continuum - and `n` is decided by the counts rather than assumed. The
//! arrangement follows [InterSpec](https://github.com/sandialabs/InterSpec)
//! (Sandia National Laboratories), whose peak fitter this module is written
//! after in Rust; two of its choices carry most of the weight:
//!
//! * **The linear parameters are never searched for.** Amplitudes, continuum
//!   coefficients and the step are linear in the model, so for any given set
//!   of centroids and width they follow exactly from one small normal-equation
//!   solve. Only the centroids and the width are left to the optimiser, which
//!   turns a nine-parameter search for a doublet into a three-parameter one -
//!   the difference between a fit that finds the answer and one that wanders
//!   off into a local minimum with one Gaussian swallowing the other.
//! * **The peaks in a region share one width.** Two free widths let the fit
//!   explain a single fat peak as a narrow spike riding a broad pedestal,
//!   which fits beautifully and means nothing. One detector has one resolution
//!   at one energy; peaks that sit within a region of each other must have
//!   very nearly the same width, and saying so out loud is what makes an
//!   unresolved doublet separable at all.
//!
//! The step is Becquerel's `gausserf` shape: real photopeaks sit on a
//! continuum that is *higher below the peak than above it*, because every
//! scatter that lands in the peak's energy could also have landed below it. A
//! straight line drawn under such a region has to split the difference, and
//! reads the area low on one side and high on the other.

use serde::{Deserialize, Serialize};

use crate::FWHM_PER_SIGMA;
use crate::calibration::solve_n;
use crate::roi::Roi;

/// One Gaussian of a fitted region.
#[derive(Clone, Copy, Debug, PartialEq, Serialize, Deserialize)]
pub struct FittedPeak {
    /// Fitted centroid, in channels.
    pub centroid: f64,
    /// Fitted standard deviation, in channels - shared across the region.
    pub sigma: f64,
    /// Height above the continuum, in counts.
    pub amplitude: f64,
    /// Analytic area of this Gaussian, `amplitude * sigma * sqrt(2 pi)`.
    pub area: f64,
    /// Uncertainty of the area from the fit's own covariance.
    ///
    /// This is the amplitude's uncertainty scaled by the width; it does not
    /// carry the width's own uncertainty, so it is a floor rather than a
    /// complete error budget. Peak Info's equation (21) remains the reported
    /// net-area uncertainty; this one is for judging whether a peak the fit
    /// proposes is worth believing at all.
    pub area_uncertainty: f64,
}

impl FittedPeak {
    /// Full width at half maximum implied by the shared width.
    pub fn fwhm(&self) -> f64 {
        FWHM_PER_SIGMA * self.sigma
    }

    /// Area over its own uncertainty - how firmly the fit holds this peak.
    pub fn significance(&self) -> f64 {
        if self.area_uncertainty > 0.0 {
            self.area / self.area_uncertainty
        } else {
            0.0
        }
    }
}

/// A whole region as the fit sees it.
#[derive(Clone, Debug, PartialEq, Serialize, Deserialize)]
pub struct RegionFit {
    /// The region fitted.
    pub roi: Roi,
    /// The peaks found in it, in channel order.
    pub peaks: Vec<FittedPeak>,
    /// Continuum coefficients, in the region's own scaled coordinate.
    pub continuum: Vec<f64>,
    /// Height of the step under the peaks, in counts; zero when none was kept.
    pub step: f64,
    /// Height of each peak's low-energy tail, in the same order as `peaks`.
    pub tails: Vec<f64>,
    /// The tail's decay length, in peak widths.
    pub tail_widths: f64,
    /// Weighted sum of squared residuals.
    pub chi_square: f64,
    /// Channels fitted less parameters used.
    pub degrees_of_freedom: usize,
}

impl RegionFit {
    /// Chi-square per degree of freedom; infinite when there are none.
    pub fn reduced_chi_square(&self) -> f64 {
        if self.degrees_of_freedom > 0 {
            self.chi_square / self.degrees_of_freedom as f64
        } else {
            f64::INFINITY
        }
    }

    /// The fitted model - continuum, step and every Gaussian - at a channel.
    pub fn evaluate(&self, channel: f64) -> f64 {
        let mut value = continuum_at(&self.continuum, self.roi, channel);
        if self.step > 0.0 && !self.peaks.is_empty() {
            value += self.step * step_shape(channel, self.step_centre(), self.peaks[0].sigma);
        }
        for (index, peak) in self.peaks.iter().enumerate() {
            let z = (channel - peak.centroid) / peak.sigma;
            value += peak.amplitude * (-0.5 * z * z).exp();
            if let Some(tail) = self.tails.get(index).filter(|tail| **tail > 0.0) {
                value += tail * tail_shape(channel, peak.centroid, peak.sigma, self.tail_widths);
            }
        }
        value
    }

    /// Where the step falls: under the middle of the peaks it belongs to.
    fn step_centre(&self) -> f64 {
        let sum: f64 = self.peaks.iter().map(|peak| peak.centroid).sum();
        sum / self.peaks.len().max(1) as f64
    }
}

/// How much better a fit must be before it may claim one more peak.
///
/// Adding a Gaussian adds two free parameters - where it is and how tall -
/// so a nested chi-square test would call an improvement of 11.8 significant
/// at three sigma. That test does not apply here: the null hypothesis sits on
/// the boundary of the alternative (a peak of zero area is a peak that is not
/// there), and a boundary null makes the nominal distribution optimistic.
///
/// So the number is measured instead. Across forty noise realisations each of
/// plain, curved-continuum, stepped and tailed singlets at twenty thousand,
/// two hundred thousand and two million counts, no singlet the guards below
/// admitted improved by more than 23. The weakest doublet that must still be
/// found - twenty thousand counts, eight tenths of a width apart - improves
/// by at least 41. Forty sits between them.
///
/// The two do overlap once a detector is far enough out of health: a singlet
/// carrying a 12% low-energy tail can improve by 94, which is more than that
/// weakest doublet. There is no threshold that separates those, because at
/// that point the counts genuinely do not distinguish a tail from a
/// companion. What is measured is where the overlap starts - see
/// `a_tailed_peak_is_not_a_doublet`, which pins the clean case at 4%.
const SPLIT_CHI_SQUARE: f64 = 40.0;

/// How close two fitted centroids may come before they are one peak twice.
///
/// Below about half a width the pair is not separable by any amount of
/// chi-square: the sum of two Gaussians that close is a Gaussian to within
/// the noise, and the fit will happily trade a wide single for two narrow
/// overlapping ones without saying anything true.
///
/// No case measured - fourteen bench spectra and every synthetic in this
/// module - reaches this rule any more, because a pair that close is nearly
/// degenerate, its covariance is enormous, and [`MIN_PEAK_SIGNIFICANCE`]
/// turns it away first. It is kept because the one thing it prevents is the
/// worst answer available: two peaks reported at the same place, which reads
/// as a resolved doublet and is not.
const MIN_SEPARATION_FWHM: f64 = 0.5;

/// How much of a peak the fit needs before it will believe in it.
///
/// A Gaussian the fit gives less than four sigma of its own area to is a
/// wiggle it found somewhere to put; both members of a real doublet clear
/// this comfortably.
const MIN_PEAK_SIGNIFICANCE: f64 = 4.0;

/// How far inside the region a fitted peak must sit, in its own widths.
///
/// A region is a window, and the counts outside it are not evidence. A
/// Gaussian placed against the edge is fitted from one side only: half of it
/// is beyond the window and its width and area are whatever the model needs
/// them to be. Worse, the window's own edge behaves like a peak's flank, so a
/// region cut through a *neighbouring* line leaves a wall of counts that a
/// spare Gaussian will happily climb.
///
/// Measured on a resolved pair one and a half widths apart, fitted over a
/// region drawn around one of them: the region's low edge fell across the
/// other line, and the fit answered with a phantom of a quarter the real
/// area sitting six tenths of a width from the far edge. Requiring a full
/// width of clearance rejects it and keeps every genuine member of every
/// doublet tested here, which sit near the middle by construction.
const EDGE_CLEARANCE_FWHM: f64 = 1.0;

/// The low-energy tail's decay length, in widths of the peak it belongs to.
///
/// A real photopeak is not a Gaussian. Charge that took too long to collect,
/// or that escaped the active volume, lands *below* the full-energy peak, so
/// every germanium line carries an exponential tail on its low side. Two
/// widths is the middle of the range detectors actually show, and the length
/// is held there rather than fitted.
///
/// The tail has to be in the model, not left to the residual. Measured on
/// singlets carrying a 12% tail, a fit without this component split *every
/// one* of forty strong peaks into a phantom pair, because a second Gaussian
/// sitting low is the only thing an untailed model has to offer.
///
/// Letting the fit choose the length was tried and is worse on both counts,
/// which is why this is a constant. It is one more direction for a simplex
/// that already has one per peak, and the tail's length trades against the
/// peaks' positions: real doublets 20,000 counts strong and one width apart
/// went from resolved forty times in forty to twenty-four, while strong
/// tailed singlets split four times more often, not less. A model with a
/// fixed, roughly-right tail beats a model free to explain a second peak as
/// a long tail.
const TAIL_WIDTHS: f64 = 2.0;

/// The narrowest and widest tail the shape is evaluated for, in peak widths.
///
/// Only a guard on the arithmetic: below half a width a tail is
/// indistinguishable from the peak itself, and past six from the continuum.
const TAIL_BOUNDS: (f64, f64) = (0.5, 6.0);

/// What to fit, beyond the peaks themselves.
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub struct FitOptions {
    /// Order of the polynomial continuum: 1 straight, 2 curved.
    pub continuum_order: usize,
    /// Whether to offer a step under the peaks.
    pub step: bool,
    /// Whether to offer each peak a low-energy tail.
    pub tail: bool,
}

impl Default for FitOptions {
    /// A curved continuum, a step and a tail - what a real photopeak on a
    /// real Compton background actually is.
    ///
    /// The continuum is quadratic rather than straight because a region wide
    /// enough to hold a multiplet is wide enough for the Compton continuum to
    /// bend across it, and a straight line that cannot bend leaves the
    /// curvature in the residual where an extra Gaussian will find it: a
    /// quarter of singlets on a curved continuum were split in two by a model
    /// that could only draw a straight line under them.
    fn default() -> Self {
        Self {
            continuum_order: 2,
            step: true,
            tail: true,
        }
    }
}

/// Fits a region without being told how many peaks are in it.
///
/// Starts from one peak at `seed` and adds another as long as the counts pay
/// for it - each new Gaussian must improve the fit by a set margin of
/// chi-square, carry several sigma of its own area, stand half a width from
/// its neighbours and a full width from the region's edges. `most` caps the
/// count; a region has to be very busy indeed to hold more than three.
///
/// `sigma_guess` is the width the resolution model expects here. It is a
/// starting point and a scale for the bounds, not a constraint: a fit that
/// could not move the width would have nothing to say about whether the
/// region holds one wide peak or two narrow ones, which is the whole
/// question.
///
/// `None` when the region is too narrow to fit, or holds nothing that looks
/// like a peak at all.
pub fn fit_multiplet(
    counts: &[f64],
    roi: Roi,
    seed: f64,
    sigma_guess: f64,
    most: usize,
    options: FitOptions,
) -> Option<RegionFit> {
    // The step and the tail are consequences of a peak, but the solve does
    // not know that, and given a wide enough region they will cheerfully
    // account for the whole bump and leave the Gaussian at zero - which reads
    // as "no peak here" and abandons the region. So the model is stepped down
    // until one of them can hold a peak at all, and whichever that is, is
    // then used for every count of peaks: the chi-square of a one-peak fit
    // and a two-peak fit are only comparable if both were asked the same
    // question.
    let (options, mut best) = [
        options,
        FitOptions {
            tail: false,
            ..options
        },
        FitOptions {
            tail: false,
            step: false,
            continuum_order: 1,
        },
    ]
    .into_iter()
    .find_map(|options| {
        fit_region(counts, roi, &[seed], sigma_guess, options).map(|fit| (options, fit))
    })?;
    for _ in 1..most.max(1) {
        // A second Gaussian has to improve the fit by `SPLIT_CHI_SQUARE`, and
        // no fit can have a chi-square below zero, so a region the single
        // Gaussian already explains for less than that cannot be split
        // whatever the search finds. Checking costs nothing and skips the
        // expensive part outright on rather over half the regions of a real
        // spectrum, with no result anywhere able to change.
        if best.chi_square < SPLIT_CHI_SQUARE {
            break;
        }
        let Some(candidate) = one_more_peak(counts, roi, &best, options) else {
            break;
        };
        if !improves_on(&candidate, &best) {
            break;
        }
        best = candidate;
    }
    Some(best)
}

/// The best fit of one more peak than `previous` holds, however it is reached.
///
/// Where the search starts decides what it finds, and one starting point is
/// not enough. The objective has a wide plateau in it: two Gaussians of the
/// same width sitting on top of each other fit exactly as well as one, so a
/// simplex begun anywhere on that plateau reports "no improvement" at once
/// and the doublet is never seen. Every opening below steps off that plateau,
/// in the two directions real regions need - a companion leaning on a strong
/// line, and a bump that is two lines of similar size - and whichever ends
/// with the lowest chi-square is the one kept.
///
/// The width each opening starts from is the previous fit's, shrunk by
/// `n/(n+1)`: when one Gaussian was covering the work of two it had to be too
/// wide to do it, and starting the pair at the old width puts them straight
/// back on the plateau. The rule is InterSpec's.
fn one_more_peak(
    counts: &[f64],
    roi: Roi,
    previous: &RegionFit,
    options: FitOptions,
) -> Option<RegionFit> {
    let n = previous.peaks.len();
    let sigma = previous.peaks[0].sigma * n as f64 / (n + 1) as f64;
    let fwhm = sigma * FWHM_PER_SIGMA;
    let existing: Vec<f64> = previous.peaks.iter().map(|peak| peak.centroid).collect();
    let mut openings: Vec<Vec<f64>> = Vec::new();
    // A shoulder: the new peak goes where the model is most short of counts.
    // This is the opening that finds a weak line leaning on a strong one.
    if let Some(where_it_hurts) = worst_residual(counts, previous) {
        let mut seeds = existing.clone();
        seeds.push(where_it_hurts);
        openings.push(seeds);
    }
    // An even pair: the strongest peak is opened out into two, half a width
    // and a whole width apart. This is the opening that finds the flat-topped
    // doublet, where the residual is symmetric and points nowhere.
    let strongest = previous
        .peaks
        .iter()
        .enumerate()
        .max_by(|a, b| {
            a.1.amplitude
                .partial_cmp(&b.1.amplitude)
                .unwrap_or(std::cmp::Ordering::Equal)
        })
        .map(|(index, peak)| (index, peak.centroid));
    if let Some((index, centre)) = strongest {
        for spread in [0.5, 1.0] {
            let mut seeds = existing.clone();
            seeds[index] = centre - spread * fwhm;
            seeds.push(centre + spread * fwhm);
            openings.push(seeds);
        }
    }
    openings
        .into_iter()
        .filter(|seeds| {
            seeds
                .iter()
                .all(|seed| *seed > roi.start as f64 && *seed < roi.end as f64)
        })
        .filter_map(|seeds| fit_region(counts, roi, &seeds, sigma, options))
        .min_by(|a, b| {
            a.chi_square
                .partial_cmp(&b.chi_square)
                .unwrap_or(std::cmp::Ordering::Equal)
        })
}

/// Whether `candidate` has earned its extra peak over `previous`.
fn improves_on(candidate: &RegionFit, previous: &RegionFit) -> bool {
    if previous.chi_square - candidate.chi_square < SPLIT_CHI_SQUARE {
        return false;
    }
    // Every peak must stand clear of the region's edges, where the counts run
    // out and a Gaussian can be anything.
    // Significance and a positive amplitude are not re-checked here:
    // `fit_region` refuses a fit containing a peak that fails either, so no
    // candidate reaching this point can contain one.
    let (low, high) = (candidate.roi.start as f64, candidate.roi.end as f64);
    if candidate.peaks.iter().any(|peak| {
        let from_edge = (peak.centroid - low).min(high - peak.centroid);
        from_edge < EDGE_CLEARANCE_FWHM * peak.fwhm()
    }) {
        return false;
    }
    candidate.peaks.windows(2).all(|pair| {
        let separation = (pair[1].centroid - pair[0].centroid).abs();
        separation >= MIN_SEPARATION_FWHM * pair[0].fwhm()
    })
}

/// The channel whose residual run is worst - where another peak would go.
///
/// Not the single worst channel, which is noise, but the middle of the
/// heaviest window a peak's width wide: a missing Gaussian shows up as a
/// whole window of counts the model is under, not one loud bin.
fn worst_residual(counts: &[f64], fit: &RegionFit) -> Option<f64> {
    let sigma = fit.peaks.first()?.sigma;
    let span = (sigma.max(1.0).round() as usize).max(1);
    let (mut best_channel, mut best_excess) = (None, 0.0);
    for channel in fit.roi.start..=fit.roi.end {
        let low = channel.saturating_sub(span).max(fit.roi.start);
        let high = (channel + span).min(fit.roi.end);
        let excess: f64 = (low..=high)
            .map(|c| {
                let residual = counts[c] - fit.evaluate(c as f64);
                // Only counts the model is missing count: an over-prediction
                // is not a place to put another peak.
                residual.max(0.0) / counts[c].max(1.0).sqrt()
            })
            .sum();
        if excess > best_excess {
            best_excess = excess;
            best_channel = Some(channel as f64);
        }
    }
    best_channel
}

/// Fits a fixed number of Gaussians, one per entry in `seeds`.
///
/// The centroids and the shared width are searched for; everything else -
/// the amplitudes, the continuum, the step - follows exactly from them.
pub fn fit_region(
    counts: &[f64],
    roi: Roi,
    seeds: &[f64],
    sigma_guess: f64,
    options: FitOptions,
) -> Option<RegionFit> {
    if seeds.is_empty() || roi.end >= counts.len() {
        return None;
    }
    let width = roi.len();
    let at = Layout {
        peaks: seeds.len(),
        order: options.continuum_order.clamp(1, 2),
        step: options.step,
        tail: options.tail,
    };
    // Twice as many channels as parameters, and five more: a model that can
    // pass through every point it is shown has learned nothing from them.
    // The nonlinear parameters - every centroid and the one shared width -
    // count too.
    //
    // The margin is generous because merely outnumbering the parameters is
    // meaningless: eleven parameters over fifteen channels - which is what a
    // region drawn around a three-channel germanium line holds - is a model
    // that can be talked into anything, and one such fit duly split Ba-133's
    // 356 keV line, a clean single peak in the counts, into itself and a
    // companion a fiftieth its size.
    //
    // On everything measured since, the significance test below turns those
    // same fits away in any case, so this is a bound on what will be
    // attempted rather than the rule that decides. It is kept because a
    // model with more parameters than data is wrong whether or not something
    // downstream notices.
    let parameters = at.terms() + seeds.len() + 1;
    if width < 2 * parameters + 5 {
        return None;
    }

    let sigma0 = sigma_guess.max(0.5);
    // The width may range widely - the guess is a resolution model's opinion,
    // and the region may be exactly where that opinion is wrong - but not so
    // far that one Gaussian can impersonate the continuum.
    let sigma_bounds = (
        (0.3 * sigma0).max(0.5),
        (3.0 * sigma0).min(width as f64 / 4.0).max(0.6),
    );
    let mut start: Vec<f64> = seeds.to_vec();
    start.push(sigma0.clamp(sigma_bounds.0, sigma_bounds.1));
    let mut bounds: Vec<(f64, f64)> = seeds
        .iter()
        .map(|_| (roi.start as f64, roi.end as f64))
        .collect();
    bounds.push(sigma_bounds);
    // The simplex starts a fifth of a width across in the centroids and a
    // fifth of itself in the width: wide enough to find a neighbouring
    // minimum, narrow enough not to leave the region.
    let mut steps: Vec<f64> = seeds
        .iter()
        .map(|_| 0.2 * sigma0 * FWHM_PER_SIGMA)
        .collect();
    steps.push(0.2 * sigma0);

    let objective = |parameters: &[f64]| -> f64 {
        let (centroids, sigma) = split_parameters(parameters);
        match solve_linear(counts, roi, centroids, sigma, TAIL_WIDTHS, at, false) {
            Some(solution) => solution.chi_square,
            None => f64::INFINITY,
        }
    };
    let (found, _) = nelder_mead(&start, &steps, &bounds, objective);

    let (centroids, sigma) = split_parameters(&found);
    let solution = solve_linear(counts, roi, centroids, sigma, TAIL_WIDTHS, at, true)?;
    // Sorted by channel, carrying each peak's own tail with it.
    let mut order: Vec<usize> = (0..centroids.len()).collect();
    order.sort_by(|a, b| {
        centroids[*a]
            .partial_cmp(&centroids[*b])
            .unwrap_or(std::cmp::Ordering::Equal)
    });
    let scale = sigma * (2.0 * std::f64::consts::PI).sqrt();
    let peaks: Vec<FittedPeak> = order
        .iter()
        .map(|index| FittedPeak {
            centroid: centroids[*index],
            sigma,
            amplitude: solution.coefficients[*index],
            area: solution.coefficients[*index] * scale,
            area_uncertainty: solution.uncertainties[*index] * scale,
        })
        .collect();
    // A Gaussian the fit cannot hold up is not a fitted peak, and saying so
    // here rather than handing it back is what lets the model ladder in
    // [`fit_multiplet`] do its work: the ladder steps down when a fit is
    // refused, and a degenerate fit that returns successfully is never
    // refused. On a three-line region the full model duly returned one
    // "peak" of area 1 plus or minus 1,183 - the continuum, the step and the
    // tail having accounted for everything between them - and the ladder,
    // seeing a success, kept the model that produced it.
    if peaks
        .iter()
        .any(|peak| peak.amplitude <= 0.0 || peak.significance() < MIN_PEAK_SIGNIFICANCE)
    {
        return None;
    }
    let tails: Vec<f64> = order
        .iter()
        .map(|index| {
            at.tail_index(*index)
                .and_then(|tail| solution.coefficients.get(tail).copied())
                .unwrap_or(0.0)
        })
        .collect();
    Some(RegionFit {
        roi,
        peaks,
        continuum: solution.coefficients[at.continuum()].to_vec(),
        step: at
            .step_index()
            .and_then(|index| solution.coefficients.get(index).copied())
            .unwrap_or(0.0),
        tails,
        tail_widths: TAIL_WIDTHS,
        chi_square: solution.chi_square,
        degrees_of_freedom: width.saturating_sub(parameters),
    })
}

/// The parameter vector is every centroid, then the one shared width.
fn split_parameters(parameters: &[f64]) -> (&[f64], f64) {
    let split = parameters.len() - 1;
    (&parameters[..split], parameters[split])
}

/// The linear half of the problem, solved exactly.
struct Linear {
    coefficients: Vec<f64>,
    uncertainties: Vec<f64>,
    chi_square: f64,
}

/// Where each linear coefficient lives in the solution vector.
///
/// Peaks first, then the continuum, then the one step, then one tail per
/// peak. Written down once because every index into the solution is a chance
/// to read the continuum as an amplitude.
#[derive(Clone, Copy)]
struct Layout {
    peaks: usize,
    order: usize,
    step: bool,
    tail: bool,
}

impl Layout {
    fn continuum(&self) -> std::ops::Range<usize> {
        self.peaks..self.peaks + self.order + 1
    }

    fn step_index(&self) -> Option<usize> {
        self.step.then_some(self.peaks + self.order + 1)
    }

    fn tail_index(&self, peak: usize) -> Option<usize> {
        self.tail
            .then(|| self.peaks + self.order + 1 + usize::from(self.step) + peak)
    }

    fn terms(&self) -> usize {
        self.peaks
            + self.order
            + 1
            + usize::from(self.step)
            + if self.tail { self.peaks } else { 0 }
    }

    /// Amplitudes, the step and the tails are counts of something and cannot
    /// be negative; the continuum's coefficients are free to be anything.
    fn non_negative(&self, index: usize) -> bool {
        index < self.peaks || !self.continuum().contains(&index)
    }
}

/// Solves for every linear coefficient at fixed centroids and width.
///
/// Weighted normal equations with Poisson weights, then an active-set pass:
/// a least-squares solve does not know that an amplitude cannot be negative.
/// A negative one is dropped and the rest re-solved, which is the classic
/// non-negative least squares restricted to the coefficients that need it.
fn solve_linear(
    counts: &[f64],
    roi: Roi,
    centroids: &[f64],
    sigma: f64,
    tail_widths: f64,
    at: Layout,
    uncertainties: bool,
) -> Option<Linear> {
    if sigma <= 0.0 || !sigma.is_finite() || centroids.iter().any(|c| !c.is_finite()) {
        return None;
    }
    let peaks = centroids.len();
    let terms = at.terms();
    let step_centre = centroids.iter().sum::<f64>() / peaks.max(1) as f64;
    let basis = |channel: f64| -> Vec<f64> {
        let mut row = vec![0.0; terms];
        for (index, centroid) in centroids.iter().enumerate() {
            let z = (channel - centroid) / sigma;
            row[index] = (-0.5 * z * z).exp();
            if let Some(tail) = at.tail_index(index) {
                row[tail] = tail_shape(channel, *centroid, sigma, tail_widths);
            }
        }
        let scaled = continuum_coordinate(roi, channel);
        for (power, index) in at.continuum().enumerate() {
            row[index] = scaled.powi(power as i32);
        }
        if let Some(index) = at.step_index() {
            row[index] = step_shape(channel, step_centre, sigma);
        }
        row
    };
    let mut free: Vec<bool> = vec![true; terms];
    let constrained = |index: usize| at.non_negative(index);
    let mut coefficients = vec![0.0; terms];
    let mut covariance = vec![0.0; terms];
    for _ in 0..terms.max(1) {
        let active: Vec<usize> = (0..terms).filter(|index| free[*index]).collect();
        if active.is_empty() {
            return None;
        }
        let n = active.len();
        let mut ata = vec![vec![0.0f64; n]; n];
        let mut atb = vec![0.0f64; n];
        for (channel, &y) in (roi.start..=roi.end).zip(&counts[roi.start..=roi.end]) {
            let weight = 1.0 / y.max(1.0);
            let row = basis(channel as f64);
            for (i, &ai) in active.iter().enumerate() {
                for (j, &aj) in active.iter().enumerate() {
                    ata[i][j] += weight * row[ai] * row[aj];
                }
                atb[i] += weight * row[ai] * y;
            }
        }
        let solution = solve_n(ata.clone(), atb)?;
        // The first negative constrained coefficient is dropped and the rest
        // re-solved; repeating until none is negative is the active set.
        let offender = active
            .iter()
            .enumerate()
            .filter(|(index, coefficient)| constrained(**coefficient) && solution[*index] < 0.0)
            .min_by(|a, b| {
                solution[a.0]
                    .partial_cmp(&solution[b.0])
                    .unwrap_or(std::cmp::Ordering::Equal)
            })
            .map(|(_, coefficient)| *coefficient);
        if let Some(offender) = offender {
            free[offender] = false;
            continue;
        }
        coefficients = vec![0.0; terms];
        covariance = vec![0.0; terms];
        for (i, &index) in active.iter().enumerate() {
            coefficients[index] = solution[i];
        }
        // The diagonal of the inverse normal matrix is each coefficient's
        // variance, the weights already being one over the counts. It costs
        // another solve per coefficient, so it is computed only when the
        // caller wants it: the optimiser below asks for a chi-square a
        // thousand times over and never looks at an uncertainty.
        if uncertainties {
            for (i, &index) in active.iter().enumerate() {
                let mut unit = vec![0.0; n];
                unit[i] = 1.0;
                if let Some(column) = solve_n(ata.clone(), unit) {
                    covariance[index] = column[i].max(0.0);
                }
            }
        }
        break;
    }
    let mut chi_square = 0.0;
    for (channel, &y) in (roi.start..=roi.end).zip(&counts[roi.start..=roi.end]) {
        let row = basis(channel as f64);
        let model: f64 = row
            .iter()
            .zip(&coefficients)
            .map(|(basis, coefficient)| basis * coefficient)
            .sum();
        let residual = y - model;
        chi_square += residual * residual / y.max(1.0);
    }
    if !chi_square.is_finite() {
        return None;
    }
    Some(Linear {
        uncertainties: covariance.iter().map(|v| v.sqrt()).collect(),
        coefficients,
        chi_square,
    })
}

/// The region mapped to `-1..=1`, so a quadratic stays conditioned.
fn continuum_coordinate(roi: Roi, channel: f64) -> f64 {
    let centre = (roi.start + roi.end) as f64 / 2.0;
    let half = (roi.len() as f64 / 2.0).max(1.0);
    (channel - centre) / half
}

/// The continuum polynomial at a channel.
fn continuum_at(coefficients: &[f64], roi: Roi, channel: f64) -> f64 {
    let scaled = continuum_coordinate(roi, channel);
    coefficients
        .iter()
        .enumerate()
        .map(|(power, coefficient)| coefficient * scaled.powi(power as i32))
        .sum()
}

/// The step under a peak: one below it, zero above, half at its centre.
///
/// `erfc(z/sqrt(2))/2` for `z` in widths - the integral of the Gaussian from
/// the channel upwards. Every count that scattered into the peak could have
/// scattered to any lower energy instead, so the continuum a photopeak stands
/// on drops across the peak by an amount proportional to the peak itself,
/// with exactly this shape.
fn step_shape(channel: f64, centre: f64, sigma: f64) -> f64 {
    0.5 * erfc((channel - centre) / (sigma * std::f64::consts::SQRT_2))
}

/// A peak's low-energy tail: an exponential smeared by the peak's own width.
///
/// `exp(d/b + s^2/(2 b^2)) * erfc(d/(sqrt(2) s) + s/(sqrt(2) b))`, the
/// standard Hypermet tail - an exponential of decay length `b` convolved with
/// the Gaussian, so it joins the peak smoothly instead of stepping. Normalised
/// to one at the centroid, so its coefficient reads as a fraction of the peak
/// height. Zero well above the peak, where the exponential's growth and the
/// error function's decay would otherwise multiply out to a floating-point
/// infinity times zero.
fn tail_shape(channel: f64, centre: f64, sigma: f64, widths: f64) -> f64 {
    let d = channel - centre;
    if d > 6.0 * sigma {
        return 0.0;
    }
    let widths = widths.clamp(TAIL_BOUNDS.0, TAIL_BOUNDS.1);
    let b = widths * sigma;
    // The peak's width cancels: `sigma / b` is one over `widths` whatever the
    // peak is, so the value at the centroid - which normalises the shape - is
    // the same number every time and is not recomputed per channel.
    let ratio = 1.0 / widths;
    let at_centre = (0.5 * ratio * ratio).exp() * erfc(ratio / std::f64::consts::SQRT_2);
    if at_centre <= 0.0 {
        return 0.0;
    }
    let shape = (d / b + 0.5 * ratio * ratio).exp()
        * erfc(d / (sigma * std::f64::consts::SQRT_2) + ratio / std::f64::consts::SQRT_2);
    shape / at_centre
}

/// Complementary error function, Abramowitz and Stegun 7.1.26.
///
/// Accurate to 1.5e-7, which is four orders of magnitude finer than counting
/// statistics ever are.
fn erfc(x: f64) -> f64 {
    let sign = if x < 0.0 { -1.0 } else { 1.0 };
    let x = x.abs();
    let t = 1.0 / (1.0 + 0.327_591_1 * x);
    let poly = t
        * (0.254_829_592
            + t * (-0.284_496_736
                + t * (1.421_413_741 + t * (-1.453_152_027 + t * 1.061_405_429))));
    let erf = 1.0 - poly * (-x * x).exp();
    1.0 - sign * erf
}

/// Nelder-Mead simplex search, with bounds by projection.
///
/// Derivative-free because the objective is a least-squares solve rather than
/// a formula, and the problem is small - a doublet is three parameters - so
/// the simplex's indifference to conditioning is worth more than a gradient
/// method's speed. Bounds are applied by clamping inside the objective, which
/// leaves the search a flat shelf outside them rather than a cliff.
fn nelder_mead(
    start: &[f64],
    steps: &[f64],
    bounds: &[(f64, f64)],
    objective: impl Fn(&[f64]) -> f64,
) -> (Vec<f64>, f64) {
    // Restarted once from where the first pass stopped, with a fresh simplex
    // a quarter the size. A simplex that has collapsed along one direction
    // reports convergence whether or not it found anything, and rebuilding it
    // is the standard cure; it costs one more short search and catches the
    // case where the first pass stalled a little short of the minimum.
    let (first, first_value) = nelder_mead_once(start, steps, bounds, &objective);
    let smaller: Vec<f64> = steps.iter().map(|step| 0.25 * step).collect();
    let (second, second_value) = nelder_mead_once(&first, &smaller, bounds, &objective);
    if second_value <= first_value {
        (second, second_value)
    } else {
        (first, first_value)
    }
}

/// One simplex search, from one starting point.
fn nelder_mead_once(
    start: &[f64],
    steps: &[f64],
    bounds: &[(f64, f64)],
    objective: impl Fn(&[f64]) -> f64,
) -> (Vec<f64>, f64) {
    let n = start.len();
    let clamp = |point: &[f64]| -> Vec<f64> {
        point
            .iter()
            .zip(bounds)
            .map(|(value, (low, high))| value.clamp(*low, *high))
            .collect()
    };
    let evaluate = |point: &[f64]| -> (Vec<f64>, f64) {
        let clamped = clamp(point);
        let value = objective(&clamped);
        (clamped, value)
    };
    let mut simplex: Vec<(Vec<f64>, f64)> = Vec::with_capacity(n + 1);
    simplex.push(evaluate(start));
    for axis in 0..n {
        let mut point = start.to_vec();
        point[axis] += steps[axis];
        simplex.push(evaluate(&point));
    }
    // Enough iterations for a three-parameter problem to converge several
    // times over; the loop exits early when the simplex has collapsed.
    for _ in 0..(200 * n) {
        simplex.sort_by(|a, b| a.1.partial_cmp(&b.1).unwrap_or(std::cmp::Ordering::Equal));
        let best = simplex[0].1;
        let worst = simplex[n].1;
        if !worst.is_finite() && !best.is_finite() {
            break;
        }
        if (worst - best).abs() <= 1e-9 * (1.0 + best.abs()) {
            break;
        }
        // The centroid of every vertex but the worst, which is what the
        // worst is reflected through.
        let centroid: Vec<f64> = (0..n)
            .map(|axis| simplex[..n].iter().map(|(p, _)| p[axis]).sum::<f64>() / n as f64)
            .collect();
        let along = |factor: f64| -> Vec<f64> {
            (0..n)
                .map(|axis| centroid[axis] + factor * (simplex[n].0[axis] - centroid[axis]))
                .collect()
        };
        let (reflected, reflected_value) = evaluate(&along(-1.0));
        if reflected_value < simplex[0].1 {
            let (expanded, expanded_value) = evaluate(&along(-2.0));
            simplex[n] = if expanded_value < reflected_value {
                (expanded, expanded_value)
            } else {
                (reflected, reflected_value)
            };
            continue;
        }
        if reflected_value < simplex[n - 1].1 {
            simplex[n] = (reflected, reflected_value);
            continue;
        }
        let (contracted, contracted_value) = evaluate(&along(0.5));
        if contracted_value < worst {
            simplex[n] = (contracted, contracted_value);
            continue;
        }
        // Nothing worked: pull every vertex halfway toward the best one.
        let anchor = simplex[0].0.clone();
        for vertex in simplex.iter_mut().skip(1) {
            let pulled: Vec<f64> = (0..n)
                .map(|axis| anchor[axis] + 0.5 * (vertex.0[axis] - anchor[axis]))
                .collect();
            *vertex = evaluate(&pulled);
        }
    }
    simplex.sort_by(|a, b| a.1.partial_cmp(&b.1).unwrap_or(std::cmp::Ordering::Equal));
    simplex.swap_remove(0)
}

#[cfg(test)]
mod tests {
    use super::*;

    /// Counting noise on a spectrum, deterministically.
    ///
    /// A normal of standard deviation `sqrt(counts)` by Box-Muller, from a
    /// seeded linear congruential stream. Every channel these tests use holds
    /// hundreds of counts or more, where the normal and the Poisson are the
    /// same distribution to well under a percent - and unlike a real Poisson
    /// sampler this costs the same at ten counts and ten thousand.
    pub(super) fn noisy(counts: Vec<f64>, seed: u64) -> Vec<f64> {
        let mut state = seed
            .wrapping_mul(6_364_136_223_846_793_005)
            .wrapping_add(1_442_695_040_888_963_407);
        let mut uniform = || {
            state = state
                .wrapping_mul(6_364_136_223_846_793_005)
                .wrapping_add(1_442_695_040_888_963_407);
            // The top bits of an LCG are the ones worth having; nudged off
            // zero so the logarithm below is always finite.
            ((state >> 11) as f64 / (1u64 << 53) as f64).max(f64::MIN_POSITIVE)
        };
        counts
            .into_iter()
            .map(|value| {
                let (a, b) = (uniform(), uniform());
                let unit = (-2.0 * a.ln()).sqrt() * (2.0 * std::f64::consts::PI * b).cos();
                (value + unit * value.max(1.0).sqrt()).max(0.0).round()
            })
            .collect()
    }

    /// Continuum plus Gaussians, in counts.
    pub(super) fn spectrum(
        length: usize,
        floor: f64,
        slope: f64,
        peaks: &[(f64, f64, f64)],
    ) -> Vec<f64> {
        (0..length)
            .map(|channel| {
                let x = channel as f64;
                floor
                    + slope * x
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

    /// One clean peak comes back with its own parameters.
    #[test]
    fn a_single_peak_fits_to_itself() {
        let counts = spectrum(400, 500.0, -0.5, &[(200.0, 6.0, 100_000.0)]);
        let fit = fit_multiplet(
            &counts,
            Roi::new(160, 240),
            200.0,
            6.0,
            3,
            FitOptions::default(),
        )
        .expect("a clear peak fits");
        assert_eq!(fit.peaks.len(), 1, "one peak in, one out: {:?}", fit.peaks);
        assert!(
            (fit.peaks[0].centroid - 200.0).abs() < 0.1,
            "{}",
            fit.peaks[0].centroid
        );
        assert!(
            (fit.peaks[0].sigma - 6.0).abs() < 0.2,
            "{}",
            fit.peaks[0].sigma
        );
        assert!(
            (fit.peaks[0].area - 100_000.0).abs() < 3_000.0,
            "{}",
            fit.peaks[0].area
        );
    }

    /// The whole point: a doublet the eye reads as one bump comes back as two.
    #[test]
    fn an_unresolved_doublet_comes_back_as_two_peaks() {
        // One FWHM apart - a single bump with a flat top, which every
        // single-Gaussian measurement reads as one peak half again too wide.
        let fwhm = 6.0 * FWHM_PER_SIGMA;
        let (left, right) = (200.0 - fwhm / 2.0, 200.0 + fwhm / 2.0);
        let counts = spectrum(
            400,
            500.0,
            -0.5,
            &[(left, 6.0, 100_000.0), (right, 6.0, 100_000.0)],
        );
        let fit = fit_multiplet(
            &counts,
            Roi::new(150, 250),
            200.0,
            6.0,
            3,
            FitOptions::default(),
        )
        .expect("a bump fits");
        assert_eq!(fit.peaks.len(), 2, "a doublet: {:?}", fit.peaks);
        assert!(
            (fit.peaks[0].centroid - left).abs() < 1.0,
            "left at {} not {left}",
            fit.peaks[0].centroid
        );
        assert!(
            (fit.peaks[1].centroid - right).abs() < 1.0,
            "right at {} not {right}",
            fit.peaks[1].centroid
        );
        // And the width it recovers is one peak's width, not the pair's.
        assert!(
            (fit.peaks[0].sigma - 6.0).abs() < 0.6,
            "the shared width should be one peak's: {}",
            fit.peaks[0].sigma
        );
    }

    /// The other half of the same claim: one peak must not become two.
    ///
    /// A split that fires on noise is worse than no splitting at all - it
    /// turns every strong line into a phantom pair and doubles the peak list.
    #[test]
    fn a_single_peak_is_not_split_into_two() {
        for seed in 0..12u64 {
            let counts = noisy(spectrum(400, 500.0, -0.5, &[(200.0, 6.0, 200_000.0)]), seed);
            let fit = fit_multiplet(
                &counts,
                Roi::new(150, 250),
                200.0,
                6.0,
                3,
                FitOptions::default(),
            )
            .expect("a clear peak fits");
            assert_eq!(
                fit.peaks.len(),
                1,
                "seed {seed} split one peak into {:?}",
                fit.peaks
                    .iter()
                    .map(|peak| peak.centroid)
                    .collect::<Vec<_>>()
            );
        }
    }

    /// A region whose edge cuts through a neighbour invents a peak.
    ///
    /// The window's edge behaves like a peak's flank: the counts stop, and a
    /// Gaussian placed against them is fitted from one side only. Drawn
    /// around the right-hand line of a resolved pair, the region's low edge
    /// falls across the left-hand one, and the fit answers with a phantom
    /// sitting six tenths of a width from the *far* edge.
    #[test]
    fn a_region_that_clips_a_neighbour_does_not_invent_a_peak() {
        let sigma = 6.0;
        let fwhm = sigma * FWHM_PER_SIGMA;
        // Two well-resolved lines, a region drawn around the right-hand one
        // that clips the left-hand one.
        let counts = spectrum(
            400,
            3000.0,
            -4.0,
            &[
                (200.0 - 0.75 * fwhm, sigma, 100_000.0),
                (200.0 + 0.75 * fwhm, sigma, 100_000.0),
            ],
        );
        let right = 200.0 + 0.75 * fwhm;
        let region = Roi::new((right - 1.5 * fwhm) as usize, (right + 1.5 * fwhm) as usize);
        let fit = fit_multiplet(&counts, region, right, sigma, 3, FitOptions::default())
            .expect("the clipped region still fits");
        for peak in &fit.peaks {
            let from_edge =
                (peak.centroid - region.start as f64).min(region.end as f64 - peak.centroid);
            assert!(
                from_edge >= peak.fwhm(),
                "a peak at {} stands {from_edge:.1} channels from the edge of {region:?}",
                peak.centroid
            );
        }
    }

    /// Nothing the fit hands back is below the significance it demands.
    ///
    /// Stated as a property over every kind of region the module is asked
    /// about, because an insignificant peak does more harm than being merely
    /// wrong: it is counted among the region's lines, and the guards
    /// downstream - which ask what share of the region each line holds -
    /// then reject the whole split on its account. Ba-133's real 79.6 and
    /// 81.0 keV pair is lost exactly this way when the significance is not
    /// demanded here.
    #[test]
    fn every_peak_the_fit_reports_is_significant() {
        let sigma = 6.0;
        let fwhm = sigma * FWHM_PER_SIGMA;
        /// One named region's worth of peaks, as (centre, sigma, area).
        type Case = (&'static str, Vec<(f64, f64, f64)>);
        let cases: Vec<Case> = vec![
            ("singlet", vec![(200.0, sigma, 200_000.0)]),
            (
                "even pair",
                vec![
                    (200.0 - fwhm / 2.0, sigma, 100_000.0),
                    (200.0 + fwhm / 2.0, sigma, 100_000.0),
                ],
            ),
            (
                "ten to one",
                vec![
                    (200.0 - 0.85 * fwhm, sigma, 200_000.0),
                    (200.0 + 0.85 * fwhm, sigma, 20_000.0),
                ],
            ),
            (
                "triplet",
                vec![
                    (200.0 - 1.4 * fwhm, sigma, 80_000.0),
                    (200.0, sigma, 80_000.0),
                    (200.0 + 1.4 * fwhm, sigma, 80_000.0),
                ],
            ),
            ("empty", vec![]),
        ];
        for (name, peaks) in cases {
            for seed in 0..6u64 {
                let counts = noisy(spectrum(400, 3000.0, -4.0, &peaks), seed + 1);
                let Some(fit) = fit_multiplet(
                    &counts,
                    Roi::new(140, 260),
                    200.0,
                    sigma,
                    3,
                    FitOptions::default(),
                ) else {
                    continue;
                };
                for peak in &fit.peaks {
                    assert!(
                        peak.significance() >= MIN_PEAK_SIGNIFICANCE,
                        "{name} seed {seed}: a peak at {:.1} carries {:.1} sigma",
                        peak.centroid,
                        peak.significance()
                    );
                }
            }
        }
    }

    /// A model with more parameters than the region can feed finds anything.
    ///
    /// Fifteen channels is what a region drawn around a three-channel
    /// germanium line holds, and a two-Gaussian model with a curved
    /// continuum, a step and two tails asks eleven parameters of them. Such a
    /// fit split Ba-133's 356 keV line - which the counts show as a single
    /// clean peak - into itself and a companion a fiftieth its size.
    #[test]
    fn a_region_too_small_for_the_model_is_refused_rather_than_fitted() {
        // A single narrow line as a real one arrives: counting noise, and a
        // few percent of tail on its low side. Nothing here is two lines.
        let (sigma, area) = (1.3, 40_000.0);
        let amplitude = area / (sigma * (2.0 * std::f64::consts::PI).sqrt());
        let mut split = Vec::new();
        for seed in 0..12u64 {
            let mut counts = spectrum(400, 300.0, -0.2, &[(200.0, sigma, area)]);
            for (channel, value) in counts.iter_mut().enumerate() {
                let d = channel as f64 - 200.0;
                if d < 0.0 {
                    *value += 0.05 * amplitude * (d / (2.5 * sigma)).exp();
                }
            }
            let narrow = Roi::new(193, 207);
            assert_eq!(narrow.len(), 15);
            let fit = fit_multiplet(
                &noisy(counts, seed + 1),
                narrow,
                200.0,
                sigma,
                3,
                FitOptions::default(),
            )
            .expect("a narrow region still measures one peak");
            if fit.peaks.len() > 1 {
                split.push((seed, fit.peaks.len()));
            }
        }
        assert!(
            split.is_empty(),
            "fifteen channels cannot support two Gaussians, but were fitted with them: {split:?}"
        );
        // And the rule itself, stated where it is made: a region this size is
        // refused a two-Gaussian model outright rather than fitted with one
        // and judged afterwards.
        let counts = spectrum(400, 300.0, -0.2, &[(200.0, sigma, area)]);
        assert!(
            fit_region(
                &counts,
                Roi::new(193, 207),
                &[198.0, 202.0],
                sigma,
                FitOptions::default()
            )
            .is_none(),
            "a two-peak model was fitted to fifteen channels"
        );
    }

    /// A doublet survives counting noise, not just arithmetic.
    #[test]
    fn a_noisy_doublet_is_still_two_peaks() {
        let fwhm = 6.0 * FWHM_PER_SIGMA;
        let (left, right) = (200.0 - 0.6 * fwhm, 200.0 + 0.6 * fwhm);
        let mut found = 0;
        for seed in 0..8u64 {
            let counts = noisy(
                spectrum(
                    400,
                    500.0,
                    -0.5,
                    &[(left, 6.0, 150_000.0), (right, 6.0, 150_000.0)],
                ),
                seed,
            );
            let fit = fit_multiplet(
                &counts,
                Roi::new(150, 250),
                200.0,
                6.0,
                3,
                FitOptions::default(),
            )
            .expect("a bump fits");
            if fit.peaks.len() == 2 {
                found += 1;
            }
        }
        assert_eq!(found, 8, "only {found} of 8 noisy doublets were resolved");
    }

    /// A peak on a step is not read as a peak on a slope.
    ///
    /// The step is what a photopeak's continuum actually does; without it the
    /// straight line has to split the difference and the fit pays for it in
    /// area. This measures that difference rather than asserting it exists.
    #[test]
    fn the_step_recovers_an_area_a_straight_line_misses() {
        let mut counts = spectrum(400, 200.0, 0.0, &[(200.0, 6.0, 60_000.0)]);
        // A Compton step: the continuum is 900 counts below the peak and 200
        // above it, falling across the peak with the peak's own shape.
        for (channel, value) in counts.iter_mut().enumerate() {
            *value += 700.0 * step_shape(channel as f64, 200.0, 6.0);
        }
        let with_step = fit_region(
            &counts,
            Roi::new(160, 240),
            &[200.0],
            6.0,
            FitOptions {
                continuum_order: 1,
                step: true,
                tail: false,
            },
        )
        .expect("fits with a step");
        let without = fit_region(
            &counts,
            Roi::new(160, 240),
            &[200.0],
            6.0,
            FitOptions {
                continuum_order: 1,
                step: false,
                tail: false,
            },
        )
        .expect("fits without one");
        let step_error = (with_step.peaks[0].area - 60_000.0).abs() / 60_000.0;
        let line_error = (without.peaks[0].area - 60_000.0).abs() / 60_000.0;
        assert!(
            step_error < 0.02,
            "the step should recover the area: off by {:.1}%",
            100.0 * step_error
        );
        assert!(
            line_error > 2.0 * step_error,
            "a straight line under a step should do worse: {:.1}% against {:.1}%",
            100.0 * line_error,
            100.0 * step_error
        );
    }

    /// A region with nothing in it does not invent a peak worth reporting.
    #[test]
    fn a_flat_region_holds_one_peak_at_most() {
        let counts = spectrum(400, 500.0, -0.2, &[]);
        let fit = fit_multiplet(
            &counts,
            Roi::new(150, 250),
            200.0,
            6.0,
            3,
            FitOptions::default(),
        );
        // Whatever it says about the one peak it was asked to seed, it must
        // not have found company for it in a flat continuum.
        if let Some(fit) = fit {
            assert_eq!(
                fit.peaks.len(),
                1,
                "peaks in a flat region: {:?}",
                fit.peaks
            );
        }
    }

    /// The error function is the one the step needs.
    #[test]
    fn the_error_function_is_accurate() {
        // erfc(0) = 1, erfc(large) -> 0, and the known value at 1.
        assert!((erfc(0.0) - 1.0).abs() < 1e-7);
        assert!((erfc(1.0) - 0.157_299_207).abs() < 1e-6);
        assert!((erfc(-1.0) - 1.842_700_793).abs() < 1e-6);
        assert!(erfc(6.0) < 1e-7);
    }

    /// A three-peak region is fitted as three, not two.
    #[test]
    fn three_lines_in_one_region_are_all_found() {
        let fwhm = 5.0 * FWHM_PER_SIGMA;
        let centres = [200.0 - 1.4 * fwhm, 200.0, 200.0 + 1.4 * fwhm];
        let counts = spectrum(
            500,
            400.0,
            -0.2,
            &[
                (centres[0], 5.0, 90_000.0),
                (centres[1], 5.0, 90_000.0),
                (centres[2], 5.0, 90_000.0),
            ],
        );
        let fit = fit_multiplet(
            &counts,
            Roi::new(140, 260),
            200.0,
            5.0,
            4,
            FitOptions::default(),
        )
        .expect("a busy region fits");
        assert_eq!(fit.peaks.len(), 3, "three lines: {:?}", fit.peaks);
        for (found, expected) in fit.peaks.iter().zip(centres) {
            assert!(
                (found.centroid - expected).abs() < 1.5,
                "{} should be {expected}",
                found.centroid
            );
        }
    }

    /// One strong peak with a low-energy tail on it, forty times over.
    ///
    /// This is the failure mode that decides whether the whole thing is
    /// usable, because every real germanium line has a tail and there are
    /// hundreds of them in a spectrum. Without a tail in the model the fit
    /// split *all forty*: a second Gaussian sitting low is the only account
    /// an untailed model can give of counts on the low side. Four percent is
    /// worse tailing than a detector in reasonable health shows.
    #[test]
    fn a_tailed_peak_is_not_a_doublet() {
        let sigma = 6.0;
        let area = 500_000.0;
        let amplitude = area / (sigma * (2.0 * std::f64::consts::PI).sqrt());
        let mut split = Vec::new();
        for seed in 0..40u64 {
            let mut counts = spectrum(400, 500.0, -0.5, &[(200.0, sigma, area)]);
            for (channel, value) in counts.iter_mut().enumerate() {
                let d = channel as f64 - 200.0;
                if d < 0.0 {
                    *value += 0.04 * amplitude * (d / (2.5 * sigma)).exp();
                }
            }
            let fit = fit_multiplet(
                &noisy(counts, seed + 1),
                Roi::new(150, 250),
                200.0,
                sigma,
                3,
                FitOptions::default(),
            )
            .expect("a tailed peak still fits");
            if fit.peaks.len() > 1 {
                split.push(seed);
            }
        }
        assert!(
            split.is_empty(),
            "a tailed singlet was split on seeds {split:?}"
        );
    }

    /// A curved continuum is not a second peak either.
    ///
    /// The curvature has to go somewhere, and a spare Gaussian is somewhere:
    /// with only a straight line available under the region, a quarter of
    /// these were split in two.
    #[test]
    fn a_curved_continuum_does_not_make_a_second_peak() {
        let mut split = Vec::new();
        for seed in 0..40u64 {
            let mut counts = spectrum(400, 500.0, -0.5, &[(200.0, 6.0, 200_000.0)]);
            for (channel, value) in counts.iter_mut().enumerate() {
                let x = (channel as f64 - 200.0) / 100.0;
                *value += 900.0 * (-x).exp();
            }
            let fit = fit_multiplet(
                &noisy(counts, seed + 1),
                Roi::new(150, 250),
                200.0,
                6.0,
                3,
                FitOptions::default(),
            )
            .expect("a peak on a curve still fits");
            if fit.peaks.len() > 1 {
                split.push(seed);
            }
        }
        assert!(
            split.is_empty(),
            "a peak on a curved continuum was split on seeds {split:?}"
        );
    }

    /// The claim the module is for, stated as a rate rather than an anecdote.
    ///
    /// A doublet one full width apart, twenty thousand counts in it, is
    /// resolved every time. This is the working limit: at 200,000 counts the
    /// same holds down to 0.6 of a width, and below about 5,000 counts a
    /// doublet this close is found only sometimes - which is what the counts
    /// support, not a fault in the fitting.
    #[test]
    fn a_doublet_one_width_apart_is_always_resolved() {
        let sigma = 6.0;
        let fwhm = sigma * FWHM_PER_SIGMA;
        let mut missed = Vec::new();
        for seed in 0..40u64 {
            let counts = noisy(
                spectrum(
                    400,
                    500.0,
                    -0.5,
                    &[
                        (200.0 - fwhm / 2.0, sigma, 10_000.0),
                        (200.0 + fwhm / 2.0, sigma, 10_000.0),
                    ],
                ),
                seed + 1,
            );
            let fit = fit_multiplet(
                &counts,
                Roi::new(150, 250),
                200.0,
                sigma,
                3,
                FitOptions::default(),
            )
            .expect("a doublet fits");
            if fit.peaks.len() != 2 {
                missed.push((seed, fit.peaks.len()));
            }
        }
        assert!(missed.is_empty(), "doublets not resolved: {missed:?}");
    }
}
