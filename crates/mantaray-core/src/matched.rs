//! Matched-filter peak search: one kernel, shaped like the detector.
//!
//! The method is Becquerel's (`becquerel.core.peakfinder`, Lawrence Berkeley
//! National Laboratory and the University of California, BSD-3; used with the
//! maintainers' permission and our thanks). A peak search that knows the
//! detector's resolution needs only one filter: a zero-sum Mexican-hat kernel
//! whose width at every channel is the width a real peak would have *there*.
//! A one-channel spike at a channel where real peaks are fifty channels wide
//! scores nothing against a fifty-channel kernel - the false positives a
//! fixed ladder of scales must threshold away are structurally impossible.
//! On the Cs-137 spectrum that prompted this - from a bench detector in a
//! degraded state, which made it a stress test - the ladder this replaces
//! reported twenty-four peaks of which about half were narrow statistical
//! spikes; the matched filter reported only physical features. On the good
//! HPGe bench corpus it finds Ba-133's five signature lines to half a keV
//! plus its sub-percent lines, and all eleven canonical Eu-152 lines with
//! 1085.9 and 1089.7 keV resolved as neighbours.
//!
//! What is Becquerel's: the width law `fwhm(x)^2 = c0 + c1*x` (their
//! `PeakFilter.fwhm` - counting statistics make width grow with the square
//! root of energy), the bin-integrated second-derivative-of-a-Gaussian
//! kernel evaluated exactly through its antiderivative, the signal-to-noise
//! ratio with exact Poisson propagation `snr = K.c / sqrt(K^2.c)`, the
//! three-point local-maximum rule, the curvature width estimate
//! `2*sqrt(snr/|snr''|)`, and the shape gate that rejects a candidate whose
//! measured width disagrees with the model.
//!
//! What is deliberately not copied, with reasons:
//! - Their kernel matrix is dense, every channel against every channel -
//!   two gigabytes at sixteen thousand channels. Here the kernel is truncated
//!   at four sigma and slid, which is the same arithmetic inside the window.
//! - Their kernels are centred on bin low edges while their centroids are
//!   reported at bin centres, a half-channel bias. Here a channel index *is*
//!   the bin centre, kernel and centroid alike.
//! - Their zero-sum normalisation is applied along the axis the convolution
//!   does not contract, so a flat continuum nulls only approximately. Here
//!   each output channel's own kernel is normalised to zero sum, so a flat
//!   continuum scores exactly zero at every channel.
//! - Their closest-pair rule keeps whichever candidate has the lower channel.
//!   Here the higher signal-to-noise wins.
//! - Their width law needs a reference peak handed in. Here, when the
//!   spectrum has no usable shape calibration to read widths from, the
//!   filter itself is scanned over a short ladder of fixed trial widths,
//!   the law is fitted to the widths the sightings *measured*, and the real
//!   search runs with the law the spectrum just taught - which also serves
//!   detectors whose width barely grows, since `c1 = 0` is inside the model.

use crate::FWHM_PER_SIGMA;
use crate::calibration::ShapeCalibration;
use crate::spectrum::Spectrum;

/// The curvature estimate against a true matched Gaussian peak.
///
/// The filter's response to a Gaussian of matching width `s` is
/// `R(d) = (A/(2*sqrt(2)*s)) * (1 - d^2/(2 s^2)) * exp(-d^2/(4 s^2))`; in the
/// continuum the estimator `2*sqrt(R/|R''|)` comes to `0.6934` of the true
/// FWHM. Measured against this implementation's discrete parabola-fitted
/// curvature, a matched peak sits at `0.779..0.806` across sigma from 1.5 to
/// 30 channels - remarkably flat - so the discrete value is what this
/// carries. The shape gate below is written against the raw estimate; the
/// fitted width law is taught with the corrected one.
const CURVATURE_ESTIMATE_PER_FWHM: f64 = 0.79;

