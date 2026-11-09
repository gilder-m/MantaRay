//! Quantitative analysis: identify the marked regions against a library and
//! turn their net areas into activities.

use chrono::NaiveDateTime;
use serde::{Deserialize, Serialize};

use crate::analysis::{PeakInfo, mda_currie, peak_info};
use crate::efficiency::{EfficiencyCalibration, EfficiencyModel};
use crate::library::{Nuclide, NuclideLibrary};
use crate::numbers::plain_zero;
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

/// Why `quantity` cannot be a sample quantity, or `None` when it can: it has
/// to be a finite number above 0. "No quantity" is 1 with no unit, which is
/// plain Bq. The command line and the interface refuse what this names; the
/// analysis itself reports per sample, with a note, when it is given one.
#[must_use]
pub fn sample_quantity_problem(quantity: f64) -> Option<String> {
    if quantity.is_finite() && quantity > 0.0 {
        None
    } else {
        Some(format!(
            "{quantity} is not a sample quantity: give a number above 0"
        ))
    }
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
    /// Minimum detectable activity for this line; `None` when there is none
    /// to report (the figure is not a number, or the line cannot measure one).
    #[serde(default)]
    pub mda: Option<f64>,
    /// Efficiency used; `None` when the report is of count rates (no curve,
    /// or one that gives no efficiency at any energy). A line the curve gives
    /// infinity, NaN or a negative number at is not reported at all; a note
    /// says so. A report saved before this was optional reads its number as a
    /// value.
    #[serde(default)]
    pub efficiency: Option<f64>,
    /// Yield used, in percent.
    pub yield_percent: f64,
    /// Whether the line took part in the weighted mean.
    pub used_in_average: bool,
}

impl LineResult {
    /// Whether the line could measure an activity at all: the efficiency
    /// curve and the library yield both give it something. A line that does
    /// not has activity 0 because it said nothing, not because it measured 0.
    pub fn measures(&self) -> bool {
        self.efficiency
            .is_some_and(|efficiency| measures(efficiency, self.yield_percent))
    }
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
    /// Smallest MDA across the lines considered; `None` when no line has one
    /// to report. A report saved before this was optional reads its 0 as a value.
    #[serde(default)]
    pub mda: Option<f64>,
    /// Whether the nuclide counts as present.
    pub detected: bool,
    /// Half life in seconds.
    pub half_life_seconds: f64,
    /// The individual lines.
    pub lines: Vec<LineResult>,
}

/// A nuclide the analysis found (it has a matched peak) and could not report.
///
/// Both texts are on one line: [`Withheld::new`] makes them so once, and every
/// output (the text report, the CSV, the dialog) prints them as they are.
#[derive(Clone, Debug, PartialEq, Serialize, Deserialize)]
pub struct Withheld {
    /// Nuclide name.
    pub nuclide: String,
    /// What is missing and why, e.g. `no activity: the spectrum has no live time`.
    pub reason: String,
}

impl Withheld {
    /// A withheld nuclide, its name and reason each made one line.
    pub fn new(nuclide: &str, reason: &str) -> Self {
        Self {
            nuclide: one_line(nuclide),
            reason: one_line(reason),
        }
    }
}

/// Text on one line: every run of whitespace, line breaks included, becomes
/// one space. Names come from user-written library files and reasons quote
/// them, so anything printed per line is cleaned once, where it is built.
pub fn one_line(text: &str) -> String {
    text.split_whitespace().collect::<Vec<_>>().join(" ")
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
    /// Nuclides with no row in [`Self::nuclides`], in library order, each with
    /// the reason. A report saved before this field existed has none.
    #[serde(default)]
    pub withheld: Vec<Withheld>,
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

    /// Sum of the activities of the detected nuclides.
    ///
    /// `None` when the sum is not a number a report can show (too large for a
    /// double, or a zero made by underflow); [`analyse`] adds a note saying
    /// why. `Some(0.0)` is a real total of nothing detected.
    pub fn total_activity(&self) -> Option<f64> {
        self.total().ok()
    }

    /// The total, or why it cannot be shown: the one place it is computed.
    fn total(&self) -> Result<f64, Unreportable> {
        let mut sum = 0.0;
        let mut seen = Members::default();
        for nuclide in self.detected() {
            sum += nuclide.activity;
            seen.add(nuclide.activity);
        }
        reportable(sum, seen.zero_may_stand()).map(plain_zero)
    }

    /// Nuclides that were detected.
    pub fn detected(&self) -> impl Iterator<Item = &NuclideResult> {
        self.nuclides.iter().filter(|n| n.detected)
    }

    /// The withheld nuclides in runs that share one reason. When every found
    /// nuclide is withheld for the same global cause (the spectrum has no live
    /// time) that is one run, and the text report and the dialog say it once
    /// instead of once per nuclide; the CSV still has a row for each.
    pub fn withheld_groups(&self) -> impl Iterator<Item = &[Withheld]> {
        self.withheld.chunk_by(|a, b| a.reason == b.reason)
    }
}

impl Withheld {
    /// The names of a group, the first ten and a count of the rest.
    pub fn names(group: &[Withheld]) -> String {
        name_list(group.iter().map(|w| w.nuclide.as_str()), 10)
    }
}

