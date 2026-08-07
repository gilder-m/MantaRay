//! Reports: ROI reports in MAESTRO's two layouts, spectrum printouts, and the
//! quantitative reports.
//!
//! The ROI report reproduces the fields and order of MAESTRO §4.1.7 (Figs. 31
//! and 32): a header naming the detector, the acquisition time and the real and
//! live times, then one block or row per region with its range, areas, centroid,
//! shape, library identification and corrected rate.

#![forbid(unsafe_code)]

use mantaray_core::{
    CalculationSettings, EnergyCalibration, NuclideLibrary, PeakInfo, QuantitativeReport, Roi,
    Spectrum, peak_info,
};

/// Which ROI report layout to produce.
#[derive(Clone, Copy, Debug, Default, PartialEq, Eq)]
pub enum ReportFormat {
    /// One indented block per region (Fig. 31).
    #[default]
    Paragraph,
    /// One row per region under a column header (Fig. 32).
    Column,
}

/// How to build a report.
#[derive(Clone, Debug)]
pub struct ReportOptions<'a> {
    /// Layout.
    pub format: ReportFormat,
    /// Peak-info settings.
    pub settings: CalculationSettings,
    /// Library used to identify peaks.
    pub library: Option<&'a NuclideLibrary>,
    /// How far a centroid may sit from a library line, in keV.
    pub match_tolerance: f64,
    /// Only report regions inside this channel range (the expanded view).
    pub displayed: Option<(usize, usize)>,
}

impl Default for ReportOptions<'_> {
    fn default() -> Self {
        Self {
            format: ReportFormat::Paragraph,
            settings: CalculationSettings::default(),
            library: None,
            match_tolerance: 2.0,
            displayed: None,
        }
    }
}

impl<'a> ReportOptions<'a> {
    /// Paragraph layout with default settings.
    pub fn paragraph() -> Self {
        Self::default()
    }

    /// Column layout with default settings.
    pub fn column() -> Self {
        Self {
            format: ReportFormat::Column,
            ..Self::default()
        }
    }

    /// Identifies peaks against a library.
    pub fn with_library(mut self, library: &'a NuclideLibrary) -> Self {
        self.library = Some(library);
        self
    }

    /// Restricts the report to the regions inside a channel range.
    pub fn displayed(mut self, low: usize, high: usize) -> Self {
        self.displayed = Some((low.min(high), low.max(high)));
        self
    }

    /// Uses specific calculation settings.
    pub fn with_settings(mut self, settings: CalculationSettings) -> Self {
        self.settings = settings;
        self
    }
}

/// One region as the report sees it.
struct Row {
    index: usize,
    roi: Roi,
    info: PeakInfo,
    identification: Option<Identified>,
}

struct Identified {
    nuclide: String,
    energy: f64,
    activity: f64,
    uncertainty: f64,
}

/// Builds an ROI report.
pub fn roi_report(spectrum: &Spectrum, options: &ReportOptions<'_>) -> String {
    let mut out = header(spectrum);
    let rows = rows(spectrum, options);
    if rows.is_empty() {
        out.push_str("\nNo regions of interest are marked.\n");
        return out;
    }
    match options.format {
        ReportFormat::Paragraph => paragraph_body(spectrum, &rows, &mut out),
        ReportFormat::Column => column_body(spectrum, &rows, &mut out),
    }
    out
}

fn rows(spectrum: &Spectrum, options: &ReportOptions<'_>) -> Vec<Row> {
    let calibration = spectrum.energy_calibration.clone();
    let mut rows = Vec::new();
    for roi in spectrum.rois.iter() {
        if let Some((low, high)) = options.displayed
            && (roi.end < low || roi.start > high)
        {
            continue;
        }
        let Ok(info) = peak_info(spectrum, *roi, &options.settings) else {
            continue;
        };
        let identification = match (&calibration, options.library) {
            (Some(calibration), Some(library)) => {
                let energy = info.centroid_energy(calibration);
                library
                    .best_match(energy, options.match_tolerance)
                    .map(|matched| Identified {
                        nuclide: matched.nuclide.name.clone(),
                        energy: matched.peak.energy,
                        activity: info.activity(matched.peak.yield_percent, spectrum.live_time),
                        uncertainty: info
                            .activity_uncertainty(matched.peak.yield_percent, spectrum.live_time),
                    })
            }
            _ => None,
        };
        rows.push(Row {
            index: rows.len() + 1,
            roi: *roi,
            info,
            identification,
        });
    }
    rows
}

fn header(spectrum: &Spectrum) -> String {
    let acquired = match spectrum.start_time {
        Some(start) => format!(
            "ACQ {} at {}",
            start.format("%d-%b-%Y"),
            start.format("%H:%M:%S")
        ),
        None => "ACQ (not recorded)".to_string(),
    };
    let mut out = format!(
        "Detector #{:<4} {}   RT = {:.1}   LT = {:.1}\n",
        spectrum.detector_id, acquired, spectrum.real_time, spectrum.live_time
    );
    if !spectrum.detector_description.trim().is_empty() {
        out.push_str(&format!(
            "            {}\n",
            spectrum.detector_description.trim()
        ));
    }
    if !spectrum.sample_description.trim().is_empty() {
        out.push_str(&format!(
            "            {}\n",
            spectrum.sample_description.trim()
        ));
    }
    out
}