/// The matched response at the top of a Gaussian of matching width `s` is
/// `area / (2*sqrt(2)*s)` - the kernel built through its antiderivative
/// carries a factor of one over sigma - so the response times this times the
/// sigma recovers a net-area estimate.
const AREA_PER_RESPONSE_SIGMA: f64 = 2.828_427_124_746_190_3;

/// Accepted width ratio, measured-estimate over model FWHM.
///
/// Becquerel's band is `(0.5, 1.5)`; this one's lower bound is retuned to
/// this implementation's own measurements, because the things it separates
/// sit close: a true matched peak measures `0.78..0.81` and a one-channel
/// glitch - a cosmic hit, an ADC artefact - measures `0.53..0.59` whatever
/// the kernel width. Half rejects neither; `0.65` splits them with margin on
/// both sides. Past one and a half the candidate is continuum structure - a
/// Compton edge measures broad because it has no top to curve over.
const WIDTH_GATE: (f64, f64) = (0.65, 1.5);

/// How a channel's expected peak width is known.
pub(crate) enum Widths<'a> {
    /// A fitted shape calibration: the width read straight off it.
    Shape(&'a ShapeCalibration),
    /// Becquerel's counting-statistics law, `fwhm(x)^2 = c0 + c1*x`.
    Law { c0: f64, c1: f64 },
}

impl Widths<'_> {
    /// Expected FWHM at a channel, floored so a kernel always has a body.
    pub(crate) fn fwhm(&self, channel: f64) -> f64 {
        let fwhm = match self {
            Widths::Shape(shape) => shape.fwhm(channel),
            Widths::Law { c0, c1 } => (c0 + c1 * channel.max(0.0)).max(0.0).sqrt(),
        };
        fwhm.max(1.0)
    }

    /// The law through one known peak, with one channel of width at zero.
    ///
    /// `c0 = 1` is Becquerel's `fwhm_at_0` default: some width at zero energy
    /// (electronic noise) keeps the low channels from demanding infinitely
    /// sharp peaks.
    fn from_anchor(ref_channel: f64, ref_fwhm: f64) -> Self {
        let c0 = 1.0;
        let c1 = (ref_fwhm * ref_fwhm - c0).max(0.0) / ref_channel.max(1.0);
        Widths::Law { c0, c1 }
    }

    /// The law fitted to measured `(channel, fwhm)` pairs, SNR-weighted.
    ///
    /// Least squares on `fwhm^2 = c0 + c1*x`, which is linear in both
    /// coefficients. `c1` is clamped to zero from below - resolution does not
    /// improve with energy - and `c0` to one, the same floor the anchor uses.
    /// `None` when the points cannot say (fewer than two, or degenerate).
    fn fit(points: &[(f64, f64, f64)]) -> Option<Self> {
        if points.len() < 2 {
            return None;
        }
        // Weighted normal equations for y = c0 + c1*x, y = fwhm^2, w = snr.
        let (mut sw, mut swx, mut swy, mut swxx, mut swxy) = (0.0, 0.0, 0.0, 0.0, 0.0);
        for &(x, fwhm, weight) in points {
            let y = fwhm * fwhm;
            sw += weight;
            swx += weight * x;
            swy += weight * y;
            swxx += weight * x * x;
            swxy += weight * x * y;
        }
        let determinant = sw * swxx - swx * swx;
        if determinant.abs() < 1e-9 {
            return None;
        }
        let c1 = ((sw * swxy - swx * swy) / determinant).max(0.0);
        // With the slope settled, the offset is the weighted mean residual -
        // refitted rather than taken from the unclamped solution, so a
        // clamped slope still leaves the law through the points.
        let c0 = ((swy - c1 * swx) / sw).max(1.0);
        Some(Widths::Law { c0, c1 })
    }
}