/// The first `limit` names, comma separated, and the count of the rest with
/// thousands separators: `A, B, C and 1,274 more`. The one way a note, the text
/// report and the dialog list names.
fn name_list<'a>(names: impl ExactSizeIterator<Item = &'a str>, limit: usize) -> String {
    let total = names.len();
    let mut listed = names.take(limit).collect::<Vec<_>>().join(", ");
    if total > limit {
        listed.push_str(&format!(" and {} more", thousands(total - limit)));
    }
    listed
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
    // A quantity that cannot divide (0, negative, NaN, infinite) used to become
    // 1 while the column still read Bq/<unit>: per-sample figures under a
    // per-unit label. They are still per sample, but now say so.
    let bad_quantity = sample_quantity_problem(options.sample.quantity).is_some();
    let quantity = if bad_quantity {
        1.0
    } else {
        options.sample.quantity
    };
    let sample_unit = if bad_quantity {
        String::new()
    } else {
        options.sample.unit.clone()
    };
    let live_time = spectrum.live_time;

    let mut notes: Vec<String> = Vec::new();
    if bad_quantity {
        let given = options.sample.quantity;
        let unit = options.sample.unit.trim();
        notes.push(if unit.is_empty() {
            format!(
                "sample quantity {given} is not a number above 0: activities are not divided by it"
            )
        } else {
            format!(
                "sample quantity {given} {unit} is not a number above 0: activities are for the \
                 whole sample, not per {unit}"
            )
        });
    }
    let mut withheld: Vec<Withheld> = Vec::new();
    // Library nuclides that were not found and whose detection limit could not
    // be built, with the reason: one note for all of them, below.
    let mut no_limit: Vec<(String, String)> = Vec::new();
    if let Some(curve) = options
        .efficiency
        .as_ref()
        .filter(|curve| curve.is_empty() && curve.model != EfficiencyModel::None)
    {
        notes.push(format!(
            "the efficiency curve gives no efficiency at any energy ({}); results are \
             count rates",
            empty_curve_cause(&curve.model)
        ));
    }
    // The missing live time is said once, and the withheld list is what
    // decides who says it: when it carries the live-time reason (some found
    // nuclide was withheld for it) it has said so, otherwise a note does,
    // once the nuclides have been examined. These are the places the notes
    // go; they are filled in after the loop.
    let live_time_slot = notes.len();
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
        if !has_live_time(spectrum.live_time) {
            possible = false;
        }
        possible
    } else {
        false
    };
    let mda_live_slot = notes.len();

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

    let context = Context {
        spectrum,
        options,
        peaks: &peaks,
        has_efficiency,
        quantity,
        live_time,
    };
    let mut nuclides: Vec<NuclideResult> = Vec::new();
    let mut beyond_reference: Vec<String> = Vec::new();
    let mut bad_half_lives: Vec<(String, f64)> = Vec::new();
    for nuclide in library.iter() {
        let matched = by_nuclide
            .get(nuclide.name.as_str())
            .map_or(&[][..], Vec::as_slice);
        if matched.is_empty() && !(mda_possible && !nuclide.flags.no_mda) {
            continue;
        }
        // The notes of a nuclide travel with its outcome: one that ends up with
        // no row leaves a single reason, never notes about a row.
        let (notes_here, reference_refused) = match examine(&context, nuclide, matched) {
            Ok(examined) => {
                if let Some(seconds) = examined.bad_half_life {
                    bad_half_lives.push((nuclide.name.clone(), seconds));
                }
                nuclides.push(examined.result);
                (examined.notes, examined.reference_refused)
            }
            Err(refusal) => {
                if let Some(seconds) = refusal.bad_half_life {
                    bad_half_lives.push((nuclide.name.clone(), seconds));
                }
                match refusal.why {
                    Why::Nothing => {}
                    Why::Withheld(reason) => withheld.push(Withheld::new(&nuclide.name, &reason)),
                    Why::NoLimit(reason) => no_limit.push((nuclide.name.clone(), reason)),
                }
                (Vec::new(), refusal.reference_refused)
            }
        };
        notes.extend(notes_here);
        if reference_refused {
            beyond_reference.push(nuclide.name.clone());
        }
    }
    if !has_live_time(live_time) {
        let said = withheld
            .iter()
            .any(|entry| entry.reason == NO_LIVE_TIME_REASON);
        let global = !said && !peaks.is_empty();
        // Later slot first, so the earlier one is still where it was.
        if options.report_mda_for_absent_nuclides && !said && !global {
            notes.insert(
                mda_live_slot,
                "MDAs for nuclides not found need a live time; the spectrum has none".to_string(),
            );
        }
        if global {
            notes.insert(
                live_time_slot,
                "the spectrum has no live time, so no activity or count rate can be computed"
                    .to_string(),
            );
        }
    }
    if !beyond_reference.is_empty() {
        notes.push(beyond_reference_note(&beyond_reference));
    }
    if !bad_half_lives.is_empty() {
        notes.push(bad_half_lives_note(&bad_half_lives));
    }
    if let Some(note) = no_limit_note(&no_limit) {
        notes.push(note);
    }
    // Names from library files can hold line breaks and the notes quote them:
    // one line each, once, here.
    for note in &mut notes {
        *note = one_line(note);
    }

    let mut report = QuantitativeReport {
        peaks,
        nuclides,
        unidentified,
        has_efficiency,
        sample_unit,
        live_time,
        real_time: spectrum.real_time,
        notes,
        withheld,
    };
    if let Err(why) = report.total() {
        report.notes.push(format!(
            "no total activity shown: the sum of the detected activities is {}",
            why.words()
        ));
    }
    report
}

/// What every nuclide of one analysis shares.
struct Context<'a> {
    spectrum: &'a Spectrum,
    options: &'a AnalysisOptions,
    peaks: &'a [PeakResult],
    has_efficiency: bool,
    quantity: f64,
    live_time: f64,
}

/// A nuclide that gets a row, with the notes about that row.
struct Examined {
    result: NuclideResult,
    notes: Vec<String>,
    /// The correction to the reference date was left out for it.
    reference_refused: bool,
    /// Not found, and the library's half-life for it (this value) is negative
    /// or not a number: said once for all such nuclides, not once each.
    bad_half_life: Option<f64>,
}

/// Why a nuclide gets no row.
enum Why {
    /// The report already says it for every nuclide (no live time) and the
    /// nuclide was not found, so there is nothing to add.
    Nothing,
    /// A nuclide that was found and cannot be reported: a row of the withheld
    /// list, with this reason (without the nuclide's name).
    Withheld(String),
    /// A nuclide that was not found and whose detection limit cannot be built:
    /// counted with the others in one general note.
    NoLimit(String),
}

/// A nuclide that gets no row, and why.
struct Refusal {
    why: Why,
    reference_refused: bool,
    /// An absent nuclide whose library half-life is negative or not a number
    /// (the value): said in the grouped note whatever the detection limit did.
    bad_half_life: Option<f64>,
}

impl Refusal {
    fn silent() -> Self {
        Self {
            why: Why::Nothing,
            reference_refused: false,
            bad_half_life: None,
        }
    }

    fn withheld(reason: String) -> Self {
        Self {
            why: Why::Withheld(reason),
            reference_refused: false,
            bad_half_life: None,
        }
    }

    /// A found nuclide refused over a quantity: when the correction to the
    /// reference date was left out for it, the reason says so too.
    fn withheld_over_a_quantity(reason: String, corrections: &Corrections) -> Self {
        let reason = if corrections.refused {
            format!(
                "{reason}; the correction to the reference date was also left out for it \
                 (too many half-lives)"
            )
        } else {
            reason
        };
        Self {
            why: Why::Withheld(reason),
            reference_refused: corrections.refused,
            bad_half_life: None,
        }
    }

    /// An absent nuclide whose limit is not a number. The reason stays the
    /// same words for every nuclide so that they group; the reference date
    /// has its own note naming the nuclide.
    fn no_limit(reason: String, corrections: &Corrections) -> Self {
        Self {
            why: Why::NoLimit(reason),
            reference_refused: corrections.refused,
            bad_half_life: None,
        }
    }
}

