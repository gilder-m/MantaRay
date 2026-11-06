//! Quantitative analysis: identify the marked regions against a library and
//! turn their net areas into activities.

use chrono::NaiveDateTime;
use serde::{Deserialize, Serialize};

use crate::analysis::{PeakInfo, mda_currie, peak_info};
use crate::efficiency::{EfficiencyCalibration, EfficiencyModel};
use crate::library::NuclideLibrary;
use crate::settings::CalculationSettings;
use crate::spectrum::Spectrum;

/// Correction that scales a measured activity back to a reference date.
///
/// `exp(+lambda * (measured - reference))`: a nuclide measured after its
/// reference date was more active then. A non-positive half life means 1.0.
pub fn decay_to_reference(
    half_life_seconds: f64,
    reference: NaiveDateTime,
    measured: NaiveDateTime,
) -> f64 {
    if half_life_seconds <= 0.0 {
        return 1.0;
    }
    let elapsed = (measured - reference).num_milliseconds() as f64 / 1000.0;
    (std::f64::consts::LN_2 * elapsed / half_life_seconds).exp()
}

/// Correction for decay while counting: `lambda*T / (1 - exp(-lambda*T))`.
///
/// It converts the average activity over the count into the activity at the
/// start of the count. A non-positive half life or count time means 1.0.
pub fn decay_during_acquisition(half_life_seconds: f64, count_seconds: f64) -> f64 {
    if half_life_seconds <= 0.0 || count_seconds <= 0.0 {
        return 1.0;
    }
    let lambda_t = std::f64::consts::LN_2 * count_seconds / half_life_seconds;
    if lambda_t < 1e-9 {
        return 1.0 + lambda_t / 2.0;
    }
    lambda_t / (1.0 - (-lambda_t).exp())
}

/// Becquerel to microcurie.
pub fn bq_to_uci(bq: f64) -> f64 {
    bq / 37_000.0
}

/// What the sample is, for per-unit results and decay correction.
#[derive(Clone, Debug, PartialEq, Serialize, Deserialize)]
pub struct SampleInfo {
    /// Free-text description saved with the spectrum.
    pub description: String,
    /// Mass, volume or item count; 1.0 means "no normalisation".
    pub quantity: f64,
    /// Unit of `quantity`, e.g. `kg`; empty for none.
    pub unit: String,
    /// Date the activity should be reported at.
    pub reference_date: Option<NaiveDateTime>,
}

impl Default for SampleInfo {
    fn default() -> Self {
        Self {
            description: String::new(),
            quantity: 1.0,
            unit: String::new(),
            reference_date: None,
        }
    }
}

/// Knobs for [`analyse`].
#[derive(Clone, Debug, PartialEq, Serialize, Deserialize)]
pub struct AnalysisOptions {
    /// Peak-info settings (background points, FW(1/x)M, sensitivity).
    pub settings: CalculationSettings,
    /// Efficiency curve; without it only count rates are reported.
    pub efficiency: Option<EfficiencyCalibration>,
    /// How far a peak centroid may sit from a library line, in keV.
    pub match_tolerance: f64,
    /// Sample description and quantity.
    pub sample: SampleInfo,
    /// List library nuclides that were not found, with an MDA.
    pub report_mda_for_absent_nuclides: bool,
    /// Correct activities back to [`SampleInfo::reference_date`].
    pub decay_correct_to_reference: bool,
    /// Correct for decay during the measurement.
    pub correct_decay_during_count: bool,
    /// How many standard deviations a net area needs to count as detected.
    pub detection_sigma: f64,
}

impl Default for AnalysisOptions {
    fn default() -> Self {
        Self {
            settings: CalculationSettings::default(),
            efficiency: None,
            match_tolerance: 2.0,
            sample: SampleInfo::default(),
            report_mda_for_absent_nuclides: false,
            decay_correct_to_reference: true,
            correct_decay_during_count: true,
            detection_sigma: 3.0,
        }
    }
}