/// One accepted peak, in channel coordinates.
pub(crate) struct Match {
    /// Interpolated centroid.
    pub centroid: f64,
    /// Signal-to-noise ratio at the top.
    pub snr: f64,
    /// The model's FWHM at the centroid - the width a region should use.
    pub fwhm: f64,
    /// The raw curvature width estimate (`0.6934` of a true peak's FWHM).
    pub estimate: f64,
    /// Net-area estimate from the matched response.
    pub area: f64,
}

/// The signal-to-noise map of the whole spectrum.
///
/// At every channel, a zero-sum kernel of that channel's expected width is
/// laid over the counts: `snr = sum(k*c) / sqrt(sum(k^2*c))`, the denominator
/// being exact Poisson propagation through the filter. Clipped at zero -
/// valleys are not peaks. A channel whose kernel does not fit inside the
/// spectrum scores zero rather than being measured with an amputated one: a
/// truncated, rebalanced kernel stops being a peak detector and starts being
/// an edge detector, and the wall of counts at a spectrum's low end read as
/// a broad peak through exactly that - which then taught the width law
/// nonsense.
pub(crate) fn snr_map(counts: &[f64], widths: &Widths) -> Vec<f64> {
    let n = counts.len();
    let mut snr = vec![0.0; n];
    let mut kernel: Vec<f64> = Vec::new();
    for (channel, out) in snr.iter_mut().enumerate() {
        let x = channel as f64;
        let sigma = widths.fwhm(x) / FWHM_PER_SIGMA;
        let reach = (4.0 * sigma).ceil() as usize + 1;
        if channel < reach || channel + reach >= n {
            continue;
        }
        let low = channel - reach;
        let high = channel + reach;
        build_kernel(x, sigma, low, high, &mut kernel);
        let mut signal = 0.0;
        let mut variance = 0.0;
        for (k, c) in kernel.iter().zip(&counts[low..=high]) {
            signal += k * c;
            variance += k * k * c;
        }
        if variance > 0.0 {
            *out = (signal / variance.sqrt()).max(0.0);
        }
    }
    snr
}

/// The bin-integrated Mexican-hat kernel over channels `low..=high`.
///
/// Each bin's coefficient is the exact integral of `(1 - z^2) exp(-z^2/2)`
/// across the bin, taken through the antiderivative `z exp(-z^2/2)` - a
/// channel index is its bin's centre, so bin `j` runs `j-0.5 ..= j+0.5`.
/// The negative wings are then scaled so the kernel sums to exactly zero
/// over the window it actually has: truncation and the spectrum's edges both
/// shave the wings, and an unbalanced kernel would read a flat continuum as
/// signal.
fn build_kernel(centre: f64, sigma: f64, low: usize, high: usize, kernel: &mut Vec<f64>) {
    let antiderivative = |u: f64| -> f64 {
        let z = (u - centre) / sigma;
        z * (-0.5 * z * z).exp()
    };
    kernel.clear();
    let mut positive = 0.0;
    let mut negative = 0.0;
    for j in low..=high {
        let value = antiderivative(j as f64 + 0.5) - antiderivative(j as f64 - 0.5);
        if value > 0.0 {
            positive += value;
        } else {
            negative -= value;
        }
        kernel.push(value);
    }
    if positive > 0.0 && negative > 0.0 {
        let balance = positive / negative;
        for value in kernel.iter_mut().filter(|value| **value < 0.0) {
            *value *= balance;
        }
    }
}