/// Why a found nuclide is withheld when the spectrum has no live time.
const NO_LIVE_TIME_REASON: &str = "no activity: the spectrum has no live time";

/// The one function that turns a nuclide's matched peaks, or its absence, into
/// a reported row or the reason there is none.
fn examine(context: &Context, nuclide: &Nuclide, matched: &[usize]) -> Result<Examined, Refusal> {
    // The one place the live time is required: nothing below can measure
    // without it.
    if !has_live_time(context.live_time) {
        return Err(if matched.is_empty() {
            Refusal::silent()
        } else {
            Refusal::withheld(NO_LIVE_TIME_REASON.to_string())
        });
    }
    let corrections =
        Corrections::new(context.spectrum, context.options, nuclide.half_life_seconds);
    // A nuclide is reported without a decay correction it could not have, and
    // says why: the library entry, not the sample's dates. Found or absent.
    let words = corrections.half_life_words(&nuclide.name);
    let mut outcome = if matched.is_empty() {
        absent(context, nuclide, &corrections)
    } else {
        found(context, nuclide, matched, &corrections)
    };
    // Absent nuclides of a large library are said once, together, whatever
    // became of their detection limit.
    if matched.is_empty()
        && let Some(seconds) = corrections.bad_half_life
    {
        match &mut outcome {
            Ok(examined) => examined.bad_half_life = Some(seconds),
            Err(refusal) => refusal.bad_half_life = Some(seconds),
        }
    }
    match (outcome, words) {
        // An absent nuclide with a row says nothing of its own: the grouped
        // note does, once for all of them.
        (Ok(examined), Some(_)) if matched.is_empty() => Ok(examined),
        (Ok(mut examined), Some(words)) => {
            examined.notes.push(format!(
                "no decay correction made for {}: {words}",
                nuclide.name
            ));
            Ok(examined)
        }
        (Ok(mut examined), None) if corrections.no_half_life && !matched.is_empty() => {
            examined
                .notes
                .push(if nuclide.half_life_seconds.is_infinite() {
                    format!(
                        "not decay-corrected: the library lists {} as stable",
                        nuclide.name
                    )
                } else {
                    format!(
                        "not decay-corrected: the library gives no half-life for {}",
                        nuclide.name
                    )
                });
            Ok(examined)
        }
        (
            Err(Refusal {
                why: Why::Withheld(reason),
                reference_refused,
                bad_half_life,
            }),
            Some(words),
        ) => Err(Refusal {
            why: Why::Withheld(format!("{reason}; no decay correction was made: {words}")),
            reference_refused,
            bad_half_life,
        }),
        (outcome, _) => outcome,
    }
}