/// The library line a peak was matched to.
#[derive(Clone, Debug, PartialEq, Serialize, Deserialize)]
pub struct Identification {
    /// Nuclide name.
    pub nuclide: String,
    /// Library line energy in keV.
    pub energy: f64,
    /// Gammas per 100 disintegrations.
    pub yield_percent: f64,
    /// Measured energy minus library energy, in keV.
    pub delta: f64,
    /// Whether the line is a key line.
    pub key_line: bool,
    /// Half life of the nuclide, in seconds.
    pub half_life_seconds: f64,
    /// Two-sigma library uncertainty of the nuclide, in percent.
    pub library_uncertainty_percent: f64,
}

/// One analysed region.
#[derive(Clone, Debug, PartialEq, Serialize, Deserialize)]
pub struct PeakResult {
    /// Peak-info figures for the region.
    pub info: PeakInfo,
    /// Centroid energy, when the spectrum is calibrated.
    pub energy: Option<f64>,
    /// Library match, when one was found.
    pub identification: Option<Identification>,
}

/// One line of a reported nuclide.
#[derive(Clone, Debug, PartialEq, Serialize, Deserialize)]
pub struct LineResult {
    /// Library line energy in keV.
    pub energy: f64,
    /// Measured centroid, in channels.
    pub centroid: f64,
    /// Net area.
    pub net_area: f64,
    /// Net-area uncertainty.
    pub net_uncertainty: f64,
    /// Net counts per second.
    pub count_rate: f64,
    /// Activity, in the report's activity unit (0 without an efficiency curve).
    pub activity: f64,
    /// Activity uncertainty.
    pub activity_uncertainty: f64,
    /// Minimum detectable activity for this line.
    pub mda: f64,
    /// Efficiency used.
    pub efficiency: f64,
    /// Yield used, in percent.
    pub yield_percent: f64,
    /// Whether the line took part in the weighted mean.
    pub used_in_average: bool,
}

/// A nuclide as reported.
#[derive(Clone, Debug, PartialEq, Serialize, Deserialize)]
pub struct NuclideResult {
    /// Nuclide name.
    pub nuclide: String,
    /// Weighted mean activity across the usable lines.
    pub activity: f64,
    /// Uncertainty of the weighted mean, including the library uncertainty.
    pub uncertainty: f64,
    /// Smallest MDA across the lines considered.
    pub mda: f64,
    /// Whether the nuclide counts as present.
    pub detected: bool,
    /// Half life in seconds.
    pub half_life_seconds: f64,
    /// The individual lines.
    pub lines: Vec<LineResult>,
}

/// The result of a quantitative analysis.
#[derive(Clone, Debug, PartialEq, Serialize, Deserialize)]
pub struct QuantitativeReport {
    /// Every analysed region, in channel order.
    pub peaks: Vec<PeakResult>,
    /// Nuclides, in library order.
    pub nuclides: Vec<NuclideResult>,
    /// Regions that matched nothing in the library.
    pub unidentified: Vec<PeakInfo>,
    /// Whether an efficiency curve was supplied.
    pub has_efficiency: bool,
    /// Sample quantity unit, if any.
    pub sample_unit: String,
    /// Live time of the analysed spectrum.
    pub live_time: f64,
    /// Real time of the analysed spectrum.
    pub real_time: f64,
    /// Things the analysis was asked for but could not do, and why - a
    /// requested correction or MDA row must not just silently vanish.
    #[serde(default)]
    pub notes: Vec<String>,
}

impl QuantitativeReport {
    /// Unit that activities are expressed in.
    pub fn activity_unit(&self) -> String {
        if !self.has_efficiency {
            return "cps".to_string();
        }
        if self.sample_unit.is_empty() {
            "Bq".to_string()
        } else {
            format!("Bq/{}", self.sample_unit)
        }
    }

    /// Sum of the activities of the detected nuclides that count towards a total.
    pub fn total_activity(&self) -> f64 {
        self.nuclides
            .iter()
            .filter(|n| n.detected)
            .map(|n| n.activity)
            .sum()
    }