/// Every peak the map supports, best signal-to-noise first among neighbours.
///
/// A candidate is a three-point local maximum of the map above `min_snr`
/// (strict rise, level-or-fall - a plateau counts once). Each is measured by
/// the curvature of the map at its top and gated on the ratio of that width
/// to the model's; survivors closer together than half the local FWHM are
/// resolved in favour of the higher signal-to-noise. At most `most` peaks
/// come back, highest first, then re-ordered by channel.
pub(crate) fn find(
    counts: &[f64],
    widths: &Widths,
    min_snr: f64,
    gate: (f64, f64),
    most: usize,
) -> Vec<Match> {
    let snr = snr_map(counts, widths);
    let n = snr.len();
    let mut candidates: Vec<Match> = Vec::new();
    for channel in 1..n.saturating_sub(1) {
        if snr[channel] < min_snr
            || !(snr[channel - 1] < snr[channel] && snr[channel] >= snr[channel + 1])
        {
            continue;
        }
        let fwhm0 = widths.fwhm(channel as f64);
        // The curvature stencil spans about a fifth of the expected width, so
        // the estimate reads the top's shape rather than two noisy bins.
        let h = ((0.2 * fwhm0) as usize).max(1);
        if channel < h || channel + h >= n {
            continue;
        }
        // Curvature from a least-squares parabola over the whole +-h window
        // rather than a three-point stencil: the stencil reads only the
        // window's ends and under-measures a wide bump's curvature - enough,
        // measured, to let a one-channel glitch's width ratio creep over the
        // gate's lower bound.
        let d2 = {
            let mean_tt = (0..=2 * h)
                .map(|i| ((i as f64) - h as f64).powi(2))
                .sum::<f64>()
                / (2 * h + 1) as f64;
            let (mut numerator, mut denominator) = (0.0, 0.0);
            for i in 0..=2 * h {
                let t = (i as f64) - h as f64;
                let basis = t * t - mean_tt;
                numerator += basis * snr[channel - h + i];
                denominator += basis * basis;
            }
            2.0 * numerator / denominator
        };
        if d2 >= 0.0 {
            continue;
        }
        let estimate = 2.0 * (snr[channel] / -d2).sqrt();
        let ratio = estimate / fwhm0;
        if !(gate.0..=gate.1).contains(&ratio) {
            continue;
        }
        // Sub-channel centroid from the same three points the maximum rule
        // used - the parabola's own top.
        let left = snr[channel - 1];
        let right = snr[channel + 1];
        let denominator = left - 2.0 * snr[channel] + right;
        let offset = if denominator < 0.0 {
            (0.5 * (left - right) / denominator).clamp(-0.5, 0.5)
        } else {
            0.0
        };
        let centroid = channel as f64 + offset;
        // The raw filter response, recovered from the ratio the map took.
        let response = {
            let sigma = fwhm0 / FWHM_PER_SIGMA;
            let reach = (4.0 * sigma).ceil() as usize + 1;
            let low = channel.saturating_sub(reach);
            let high = (channel + reach).min(n - 1);
            let mut kernel = Vec::new();
            build_kernel(channel as f64, sigma, low, high, &mut kernel);
            kernel
                .iter()
                .zip(&counts[low..=high])
                .map(|(k, c)| k * c)
                .sum::<f64>()
        };
        let sigma = fwhm0 / FWHM_PER_SIGMA;
        candidates.push(Match {
            centroid,
            snr: snr[channel],
            fwhm: widths.fwhm(centroid),
            estimate,
            area: (response * AREA_PER_RESPONSE_SIGMA * sigma).max(0.0),
        });
    }
    // Neighbours inside half a width are one peak wearing two tops: the
    // stronger keeps the name. Strongest first, so the keep-list is built in
    // order of authority; the cap falls out of the same pass.
    candidates.sort_by(|a, b| {
        b.snr
            .partial_cmp(&a.snr)
            .unwrap_or(std::cmp::Ordering::Equal)
    });
    let mut kept: Vec<Match> = Vec::new();
    for candidate in candidates {
        let separation = (candidate.fwhm * 0.5).max(2.0);
        if kept.len() < most
            && kept
                .iter()
                .all(|held| (held.centroid - candidate.centroid).abs() >= separation)
        {
            kept.push(candidate);
        }
    }
    kept.sort_by(|a, b| {
        a.centroid
            .partial_cmp(&b.centroid)
            .unwrap_or(std::cmp::Ordering::Equal)
    });
    kept
}