/// A nuclide with at least one matched peak.
fn found(
    context: &Context,
    nuclide: &Nuclide,
    matched: &[usize],
    corrections: &Corrections,
) -> Result<Examined, Refusal> {
    let has_efficiency = context.has_efficiency;
    let built: Vec<(LineResult, Figures, Option<String>)> = matched
        .iter()
        .map(|&index| &context.peaks[index])
        .filter_map(|peak| {
            let id = peak.identification.as_ref()?;
            let library_peak = nuclide
                .peaks
                .iter()
                .find(|p| (p.energy - id.energy).abs() < 1e-9);
            // With a curve that gives an efficiency; otherwise (count rates)
            // there is none to show. A curve that gives infinity, NaN or a
            // negative number at this line is not "no efficiency": the line is
            // dropped below, with a note saying what the curve gave.
            let given = context
                .options
                .efficiency
                .as_ref()
                .filter(|_| has_efficiency)
                .map(|curve| curve.efficiency(id.energy));
            let fault = given.and_then(|value| {
                curve_fault(value).map(|what| {
                    format!("the efficiency curve gives {what} at {:.1} keV", id.energy)
                })
            });
            let efficiency = given.filter(|_| fault.is_none());
            let figures = line_figures(
                context,
                corrections,
                &peak.info,
                efficiency.unwrap_or(0.0),
                id.yield_percent,
            );
            let line = LineResult {
                energy: id.energy,
                centroid: peak.info.centroid,
                net_area: peak.info.net_area,
                net_uncertainty: peak.info.net_area_uncertainty,
                count_rate: peak.info.net_count_rate(context.live_time),
                activity: plain_zero(figures.activity),
                activity_uncertainty: plain_zero(figures.uncertainty),
                mda: figures.mda,
                efficiency,
                yield_percent: id.yield_percent,
                used_in_average: library_peak
                    .map(|p| !p.not_in_average && p.photon.usable_for_activity())
                    .unwrap_or(true),
            };
            Some((line, figures, fault))
        })
        .collect();

    // A nuclide found where no line can carry a number used to be reported
    // detected at 0 Bq. Its peaks stay in the peak table; the activity row
    // is withheld, and the note says why. A line the curve gave a bad number
    // at is named as that, not as "zero".
    if has_efficiency
        && !built
            .iter()
            .any(|(line, _, fault)| fault.is_none() && line.measures())
    {
        let faults: Vec<&str> = built
            .iter()
            .filter_map(|(_, _, fault)| fault.as_deref())
            .collect();
        if !faults.is_empty() {
            return Err(Refusal::withheld(format!(
                "no activity: {}",
                faults.join("; ")
            )));
        }
        let energies = built
            .iter()
            .map(|(line, _, _)| format!("{:.1}", line.energy))
            .collect::<Vec<_>>()
            .join(", ");
        return Err(Refusal::withheld(format!(
            "no activity: the efficiency curve or the library yield is zero at \
             its {energies} keV line(s)"
        )));
    }
    // Whether the peak is there does not depend on which lines can be
    // quantified, so it asks every matched line.
    let detected = is_detected(
        nuclide,
        built.iter().map(|(line, _, _)| line),
        context.options.detection_sigma,
    );
    // The lines that make up the reported figure are decided once, here, and
    // the withholding, the weighted mean and the zero check all use it.
    let mean = Mean::new(has_efficiency, built.iter().map(|(line, _, _)| line));

    // A quantity that is not a measurement (too large or too small for a
    // double, or NaN) is never shown. A line that enters the reported mean and
    // has one withholds the nuclide; any other such line is dropped, with its
    // own note, and the rest report as usual.
    let mut notes: Vec<String> = Vec::new();
    let mut lines: Vec<LineResult> = Vec::with_capacity(built.len());
    let mut mda_problem: Option<(f64, Unreportable)> = None;
    for (line, figures, fault) in built {
        if let Some(fault) = fault {
            notes.push(format!(
                "line {:.1} keV of {} left out: {fault}",
                line.energy, nuclide.name
            ));
            continue;
        }
        if let Some(problem) = figures.problem {
            if mean.includes(&line) {
                return Err(Refusal::withheld_over_a_quantity(
                    format!(
                        "no activity at its {:.1} keV line: its {}; none is reported",
                        line.energy,
                        problem.words()
                    ),
                    corrections,
                ));
            }
            notes.push(format!(
                "line {:.1} keV of {} left out: its {}",
                line.energy,
                nuclide.name,
                problem.words(),
            ));
            continue;
        }
        if mda_problem.is_none()
            && let Some(why) = figures.mda_problem
        {
            mda_problem = Some((line.energy, why));
        }
        lines.push(line);
    }
    if let Some((energy, why)) = mda_problem {
        notes.push(format!(
            "no detection limit shown for {} at its {energy:.1} keV line: it is {}",
            nuclide.name,
            why.words()
        ));
    }

    // With an efficiency curve every usable line estimates the same activity,
    // so they average; without one the lines are just rates, and the honest
    // per-nuclide number is their sum.
    let (activity, mut uncertainty) = if has_efficiency {
        weighted_mean(&lines, mean)
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

    let (activity, uncertainty) = match shown_mean(&lines, mean, activity, uncertainty) {
        Ok(shown) => shown,
        Err((what, why)) => {
            return Err(Refusal::withheld_over_a_quantity(
                format!(
                    "no activity: its {what} is {}; none is reported",
                    why.words()
                ),
                corrections,
            ));
        }
    };
    let mda = lines.iter().filter_map(|line| line.mda).reduce(f64::min);
    Ok(Examined {
        result: NuclideResult {
            nuclide: nuclide.name.clone(),
            activity,
            uncertainty,
            mda,
            detected,
            half_life_seconds: nuclide.half_life_seconds,
            lines,
        },
        notes,
        reference_refused: corrections.refused,
        bad_half_life: None,
    })
}

/// A nuclide with no matched peak, for which a detection limit was asked.
fn absent(
    context: &Context,
    nuclide: &Nuclide,
    corrections: &Corrections,
) -> Result<Examined, Refusal> {
    let raw = absent_mda(context, nuclide, corrections)
        .map_err(|reason| Refusal::no_limit(reason.to_string(), corrections))?;
    match reportable(raw, false) {
        Ok(mda) => Ok(Examined {
            result: NuclideResult {
                nuclide: nuclide.name.clone(),
                activity: 0.0,
                uncertainty: 0.0,
                mda: Some(mda),
                detected: false,
                half_life_seconds: nuclide.half_life_seconds,
                lines: Vec::new(),
            },
            notes: Vec::new(),
            reference_refused: corrections.refused,
            bad_half_life: None,
        }),
        Err(why) => Err(Refusal::no_limit(
            format!("its detection limit is {}", why.words()),
            corrections,
        )),
    }
}

/// How a library half-life reads, by the library's own convention
/// (`Nuclide::decay_constant`): a decay constant exists only for a positive
/// half-life, zero means "stable or unspecified" and infinity is stable.
/// Only a negative or NaN entry is a mistake in the library. The one place
/// that decides this, for nuclides found and absent alike.
enum HalfLife {
    Stable,
    Decays,
    Invalid,
}

impl HalfLife {
    fn of(seconds: f64) -> Self {
        if seconds.is_nan() || seconds < 0.0 {
            Self::Invalid
        } else if seconds == 0.0 || seconds.is_infinite() {
            Self::Stable
        } else {
            Self::Decays
        }
    }
}

/// How the correction for one nuclide is made up.
///
/// The reference-date part is kept apart from the part for the decay during
/// the count so a figure that cannot be shown can be blamed on the right one.
struct Corrections {
    /// Decay while counting (1.0 when not asked for).
    during: f64,
    /// Decay to the reference date (1.0 when not asked for, or left out).
    reference: f64,
    /// The reference-date part was asked for and had to be left out.
    refused: bool,
    /// A decay correction was asked for and the library's half-life for the
    /// nuclide is negative or not a number: the corrections are 1.0 and the
    /// library entry is to blame, not the sample's dates.
    bad_half_life: Option<f64>,
    /// A decay correction was asked for and the library gives the nuclide no
    /// half-life (0, "stable or not given", or infinity): the corrections are
    /// 1.0, which is the figure, and a found nuclide says so once.
    no_half_life: bool,
}

impl Corrections {
    /// Past about 1024 half-lives the correction to the reference date is more
    /// than a double can hold. Applied as infinity, it turned the nuclide's
    /// activity into NaN, and the report's total with it - three days is
    /// enough for a nuclide with a three-minute half-life. Past about 1075
    /// half-lives the other way it underflows to 0.0 (or to a subnormal that
    /// has lost its digits), and every activity built from it would be a
    /// measured-looking zero. Either way the reference-date part is left out.
    fn new(spectrum: &Spectrum, options: &AnalysisOptions, half_life_seconds: f64) -> Self {
        let reference_asked = options.decay_correct_to_reference
            && options.sample.reference_date.is_some()
            && spectrum.start_time.is_some();
        match HalfLife::of(half_life_seconds) {
            HalfLife::Stable => {
                return Self {
                    during: 1.0,
                    reference: 1.0,
                    refused: false,
                    bad_half_life: None,
                    no_half_life: options.correct_decay_during_count || reference_asked,
                };
            }
            HalfLife::Invalid if options.correct_decay_during_count || reference_asked => {
                // Neither a correction nor "too many half-lives": that would
                // blame the sample for the library's entry.
                return Self {
                    during: 1.0,
                    reference: 1.0,
                    refused: false,
                    bad_half_life: Some(half_life_seconds),
                    no_half_life: false,
                };
            }
            HalfLife::Invalid | HalfLife::Decays => {}
        }
        let during = if options.correct_decay_during_count {
            let count_time = if spectrum.real_time > 0.0 {
                spectrum.real_time
            } else {
                spectrum.live_time
            };
            decay_during_acquisition(half_life_seconds, count_time)
        } else {
            1.0
        };
        let mut reference = 1.0;
        let mut refused = false;
        if options.decay_correct_to_reference
            && let (Some(date), Some(measured)) =
                (options.sample.reference_date, spectrum.start_time)
        {
            let factor = decay_to_reference(half_life_seconds, date, measured);
            // A count-time correction that is itself no number is not the
            // reference date's doing.
            if reportable(during * factor, false).is_err() && reportable(during, false).is_ok() {
                refused = true;
            } else {
                reference = factor;
            }
        }
        Self {
            during,
            reference,
            refused,
            bad_half_life: None,
            no_half_life: false,
        }
    }

    /// What to say about the library's half-life when the decay correction was
    /// asked for and could not be made.
    fn half_life_words(&self, nuclide: &str) -> Option<String> {
        self.bad_half_life
            .map(|seconds| format!("the library gives {nuclide} {}", half_life_phrase(seconds)))
    }

    fn total(&self) -> f64 {
        self.during * self.reference
    }
}

/// A half-life, or the other quantity in seconds a note quotes, as a reader
/// wants it: plain from a millisecond to a billion seconds, scientific
/// outside that (a library can hold -1e300), and NaN in words.
fn seconds_words(seconds: f64) -> String {
    if seconds.is_nan() {
        "not a number".to_string()
    } else if seconds == 0.0 || (1e-3..=1e9).contains(&seconds.abs()) {
        format!("{seconds} s")
    } else {
        format!("{seconds:e} s")
    }
}

/// "a half-life of 5 s", or "a half-life that is not a number".
fn half_life_phrase(seconds: f64) -> String {
    if seconds.is_nan() {
        "a half-life that is not a number".to_string()
    } else {
        format!("a half-life of {}", seconds_words(seconds))
    }
}

/// Why a number cannot be shown as a measurement.
#[derive(Clone, Copy, Debug, PartialEq)]
enum Unreportable {
    NotANumber,
    TooLarge,
    TooSmall,
}

impl Unreportable {
    fn words(self) -> &'static str {
        match self {
            Self::NotANumber => "not a number",
            Self::TooLarge => "too large to be a number",
            Self::TooSmall => "too small to be a number",
        }
    }
}