fn paragraph_body(spectrum: &Spectrum, rows: &[Row], out: &mut String) {
    let calibration = spectrum.energy_calibration.as_ref();
    for row in rows {
        out.push('\n');
        out.push_str(&format!("ROI # {}\n", row.index));
        match calibration {
            Some(calibration) => {
                out.push_str(&format!(
                    "     RANGE: {} = {:.2}keV to {} = {:.2}keV\n",
                    row.roi.start,
                    calibration.energy(row.roi.start as f64),
                    row.roi.end,
                    calibration.energy(row.roi.end as f64)
                ));
            }
            None => {
                out.push_str(&format!(
                    "     RANGE: {} to {}\n",
                    row.roi.start, row.roi.end
                ));
            }
        }
        out.push_str(&format!(
            "     AREA : Gross = {:.0}    Net = {:.0}   +/- {:.0}\n",
            row.info.gross_area, row.info.net_area, row.info.net_area_uncertainty
        ));
        match calibration {
            Some(calibration) => out.push_str(&format!(
                "     CENTROID: {:.2}  = {:.2}keV\n",
                row.info.centroid,
                row.info.centroid_energy(calibration)
            )),
            None => out.push_str(&format!("     CENTROID: {:.2}\n", row.info.centroid)),
        }
        out.push_str(&format!(
            "     SHAPE: FWHM = {:.2}    Fw(1/{})M = {:.2}\n",
            row.info.fwhm, row.info.fw_x, row.info.fw_x_m
        ));
        if let Some(identified) = &row.identification {
            out.push_str(&format!(
                "     ID: {} at {:.2}keV\n",
                identified.nuclide, identified.energy
            ));
            out.push_str(&format!(
                "     Corrected Rate = {:.2} +/- {:.2} cA\n",
                identified.activity, identified.uncertainty
            ));
        }
    }
}

fn column_body(spectrum: &Spectrum, rows: &[Row], out: &mut String) {
    let calibration = spectrum.energy_calibration.as_ref();
    match calibration {
        Some(_) => out.push_str(
            "ROI#   RANGE( keV)        GROSS       NET   +/-  CENTROID  FWHM FW(1/5)M  LIBRARY ( keV)          cA      +/-\n",
        ),
        None => out.push_str(
            "ROI#  RANGE( ch)          GROSS       NET   +/-  CENTROID  FWHM FW(1/5)M\n",
        ),
    }
    for row in rows {
        match calibration {
            Some(calibration) => {
                out.push_str(&format!(
                    "{:>3} {:>8.2} {:>8.2} {:>10.0} {:>9.0} {:>5.0} {:>9.2} {:>5.2} {:>7.2}",
                    row.index,
                    calibration.energy(row.roi.start as f64),
                    calibration.energy(row.roi.end as f64),
                    row.info.gross_area,
                    row.info.net_area,
                    row.info.net_area_uncertainty,
                    row.info.centroid_energy(calibration),
                    row.info.fwhm,
                    row.info.fw_x_m,
                ));
                match &row.identification {
                    Some(identified) => out.push_str(&format!(
                        "  {:<8} {:>8.2} {:>11.2} {:>8.2}\n",
                        identified.nuclide,
                        identified.energy,
                        identified.activity,
                        identified.uncertainty
                    )),
                    None => out.push('\n'),
                }
            }
            None => out.push_str(&format!(
                "{:>3} {:>8} {:>8} {:>10.0} {:>9.0} {:>5.0} {:>9.2} {:>5.2} {:>7.2}\n",
                row.index,
                row.roi.start,
                row.roi.end,
                row.info.gross_area,
                row.info.net_area,
                row.info.net_area_uncertainty,
                row.info.centroid,
                row.info.fwhm,
                row.info.fw_x_m,
            )),
        }
    }
}

/// Formats channel contents for printing, seven channels a line (§4.1.6).
pub fn print_spectrum(spectrum: &Spectrum, range: Option<(usize, usize)>) -> String {
    const COLUMNS: usize = 7;
    let (low, high) = match range {
        Some((low, high)) => (
            low.min(high).min(spectrum.len().saturating_sub(1)),
            high.max(low).min(spectrum.len().saturating_sub(1)),
        ),
        None => (0, spectrum.len().saturating_sub(1)),
    };
    let mut out = header(spectrum);
    if spectrum.is_empty() {
        return out;
    }
    out.push('\n');
    let mut channel = low;
    while channel <= high {
        let last = (channel + COLUMNS - 1).min(high);
        out.push_str(&format!("{channel:>6}"));
        for current in channel..=last {
            out.push_str(&format!(" {:>9}", spectrum.counts(current)));
        }
        out.push('\n');
        channel = last + 1;
    }
    out
}