    /// Nuclides that were detected.
    pub fn detected(&self) -> impl Iterator<Item = &NuclideResult> {
        self.nuclides.iter().filter(|n| n.detected)
    }
}

/// Analyses every marked region of a spectrum against a nuclide library.
pub fn analyse(
    spectrum: &Spectrum,
    library: &NuclideLibrary,
    options: &AnalysisOptions,
) -> QuantitativeReport {
    let calibration = spectrum.energy_calibration.clone();
    let mut peaks: Vec<PeakResult> = Vec::new();
    let mut unidentified: Vec<PeakInfo> = Vec::new();

    for roi in spectrum.rois.iter() {
        let Ok(info) = peak_info(spectrum, *roi, &options.settings) else {
            continue;
        };
        let energy = calibration.as_ref().map(|cal| info.centroid_energy(cal));
        let identification = energy.and_then(|energy| {
            library
                .best_match(energy, options.match_tolerance)
                .map(|matched| Identification {
                    nuclide: matched.nuclide.name.clone(),
                    energy: matched.peak.energy,
                    yield_percent: matched.peak.yield_percent,
                    delta: matched.delta,
                    key_line: matched.peak.key_line,
                    half_life_seconds: matched.nuclide.half_life_seconds,
                    library_uncertainty_percent: matched.nuclide.uncertainty_percent,
                })
        });
        if identification.is_none() {
            unidentified.push(info.clone());
        }
        peaks.push(PeakResult {
            info,
            energy,
            identification,
        });
    }

    let has_efficiency = options
        .efficiency
        .as_ref()
        .is_some_and(|efficiency| !efficiency.is_empty());
    let quantity = if options.sample.quantity > 0.0 {
        options.sample.quantity
    } else {
        1.0
    };
    let live_time = spectrum.live_time;

    let mut notes: Vec<String> = Vec::new();
    if options
        .efficiency
        .as_ref()
        .is_some_and(|curve| curve.is_empty() && curve.model != EfficiencyModel::None)
    {
        notes.push(
            "the efficiency curve gives no efficiency at any energy (no points above zero, \
             or a constant that is not a positive number); results are count rates"
                .to_string(),
        );
    }
    if live_time <= 0.0 && !peaks.is_empty() {
        notes.push(
            "the spectrum has no live time, so no activity or count rate can be computed"
                .to_string(),
        );
    }
    if options.decay_correct_to_reference
        && options.sample.reference_date.is_some()
        && spectrum.start_time.is_none()
    {
        notes.push(
            "decay correction to the reference date needs the spectrum's start time, \
             which is not recorded; activities are uncorrected"
                .to_string(),
        );
    }
    // MDA rows for absent nuclides have preconditions shared by every nuclide;
    // say those once rather than once per library entry.
    let mda_possible = if options.report_mda_for_absent_nuclides {
        let mut possible = true;
        if spectrum.energy_calibration.is_none() {
            notes.push(
                "MDAs for nuclides not found need an energy calibration; the spectrum has none"
                    .to_string(),
            );
            possible = false;
        }
        if spectrum.live_time <= 0.0 {
            notes.push(
                "MDAs for nuclides not found need a live time; the spectrum has none".to_string(),
            );
            possible = false;
        }
        possible
    } else {
        false
    };

    // The identified peaks bucketed by nuclide name, once. The loop below
    // used to walk every analysed peak for every library nuclide - the whole
    // peak list, thousands of times over an evaluated library, almost all of
    // it filtering out everything. A nuclide with no peaks and no MDA to
    // report now costs one lookup, and skips its decay arithmetic too.
    let mut by_nuclide: std::collections::HashMap<&str, Vec<usize>> =
        std::collections::HashMap::new();
    for (index, peak) in peaks.iter().enumerate() {
        if let Some(id) = &peak.identification {
            by_nuclide
                .entry(id.nuclide.as_str())
                .or_default()
                .push(index);
        }
    }

    let mut nuclides: Vec<NuclideResult> = Vec::new();
    let mut beyond_reference: Vec<String> = Vec::new();
    for nuclide in library.iter() {
        let matched = by_nuclide.get(nuclide.name.as_str());
        if matched.is_none() && !(mda_possible && !nuclide.flags.no_mda) {
            continue;
        }
        let (decay_correction, refused) =
            decay_corrections(spectrum, options, nuclide.half_life_seconds);
        let lines: Vec<LineResult> = matched
            .into_iter()
            .flatten()
            .map(|&index| &peaks[index])
            .map(|peak| {
                let id = peak.identification.as_ref().expect("filtered above");
                let library_peak = nuclide
                    .peaks
                    .iter()
                    .find(|p| (p.energy - id.energy).abs() < 1e-9);
                let efficiency = options
                    .efficiency
                    .as_ref()
                    .map(|curve| curve.efficiency(id.energy))
                    .unwrap_or(0.0);
                let yield_fraction = id.yield_percent / 100.0;
                let scale = if efficiency > 0.0 && yield_fraction > 0.0 && live_time > 0.0 {
                    decay_correction / (live_time * efficiency * yield_fraction * quantity)
                } else {
                    0.0
                };
                let mda = if efficiency > 0.0 && yield_fraction > 0.0 && live_time > 0.0 {
                    mda_currie(peak.info.background, efficiency, yield_fraction, live_time)
                        * decay_correction
                        / quantity
                } else if !has_efficiency && live_time > 0.0 {
                    // The cps fallback: the detection limit for the line's
                    // count rate, in the same unit the activity column uses.
                    mda_currie(peak.info.background, 1.0, 1.0, live_time) * decay_correction
                        / quantity
                } else {
                    0.0
                };
                // Without an efficiency curve there is no becquerel to report,
                // so the "activity" columns fall back to the count rate - the
                // report's activity unit already says "cps" in that case, and
                // an all-zero table would contradict it.
                let (activity, activity_uncertainty) = if has_efficiency {
                    (
                        peak.info.net_area * scale,
                        peak.info.net_area_uncertainty * scale,
                    )
                } else if live_time > 0.0 {
                    (
                        peak.info.net_count_rate(live_time),
                        peak.info.net_area_uncertainty / live_time,
                    )
                } else {
                    (0.0, 0.0)
                };
                LineResult {
                    energy: id.energy,
                    centroid: peak.info.centroid,
                    net_area: peak.info.net_area,
                    net_uncertainty: peak.info.net_area_uncertainty,
                    count_rate: peak.info.net_count_rate(live_time),
                    activity,
                    activity_uncertainty,
                    mda,
                    efficiency,
                    yield_percent: id.yield_percent,
                    used_in_average: library_peak
                        .map(|p| !p.not_in_average && p.photon.usable_for_activity())
                        .unwrap_or(true),
                }
            })
            .collect();

        if lines.is_empty() {
            if mda_possible && !nuclide.flags.no_mda {
                match mda_only_result(spectrum, options, nuclide, decay_correction) {
                    Ok(result) => {
                        if refused {
                            beyond_reference.push(nuclide.name.clone());
                        }
                        nuclides.push(result)
                    }
                    Err(reason) => notes.push(format!("no MDA for {}: {reason}", nuclide.name)),
                }
            }
            continue;
        }
        // A nuclide found where no line can carry a number used to be reported
        // detected at 0 Bq. Its peaks stay in the peak table; the activity row
        // is withheld, and the note says why.
        if live_time <= 0.0 {
            continue;
        }
        if has_efficiency
            && !lines
                .iter()
                .any(|line| line.efficiency > 0.0 && line.yield_percent > 0.0)
        {
            let energies = lines
                .iter()
                .map(|line| format!("{:.1}", line.energy))
                .collect::<Vec<_>>()
                .join(", ");
            notes.push(format!(
                "no activity for {}: the efficiency curve or the library yield is zero at \
                 its {energies} keV line(s)",
                nuclide.name
            ));
            continue;
        }
        if refused {
            beyond_reference.push(nuclide.name.clone());
        }

        // With an efficiency curve every usable line estimates the same
        // activity, so they average; without one the lines are just rates, and
        // the honest per-nuclide number is their sum.
        let (activity, mut uncertainty) = if has_efficiency {
            weighted_mean(&lines)
        } else {
            let total: f64 = lines.iter().map(|line| line.activity).sum();
            let spread = lines
                .iter()
                .map(|line| line.activity_uncertainty)
                .fold(0.0, f64::hypot);
            (total, spread)
        };
        let library_sigma = activity * nuclide.uncertainty_percent / 200.0;
        uncertainty = uncertainty.hypot(library_sigma);
        let mda = lines
            .iter()
            .map(|line| line.mda)
            .filter(|mda| *mda > 0.0)
            .fold(f64::INFINITY, f64::min);

        nuclides.push(NuclideResult {
            nuclide: nuclide.name.clone(),
            activity,
            uncertainty,
            mda: if mda.is_finite() { mda } else { 0.0 },
            detected: is_detected(nuclide, &lines, options.detection_sigma),
            half_life_seconds: nuclide.half_life_seconds,
            lines,
        });
    }
    if !beyond_reference.is_empty() {
        notes.push(beyond_reference_note(&beyond_reference));
    }

    QuantitativeReport {
        peaks,
        nuclides,
        unidentified,
        has_efficiency,
        sample_unit: options.sample.unit.clone(),
        live_time,
        real_time: spectrum.real_time,
        notes,
    }
}