/// Trial widths the bootstrap scans, in channels of FWHM.
///
/// A geometric ladder from a two-channel HPGe line to the widest peak a
/// scintillator puts on a long spectrum. It exists only to *notice* peaks;
/// their widths are then measured off the counts, not off the rung.
const BOOTSTRAP_FWHMS: [f64; 6] = [2.0, 4.0, 8.0, 16.0, 32.0, 64.0];

/// The width source for a spectrum: its shape calibration, or self-taught.
///
/// With no usable shape calibration, the spectrum teaches itself. The
/// matched filter is run at each fixed trial width in [`BOOTSTRAP_FWHMS`]
/// under a loose gate to *notice* peaks; each sighting's width is then
/// measured directly on the counts - a local background line and a walk to
/// the half-maximum crossings, the measurement `peak_info` has always
/// trusted - because every kernel-derived width estimate answers partly to
/// the kernel that made it. (The lesson was paid for twice on the way here:
/// an over-wide kernel's curvature reads its own width, and on a quiet
/// spectrum it reads it *louder* than the matched kernel reads the truth.)
/// A sighting too faint for its top to clear the local noise keeps its
/// curvature estimate instead, marked as the lesser word; among duplicate
/// sightings of one peak, a direct measurement beats any curvature one, and
/// among curvature ones the most self-consistent rung speaks. The width law
/// is fitted to the survivors, weighted by what each measurement is worth.
///
/// One sighting cannot fit a two-coefficient law, so it anchors one; none
/// means there is nothing to find, and the caller hears so.
///
/// `notice_snr` is how faint a sighting the bootstrap listens for - the
/// caller's own threshold, capped at three: a search set more sensitive than
/// the bootstrap's notice level would hunt for peaks the law never heard of.
pub(crate) fn widths_for<'a>(
    spectrum: &'a Spectrum,
    counts: &[f64],
    notice_snr: f64,
) -> Option<Widths<'a>> {
    if let Some(shape) = spectrum
        .shape_calibration
        .as_ref()
        .filter(|shape| shape.is_usable())
    {
        return Some(Widths::Shape(shape));
    }
    /// One width as one rung measured it.
    struct Opinion {
        fwhm: f64,
        direct: bool,
        /// For curvature widths: how far the raw estimate sat from a matched
        /// kernel's own fraction of its trial - zero is a kernel that
        /// measured exactly what it was shaped for.
        inconsistency: f64,
    }
    /// One peak as the rungs saw it: every width any rung measured for it.
    struct Cluster {
        centroid: f64,
        widths: Vec<Opinion>,
        weight: f64,
    }
    let mut clusters: Vec<Cluster> = Vec::new();
    for trial in BOOTSTRAP_FWHMS {
        let law = Widths::Law {
            c0: trial * trial,
            c1: 0.0,
        };
        for candidate in find(counts, &law, notice_snr, (0.3, 3.0), 40) {
            let (centroid, opinion, weight) = match direct_width(counts, candidate.centroid, trial)
            {
                Some((centroid, fwhm, significance)) => (
                    centroid,
                    Opinion {
                        fwhm,
                        direct: true,
                        inconsistency: 0.0,
                    },
                    significance,
                ),
                // A curvature width from a possibly mismatched kernel is a
                // weak witness however loud the response was.
                None => (
                    candidate.centroid,
                    Opinion {
                        fwhm: (candidate.estimate / CURVATURE_ESTIMATE_PER_FWHM).max(2.0),
                        direct: false,
                        inconsistency: (candidate.estimate / trial - CURVATURE_ESTIMATE_PER_FWHM)
                            .abs(),
                    },
                    candidate.snr.min(3.0),
                ),
            };
            match clusters
                .iter_mut()
                .find(|held| (held.centroid - centroid).abs() < trial.max(opinion.fwhm))
            {
                Some(held) => {
                    held.widths.push(opinion);
                    held.weight = held.weight.max(weight);
                }
                None => clusters.push(Cluster {
                    centroid,
                    widths: vec![opinion],
                    weight,
                }),
            }
        }
    }
    // One width per peak. Direct measurements take their median: every
    // window errs in its own direction - tight clips, wide strays onto the
    // neighbours - and the middle of several opinions is right whenever
    // most roughly are. Curvature widths get no median, because theirs is
    // not measurement scatter: a mismatched kernel's curvature answers to
    // the kernel, scaling with the rung that made it. There the one
    // self-consistent rung speaks - the one that measured very nearly what
    // it was shaped for, which on a true peak recovers the width to a few
    // percent.
    let median_width = |cluster: &Cluster| -> f64 {
        let mut widths: Vec<f64> = cluster
            .widths
            .iter()
            .filter(|opinion| opinion.direct)
            .map(|opinion| opinion.fwhm)
            .collect();
        if widths.is_empty() {
            return cluster
                .widths
                .iter()
                .min_by(|a, b| {
                    a.inconsistency
                        .partial_cmp(&b.inconsistency)
                        .unwrap_or(std::cmp::Ordering::Equal)
                })
                .map(|opinion| opinion.fwhm)
                .unwrap_or(2.0);
        }
        widths.sort_by(|a, b| a.partial_cmp(b).unwrap_or(std::cmp::Ordering::Equal));
        widths[widths.len() / 2]
    };
    // Only clusters confirmed by at least two rungs' direct measurements
    // teach the law. A real peak is sighted and measured from several rungs;
    // a statistical wiggle is one rung's one curvature whisper - and a
    // handful of such singles, all saying "narrow" at high channels, is
    // enough weight to flatten the slope a real X-ray line and a real
    // photopeak agree on. When the spectrum is too poor to confirm anything
    // twice, every cluster speaks - a rough law beats none.
    let confirmed = |cluster: &Cluster| {
        cluster
            .widths
            .iter()
            .filter(|opinion| opinion.direct)
            .count()
            >= 2
    };
    let mut measured: Vec<(f64, f64, f64)> = clusters
        .iter()
        .filter(|cluster| confirmed(cluster))
        .map(|cluster| (cluster.centroid, median_width(cluster), cluster.weight))
        .collect();
    if measured.len() < 2 {
        measured = clusters
            .iter()
            .map(|cluster| (cluster.centroid, median_width(cluster), cluster.weight))
            .collect();
    }
    let first = match measured.as_slice() {
        [] => return None,
        [(channel, fwhm, _)] => return Some(Widths::from_anchor(*channel, *fwhm)),
        many => Widths::fit(many)?,
    };
    // One trimmed refit: a cluster whose median still disagrees with the
    // law by more than the search gate would ever forgive was measured
    // against a neighbour or an artefact, and gets no say in the law it
    // would bend.
    measured.retain(|(channel, fwhm, _)| {
        let predicted = first.fwhm(*channel);
        (0.6..=1.6).contains(&(fwhm / predicted))
    });
    match measured.as_slice() {
        [] | [_] => Some(first),
        many => Widths::fit(many).or(Some(first)),
    }
}