/// Formats a quantitative analysis as a nuclide report.
pub fn nuclide_report(analysis: &QuantitativeReport) -> String {
    let unit = analysis.activity_unit();
    let mut out = String::new();
    out.push_str(&format!(
        "Nuclide report    LT = {:.1} s   RT = {:.1} s   activity in {unit}\n",
        analysis.live_time, analysis.real_time
    ));
    out.push_str(&format!(
        "{:<10} {:>12} {:>12} {:>12}  {:<12} {}\n",
        "NUCLIDE", "ACTIVITY", "+/-", "MDA", "STATUS", "LINES (keV)"
    ));
    for nuclide in &analysis.nuclides {
        let lines: Vec<String> = nuclide
            .lines
            .iter()
            .map(|line| format!("{:.2}", line.energy))
            .collect();
        out.push_str(&format!(
            "{:<10} {:>12.3} {:>12.3} {:>12.3}  {:<12} {}\n",
            nuclide.nuclide,
            nuclide.activity,
            nuclide.uncertainty,
            nuclide.mda,
            if nuclide.detected {
                "detected"
            } else {
                "not detected"
            },
            lines.join(" ")
        ));
    }
    if !analysis.unidentified.is_empty() {
        out.push_str(&format!(
            "\n{} region(s) matched no library line:\n",
            analysis.unidentified.len()
        ));
        for info in &analysis.unidentified {
            out.push_str(&format!(
                "     channels {}-{}, centroid {:.2}, net {:.0} +/- {:.0}\n",
                info.roi.start,
                info.roi.end,
                info.centroid,
                info.net_area,
                info.net_area_uncertainty
            ));
        }
    }
    let total = analysis.total_activity();
    if total > 0.0 {
        out.push_str(&format!("\nTotal detected activity: {total:.3} {unit}\n"));
    }
    if !analysis.notes.is_empty() {
        out.push('\n');
        for note in &analysis.notes {
            out.push_str(&format!("Note: {note}\n"));
        }
    }
    out
}

/// Quotes a value for a CSV cell when it holds anything that would break the
/// row - names come from user-written library files and may contain commas.
fn csv_field(value: &str) -> String {
    if value.contains([',', '"', '\n', '\r']) {
        format!("\"{}\"", value.replace('"', "\"\""))
    } else {
        value.to_string()
    }
}

/// Exports the per-nuclide results as CSV.
pub fn results_csv(analysis: &QuantitativeReport) -> String {
    let mut out = String::from(
        "nuclide,activity,uncertainty,mda,unit,detected,energy_kev,net_area,net_uncertainty,count_rate,efficiency,yield_percent\n",
    );
    let unit = csv_field(&analysis.activity_unit());
    for nuclide in &analysis.nuclides {
        let name = csv_field(&nuclide.nuclide);
        if nuclide.lines.is_empty() {
            out.push_str(&format!(
                "{name},{:.6},{:.6},{:.6},{unit},{},,,,,,\n",
                nuclide.activity, nuclide.uncertainty, nuclide.mda, nuclide.detected
            ));
            continue;
        }
        for line in &nuclide.lines {
            out.push_str(&format!(
                "{name},{:.6},{:.6},{:.6},{unit},{},{:.3},{:.1},{:.1},{:.4},{:.6},{:.4}\n",
                nuclide.activity,
                nuclide.uncertainty,
                nuclide.mda,
                nuclide.detected,
                line.energy,
                line.net_area,
                line.net_uncertainty,
                line.count_rate,
                line.efficiency,
                line.yield_percent,
            ));
        }
    }
    out
}

/// Exports channel data as CSV, one row per channel.
pub fn spectrum_csv(spectrum: &Spectrum) -> String {
    let mut out = String::from("channel,energy,counts\n");
    let calibration = spectrum.energy_calibration.clone();
    for (channel, counts) in spectrum.channels.iter().enumerate() {
        match &calibration {
            Some(calibration) => out.push_str(&format!(
                "{channel},{:.4},{counts}\n",
                calibration.energy(channel as f64)
            )),
            None => out.push_str(&format!("{channel},,{counts}\n")),
        }
    }
    out
}

/// A compact one-line summary, for logs and the status bar.
pub fn summary(spectrum: &Spectrum) -> String {
    let calibration: Option<&EnergyCalibration> = spectrum.energy_calibration.as_ref();
    format!(
        "{} channels, {} counts, LT {:.1} s, RT {:.1} s, {}",
        spectrum.len(),
        spectrum.total_counts(),
        spectrum.live_time,
        spectrum.real_time,
        match calibration {
            Some(calibration) => format!(
                "{:.4} {}/ch",
                calibration.coefficients[1], calibration.units
            ),
            None => "uncalibrated".to_string(),
        }
    )
}
