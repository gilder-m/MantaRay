//! Matched-filter peak search: one kernel, shaped like the detector.
//!
//! The method comes from the [Becquerel](https://github.com/lbl-anp/becquerel)
//! project of the Lawrence Berkeley National Laboratory Applied Nuclear
//! Physics group, rewritten in Rust and used with the maintainers'
//! permission and our thanks.
//!
//! A peak search that knows the detector's resolution needs only one filter:
//! a zero-sum Mexican-hat kernel whose width at every channel is the width a
//! real peak would have *there*, slid over the spectrum to make a
//! signal-to-noise map with exact Poisson propagation. A one-channel spike
//! at a channel where real peaks are fifty channels wide scores nothing
//! against a fifty-channel kernel - the false positives a fixed ladder of
//! scales must threshold away are structurally impossible. A peak is a local
//! maximum of the map that also passes a width gate: the curvature of the
//! map at its top must imply a width consistent with the resolution model,
//! which is what rejects glitches and Compton edges alike.
//!
//! The kernel width comes from the spectrum's shape calibration when it
//! holds a usable one. Otherwise the spectrum teaches itself: the filter is
//! scanned over a short ladder of fixed trial widths to *notice* peaks, each
//! sighting's width is measured directly off the counts - each side of a
//! peak judged against its own floor, so a photopeak on a stepped continuum
//! reads true - and the width law `fwhm(x)^2 = c0 + c1*x` is fitted to what
//! was measured, medians across rungs for direct measurements and the
//! self-consistent rung for curvature ones, from clusters more than one rung
//! confirmed. `c1 = 0` is inside the model, so detectors whose width barely
//! grows are served by the same law.
//!
//! On the HPGe bench corpus this finds Ba-133's five signature lines to
//! half a keV plus its sub-percent lines, and all eleven canonical Eu-152
//! lines with 1085.9 and 1089.7 keV resolved as neighbours.

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
    ///
    /// A quadratic shape calibration is read only up to its vertex: real
    /// files carry fits whose negative curvature turns downward past the
    /// last calibrated peak - one bench file crosses zero at channel 4,800
    /// of 8,192 - and no detector's resolution improves with energy. Read
    /// literally, the falling branch shrinks the kernel until it matches
    /// one-channel spikes over exactly the sparse tail where spikes live.
    pub(crate) fn fwhm(&self, channel: f64) -> f64 {
        let fwhm = match self {
            Widths::Shape(shape) => {
                let [_, b, c] = shape.coefficients;
                let channel = if c < 0.0 {
                    channel.min(-b / (2.0 * c))
                } else {
                    channel
                };
                shape.fwhm(channel)
            }
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

    /// A robust first law: median of pairwise slopes on `fwhm^2 = c0 + c1*x`.
    ///
    /// The trimmed refit needs a reference the outliers cannot drag, and a
    /// weighted least-squares first pass is no such thing: on the bench
    /// background it handed the annihilation line - genuinely
    /// Doppler-broadened, honestly measured, and very loud - the authority
    /// to raise the whole law until the real 295 and 300 keV lines failed
    /// the width gate against it. The median of pairwise slopes shrugs off
    /// a quarter of the points being physics of their own; the weighted fit
    /// afterwards polishes what survives the trim. Pairs closer than 32
    /// channels are skipped - their slope is mostly measurement noise
    /// divided by a small number.
    fn robust(points: &[(f64, f64, f64)]) -> Option<Self> {
        let mut slopes = Vec::new();
        for (i, &(xi, fi, _)) in points.iter().enumerate() {
            for &(xj, fj, _) in &points[i + 1..] {
                if (xj - xi).abs() >= 32.0 {
                    slopes.push((fj * fj - fi * fi) / (xj - xi));
                }
            }
        }
        let median = |values: &mut Vec<f64>| -> Option<f64> {
            if values.is_empty() {
                return None;
            }
            values.sort_by(|a, b| a.partial_cmp(b).unwrap_or(std::cmp::Ordering::Equal));
            Some(values[values.len() / 2])
        };
        let c1 = median(&mut slopes)?.max(0.0);
        let mut intercepts: Vec<f64> = points
            .iter()
            .map(|(x, fwhm, _)| fwhm * fwhm - c1 * x)
            .collect();
        let c0 = median(&mut intercepts)?.max(1.0);
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
        if ratio > gate.1 {
            continue;
        }
        if ratio < gate.0 {
            // The curvature estimate compresses every width mismatch toward
            // one, so by the time it reads narrow the candidate is either a
            // glitch or the model is locally wrong - a linear width law
            // cannot follow a drift-broadened long acquisition everywhere,
            // and on the bench background it sat 20% high at 295 keV and
            // failed two real Pb lines here. The counts can say which is
            // which: a direct half-maximum measurement inside the same band
            // keeps the peak. Only a candidate three sigmas clear of the
            // threshold earns the appeal - one at the edge of existence has
            // no evidence to overrule a shape prior with - and a one-channel
            // spike measures exactly the measurement's 2.0-channel clamp,
            // which is kept out.
            let saved = snr[channel] >= min_snr + 3.0
                && direct_width(counts, channel as f64, fwhm0).is_some_and(|(_, measured, _)| {
                    measured > 2.05 && (gate.0..=gate.1).contains(&(measured / fwhm0))
                });
            if !saved {
                continue;
            }
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

/// The width source for a spectrum: its shape calibration if the counts
/// agree with it, otherwise self-taught.
///
/// A shape calibration is never taken on faith. MAESTRO writes a
/// `$SHAPE_CAL` block whether or not anyone calibrated shape, so real files
/// carry a constant fitted once under other conditions, or a quadratic from
/// another geometry - and a wrong width claim does not merely mislabel
/// peaks, it decides which peaks exist at all. So the spectrum is always
/// measured first, and the calibration is kept only when it agrees with
/// what was measured; a lying calibration is outvoted by its own counts.
///
/// The measuring pass: the matched filter is run at each fixed trial width
/// in [`BOOTSTRAP_FWHMS`] under a loose gate to *notice* peaks; each
/// sighting's width is then measured directly on the counts - a local background line and a walk to
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
    let claimed = spectrum
        .shape_calibration
        .as_ref()
        .filter(|shape| shape.is_usable())
        .map(Widths::Shape);
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
                // A measurement at the 2.0-channel clamp is not testimony:
                // it means "at or below what half-maximum walking can see",
                // which is a glitch's signature, and five glitches measured
                // that way with spectacular significance once outvoted an
                // honest calibration. It still marks the sighting, weakly.
                Some((centroid, fwhm, significance)) if fwhm > 2.0 => (
                    centroid,
                    Opinion {
                        fwhm,
                        direct: true,
                        inconsistency: 0.0,
                    },
                    significance,
                ),
                Some((centroid, fwhm, _)) => (
                    centroid,
                    Opinion {
                        fwhm,
                        direct: false,
                        inconsistency: f64::INFINITY,
                    },
                    candidate.snr.min(3.0),
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
    let confirmed_measured: Vec<(f64, f64, f64)> = clusters
        .iter()
        .filter(|cluster| confirmed(cluster))
        .map(|cluster| (cluster.centroid, median_width(cluster), cluster.weight))
        .collect();
    // The calibration's audit: it claims a width at every channel where one
    // was just measured, and it keeps its authority unless most of the
    // *confirmed* measurements sit outside the search gate's band of the
    // claim - the same band a peak must hit to be found, so "the calibration
    // is right" and "the peaks it predicts are findable" are one test. The
    // burden of proof is on the counts: only clusters two rungs measured
    // directly may vote, and fewer than two such witnesses cannot overrule -
    // a presumption that protects an honest calibration on a spectrum whose
    // one strong line is broadened physics of its own, the way annihilation
    // quanta are. A truthful calibration then beats any law fitted to a
    // handful of peaks, being smooth and whole-range.
    if let Some(claimed) = claimed {
        let agreeing = confirmed_measured
            .iter()
            .filter(|(channel, fwhm, _)| {
                (WIDTH_GATE.0..=WIDTH_GATE.1).contains(&(fwhm / claimed.fwhm(*channel)))
            })
            .count();
        if confirmed_measured.len() < 2 || 2 * agreeing >= confirmed_measured.len() {
            return Some(claimed);
        }
    }
    let mut measured = confirmed_measured;
    if measured.len() < 2 {
        measured = clusters
            .iter()
            .map(|cluster| (cluster.centroid, median_width(cluster), cluster.weight))
            .collect();
    }
    let first = match measured.as_slice() {
        [] => return None,
        [(channel, fwhm, _)] => return Some(Widths::from_anchor(*channel, *fwhm)),
        many => Widths::robust(many).or_else(|| Widths::fit(many))?,
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
/// The window starts sized to the rung that noticed the peak, then serves
/// the peak: when the measurement comes back much narrower than the trial,
/// it is retaken with a window matched to the measured width. A wide rung's
/// window spans a hundred channels of *sloping* continuum, its per-side
/// floors sit far down the slope, the half-level sinks with them, and a
/// 3.6-channel background line reads five and a half - which taught the
/// width law to expect widths the real peaks then failed against. The
/// per-side floors answer the step problem; the matched window answers the
/// slope problem; both lessons were paid for on the bench.
fn direct_width(counts: &[f64], centroid: f64, trial_fwhm: f64) -> Option<(f64, f64, f64)> {
    let mut trial = trial_fwhm;
    let mut best: Option<(f64, f64, f64)> = None;
    // To the fixed point: remeasure with the window the last measurement
    // implies, until the width stops shrinking. Stopping at the first
    // "close enough" left a quarter of the slope bias in - the background
    // bench file measured 4.7 channels at 609 keV where the same detector's
    // Eu-152 files say 3.7 - and a law taught 27% wide fails real peaks at
    // the width gate. A measurement that grows instead means a neighbour
    // has entered the window: keep the smaller word and stop.
    for _ in 0..6 {
        let Some(measured) = direct_width_once(counts, centroid, trial) else {
            return best;
        };
        let fwhm = measured.1;
        if fwhm >= trial * 0.95 {
            // Settled (a hair under the window) keeps the fresh measurement;
            // grown (over it) keeps the smaller word already in hand.
            if fwhm <= trial || best.is_none() {
                best = Some(measured);
            }
            return best;
        }
        best = Some(measured);
        trial = fwhm;
    }
    best
}

/// One measurement of [`direct_width`], at one window size.
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
fn direct_width_once(counts: &[f64], centroid: f64, trial_fwhm: f64) -> Option<(f64, f64, f64)> {
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
    // Each side's floor is its median, not its minimum. The minimum of a
    // window of Poisson counts sits two or three noise sigmas below the true
    // level, which let any +2-sigma wiggle on a flat continuum pass the
    // significance test below as a "measured peak" - enough of them to
    // outvote a real calibration in the audit. The median is the level.
    let side_median = |side: &[f64]| -> f64 {
        let mut sorted: Vec<f64> = side.to_vec();
        sorted.sort_by(|a, b| a.partial_cmp(b).unwrap_or(std::cmp::Ordering::Equal));
        sorted[sorted.len() / 2]
    };
    let floor_left = side_median(&counts[low..apex]);
    let floor_right = side_median(&counts[apex + 1..=high]);
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

    /// A downturned quadratic calibration is read only up to its vertex.
    ///
    /// The coefficients are a real HPGe file's; read literally they cross
    /// zero at channel ~10,300, and the floor of one channel would then
    /// shape the kernel to match exactly the one-channel spikes the whole
    /// search exists to reject.
    #[test]
    fn a_downturned_shape_calibration_is_clamped_at_its_vertex() {
        let shape = ShapeCalibration::new([2.4946, 1.745e-3, -1.9356e-7]);
        let widths = Widths::Shape(&shape);
        let vertex = 1.745e-3 / (2.0 * 1.9356e-7);
        assert!(
            (widths.fwhm(16_000.0) - widths.fwhm(vertex)).abs() < 1e-9,
            "past the vertex the width must hold at the vertex's"
        );
        assert!(
            widths.fwhm(16_000.0) > 5.0,
            "the clamp keeps the widest honest width, not the one-channel floor: {}",
            widths.fwhm(16_000.0)
        );
    }

    /// A calibration far narrower than the peaks is outvoted by the counts.
    ///
    /// The bench background carries `[1.62, 0, 0]` from some other day while
    /// its real peaks run four to eight channels wide; trusted, that claim
    /// rejected Tl-208 at the top of a 2.8-day acquisition through the width
    /// gate's upper bound.
    #[test]
    fn a_too_narrow_shape_calibration_is_overruled() {
        let peaks = [
            (250.0, 4.0, 30_000.0),
            (512.0, 4.0, 50_000.0),
            (770.0, 4.0, 20_000.0),
        ];
        let counts = synthetic(1024, 30.0, &peaks);
        let mut spectrum = Spectrum::from_counts(counts.iter().map(|c| *c as u64).collect());
        spectrum.shape_calibration = Some(ShapeCalibration::new([1.62, 0.0, 0.0]));
        let data = spectrum.as_f64();
        let widths = widths_for(&spectrum, &data, 3.0).expect("a width source");
        assert!(
            matches!(widths, Widths::Law { .. }),
            "the lying calibration must lose its authority"
        );
        let found = find(&data, &widths, 3.0, width_gate(), 40);
        assert_eq!(found.len(), 3, "all three peaks despite the calibration");
    }

    /// And one far wider - the constant-at-high-energy case real files carry.
    #[test]
    fn a_too_wide_shape_calibration_is_overruled() {
        let peaks = [(250.0, 1.4, 20_000.0), (512.0, 1.4, 30_000.0)];
        let counts = synthetic(1024, 30.0, &peaks);
        let mut spectrum = Spectrum::from_counts(counts.iter().map(|c| *c as u64).collect());
        spectrum.shape_calibration = Some(ShapeCalibration::new([11.4, 0.0, 0.0]));
        let data = spectrum.as_f64();
        let widths = widths_for(&spectrum, &data, 3.0).expect("a width source");
        let found = find(&data, &widths, 3.0, width_gate(), 40);
        assert_eq!(found.len(), 2, "narrow real peaks despite the wide claim");
        for ((centre, _, _), found) in peaks.iter().zip(&found) {
            assert!((found.centroid - centre).abs() < 1.0, "{}", found.centroid);
        }
    }

    /// A truthful calibration keeps its authority over any fitted law.
    #[test]
    fn an_honest_shape_calibration_is_kept() {
        let peaks = [(250.0, 4.0, 30_000.0), (700.0, 4.0, 25_000.0)];
        let counts = synthetic(1024, 30.0, &peaks);
        let mut spectrum = Spectrum::from_counts(counts.iter().map(|c| *c as u64).collect());
        spectrum.shape_calibration = Some(ShapeCalibration::new([4.0 * FWHM_PER_SIGMA, 0.0, 0.0]));
        let data = spectrum.as_f64();
        let widths = widths_for(&spectrum, &data, 3.0).expect("a width source");
        assert!(
            matches!(widths, Widths::Shape(_)),
            "agreement must keep the calibration"
        );
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