/// The decay correction for one nuclide, and whether the reference-date part
/// had to be left out.
///
/// Past about 1024 half-lives the correction to the reference date is more
/// than a double can hold. Applied as infinity, it turned the nuclide's
/// activity into NaN, and the report's total with it - three days is enough
/// for a nuclide with a three-minute half-life.
fn decay_corrections(
    spectrum: &Spectrum,
    options: &AnalysisOptions,
    half_life_seconds: f64,
) -> (f64, bool) {
    let mut correction = 1.0;
    if options.correct_decay_during_count {
        let count_time = if spectrum.real_time > 0.0 {
            spectrum.real_time
        } else {
            spectrum.live_time
        };
        correction *= decay_during_acquisition(half_life_seconds, count_time);
    }
    if options.decay_correct_to_reference
        && let (Some(reference), Some(measured)) =
            (options.sample.reference_date, spectrum.start_time)
    {
        let corrected = correction * decay_to_reference(half_life_seconds, reference, measured);
        if !corrected.is_finite() {
            return (correction, true);
        }
        correction = corrected;
    }
    (correction, false)
}

/// The note for nuclides whose reference-date correction was left out.
fn beyond_reference_note(names: &[String]) -> String {
    const LISTED: usize = 10;
    let mut listed = names
        .iter()
        .take(LISTED)
        .cloned()
        .collect::<Vec<_>>()
        .join(", ");
    if names.len() > LISTED {
        listed.push_str(&format!(" and {} more", names.len() - LISTED));
    }
    format!(
        "decay correction to the reference date left out for {listed}: too many half-lives \
         lie between that date and the count for the correction to be computed, so these \
         are reported at the time of the count"
    )
}