/// A peak's FWHM measured straight off the counts, with its centroid and how
/// many local-noise sigmas its top stands above the higher of its floors.
///
/// Each side is judged against its own floor - the smallest count between
/// the apex and the window's end on that side - and walked out to where the
/// counts fall halfway from the apex to *that* floor, interpolated between
/// channels. Judged per side because a real photopeak sits on a step: the
/// continuum is higher on its low side, and a single background line drawn
/// under both sides put the half level so low that the walk ran into the
/// Compton valley - the photopeak on the bench spectrum measured 104
/// channels wide where its own half-maximum is 64. `None` when the top does
/// not clearly beat the local noise or a crossing never comes - a faint or
/// crowded peak is not a width measurement.
fn direct_width(counts: &[f64], centroid: f64, trial_fwhm: f64) -> Option<(f64, f64, f64)> {
    let centre = centroid.round().max(0.0) as usize;
    let reach = ((3.0 * trial_fwhm).ceil() as usize).max(8);
    let low = centre.checked_sub(reach)?;
    let high = centre + reach;
    if high >= counts.len() {
        return None;
    }
    // The apex may sit a channel or two from where the filter put it.
    let near = trial_fwhm.ceil() as usize;
    let apex =
        (centre.saturating_sub(near).max(low + 1)..(centre + near).min(high)).max_by(|a, b| {
            counts[*a]
                .partial_cmp(&counts[*b])
                .unwrap_or(std::cmp::Ordering::Equal)
        })?;
    let floor_left = counts[low..apex]
        .iter()
        .copied()
        .fold(f64::INFINITY, f64::min);
    let floor_right = counts[apex + 1..=high]
        .iter()
        .copied()
        .fold(f64::INFINITY, f64::min);
    let top = counts[apex];
    // The taller floor is the harder test, and the one the peak must beat.
    let floor = floor_left.max(floor_right);
    if top - floor < 4.0 * floor.max(1.0).sqrt() {
        return None;
    }
    let crossing = |mut at: usize, step: isize, half: f64| -> Option<f64> {
        loop {
            let next = at as isize + step;
            if next < low as isize || next > high as isize {
                return None;
            }
            let next = next as usize;
            if counts[next] <= half {
                // Interpolate between the last channel above and this one.
                let inside = counts[at];
                let outside = counts[next];
                let fraction = (inside - half) / (inside - outside).max(f64::EPSILON);
                return Some(at as f64 + fraction * step as f64);
            }
            at = next;
        }
    };
    let left = crossing(apex, -1, floor_left + (top - floor_left) / 2.0)?;
    let right = crossing(apex, 1, floor_right + (top - floor_right) / 2.0)?;
    let fwhm = (right - left).max(2.0);
    let centroid = (left + right) / 2.0;
    let significance = (top - floor) / floor.max(1.0).sqrt();
    Some((centroid, fwhm, significance))
}