/// The one place that decides whether a computed quantity is a measurement.
///
/// NaN, infinity and subnormals (digits lost) never are. Zero is only when the
/// inputs were zero too (`may_be_zero`); a zero made by underflow from inputs
/// that were not is exactly the "measured 0 Bq" this refuses.
fn reportable(x: f64, may_be_zero: bool) -> Result<f64, Unreportable> {
    if x.is_nan() {
        Err(Unreportable::NotANumber)
    } else if x.is_infinite() {
        Err(Unreportable::TooLarge)
    } else if x == 0.0 && may_be_zero {
        Ok(x)
    } else if !x.is_normal() {
        Err(Unreportable::TooSmall)
    } else {
        Ok(x)
    }
}

/// What is wrong with a curve that [`EfficiencyCalibration::is_empty`] judged
/// empty, for each model it can say so of.
fn empty_curve_cause(model: &EfficiencyModel) -> &'static str {
    match model {
        EfficiencyModel::None => "the curve has no model",
        EfficiencyModel::Constant(_) => "the constant is not a positive number",
        EfficiencyModel::Interpolated => "no point has an efficiency above zero",
        EfficiencyModel::LogLogPolynomial(coefficients) if coefficients.is_empty() => {
            "the polynomial has no coefficients"
        }
        EfficiencyModel::LogLogPolynomial(_) => "a polynomial coefficient is not a finite number",
    }
}

/// Whether a live time is one an activity can be computed from. A time that
/// is NaN, infinite, zero or negative is "no live time": the one global note,
/// never a "not a number" reason for every nuclide.
fn has_live_time(live_time: f64) -> bool {
    live_time.is_finite() && live_time > 0.0
}

/// Whether a result of 0 may stand as a measurement, given the signed values
/// that made it (the members of a sum or a weighted mean).
///
/// It may when every member was zero too, and when the members were of both
/// signs and some of each were ordinary numbers: that is cancellation (a
/// source and its background, +3 and -3 counts), and a lab can see it happen.
/// It may not otherwise - one member, or members all of one sign, cannot make
/// a mean of 0 - and not when the members are all below the normal range:
/// that is the underflow of a decay correction, a number that lost its
/// digits.
fn zero_may_stand(members: impl Iterator<Item = f64>) -> bool {
    let mut seen = Members::default();
    for value in members {
        seen.add(value);
    }
    seen.zero_may_stand()
}

/// What [`zero_may_stand`] needs to know of the members of a sum or mean,
/// gathered as they go by so a sum and its zero check share one pass.
#[derive(Clone, Copy)]
struct Members {
    all_zero: bool,
    positive: bool,
    negative: bool,
}

impl Default for Members {
    fn default() -> Self {
        Self {
            all_zero: true,
            positive: false,
            negative: false,
        }
    }
}

impl Members {
    fn add(&mut self, value: f64) {
        self.all_zero &= value == 0.0;
        self.positive |= value.is_normal() && value > 0.0;
        self.negative |= value.is_normal() && value < 0.0;
    }

    fn zero_may_stand(self) -> bool {
        self.all_zero || (self.positive && self.negative)
    }
}

/// What a curve gave at a line when that is not an efficiency: infinity, NaN
/// or a negative number. Zero is not one - it is a curve that says nothing
/// there, which is "no efficiency".
fn curve_fault(value: f64) -> Option<String> {
    if value.is_nan() {
        Some("NaN".into())
    } else if value == f64::INFINITY {
        Some("infinity".into())
    } else if value == f64::NEG_INFINITY {
        Some("negative infinity".into())
    } else if value < 0.0 {
        Some(format!("a negative number ({value:e})"))
    } else {
        None
    }
}

/// Whether a line can measure an activity at all: the efficiency curve and the
/// library yield both give it something. The one definition; the live time it
/// also needs is required once, in [`examine`].
fn measures(efficiency: f64, yield_percent: f64) -> bool {
    efficiency > 0.0 && yield_percent > 0.0
}

/// Which lines make up a nuclide's reported figure: the one answer that
/// [`weighted_mean`], the decision to withhold a nuclide and the check for a
/// zero made by underflow all use.
///
/// With an efficiency curve a line takes part when it measures and, if any
/// line is in the library's average with an uncertainty, only those are used;
/// when none is, every line that measures is. Without a curve the figure is
/// the sum of all the lines.
#[derive(Clone, Copy)]
struct Mean {
    has_efficiency: bool,
    /// Some line measures, is in the average and has an uncertainty.
    weighted: bool,
}

impl Mean {
    fn new<'a>(has_efficiency: bool, lines: impl IntoIterator<Item = &'a LineResult>) -> Self {
        Self {
            has_efficiency,
            weighted: lines.into_iter().any(Self::is_weighted),
        }
    }

    fn is_weighted(line: &LineResult) -> bool {
        line.measures() && line.used_in_average && line.activity_uncertainty > 0.0
    }

    fn includes(self, line: &LineResult) -> bool {
        if !self.has_efficiency {
            return true;
        }
        if self.weighted {
            Self::is_weighted(line)
        } else {
            line.measures()
        }
    }
}