fn weighted_mean(lines: &[LineResult]) -> (f64, f64) {
    let usable: Vec<&LineResult> = lines
        .iter()
        .filter(|line| line.used_in_average && line.activity_uncertainty > 0.0)
        .collect();
    if usable.is_empty() {
        let count = lines.len().max(1) as f64;
        let mean = lines.iter().map(|line| line.activity).sum::<f64>() / count;
        let spread = lines
            .iter()
            .map(|line| line.activity_uncertainty)
            .fold(0.0f64, f64::max);
        return (mean, spread);
    }
    // Weights relative to the most precise line. As 1/sigma^2 they overflowed
    // for large activities: the squares became infinite, the weights zero and
    // the mean 0/0.
    let best = usable
        .iter()
        .map(|line| line.activity_uncertainty)
        .fold(f64::INFINITY, f64::min);
    let mut weight_sum = 0.0;
    let mut weighted = 0.0;
    for line in &usable {
        let weight = (best / line.activity_uncertainty).powi(2);
        weight_sum += weight;
        weighted += weight * line.activity;
    }
    (weighted / weight_sum, best / weight_sum.sqrt())
}

fn is_detected(
    nuclide: &crate::library::Nuclide,
    lines: &[LineResult],
    detection_sigma: f64,
) -> bool {
    let present = |energy: f64| {
        lines.iter().any(|line| {
            (line.energy - energy).abs() < 1e-9
                && line.net_area > detection_sigma * line.net_uncertainty
        })
    };
    let key_lines: Vec<f64> = nuclide
        .peaks
        .iter()
        .filter(|peak| peak.key_line)
        .map(|peak| peak.energy)
        .collect();
    if key_lines.is_empty() {
        nuclide
            .peaks
            .first()
            .map(|peak| present(peak.energy))
            .unwrap_or(false)
    } else {
        key_lines.iter().all(|energy| present(*energy))
    }
}