/// The gate the real search runs with - see [`WIDTH_GATE`].
pub(crate) fn width_gate() -> (f64, f64) {
    WIDTH_GATE
}

#[cfg(test)]
mod tests {
    use super::*;

    /// Counts with Gaussian peaks of chosen widths on a flat floor.
    fn synthetic(length: usize, floor: f64, peaks: &[(f64, f64, f64)]) -> Vec<f64> {
        (0..length)
            .map(|channel| {
                let x = channel as f64;
                floor
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

    /// A flat continuum scores exactly zero at every channel.
    ///
    /// This is what the per-output zero-sum normalisation buys: the reference
    /// implementation normalises along the other axis and nulls a flat
    /// background only approximately where the width law has slope.
    #[test]
    fn a_flat_continuum_is_silent() {
        let counts = vec![500.0; 2048];
        let widths = Widths::Law { c0: 4.0, c1: 0.5 };
        let snr = snr_map(&counts, &widths);
        let loudest = snr.iter().copied().fold(0.0, f64::max);
        assert!(loudest < 1e-9, "a flat spectrum spoke: {loudest}");
    }

    /// Matched peaks are found at their centroids, with their widths.
    #[test]
    fn matched_peaks_are_found_where_they_are() {
        let peaks = [(300.0, 4.0, 40_000.0), (900.0, 6.5, 25_000.0)];
        let counts = synthetic(1200, 50.0, &peaks);
        let widths = Widths::fit(&[
            (300.0, 4.0 * FWHM_PER_SIGMA, 1.0),
            (900.0, 6.5 * FWHM_PER_SIGMA, 1.0),
        ])
        .expect("two points fit the law");
        let found = find(&counts, &widths, 3.0, width_gate(), 40);
        assert_eq!(found.len(), 2, "two peaks in, two out");
        assert!(
            (found[0].centroid - 300.0).abs() < 0.5,
            "{}",
            found[0].centroid
        );
        assert!(
            (found[1].centroid - 900.0).abs() < 0.5,
            "{}",
            found[1].centroid
        );
        // The area estimate is honest to a few percent on clean data.
        assert!((found[0].area - 40_000.0).abs() / 40_000.0 < 0.05);
        // The curvature estimate lands at its measured fraction of the FWHM.
        let ratio = found[0].estimate / (4.0 * FWHM_PER_SIGMA);
        assert!(
            (ratio - CURVATURE_ESTIMATE_PER_FWHM).abs() < 0.03,
            "estimate ratio {ratio}"
        );
    }

    /// A one-channel spike where peaks are wide is not a peak.
    ///
    /// The reason this search exists: the ladder it replaces reported a
    /// dozen of these on one bench spectrum.
    #[test]
    fn a_narrow_spike_is_not_a_peak() {
        let mut counts = synthetic(1200, 200.0, &[(300.0, 5.0, 60_000.0)]);
        counts[800] += 400.0;
        let widths = Widths::Law {
            c0: 16.0,
            c1: (5.0 * FWHM_PER_SIGMA).powi(2) / 300.0,
        };
        let found = find(&counts, &widths, 3.0, width_gate(), 40);
        assert_eq!(found.len(), 1, "only the real peak");
        assert!((found[0].centroid - 300.0).abs() < 1.0);
    }

    /// The spectrum teaches itself its width law when nothing else can.
    ///
    /// Constant-width peaks - a synthetic spectrum, or a detector whose
    /// resolution barely moves - are inside the law (`c1 = 0`), so the
    /// bootstrap must serve them even though its anchor pass assumes growth.
    #[test]
    fn the_bootstrap_learns_a_constant_width() {
        let peaks = [
            (250.0, 3.0, 30_000.0),
            (512.0, 3.0, 50_000.0),
            (770.0, 3.0, 20_000.0),
        ];
        let counts = synthetic(1024, 30.0, &peaks);
        let spectrum = Spectrum::from_counts(counts.iter().map(|c| *c as u64).collect());
        let data = spectrum.as_f64();
        let widths = widths_for(&spectrum, &data, 3.0).expect("a law is learned");
        let found = find(&data, &widths, 3.0, width_gate(), 40);
        assert_eq!(found.len(), 3, "all three constant-width peaks");
        for ((centre, _, _), found) in peaks.iter().zip(&found) {
            assert!((found.centroid - centre).abs() < 1.0, "{}", found.centroid);
        }
    }

    /// And a square-root width law, as a real detector has.
    #[test]
    fn the_bootstrap_learns_a_growing_width() {
        let peaks = [
            (200.0, 2.5, 40_000.0),
            (800.0, 5.0, 40_000.0),
            (1800.0, 7.5, 40_000.0),
        ];
        let counts = synthetic(2048, 40.0, &peaks);
        let spectrum = Spectrum::from_counts(counts.iter().map(|c| *c as u64).collect());
        let data = spectrum.as_f64();
        let widths = widths_for(&spectrum, &data, 3.0).expect("a law is learned");
        let found = find(&data, &widths, 3.0, width_gate(), 40);
        assert_eq!(found.len(), 3, "all three growing-width peaks");
        // The learned law tracks the truth at both ends.
        let narrow = widths.fwhm(200.0) / (2.5 * FWHM_PER_SIGMA);
        let wide = widths.fwhm(1800.0) / (7.5 * FWHM_PER_SIGMA);
        assert!((0.7..1.4).contains(&narrow), "law at 200: {narrow}");
        assert!((0.7..1.4).contains(&wide), "law at 1800: {wide}");
    }
}