/// The nuclide's mean activity and its uncertainty, if both are measurements.
///
/// They are products of the lines' numbers and can leave the range of a double
/// though every line did not. A zero (of either sign) is a measurement when
/// every line that made it was zero too, or when the lines cancelled (see
/// [`zero_may_stand`]); otherwise it is an underflow. An uncertainty cannot
/// cancel, so its zero stands only when every line's was zero. Only the lines
/// that enter the mean count here. A negative zero is shown as zero.
fn shown_mean(
    lines: &[LineResult],
    mean: Mean,
    activity: f64,
    uncertainty: f64,
) -> Result<(f64, f64), (&'static str, Unreportable)> {
    let members = || lines.iter().filter(|line| mean.includes(line));
    let activity_zero_ok = zero_may_stand(members().map(|line| line.activity));
    let uncertainty_zero_ok = members().all(|line| line.activity_uncertainty == 0.0);
    let activity = reportable(activity, activity_zero_ok).map_err(|why| ("activity", why))?;
    let uncertainty =
        reportable(uncertainty, uncertainty_zero_ok).map_err(|why| ("uncertainty", why))?;
    Ok((plain_zero(activity), plain_zero(uncertainty)))
}

/// Which correction made a line's figure unreportable.
#[derive(Clone, Copy, Debug, PartialEq)]
enum Blame {
    /// Neither: the figure is no number without any correction.
    None,
    /// The correction to the reference date.
    Reference,
    /// The correction for the decay during the count.
    Count,
    /// Each breaks the figure when applied alone.
    Each,
    /// Each is fine alone; together they are not.
    Both,
}

/// The quantity of a line that cannot be shown, and whether a decay
/// correction is to blame.
#[derive(Clone, Copy, Debug)]
struct Problem {
    what: &'static str,
    why: Unreportable,
    blame: Blame,
}

impl Problem {
    fn words(self) -> String {
        let mut words = format!("{} is {}", self.what, self.why.words());
        words.push_str(match self.blame {
            Blame::None => "",
            Blame::Reference => {
                " because of the decay correction to the reference date; \
                 check the sample's reference date"
            }
            Blame::Count => " because of the decay correction for the count",
            Blame::Each => {
                " because the decay during the count and the decay to the reference date \
                 each break it; check the sample's reference date"
            }
            Blame::Both => {
                " because of the decay corrections together; check the sample's reference date"
            }
        });
        words
    }
}

/// One line's activity, uncertainty and detection limit, and what is wrong
/// with them if they are not numbers.
struct Figures {
    activity: f64,
    uncertainty: f64,
    /// `None` when the line has no detection limit, or it is not a number.
    mda: Option<f64>,
    problem: Option<Problem>,
    /// The detection limit alone is not a number.
    mda_problem: Option<Unreportable>,
}

/// The first of an activity and its uncertainty that is not a measurement.
fn first_problem(
    activity: f64,
    activity_zero_ok: bool,
    uncertainty: f64,
    uncertainty_zero_ok: bool,
) -> Option<(&'static str, Unreportable)> {
    reportable(activity, activity_zero_ok)
        .err()
        .map(|why| ("activity", why))
        .or_else(|| {
            reportable(uncertainty, uncertainty_zero_ok)
                .err()
                .map(|why| ("uncertainty", why))
        })
}

/// The Currie detection limit for a line, the one place its units are decided.
///
/// With an efficiency curve it is an activity: at this line's efficiency and
/// yield, decay-corrected, per unit of sample. Without one the report is in
/// count rates, so the limit is the plain Currie limit per second of live
/// time: no decay correction, no division by the sample quantity, and a
/// correction that overflows never touches it. Both the lines of a nuclide
/// that was found and the limit of one that was not come through here.
fn detection_limit(
    context: &Context,
    corrections: &Corrections,
    background: f64,
    efficiency: f64,
    yield_fraction: f64,
) -> f64 {
    if !context.has_efficiency {
        mda_currie(background, 1.0, 1.0, context.live_time)
    } else {
        mda_currie(background, efficiency, yield_fraction, context.live_time) * corrections.total()
            / context.quantity
    }
}

fn line_figures(
    context: &Context,
    corrections: &Corrections,
    info: &PeakInfo,
    efficiency: f64,
    yield_percent: f64,
) -> Figures {
    let live_time = context.live_time;
    let measuring = measures(efficiency, yield_percent);
    let yield_fraction = yield_percent / 100.0;
    let correction = corrections.total();
    let mda_raw = (!context.has_efficiency || measuring).then(|| {
        detection_limit(
            context,
            corrections,
            info.background,
            efficiency,
            yield_fraction,
        )
    });
    let area_zero = info.net_area == 0.0;
    let uncertainty_zero = info.net_area_uncertainty == 0.0;
    let (activity, uncertainty, problem) = if !context.has_efficiency {
        // Without an efficiency curve there is no becquerel to report, so the
        // "activity" columns fall back to the count rate - the report's
        // activity unit already says "cps" in that case, and an all-zero table
        // would contradict it. That rate does not use the decay correction.
        let (activity, uncertainty) = (
            info.net_count_rate(live_time),
            info.net_area_uncertainty / live_time,
        );
        let problem =
            first_problem(activity, area_zero, uncertainty, uncertainty_zero).map(|(what, why)| {
                Problem {
                    what,
                    why,
                    blame: Blame::None,
                }
            });
        (activity, uncertainty, problem)
    } else if measuring {
        let denominator = live_time * efficiency * yield_fraction * context.quantity;
        let at = |correction: f64| {
            let scale = correction / denominator;
            (info.net_area * scale, info.net_area_uncertainty * scale)
        };
        let bad = |correction: f64| {
            let (activity, uncertainty) = at(correction);
            first_problem(activity, area_zero, uncertainty, uncertainty_zero).is_some()
        };
        let (activity, uncertainty) = at(correction);
        let problem =
            first_problem(activity, area_zero, uncertainty, uncertainty_zero).map(|(what, why)| {
                // The same figure with each part of the correction taken away
                // says which one broke it.
                let blame = if bad(1.0) {
                    Blame::None
                } else {
                    let count = corrections.during != 1.0 && bad(corrections.during);
                    let reference = corrections.reference != 1.0 && bad(corrections.reference);
                    match (count, reference) {
                        (true, false) => Blame::Count,
                        (false, true) => Blame::Reference,
                        (true, true) => Blame::Each,
                        (false, false) => Blame::Both,
                    }
                };
                Problem { what, why, blame }
            });
        (activity, uncertainty, problem)
    } else {
        // A line that said nothing has activity 0 on purpose.
        (0.0, 0.0, None)
    };
    let (mda, mda_problem) = match mda_raw.map(|raw| reportable(raw, false)) {
        None => (None, None),
        Some(Ok(mda)) => (Some(mda), None),
        Some(Err(why)) => (None, Some(why)),
    };
    Figures {
        activity,
        uncertainty,
        mda,
        problem,
        mda_problem,
    }
}