/// Builds an MDA-only entry for a nuclide that was not found.
///
/// The error is the reason no entry can be built, worded for the report's
/// notes: the caller asked for this row, so its absence must be explained.
fn mda_only_result(
    spectrum: &Spectrum,
    options: &AnalysisOptions,
    nuclide: &crate::library::Nuclide,
    decay_correction: f64,
) -> Result<NuclideResult, String> {
    let calibration = spectrum
        .energy_calibration
        .as_ref()
        .ok_or("the spectrum has no energy calibration")?;
    // Without an efficiency curve the report is in count-rate units, and the
    // detection limit follows suit rather than disappearing.
    let efficiency = options
        .efficiency
        .as_ref()
        .filter(|curve| !curve.is_empty());
    if spectrum.live_time <= 0.0 {
        return Err("the spectrum has no live time".to_string());
    }
    let quantity = if options.sample.quantity > 0.0 {
        options.sample.quantity
    } else {
        1.0
    };
    let reference_line = nuclide
        .peaks
        .iter()
        .find(|peak| peak.key_line)
        .or_else(|| nuclide.peaks.first())
        .ok_or("the library holds no line for it")?;
    let channel = calibration.channel(reference_line.energy).ok_or_else(|| {
        format!(
            "the calibration gives no channel for its {:.1} keV line",
            reference_line.energy
        )
    })?;
    if channel < 0.0 || channel >= spectrum.len() as f64 {
        return Err(format!(
            "its {:.1} keV line falls outside the spectrum",
            reference_line.energy
        ));
    }
    let fwhm = spectrum.fwhm_at(channel).unwrap_or(4.0);
    let half = (1.5 * fwhm).max(2.0);
    let low = (channel - half).max(0.0) as usize;
    let high = ((channel + half) as usize).min(spectrum.len() - 1);
    let background = spectrum.sum_range(low, high) as f64;
    // With a curve the MDA is in activity units at this line's efficiency and
    // yield; without one it is the smallest detectable count rate.
    let (curve_efficiency, yield_fraction) = match efficiency {
        Some(curve) => (
            curve.efficiency(reference_line.energy),
            reference_line.yield_percent / 100.0,
        ),
        None => (1.0, 1.0),
    };
    if curve_efficiency <= 0.0 || yield_fraction <= 0.0 {
        return Err(format!(
            "the efficiency curve gives nothing at its {:.1} keV line",
            reference_line.energy
        ));
    }
    let mda = mda_currie(
        background,
        curve_efficiency,
        yield_fraction,
        spectrum.live_time,
    ) * decay_correction
        / quantity;
    Ok(NuclideResult {
        nuclide: nuclide.name.clone(),
        activity: 0.0,
        uncertainty: 0.0,
        mda,
        detected: false,
        half_life_seconds: nuclide.half_life_seconds,
        lines: Vec::new(),
    })
}