/// The note for nuclides whose reference-date correction was left out.
fn beyond_reference_note(names: &[String]) -> String {
    let listed = name_list(names.iter().map(String::as_str), 10);
    format!(
        "decay correction to the reference date left out for {listed}: too many half-lives \
         lie between that date and the count for the correction to be computed, so any of \
         these that is reported is at the time of the count"
    )
}

/// The one note for library nuclides that were not found and whose half-life
/// is negative or not a number: how many, and the first few names, each with
/// the value the library gives it.
fn bad_half_lives_note(entries: &[(String, f64)]) -> String {
    let words: Vec<String> = entries
        .iter()
        .map(|(name, seconds)| format!("{name}: {}", seconds_words(*seconds)))
        .collect();
    let listed = name_list(words.iter().map(String::as_str), 3);
    format!(
        "no decay correction made for {} library nuclide{} not found: the library gives {} a \
         half-life that is negative or not a number ({listed})",
        thousands(entries.len()),
        if entries.len() == 1 { "" } else { "s" },
        if entries.len() == 1 { "it" } else { "each" },
    )
}

/// The weighted mean of the lines `mean` takes, which the caller built once
/// from the nuclide's lines.
fn weighted_mean(lines: &[LineResult], mean: Mean) -> (f64, f64) {
    let usable: Vec<&LineResult> = lines.iter().filter(|line| mean.includes(line)).collect();
    if !mean.weighted {
        // Only lines that measured something: a line past the efficiency
        // curve, or with no yield, has activity 0 because it said nothing.
        // The caller has checked that one of them measures; were it none, the
        // 0/0 is NaN and `analyse` refuses it.
        let mean = usable.iter().map(|line| line.activity).sum::<f64>() / usable.len() as f64;
        let spread = usable
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

fn is_detected<'a>(
    nuclide: &Nuclide,
    lines: impl Iterator<Item = &'a LineResult> + Clone,
    detection_sigma: f64,
) -> bool {
    let present = |energy: f64| {
        lines.clone().any(|line| {
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

/// The one note for the library nuclides that were not found and whose
/// detection limit could not be built: how many, grouped by reason, with the
/// first few names of each.
fn no_limit_note(gaps: &[(String, String)]) -> Option<String> {
    const NAMED: usize = 3;
    let mut reasons: Vec<&str> = Vec::new();
    for (_, reason) in gaps {
        if !reasons.contains(&reason.as_str()) {
            reasons.push(reason);
        }
    }
    let parts: Vec<String> = reasons
        .iter()
        .map(|reason| {
            let names: Vec<&str> = gaps
                .iter()
                .filter(|(_, why)| why == *reason)
                .map(|(name, _)| name.as_str())
                .collect();
            let listed = name_list(names.iter().copied(), NAMED);
            format!(
                "no detection limit for {} library nuclide{}: {reason} ({listed})",
                thousands(names.len()),
                if names.len() == 1 { "" } else { "s" },
            )
        })
        .collect();
    (!parts.is_empty()).then(|| parts.join("; "))
}

/// A count with a comma between the thousands: 1,284.
fn thousands(n: usize) -> String {
    let digits = n.to_string();
    let mut out = String::with_capacity(digits.len() + digits.len() / 3);
    for (i, c) in digits.chars().enumerate() {
        if i > 0 && (digits.len() - i).is_multiple_of(3) {
            out.push(',');
        }
        out.push(c);
    }
    out
}

/// The detection limit, as computed and not yet checked, of a nuclide that was
/// not found.
///
/// The error is the reason none can be built, worded for the report's notes:
/// the caller asked for this row, so its absence must be explained. The
/// reasons are the same words for every nuclide, so that the thousands a
/// large library can produce group into a handful of lines.
fn absent_mda(
    context: &Context,
    nuclide: &Nuclide,
    corrections: &Corrections,
) -> Result<f64, &'static str> {
    let spectrum = context.spectrum;
    let calibration = spectrum
        .energy_calibration
        .as_ref()
        .ok_or("the spectrum has no energy calibration")?;
    // Without an efficiency curve the report is in count-rate units, and the
    // detection limit follows suit rather than disappearing.
    let efficiency = context
        .options
        .efficiency
        .as_ref()
        .filter(|curve| !curve.is_empty());
    let reference_line = nuclide
        .peaks
        .iter()
        .find(|peak| peak.key_line)
        .or_else(|| nuclide.peaks.first())
        .ok_or("the library holds no line to use for the limit")?;
    let channel = calibration
        .channel(reference_line.energy)
        .ok_or("the calibration gives no channel for the line used for the limit")?;
    if channel < 0.0 || channel >= spectrum.len() as f64 {
        return Err("the line used for the limit falls outside the spectrum");
    }
    let fwhm = spectrum.fwhm_at(channel).unwrap_or(4.0);
    let half = (1.5 * fwhm).max(2.0);
    let low = (channel - half).max(0.0) as usize;
    let high = ((channel + half) as usize).min(spectrum.len() - 1);
    let background = spectrum.sum_range(low, high) as f64;
    // With a curve the MDA is in activity units at this line's efficiency and
    // yield; without one it is the smallest detectable count rate.
    let (curve_efficiency, yield_percent) = match efficiency {
        Some(curve) => (
            curve.efficiency(reference_line.energy),
            reference_line.yield_percent,
        ),
        None => (1.0, 100.0),
    };
    if curve_fault(curve_efficiency).is_some() {
        return Err("the efficiency curve gives no usable number at the line used for the limit");
    }
    if !measures(curve_efficiency, yield_percent) {
        return Err("the efficiency curve gives nothing at the line used for the limit");
    }
    Ok(detection_limit(
        context,
        corrections,
        background,
        curve_efficiency,
        yield_percent / 100.0,
    ))
}

#[cfg(test)]
mod tests {
    use super::*;

    fn line(activity: f64, uncertainty: f64, used_in_average: bool) -> LineResult {
        LineResult {
            energy: 100.0,
            centroid: 100.0,
            net_area: 1.0,
            net_uncertainty: 1.0,
            count_rate: 1.0,
            activity,
            activity_uncertainty: uncertainty,
            mda: None,
            efficiency: Some(0.05),
            yield_percent: 50.0,
            used_in_average,
        }
    }

    #[test]
    fn a_zero_that_no_cancellation_made_is_refused_whatever_its_sign() {
        // One line of -3 cannot make a mean of 0: that is an underflow.
        let single = [line(-3.0, 1.0, true)];
        let mean = Mean::new(true, &single);
        for zero in [0.0, -0.0] {
            let shown = shown_mean(&single, mean, zero, 1.0);
            assert_eq!(shown, Err(("activity", Unreportable::TooSmall)));
        }
        // Every line a subnormal: whatever they made, a 0 is a number that
        // lost its digits, not a measurement, even with both signs present.
        let tiny = f64::MIN_POSITIVE / 4.0;
        let mixed = [line(tiny, 1.0, true), line(-tiny, 1.0, true)];
        let mean = Mean::new(true, &mixed);
        assert!(shown_mean(&mixed, mean, 0.0, 1.0).is_err());
        let negative = [line(-tiny, 1.0, true)];
        let mean = Mean::new(true, &negative);
        for zero in [0.0, -0.0] {
            let shown = shown_mean(&negative, mean, zero, 1.0);
            assert_eq!(shown, Err(("activity", Unreportable::TooSmall)));
        }
        let positive = [line(tiny, 1.0, true), line(tiny, 1.0, true)];
        let mean = Mean::new(true, &positive);
        assert!(shown_mean(&positive, mean, 0.0, 1.0).is_err());
        // An uncertainty is a root of squares: it cannot cancel, so a 0 made
        // from lines that had one is always an underflow.
        let normal = [line(3.0, 1.0, true)];
        let mean = Mean::new(true, &normal);
        let spread = shown_mean(&normal, mean, 3.0, 0.0);
        assert_eq!(spread, Err(("uncertainty", Unreportable::TooSmall)));
    }

    #[test]
    fn lines_that_cancel_exactly_are_a_measured_zero() {
        // +3 and -3 counts per second in count-rate mode: the sum is 0 cps.
        let lines = [line(3.0, 1.0, true), line(-3.0, 1.0, true)];
        let mean = Mean::new(false, &lines);
        let sum: f64 = lines.iter().map(|l| l.activity).sum();
        let (activity, _) = shown_mean(&lines, mean, sum, 1.4).unwrap();
        assert_eq!(activity.to_bits(), 0.0f64.to_bits());
        // Equal uncertainties, opposite activities: the weighted mean is 0.
        let mean = Mean::new(true, &lines);
        let (value, spread) = weighted_mean(&lines, mean);
        let (activity, _) = shown_mean(&lines, mean, value, spread).unwrap();
        assert_eq!(activity.to_bits(), 0.0f64.to_bits());
        // A negative zero from the same arithmetic is shown as plain zero.
        let (activity, _) = shown_mean(&lines, mean, -0.0, 1.0).unwrap();
        assert_eq!(activity.to_bits(), 0.0f64.to_bits());
    }

    #[test]
    fn a_detected_total_that_cancels_is_zero_and_one_below_the_normal_range_is_not() {
        fn nuclide(activity: f64) -> NuclideResult {
            NuclideResult {
                nuclide: "X-1".into(),
                activity,
                uncertainty: 1.0,
                mda: None,
                detected: true,
                half_life_seconds: 1.0,
                lines: Vec::new(),
            }
        }
        fn report(activities: &[f64]) -> QuantitativeReport {
            QuantitativeReport {
                peaks: Vec::new(),
                nuclides: activities.iter().map(|a| nuclide(*a)).collect(),
                unidentified: Vec::new(),
                has_efficiency: true,
                sample_unit: String::new(),
                live_time: 1.0,
                real_time: 1.0,
                notes: Vec::new(),
                withheld: Vec::new(),
            }
        }
        let total = report(&[5.0, -5.0]).total_activity();
        assert_eq!(total.map(f64::to_bits), Some(0.0f64.to_bits()));
        let tiny = f64::MIN_POSITIVE / 4.0;
        assert_eq!(report(&[tiny, -tiny]).total_activity(), None);
        assert_eq!(report(&[0.0, 0.0]).total_activity(), Some(0.0));
    }

    #[test]
    fn a_mean_of_lines_that_were_zero_is_zero_and_never_negative_zero() {
        let lines = [line(-0.0, 0.0, true), line(0.0, 0.0, true)];
        let mean = Mean::new(true, &lines);
        let (activity, uncertainty) = shown_mean(&lines, mean, -0.0, -0.0).unwrap();
        assert_eq!(activity.to_bits(), 0.0f64.to_bits());
        assert_eq!(uncertainty.to_bits(), 0.0f64.to_bits());
    }

    #[test]
    fn a_line_outside_the_mean_cannot_make_a_zero_mean_acceptable() {
        // The first line is the mean; the second is not in the library's
        // average. Its zero says nothing about the mean of the first.
        let lines = [line(2.0, 0.5, true), line(0.0, 1.0, false)];
        let mean = Mean::new(true, &lines);
        assert!(mean.includes(&lines[0]) && !mean.includes(&lines[1]));
        assert!(shown_mean(&lines, mean, 0.0, 0.5).is_err());
    }

    #[test]
    fn the_mean_takes_the_lines_the_weighted_mean_takes() {
        let mut idle = line(0.0, 0.0, true);
        idle.efficiency = Some(0.0);
        let lines = [
            line(10.0, 1.0, true),
            line(100.0, 1.0, false),
            line(50.0, 0.0, true),
            idle,
        ];
        let mean = Mean::new(true, &lines);
        let taken: Vec<bool> = lines.iter().map(|l| mean.includes(l)).collect();
        assert_eq!(taken, [true, false, false, false]);
        assert_eq!(weighted_mean(&lines, mean).0, 10.0);
        // With none in the average the lines that measure are averaged.
        let lines = [
            line(10.0, 1.0, false),
            line(30.0, 1.0, false),
            lines[3].clone(),
        ];
        let mean = Mean::new(true, &lines);
        let taken: Vec<bool> = lines.iter().map(|l| mean.includes(l)).collect();
        assert_eq!(taken, [true, true, false]);
        assert_eq!(weighted_mean(&lines, mean).0, 20.0);
        // Without a curve every line is part of the sum.
        let mean = Mean::new(false, &lines);
        assert!(lines.iter().all(|l| mean.includes(l)));
    }
}
