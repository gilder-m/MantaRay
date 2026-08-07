//! Menus, toolbar, status sidebar and every dialog.

use std::path::PathBuf;

use ortseam_core::{
    CalculationSettings, EfficiencyCalibration, EfficiencyPoint, LibraryPeak, Nuclide, QaStatus,
};
use ortseam_device::{
    DetectorEntry, DetectorKind, JobState, Mcb, MdaPreset, Presets, SampleJob, UncertaintyPreset,
    tile,
};

use crate::app::{Action, App, Target};
use crate::theme::{Rgb, Theme};
use crate::view::{MarkMode, format_counts, format_rate};
use crate::viewmodel::{FillMode, VerticalScale};

/// Every dialog the application can show.
#[derive(Clone, Copy, Debug, PartialEq, Eq, Hash)]
pub enum Dialog {
    /// Acquire/MCB Properties.
    McbProperties,
    /// Calculate/Settings.
    Settings,
    /// Calculate/Calibration.
    Calibration,
    /// File/ROI Report options.
    RoiReportOptions,
    /// The report viewer.
    Report,
    /// Nuclide analysis.
    Analysis,
    /// Efficiency calibration.
    Efficiency,
    /// Services/Library.
    Library,
    /// Services/Job Control.
    JobControl,
    /// Services/Edit Detector List.
    DetectorList,
    /// Finding and numbering the instruments on this machine.
    Instruments,
    /// Services/Sample Description.
    Sample,
    /// Services/Lock-Unlock Detector.
    Lock,
    /// Calculate/Strip.
    Strip,
    /// Calculate/List Data Range.
    ListRange,
    /// Display/Preferences.
    Preferences,
    /// The multi-detector dashboard.
    Dashboard,
    /// Quality-assurance charts.
    Qa,
    /// The batch queue.
    Batch,
    /// Acquire/InSight: the virtual oscilloscope.
    Insight,
    /// Help/Keyboard Commands.
    Shortcuts,
    /// About.
    About,
    /// Confirm exit.
    Exit,
    /// Save, discard or keep a modified buffer whose window is being closed.
    ConfirmClose,
    /// Offer back the work a run that did not finish left behind.
    Recover,
    /// Type a path, when native file dialogs are not available.
    OpenPath,
}

impl Dialog {
    /// Every dialog, so a test can open each one in turn.
    pub fn all() -> &'static [Dialog] {
        &[
            Dialog::McbProperties,
            Dialog::Settings,
            Dialog::Calibration,
            Dialog::RoiReportOptions,
            Dialog::Report,
            Dialog::Analysis,
            Dialog::Efficiency,
            Dialog::Library,
            Dialog::JobControl,
            Dialog::DetectorList,
            Dialog::Instruments,
            Dialog::Sample,
            Dialog::Lock,
            Dialog::Strip,
            Dialog::ListRange,
            Dialog::Preferences,
            Dialog::Dashboard,
            Dialog::Qa,
            Dialog::Batch,
            Dialog::Insight,
            Dialog::Shortcuts,
            Dialog::About,
            Dialog::Exit,
            Dialog::ConfirmClose,
            Dialog::OpenPath,
        ]
    }
}

/// Which dialogs are open, and their scratch state.
pub struct Dialogs {
    open: std::collections::HashSet<Dialog>,
    /// Selected tab of the MCB Properties dialog.
    pub properties_tab: usize,
    /// Energy being entered in the Calibration dialog.
    pub calibration_energy: String,
    /// Units being entered in the Calibration dialog.
    pub calibration_units: String,
    /// Whether the ROI report uses the column layout.
    pub report_columns: bool,
    /// Whether the ROI report covers only the displayed region.
    pub report_displayed_only: bool,
    /// Whether analysis reports detection limits for absent nuclides.
    pub report_mda: bool,
    /// Constant efficiency being entered.
    pub efficiency_constant: f64,
    /// Efficiency points being entered, as text.
    pub efficiency_points: Vec<(String, String)>,
    /// Polynomial order for the efficiency fit.
    pub efficiency_order: usize,
    /// Path typed into the fallback open dialog.
    pub path_input: String,
    /// Energy typed into the toolbar's go-to box.
    pub goto_input: String,
    /// Nuclide chosen for calibrating from peaks.
    pub auto_nuclide: String,
    /// Whether activities are corrected for decay during the count.
    pub correct_during_count: bool,
    /// Whether activities are decay-corrected to the sample's reference date.
    pub correct_to_reference: bool,
    /// The reference date as typed, before parsing.
    pub reference_input: String,
    /// Nuclide of the certified efficiency source.
    pub source_nuclide: String,
    /// Certified activity of the source, in becquerel.
    pub source_activity_bq: f64,
    /// One-sigma uncertainty of the certificate, in percent.
    pub source_uncertainty_pct: f64,
    /// Days between the certificate date and the measurement.
    pub source_age_days: f64,
    /// Name typed for a new network instrument.
    pub network_name: String,
    /// Name for a local instrument reached through the bridge.
    pub local_name: String,
    /// The detector number ORTEC's configuration gave it.
    pub local_detector: i32,
    /// Where ORTEC's library is, when it is not installed system-wide.
    pub local_umcbi: String,
    /// What the last scan of local instruments reported.
    pub local_scan: String,
    /// What has been typed into the calibration table, per row.
    pub calibration_edits: Vec<(String, String)>,
    /// Address typed for a new network instrument.
    pub network_address: String,
    /// Strip file.
    pub strip_path: String,
    /// Strip factor.
    pub strip_factor: f64,
    /// Whether the strip uses the live-time ratio.
    pub strip_use_ratio: bool,
    /// List-mode slice start, in seconds.
    pub list_start: f64,
    /// List-mode slice duration, in seconds.
    pub list_duration: f64,
    /// Whether the list-mode replay is running.
    pub list_playing: bool,
    /// Replay speed, in acquisition seconds per real second.
    pub list_speed: f64,
    /// Job directory.
    pub job_directory: String,
    /// Selected job file.
    pub job_file: Option<PathBuf>,
    /// Password for the lock dialog.
    pub lock_password: String,
    /// Owner for the lock dialog.
    pub lock_owner: String,
    /// New detector being defined.
    pub new_detector: DetectorEntry,
    /// Selected nuclide in the library editor.
    pub library_selection: Option<usize>,
    /// New sample being queued.
    pub queue_id: String,
    /// Live time for queued samples.
    pub queue_live_time: f64,
    /// The Regions list rows, cached against a fingerprint of the data.
    ///
    /// Peak information is a fit per region; recomputing sixty-five fits every
    /// frame is wasted heat when nothing changed.
    #[allow(clippy::type_complexity)]
    pub region_cache: Option<(u64, Vec<(usize, usize, usize, String)>)>,
    /// A detector name being typed, and whose detector it is.
    ///
    /// A name is typed over many frames, so re-reading the saved one each
    /// frame would erase every keystroke; the half-typed name lives here
    /// until Enter or leaving the box commits it.
    pub detector_name_edit: Option<(u16, String)>,
    /// Presets being typed in MCB Properties, and whose instrument they are.
    ///
    /// A preset is entered over several frames - tick the box, then type the
    /// number - and only reaches the instrument when Apply is pressed. Reading
    /// the instrument's own presets back each frame would therefore erase the
    /// tick the moment it was made, so the half-finished edit lives here until
    /// it is applied or the dialog is closed.
    pub presets_edit: Option<(usize, Presets)>,
}

impl Default for Dialogs {
    fn default() -> Self {
        Self {
            open: std::collections::HashSet::new(),
            properties_tab: 0,
            calibration_energy: String::new(),
            calibration_units: "keV".into(),
            report_columns: false,
            report_displayed_only: false,
            report_mda: false,
            efficiency_constant: 0.05,
            efficiency_points: vec![
                ("59.5".into(), "0.08".into()),
                ("661.7".into(), "0.02".into()),
                ("1332.5".into(), "0.01".into()),
            ],
            efficiency_order: 2,
            path_input: String::new(),
            goto_input: String::new(),
            auto_nuclide: String::new(),
            correct_during_count: false,
            correct_to_reference: false,
            reference_input: String::new(),
            source_nuclide: String::new(),
            source_activity_bq: 10_000.0,
            source_uncertainty_pct: 3.0,
            source_age_days: 0.0,
            network_name: String::new(),
            local_name: String::new(),
            local_detector: 1,
            local_umcbi: String::new(),
            local_scan: String::new(),
            calibration_edits: Vec::new(),
            network_address: String::new(),
            strip_path: String::new(),
            strip_factor: 1.0,
            strip_use_ratio: true,
            list_start: 0.0,
            list_duration: 60.0,
            list_playing: false,
            list_speed: 5.0,
            job_directory: ".".into(),
            job_file: None,
            lock_password: String::new(),
            lock_owner: "operator".into(),
            // Only the simulator build has a form that reads this; a shipped
            // build carries no notion of a simulated detector at all.
            #[cfg(feature = "simulator")]
            new_detector: DetectorEntry::simulated(2, "SIM-02"),
            #[cfg(not(feature = "simulator"))]
            new_detector: DetectorEntry {
                number: 2,
                name: String::new(),
                model: String::new(),
                kind: DetectorKind::Network,
                channels: 8192,
                description: String::new(),
                serial: String::new(),
            },
            library_selection: Some(0),
            queue_id: "S001".into(),
            queue_live_time: 300.0,
            region_cache: None,
            detector_name_edit: None,
            presets_edit: None,
        }
    }
}

impl Dialogs {
    /// Opens a dialog.
    pub fn open(&mut self, dialog: Dialog) {
        self.open.insert(dialog);
    }

    /// Whether a dialog is open.
    pub fn is_open(&self, dialog: Dialog) -> bool {
        self.open.contains(&dialog)
    }

    fn flag(&mut self, dialog: Dialog) -> bool {
        self.open.contains(&dialog)
    }

    /// Whether any dialog is open, for Escape's layering.
    pub fn any_open(&self) -> bool {
        !self.open.is_empty()
    }

    /// Closes every open dialog (Escape).
    pub fn close_all(&mut self) {
        for dialog in std::mem::take(&mut self.open) {
            self.on_close(dialog);
        }
    }

    /// Opens or closes a dialog.
    pub fn set(&mut self, dialog: Dialog, open: bool) {
        if open {
            self.open.insert(dialog);
        } else if self.open.remove(&dialog) {
            self.on_close(dialog);
        }
    }

    /// Lets go of state that belongs to one showing of a dialog.
    fn on_close(&mut self, dialog: Dialog) {
        match dialog {
            // Half-typed calibration texts must not greet the next opening -
            // the table would show the old typing over different points.
            Dialog::Calibration => self.calibration_edits.clear(),
            // A half-finished preset edit dies with its dialog.
            Dialog::McbProperties => self.presets_edit = None,
            _ => {}
        }
    }
}

/// Every spelling of an extension a file dialog might have to match.
///
/// ORTEC writes its files capitalised - `.Spe`, `.Chn`, `.Spc` - and GTK and
/// the XDG portal match filter patterns **case-sensitively**, where Windows
/// does not. A filter of `spe` alone therefore hides every real instrument
/// file on Linux: exactly the files it was written to show. Lower, capitalised
/// and upper cover what instruments and people actually write; the extra
/// patterns cost nothing where the platform ignores case anyway.
// A --no-default-features build has no dialog to filter for, but the rule is
// still worth keeping compiled and tested.
#[cfg_attr(not(feature = "native-dialogs"), allow(dead_code))]
fn case_variants(extensions: &[&str]) -> Vec<String> {
    let mut out: Vec<String> = Vec::new();
    for extension in extensions {
        let lower = extension.to_lowercase();
        let mut capitalised = lower.clone();
        if let Some(first) = capitalised.get_mut(..1) {
            first.make_ascii_uppercase();
        }
        for variant in [lower, capitalised, extension.to_uppercase()] {
            if !out.contains(&variant) {
                out.push(variant);
            }
        }
    }
    out
}

/// Opens a native file dialog, when one is available.
pub fn pick_open_file(filters: &[(&str, &[&str])]) -> Option<PathBuf> {
    #[cfg(feature = "native-dialogs")]
    {
        let mut dialog = rfd::FileDialog::new();
        for (name, extensions) in filters {
            dialog = dialog.add_filter(*name, &case_variants(extensions));
        }
        dialog.pick_file()
    }
    #[cfg(not(feature = "native-dialogs"))]
    {
        let _ = filters;
        None
    }
}

/// Opens a native save dialog, when one is available.
pub fn pick_save_file(filters: &[(&str, &[&str])]) -> Option<PathBuf> {
    pick_save_file_named(filters, None)
}

/// Like [`pick_save_file`], with a name to offer first (§4.1.1's default
/// format: the suggestion carries the preferred extension).
pub fn pick_save_file_named(
    filters: &[(&str, &[&str])],
    default_name: Option<&str>,
) -> Option<PathBuf> {
    #[cfg(feature = "native-dialogs")]
    {
        let mut dialog = rfd::FileDialog::new();
        for (name, extensions) in filters {
            dialog = dialog.add_filter(*name, &case_variants(extensions));
        }
        if let Some(name) = default_name {
            dialog = dialog.set_file_name(name);
        }
        dialog.save_file()
    }
    #[cfg(not(feature = "native-dialogs"))]
    {
        let _ = (filters, default_name);
        None
    }
}

/// Draws the menu bar.
pub fn menu_bar(app: &mut App, ui: &mut egui::Ui) -> Vec<Action> {
    let mut actions = Vec::new();
    let undo_label = app
        .history
        .undo_label()
        .map(|label| format!("Undo {label}"))
        .unwrap_or_else(|| "Undo".into());
    let redo_label = app
        .history
        .redo_label()
        .map(|label| format!("Redo {label}"))
        .unwrap_or_else(|| "Redo".into());
    let can_undo = app.history.can_undo();
    let can_redo = app.history.can_redo();
    let detectors: Vec<(usize, String)> = app
        .detectors
        .iter()
        .enumerate()
        .map(|(index, detector)| {
            (
                index,
                format!(
                    "{:04} {}",
                    detector.identity().number,
                    detector.identity().name
                ),
            )
        })
        .collect();
    // Ids, not positions: Activate resolves by id, so the row cannot focus
    // the wrong window if the list shifted under the menu.
    let active_id = app
        .active
        .and_then(|index| app.windows.get(index))
        .map(|window| window.id);
    let window_titles: Vec<(usize, String)> = app
        .windows
        .iter()
        .map(|window| (window.id, window.title.clone()))
        .collect();
    let mark_mode = app.mark_mode;
    let fill = app
        .active
        .and_then(|index| app.windows.get(index))
        .map(|window| window.display.fill)
        .unwrap_or_default();

    egui::MenuBar::new().ui(ui, |ui| {
        ui.menu_button("File", |ui| {
            if ui.button("Recall...\tCtrl+O").clicked() {
                actions.push(Action::Recall);
                ui.close();
            }
            if ui.button("Open path...").clicked() {
                actions.push(Action::Show(Dialog::OpenPath));
                ui.close();
            }
            let recent: Vec<PathBuf> = app.recent_files().to_vec();
            ui.add_enabled_ui(!recent.is_empty(), |ui| {
                ui.menu_button("Recent", |ui| {
                    for path in &recent {
                        let name = path
                            .file_name()
                            .map(|name| name.to_string_lossy().to_string())
                            .unwrap_or_else(|| path.display().to_string());
                        if ui
                            .button(&name)
                            .on_hover_text(path.display().to_string())
                            .clicked()
                        {
                            actions.push(Action::RecallFile(path.clone()));
                            ui.close();
                        }
                    }
                });
            });
            ui.label("or drag a spectrum onto the window");
            ui.separator();
            // Save As first, and on Ctrl+S: it is the safe one, and the one
            // reached for by habit. Overwriting in place is a deliberate act
            // and says which file it will replace.
            if ui.button("Save As...\tCtrl+S").clicked() {
                actions.push(Action::SaveAs);
                ui.close();
            }
            let overwrites = app
                .active
                .and_then(|index| app.windows.get(index))
                .and_then(|window| window.path.as_ref())
                .map(|path| {
                    std::path::Path::new(path)
                        .file_name()
                        .map(|name| name.to_string_lossy().into_owned())
                        .unwrap_or_else(|| path.display().to_string())
                });
            let save = ui.button("Save\tCtrl+Shift+S");
            let save = match &overwrites {
                Some(name) => save.on_hover_text(format!("replaces {name}")),
                None => save.on_hover_text("asks where to put it - nothing has been saved yet"),
            };
            if save.clicked() {
                actions.push(Action::Save);
                ui.close();
            }
            if ui.button("Export...").clicked() {
                actions.push(Action::Export);
                ui.close();
            }
            if ui
                .button("Export Plot...")
                .on_hover_text("the plot as it looks now, as an SVG picture")
                .clicked()
            {
                actions.push(Action::ExportPlot);
                ui.close();
            }
            if ui.button("Import...").clicked() {
                actions.push(Action::Import);
                ui.close();
            }
            ui.separator();
            if ui.button("Print...").clicked() {
                actions.push(Action::Print);
                ui.close();
            }
            if ui.button("ROI Report...").clicked() {
                actions.push(Action::Show(Dialog::RoiReportOptions));
                ui.close();
            }
            ui.separator();
            if ui.button("Compare...").clicked() {
                actions.push(Action::Compare);
                ui.close();
            }
            if ui.button("Clear comparison").clicked() {
                actions.push(Action::ClearCompare);
                ui.close();
            }
            ui.separator();
            if ui.button("Exit").clicked() {
                actions.push(Action::Exit);
                ui.close();
            }
        });

        ui.menu_button("Edit", |ui| {
            if ui
                .add_enabled(can_undo, egui::Button::new(format!("{undo_label}\tCtrl+Z")))
                .clicked()
            {
                actions.push(Action::Undo);
                ui.close();
            }
            if ui
                .add_enabled(can_redo, egui::Button::new(format!("{redo_label}\tCtrl+Y")))
                .clicked()
            {
                actions.push(Action::Redo);
                ui.close();
            }
            ui.separator();
            if ui
                .button("Copy Peak Info\tCtrl+C")
                .on_hover_text("the open peak card's text, for a lab book or an email")
                .clicked()
            {
                actions.push(Action::CopyPeakInfo);
                ui.close();
            }
            if ui.button("Copy Region Report").clicked() {
                actions.push(Action::CopyReport);
                ui.close();
            }
            if ui
                .button("Copy Spectrum as CSV")
                .on_hover_text("channel and counts columns, ready for a spreadsheet")
                .clicked()
            {
                actions.push(Action::CopyCsv);
                ui.close();
            }
            ui.separator();
            ui.label(format!("{} step(s) remembered", app.history.depth()));
        });

        ui.menu_button("Acquire", |ui| {
            if ui.button("Start\tAlt+1").clicked() {
                actions.push(Action::Start);
                ui.close();
            }
            if ui.button("Stop\tAlt+2").clicked() {
                actions.push(Action::Stop);
                ui.close();
            }
            if ui.button("Clear\tAlt+3").clicked() {
                actions.push(Action::Clear);
                ui.close();
            }
            if ui.button("Copy to Buffer\tAlt+5").clicked() {
                actions.push(Action::CopyToBuffer);
                ui.close();
            }
            ui.separator();
            if ui.button("List Mode").clicked() {
                actions.push(Action::ToggleListMode);
                ui.close();
            }
            if ui.button("View ZDT Corrected\tF3").clicked() {
                actions.push(Action::ViewZdt);
                ui.close();
            }
            ui.separator();
            if ui
                .button("Store to Instrument Memory")
                .on_hover_text("snapshot the spectrum into the instrument and clear for the next sample (field mode)")
                .clicked()
            {
                actions.push(Action::StoreSpectrum);
                ui.close();
            }
            if ui
                .button("Download Spectra")
                .on_hover_text("pull every stored spectrum into buffer windows (§4.2.6)")
                .clicked()
            {
                actions.push(Action::DownloadSpectra);
                ui.close();
            }
            ui.separator();
            if ui
                .button("InSight...")
                .on_hover_text("virtual oscilloscope: shaping, pole zero and the auto routines (§4.2.9)")
                .clicked()
            {
                actions.push(Action::Show(Dialog::Insight));
                ui.close();
            }
            if ui.button("MCB Properties...").clicked() {
                actions.push(Action::Show(Dialog::McbProperties));
                ui.close();
            }
        });

        ui.menu_button("Calculate", |ui| {
            if ui.button("Settings...").clicked() {
                actions.push(Action::Show(Dialog::Settings));
                ui.close();
            }
            if ui.button("Calibration...").clicked() {
                actions.push(Action::Show(Dialog::Calibration));
                ui.close();
            }
            if ui.button("List Data Range...").clicked() {
                actions.push(Action::ListDataRange);
                ui.close();
            }
            ui.separator();
            if ui.button("Peak Search").clicked() {
                actions.push(Action::PeakSearch);
                ui.close();
            }
            if ui.button("Peak Info").clicked() {
                actions.push(Action::PeakInfoAtMarker);
                ui.close();
            }
            if ui.button("Input Count Rate").clicked() {
                actions.push(Action::InputCountRate);
                ui.close();
            }
            if ui.button("Sum").clicked() {
                actions.push(Action::Sum);
                ui.close();
            }
            if ui.button("Smooth").clicked() {
                actions.push(Action::Smooth);
                ui.close();
            }
            if ui.button("Strip...").clicked() {
                actions.push(Action::Strip);
                ui.close();
            }
            ui.separator();
            if ui.button("Analyse nuclides...").clicked() {
                actions.push(Action::Analyse);
                ui.close();
            }
            if ui.button("Efficiency calibration...").clicked() {
                actions.push(Action::Show(Dialog::Efficiency));
                ui.close();
            }
        });

        ui.menu_button("Services", |ui| {
            if ui.button("Job Control...").clicked() {
                actions.push(Action::RunJob);
                ui.close();
            }
            if ui.button("Library file...").clicked() {
                actions.push(Action::Show(Dialog::Library));
                ui.close();
            }
            if ui.button("Sample Description...").clicked() {
                actions.push(Action::Show(Dialog::Sample));
                ui.close();
            }
            if ui.button("Lock/Unlock Detector...").clicked() {
                actions.push(Action::Show(Dialog::Lock));
                ui.close();
            }
            if ui.button("Set up instruments...").clicked() {
                actions.push(Action::Show(Dialog::Instruments));
                ui.close();
            }
            if ui.button("Edit Detector List...").clicked() {
                actions.push(Action::Show(Dialog::DetectorList));
                ui.close();
            }
            ui.separator();
            if ui.button("Detector dashboard...").clicked() {
                actions.push(Action::Show(Dialog::Dashboard));
                ui.close();
            }
            if ui.button("Batch queue...").clicked() {
                actions.push(Action::Show(Dialog::Batch));
                ui.close();
            }
            if ui.button("Quality assurance...").clicked() {
                actions.push(Action::Show(Dialog::Qa));
                ui.close();
            }
            if ui.button("Record QA point").clicked() {
                actions.push(Action::RecordQa);
                ui.close();
            }
            if cfg!(windows) {
                ui.separator();
                if ui
                    .button("Register spectrum file types")
                    .on_hover_text(
                        "double-clicking .Spe, .Chn or .Spc in Explorer opens ortseam \
                         (for this user only; reversible below)",
                    )
                    .clicked()
                {
                    actions.push(Action::RegisterFileTypes);
                    ui.close();
                }
                if ui.button("Unregister spectrum file types").clicked() {
                    actions.push(Action::UnregisterFileTypes);
                    ui.close();
                }
            }
        });

        ui.menu_button("ROI", |ui| {
            for (mode, label) in [
                (MarkMode::Off, "Off\tF2"),
                (MarkMode::Mark, "Mark"),
                (MarkMode::UnMark, "UnMark"),
            ] {
                if ui.selectable_label(mark_mode == mode, label).clicked() {
                    actions.push(Action::MarkMode(mode));
                    ui.close();
                }
            }
            ui.separator();
            let mut auto_clear = app.auto_clear_roi;
            if ui
                .checkbox(&mut auto_clear, "Auto Clear")
                .on_hover_text("a peak search clears the marked regions first")
                .changed()
            {
                app.auto_clear_roi = auto_clear;
            }
            ui.separator();
            if ui.button("Mark Peak\tInsert").clicked() {
                actions.push(Action::MarkPeak);
                ui.close();
            }
            if ui.button("Clear\tDelete").clicked() {
                actions.push(Action::ClearRoi);
                ui.close();
            }
            if ui.button("Clear All").clicked() {
                actions.push(Action::ClearAllRois);
                ui.close();
            }
            ui.separator();
            if ui.button("Save File...").clicked() {
                actions.push(Action::SaveRoiFile);
                ui.close();
            }
            if ui.button("Recall File...").clicked() {
                actions.push(Action::RecallRoiFile);
                ui.close();
            }
        });

        ui.menu_button("Display", |ui| {
            ui.menu_button("Detector", |ui| {
                for (index, label) in &detectors {
                    if ui.button(label).clicked() {
                        actions.push(Action::OpenDetector(*index));
                        ui.close();
                    }
                }
            });
            if ui.button("Buffer\tF4").clicked() {
                actions.push(Action::OpenBuffer);
                ui.close();
            }
            ui.separator();
            if ui.button("Logarithmic\tKeypad /").clicked() {
                actions.push(Action::ToggleLog);
                ui.close();
            }
            // How high a logarithmic axis goes. MAESTRO tops out on the decade
            // above the tallest channel, so a thousand-count peak is read
            // against a drawn 10,000 line rather than against the ceiling.
            let mut decade_top = app.log_decade_top;
            if ui
                .checkbox(&mut decade_top, "Log to the next decade")
                .on_hover_text(
                    "top the logarithmic axis at the next power of ten, as MAESTRO does, \
                     instead of the nearest step above the peak",
                )
                .changed()
            {
                app.log_decade_top = decade_top;
            }
            if ui.button("Automatic\tKeypad *").clicked() {
                actions.push(Action::AutoScale);
                ui.close();
            }
            if ui.button("Baseline Zoom").clicked() {
                actions.push(Action::BaselineZoom);
                ui.close();
            }
            ui.separator();
            if ui.button("Zoom In\tKeypad +").clicked() {
                actions.push(Action::ZoomIn);
                ui.close();
            }
            if ui.button("Zoom Out\tKeypad -").clicked() {
                actions.push(Action::ZoomOut);
                ui.close();
            }
            if ui.button("Center\tKeypad 5").clicked() {
                actions.push(Action::Center);
                ui.close();
            }
            if ui.button("Full View").clicked() {
                actions.push(Action::FullView);
                ui.close();
            }
            ui.separator();
            if ui.button("Isotope Markers").clicked() {
                actions.push(Action::ToggleIsotopeMarkers);
                ui.close();
            }
            if ui.button("Energy axis").clicked() {
                actions.push(Action::ToggleEnergyAxis);
                ui.close();
            }
            ui.menu_button("Preferences", |ui| {
                for (mode, label) in [
                    (FillMode::Points, "Points"),
                    (FillMode::FillRoi, "Fill ROI"),
                    (FillMode::FillAll, "Fill All"),
                ] {
                    if ui.selectable_label(fill == mode, label).clicked() {
                        actions.push(Action::SetFill(mode));
                        ui.close();
                    }
                }
                ui.separator();
                if ui.button("Spectrum Colors...").clicked() {
                    actions.push(Action::Show(Dialog::Preferences));
                    ui.close();
                }
            });
        });

        ui.menu_button("Window", |ui| {
            if ui.button("Maximise / restore\tCtrl+M").clicked() {
                actions.push(Action::MaximizeActive);
                ui.close();
            }
            ui.separator();
            if ui.button("Cascade").clicked() {
                actions.push(Action::Cascade);
                ui.close();
            }
            if ui.button("Tile Horizontally").clicked() {
                actions.push(Action::TileHorizontally);
                ui.close();
            }
            if ui.button("Tile Vertically").clicked() {
                actions.push(Action::TileVertically);
                ui.close();
            }
            if ui
                .button("Collapse All")
                .on_hover_text("every window folds to its title bar (Arrange Icons)")
                .clicked()
            {
                actions.push(Action::CollapseAll);
                ui.close();
            }
            if ui.button("Expand All").clicked() {
                actions.push(Action::ExpandAll);
                ui.close();
            }
            ui.separator();
            for (id, title) in &window_titles {
                // The active window is ticked, as any window manager would.
                if ui
                    .selectable_label(active_id == Some(*id), title)
                    .clicked()
                {
                    actions.push(Action::Activate(*id));
                    ui.close();
                }
            }
        });

        ui.menu_button("Help", |ui| {
            if ui.button("Keyboard Commands\tF1").clicked() {
                actions.push(Action::Show(Dialog::Shortcuts));
                ui.close();
            }
            if ui.button("About ortseam...").clicked() {
                actions.push(Action::Show(Dialog::About));
                ui.close();
            }
        });

        ui.with_layout(egui::Layout::right_to_left(egui::Align::Center), |ui| {
            ui.label(format!("Marking: {}", mark_mode.label()));
        });
    });
    actions
}

/// Draws the toolbar.
pub fn toolbar(app: &mut App, ui: &mut egui::Ui) -> Vec<Action> {
    let mut actions = Vec::new();
    let is_detector = app
        .active
        .and_then(|index| app.windows.get(index))
        .map(|window| !window.is_buffer())
        .unwrap_or(false);
    let counting = app
        .active
        .and_then(|index| app.windows.get(index))
        .and_then(|window| match window.target {
            Target::Detector(detector) => app.detectors.get(detector),
            Target::Buffer => None,
        })
        .map(|mcb| mcb.is_active())
        .unwrap_or(false);
    let has_window = app.active.is_some();
    let (scale_label, width_label) = app
        .active
        .and_then(|index| app.windows.get(index))
        .map(|window| {
            (
                match window.display.vertical {
                    VerticalScale::Logarithmic => "LOG".to_string(),
                    VerticalScale::Automatic => "AUTO".to_string(),
                    VerticalScale::Linear(max) => format_counts(max),
                },
                // The zoom width has a floor of eight channels, but a spectrum
                // with no channels has no width worth reporting - "8 ch" beside
                // a header reading "0 ch" is the toolbar contradicting the
                // window.
                match app.active_spectrum() {
                    Some(spectrum) if spectrum.is_empty() => "-".to_string(),
                    _ => window.display.view_width.to_string(),
                },
            )
        })
        .unwrap_or_else(|| ("-".into(), "-".into()));
    let detector_label = app
        .active
        .and_then(|index| app.windows.get(index))
        .map(|window| match window.target {
            Target::Buffer => "Buffer".to_string(),
            Target::Detector(detector) => app
                .detectors
                .get(detector)
                .map(|mcb| format!("{:04} {}", mcb.identity().number, mcb.identity().name))
                .unwrap_or_else(|| "Detector".into()),
        })
        .unwrap_or_else(|| "no window".into());
    let detectors: Vec<(usize, String)> = app
        .detectors
        .iter()
        .enumerate()
        .map(|(index, detector)| {
            (
                index,
                format!(
                    "{:04} {}",
                    detector.identity().number,
                    detector.identity().name
                ),
            )
        })
        .collect();

    let already_full = app
        .active
        .and_then(|index| app.windows.get(index))
        .map(|window| window.display.is_full_view())
        .unwrap_or(true);
    let mark_mode = app.mark_mode;
    let fill = app
        .active
        .and_then(|index| app.windows.get(index))
        .map(|window| window.display.fill)
        .unwrap_or_default();

    // Wrapped, so a narrow window folds the toolbar onto a second row
    // instead of clipping the buttons at the right edge.
    ui.horizontal_wrapped(|ui| {
        // Files.
        if ui
            .button("Recall…")
            .on_hover_text("open a spectrum (Ctrl+O)")
            .clicked()
        {
            actions.push(Action::Recall);
        }
        if ui
            .add_enabled(has_window, egui::Button::new("Save"))
            .on_hover_text("save the active spectrum (Ctrl+S)")
            .clicked()
        {
            actions.push(Action::Save);
        }
        ui.separator();

        // Acquisition. Start and Stop take their colours from the palette's
        // status roles: green means "will begin", red is reserved for the one
        // button that ends a measurement.
        if ui
            .add_enabled(
                is_detector && !counting,
                egui::Button::new(egui::RichText::new("▶ Start").color(
                    if is_detector && !counting {
                        app.colors.healthy.to_color()
                    } else {
                        ui.visuals().text_color()
                    },
                )),
            )
            .on_hover_text("begin collecting (Alt+1)")
            .clicked()
        {
            actions.push(Action::Start);
        }
        if ui
            .add_enabled(
                is_detector && counting,
                egui::Button::new(egui::RichText::new("■ Stop").color(
                    if is_detector && counting {
                        app.colors.alarm.to_color()
                    } else {
                        ui.visuals().text_color()
                    },
                )),
            )
            .on_hover_text("stop collecting (Alt+2)")
            .clicked()
        {
            actions.push(Action::Stop);
        }
        if ui
            .add_enabled(has_window, egui::Button::new("Clear"))
            .on_hover_text("erase the data - Ctrl+Z brings it back (Alt+3)")
            .clicked()
        {
            actions.push(Action::Clear);
        }
        if ui
            .add_enabled(is_detector, egui::Button::new("→ Buffer"))
            .on_hover_text("copy the detector data into a buffer window (Alt+5)")
            .clicked()
        {
            actions.push(Action::CopyToBuffer);
        }
        ui.separator();

        // Undo and redo. Words, not arrow glyphs: the pretty curled arrows are
        // not in egui's fonts and drew as empty boxes.
        if ui
            .add_enabled(app.history.can_undo(), egui::Button::new("Undo"))
            .on_hover_text(match app.history.undo_label() {
                Some(label) => format!("undo {label} (Ctrl+Z)"),
                None => "nothing to undo".to_string(),
            })
            .clicked()
        {
            actions.push(Action::Undo);
        }
        if ui
            .add_enabled(app.history.can_redo(), egui::Button::new("Redo"))
            .on_hover_text(match app.history.redo_label() {
                Some(label) => format!("redo {label} (Ctrl+Y)"),
                None => "nothing to redo".to_string(),
            })
            .clicked()
        {
            actions.push(Action::Redo);
        }
        ui.separator();

        // Analysis.
        if ui
            .add_enabled(has_window, egui::Button::new("Peaks"))
            .on_hover_text("search for peaks and mark them")
            .clicked()
        {
            actions.push(Action::PeakSearch);
        }
        if ui
            .add_enabled(has_window, egui::Button::new("Peak Info"))
            .on_hover_text("measure the peak at the marker")
            .clicked()
        {
            actions.push(Action::PeakInfoAtMarker);
        }
        if ui
            .add_enabled(has_window, egui::Button::new("Report"))
            .on_hover_text("ROI report")
            .clicked()
        {
            actions.push(Action::Show(Dialog::RoiReportOptions));
        }
        if ui
            .add_enabled(has_window, egui::Button::new("Nuclides"))
            .on_hover_text("identify nuclides and compute activities")
            .clicked()
        {
            actions.push(Action::Analyse);
        }
        ui.separator();

        // Region marking.
        let marking_colour = match mark_mode {
            MarkMode::Off => None,
            MarkMode::Mark => Some(egui::Color32::from_rgb(230, 110, 110)),
            MarkMode::UnMark => Some(egui::Color32::from_rgb(120, 170, 230)),
        };
        let marking_label = egui::RichText::new(format!("ROI: {}", mark_mode.label()));
        if ui
            .button(match marking_colour {
                Some(colour) => marking_label.color(colour),
                None => marking_label.weak(),
            })
            .on_hover_text("cycle Off / Mark / UnMark, then drag on the spectrum (F2)")
            .clicked()
        {
            actions.push(Action::MarkMode(match mark_mode {
                MarkMode::Off => MarkMode::Mark,
                MarkMode::Mark => MarkMode::UnMark,
                MarkMode::UnMark => MarkMode::Off,
            }));
        }
        ui.separator();

        // Display.
        if ui
            .button(&scale_label)
            .on_hover_text("switch between logarithmic and linear (keypad /)")
            .clicked()
        {
            actions.push(Action::ToggleLog);
        }
        if ui
            .button("Auto")
            .on_hover_text("scale to the tallest visible channel (A)")
            .clicked()
        {
            actions.push(Action::AutoScale);
        }
        if ui
            .button(match fill {
                FillMode::Points => "Points",
                FillMode::FillRoi => "Fill ROI",
                FillMode::FillAll => "Filled",
            })
            .on_hover_text("how the spectrum is drawn")
            .clicked()
        {
            actions.push(Action::SetFill(match fill {
                FillMode::FillAll => FillMode::FillRoi,
                FillMode::FillRoi => FillMode::Points,
                FillMode::Points => FillMode::FillAll,
            }));
        }
        ui.separator();
        if ui
            .button("−")
            .on_hover_text("zoom out (keypad -)")
            .clicked()
        {
            actions.push(Action::ZoomOut);
        }
        ui.label(
            egui::RichText::new(format!("{width_label} ch"))
                .monospace()
                .weak(),
        );
        if ui.button("+").on_hover_text("zoom in (keypad +)").clicked() {
            actions.push(Action::ZoomIn);
        }
        if ui
            .add_enabled(has_window, egui::Button::new("Center"))
            .on_hover_text("put the marker in the middle (keypad 5)")
            .clicked()
        {
            actions.push(Action::Center);
        }
        if ui
            .add_enabled(!already_full, egui::Button::new("Full"))
            .on_hover_text("show the whole spectrum")
            .clicked()
        {
            actions.push(Action::FullView);
        }
        if ui
            .add_enabled(has_window, egui::Button::new("⛶"))
            .on_hover_text("fill the spectrum area (Ctrl+M)")
            .clicked()
        {
            actions.push(Action::MaximizeActive);
        }

        ui.separator();
        // Go straight to an energy: type it and press Enter.
        let goto = ui
            .add(
                egui::TextEdit::singleline(&mut app.dialogs.goto_input)
                    .desired_width(64.0)
                    .hint_text("energy"),
            )
            .on_hover_text("jump to an energy: type keV and press Enter");
        if goto.lost_focus() && ui.input(|input| input.key_pressed(egui::Key::Enter)) {
            match app.dialogs.goto_input.trim().parse::<f64>() {
                Ok(value) => {
                    actions.push(Action::GotoEnergy(value));
                    app.dialogs.goto_input.clear();
                }
                Err(_) if !app.dialogs.goto_input.trim().is_empty() => {
                    actions.push(Action::Status("type an energy as a number".into()));
                }
                Err(_) => {}
            }
        }
        goto.on_hover_text("jump the marker to an energy (a channel, when uncalibrated)");

        // The detector picker hugs the right edge. In a wrapped toolbar it has
        // to claim a block of its own first: a right-to-left layout handed the
        // leftovers of a full row draws itself on top of the buttons already
        // there, which is exactly what happened when a build with no
        // instruments made the row short enough to stop wrapping.
        let picker_width = 190.0_f32;
        ui.allocate_ui_with_layout(
            egui::vec2(
                ui.available_width().max(picker_width),
                ui.spacing().interact_size.y,
            ),
            egui::Layout::right_to_left(egui::Align::Center),
            |ui| {
                egui::ComboBox::from_id_salt("detector-picker")
                    .selected_text(detector_label)
                    .show_ui(ui, |ui| {
                        if ui.selectable_label(false, "Buffer").clicked() {
                            actions.push(Action::OpenBuffer);
                        }
                        for (index, label) in &detectors {
                            if ui.selectable_label(false, label).clicked() {
                                actions.push(Action::OpenDetector(*index));
                            }
                        }
                    });
                ui.label(egui::RichText::new("detector").weak());
            },
        );
    });
    actions
}

/// Draws the status sidebar (MAESTRO §3.6).
pub fn status_sidebar(app: &mut App, ui: &mut egui::Ui) -> Vec<Action> {
    let mut actions = Vec::new();
    let active = app.active.and_then(|index| app.windows.get(index));
    let (status, presets, name) = match active.map(|window| window.target) {
        Some(Target::Detector(detector)) => match app.detectors.get(detector) {
            Some(mcb) => (
                Some(mcb.status()),
                Some(*mcb.presets()),
                mcb.identity().name.clone(),
            ),
            None => (None, None, String::new()),
        },
        _ => (None, None, "Buffer".into()),
    };

    ui.add_space(4.0);
    ui.heading("Pulse Ht. Analysis");
    ui.label(name);
    egui::Grid::new("times").num_columns(2).show(ui, |ui| {
        let spectrum = app.active_spectrum();
        ui.label("Start");
        ui.label(
            spectrum
                .and_then(|spectrum| spectrum.start_time)
                .map(|start| start.format("%H:%M:%S %d/%m/%Y").to_string())
                .unwrap_or_else(|| "-".into()),
        );
        ui.end_row();
        let (real, live, dead) = match &status {
            Some(status) => (
                format!("{:.2}", status.real_time),
                format!("{:.2}", status.live_time),
                format!("{:.2} %", status.dead_time_percent),
            ),
            None => match spectrum {
                Some(spectrum) => (
                    format!("{:.2}", spectrum.real_time),
                    format!("{:.2}", spectrum.live_time),
                    spectrum
                        .dead_time_percent()
                        .map(|dead| format!("{dead:.2} %"))
                        .unwrap_or_else(|| "-".into()),
                ),
                None => ("-".into(), "-".into(), "-".into()),
            },
        };
        ui.label("Real");
        ui.label(real);
        ui.end_row();
        ui.label("Live");
        ui.label(live);
        ui.end_row();
        ui.label("Dead");
        ui.horizontal(|ui| {
            ui.label(dead);
            // A small gauge, coloured like the header's dead-time readout, so a
            // stressed detector is visible at a glance from across the room.
            let fraction = match &status {
                Some(status) => Some(status.dead_time_percent),
                None => app
                    .active_spectrum()
                    .and_then(|spectrum| spectrum.dead_time_percent()),
            };
            if let Some(percent) = fraction {
                let colour = if percent > 50.0 {
                    app.colors.alarm.to_color()
                } else if percent > 20.0 {
                    app.colors.roi.to_color()
                } else {
                    app.colors.healthy.to_color()
                };
                let (rect, _) = ui.allocate_exact_size(egui::vec2(56.0, 7.0), egui::Sense::hover());
                ui.painter().rect_filled(
                    rect,
                    egui::CornerRadius::same(2),
                    ui.visuals().extreme_bg_color,
                );
                let filled = egui::Rect::from_min_size(
                    rect.min,
                    egui::vec2(rect.width() * (percent as f32 / 100.0).clamp(0.0, 1.0), 7.0),
                );
                ui.painter()
                    .rect_filled(filled, egui::CornerRadius::same(2), colour);
            }
        });
        ui.end_row();
        ui.label("Counts");
        ui.label(
            spectrum
                .map(|spectrum| format_counts(spectrum.total_counts()))
                .unwrap_or_else(|| "-".into()),
        );
        ui.end_row();
        // Under the total, the rate that produced it. Over live time rather
        // than real, because live time is what the system was able to accept
        // events during - the same convention the counting-activity equation
        // uses - so the figure does not sag as dead time rises.
        ui.label("Rate");
        let rate = spectrum.and_then(|spectrum| {
            (spectrum.live_time > 0.0).then(|| spectrum.total_counts() as f64 / spectrum.live_time)
        });
        ui.label(match rate {
            Some(rate) => format_rate(rate),
            None => "-".into(),
        })
        .on_hover_text("gross counts per second of live time");
        ui.end_row();
    });

    stability_trace(app, ui);

    ui.separator();
    ui.heading("Preset Limits");
    match presets {
        Some(presets) if !presets.is_empty() => {
            egui::Grid::new("presets").num_columns(2).show(ui, |ui| {
                if let Some(value) = presets.real_time {
                    ui.label("Real");
                    ui.label(format!("{value:.0} s"));
                    ui.end_row();
                }
                if let Some(value) = presets.live_time {
                    ui.label("Live");
                    ui.label(format!("{value:.0} s"));
                    ui.end_row();
                }
                if let Some(value) = presets.roi_peak {
                    ui.label("Peak");
                    ui.label(value.to_string());
                    ui.end_row();
                }
                if let Some(value) = presets.roi_integral {
                    ui.label("Intg");
                    ui.label(value.to_string());
                    ui.end_row();
                }
                if let Some(value) = presets.uncertainty {
                    ui.label("Uncert");
                    ui.label(format!("{:.2} %", value.limit_percent));
                    ui.end_row();
                }
                if let Some(value) = presets.mda {
                    ui.label("MDA");
                    ui.label(format!("{:.1}", value.target));
                    ui.end_row();
                }
            });
            if let Some(index) = app.active
                && let Target::Detector(detector) = app.windows[index].target
                && let Some(progress) = app
                    .detectors
                    .get(detector)
                    .and_then(|mcb| mcb.preset_progress())
            {
                ui.add(
                    egui::ProgressBar::new(progress as f32)
                        .show_percentage()
                        .desired_height(10.0),
                );
            }
        }
        _ => {
            ui.label("none set");
        }
    }

    // Field mode: what the instrument itself is holding.
    if let Some(detector) = app.active_detector_index() {
        let stored = app.detectors[detector].stored_spectra();
        if stored > 0 {
            ui.separator();
            ui.horizontal(|ui| {
                ui.label(format!("Stored in instrument: {stored}"));
                if ui
                    .small_button("Download")
                    .on_hover_text("pull every stored spectrum into buffer windows")
                    .clicked()
                {
                    actions.push(Action::DownloadSpectra);
                }
            });
        }
    }

    ui.separator();
    ui.heading("Regions");
    // The regions of the active spectrum, with the energy and net area of each.
    // `at_marker` is the row the marker sits in: it is highlighted and kept in
    // view, so the list always answers "which region am I on?".
    let marker = app
        .active
        .and_then(|index| app.windows.get(index))
        .map(|window| window.display.marker);
    #[allow(clippy::type_complexity)]
    let (regions, at_marker, refreshed): (
        Vec<(usize, usize, usize, String)>,
        Option<usize>,
        Option<u64>,
    ) = {
        let settings = app.settings;
        let library = &app.library;
        match app.active_spectrum() {
            Some(spectrum) => {
                // A fingerprint of everything the rows depend on: recompute the
                // fits only when the data or the regions really changed, not
                // sixty-five times a second because a frame was drawn.
                let fingerprint = {
                    use std::hash::{Hash, Hasher};
                    let mut hasher = std::collections::hash_map::DefaultHasher::new();
                    app.active.hash(&mut hasher);
                    spectrum.total_counts().hash(&mut hasher);
                    spectrum.live_time.to_bits().hash(&mut hasher);
                    for roi in spectrum.rois.iter() {
                        roi.start.hash(&mut hasher);
                        roi.end.hash(&mut hasher);
                    }
                    // The rows also render through the calibration, the
                    // library and the calculation settings: calibrating used
                    // to leave every row saying "ch NNN" under a freshly
                    // drawn keV header, because none of these were part of
                    // the fingerprint.
                    if let Some(calibration) = &spectrum.energy_calibration {
                        for coefficient in calibration.coefficients {
                            coefficient.to_bits().hash(&mut hasher);
                        }
                        calibration.units.hash(&mut hasher);
                    }
                    library.len().hash(&mut hasher);
                    library.name.hash(&mut hasher);
                    settings.fw_x.hash(&mut hasher);
                    settings.sensitivity.hash(&mut hasher);
                    settings.background_points.hash(&mut hasher);
                    hasher.finish()
                };
                let (rows, refreshed) = match &app.dialogs.region_cache {
                    Some((cached, rows)) if *cached == fingerprint => (rows.clone(), None),
                    _ => {
                        let rows: Vec<(usize, usize, usize, String)> = spectrum
                            .rois
                            .iter()
                            .enumerate()
                            .map(|(index, roi)| {
                                let info = ortseam_core::peak_info(spectrum, *roi, &settings).ok();
                                // The marker jumps to the fitted centroid, and
                                // the name comes from it too - the region's
                                // midpoint can sit a channel or two off the
                                // peak, which is enough to name the wrong
                                // nuclide when two lines are close.
                                let centre = info
                                    .as_ref()
                                    .map(|info| info.centroid.round().max(0.0) as usize)
                                    .unwrap_or_else(|| roi.center().round() as usize);
                                let name = crate::view::nuclide_at(spectrum, Some(library), centre)
                                    .unwrap_or_default();
                                let label = match (&spectrum.energy_calibration, info) {
                                    (Some(cal), Some(info)) => format!(
                                        "{:>2} {:>7.1} {:<7} {:>8}",
                                        index + 1,
                                        info.centroid_energy(cal),
                                        name,
                                        format_counts(info.net_area.max(0.0) as u64)
                                    ),
                                    (None, Some(info)) => format!(
                                        "{:>2}  ch {:>6.1}  {:>9}",
                                        index + 1,
                                        info.centroid,
                                        format_counts(info.net_area.max(0.0) as u64)
                                    ),
                                    _ => format!("{:>2}  {}-{}", index + 1, roi.start, roi.end),
                                };
                                (centre, roi.start, roi.end, label)
                            })
                            .collect();
                        (rows, Some(fingerprint))
                    }
                };
                let current = marker
                    .and_then(|marker| spectrum.rois.iter().position(|roi| roi.contains(marker)));
                (rows, current, refreshed)
            }
            None => (Vec::new(), None, None),
        }
    };
    // Written outside the block: the cache cannot be updated while the
    // spectrum it fingerprints is still borrowed.
    if let Some(fingerprint) = refreshed {
        app.dialogs.region_cache = Some((fingerprint, regions.clone()));
    }
    if regions.is_empty() {
        ui.label(egui::RichText::new("none marked").weak());
    } else {
        if app
            .active_spectrum()
            .is_some_and(|spectrum| spectrum.energy_calibration.is_some())
        {
            ui.label(
                egui::RichText::new("      keV nuclide      net")
                    .weak()
                    .small()
                    .monospace(),
            );
        }
        egui::ScrollArea::vertical()
            .id_salt("region-list")
            .max_height(150.0)
            .show(ui, |ui| {
                for (index, (centre, start, end, label)) in regions.iter().enumerate() {
                    let here = at_marker == Some(index);
                    let text = if here {
                        egui::RichText::new(label)
                            .monospace()
                            .color(app.colors.roi.to_color())
                    } else {
                        egui::RichText::new(label).monospace()
                    };
                    let row = ui
                        .add(egui::Button::new(text).frame(false))
                        .on_hover_text("click: marker and peak info · double-click: zoom to it");
                    if row.double_clicked() {
                        // Zoom to the region with breathing room either side.
                        let width = (end - start).max(4);
                        actions.push(Action::Marker(*centre));
                        actions.push(Action::ZoomTo(
                            start.saturating_sub(width * 4),
                            end + width * 4,
                        ));
                        continue;
                    }
                    if here {
                        // A quiet spine, matching the peak-information card.
                        let spine = egui::Rect::from_min_size(
                            egui::Pos2::new(row.rect.left() - 6.0, row.rect.top() + 2.0),
                            egui::Vec2::new(2.5, row.rect.height() - 4.0),
                        );
                        ui.painter().rect_filled(
                            spine,
                            egui::CornerRadius::same(1),
                            app.colors.roi.to_color(),
                        );
                        ui.scroll_to_rect(row.rect, None);
                    }
                    if row.clicked() {
                        actions.push(Action::Marker(*centre));
                        actions.push(Action::PeakInfoAtMarker);
                    }
                }
            });
    }
    ui.horizontal(|ui| {
        if ui.button("◀").on_hover_text("previous region").clicked() {
            actions.extend(step_region(app, false));
        }
        ui.label("step");
        if ui.button("▶").on_hover_text("next region").clicked() {
            actions.extend(step_region(app, true));
        }
    });
    ui.horizontal(|ui| {
        if ui.button("Ins").on_hover_text("mark a peak here").clicked() {
            actions.push(Action::MarkPeak);
        }
        if ui
            .button("Del")
            .on_hover_text("clear this region")
            .clicked()
        {
            actions.push(Action::ClearRoi);
        }
        if ui.button("Info").clicked() {
            actions.push(Action::PeakInfoAtMarker);
        }
    });
    ui.horizontal(|ui| {
        if ui
            .button("<")
            .on_hover_text("previous library line")
            .clicked()
        {
            actions.extend(step_library(app, false));
        }
        ui.label("Library");
        if ui.button(">").on_hover_text("next library line").clicked() {
            actions.extend(step_library(app, true));
        }
    });

    #[cfg(feature = "simulator")]
    {
        ui.separator();
        ui.heading("Simulation");
        ui.horizontal(|ui| {
            ui.label("speed");
            ui.add(
                egui::Slider::new(&mut app.time_scale, 1.0..=200.0)
                    .logarithmic(true)
                    .suffix("x"),
            );
        });
        ui.label("simulated seconds per real second");
    }

    ui.with_layout(egui::Layout::bottom_up(egui::Align::Center), |ui| {
        ui.add_space(6.0);
        ui.label(egui::RichText::new("ortseam").weak());
        ui.label(
            egui::RichText::new(
                chrono::Local::now()
                    .format("%H:%M:%S  %a %d/%m/%Y")
                    .to_string(),
            )
            .monospace(),
        );
    });
    actions
}

fn step_region(app: &App, forward: bool) -> Vec<Action> {
    let Some(spectrum) = app.active_spectrum() else {
        return Vec::new();
    };
    let marker = app
        .active
        .and_then(|index| app.windows.get(index))
        .map(|window| window.display.marker)
        .unwrap_or(0);
    let region = if forward {
        spectrum.rois.next_after(marker)
    } else {
        spectrum.rois.previous_before(marker)
    };
    match region {
        Some(roi) => vec![
            Action::Marker(roi.center().round() as usize),
            Action::Status(format!("region {}-{}", roi.start, roi.end)),
        ],
        None => vec![Action::Status("no more entries".into())],
    }
}

fn step_library(app: &App, forward: bool) -> Vec<Action> {
    let Some(spectrum) = app.active_spectrum() else {
        return Vec::new();
    };
    let Some(cal) = spectrum.energy_calibration.as_ref() else {
        return vec![Action::Status(
            "calibrate first to step through the library".into(),
        )];
    };
    let marker = app
        .active
        .and_then(|index| app.windows.get(index))
        .map(|window| window.display.marker)
        .unwrap_or(0);
    let here = cal.energy(marker as f64);
    let mut hits: Vec<(f64, String)> = app
        .library
        .iter()
        .flat_map(|nuclide| {
            nuclide
                .peaks
                .iter()
                .map(move |peak| (peak.energy, nuclide.name.clone()))
        })
        .collect();
    hits.sort_by(|a, b| a.0.partial_cmp(&b.0).unwrap_or(std::cmp::Ordering::Equal));
    let found = if forward {
        hits.into_iter().find(|(energy, _)| *energy > here + 0.01)
    } else {
        hits.into_iter()
            .rev()
            .find(|(energy, _)| *energy < here - 0.01)
    };
    match found {
        Some((energy, name)) => match cal.channel(energy) {
            Some(channel) => vec![
                Action::Marker(channel.max(0.0) as usize),
                Action::Status(format!("{name} at {energy:.2} {}", cal.units)),
            ],
            None => vec![Action::Status("that line is off scale".into())],
        },
        None => vec![Action::Status("no more entries".into())],
    }
}

/// Draws every open dialog.
pub fn windows(app: &mut App, ctx: &egui::Context) -> Vec<Action> {
    let mut actions = Vec::new();
    mcb_properties(app, ctx, &mut actions);
    settings_dialog(app, ctx);
    calibration_dialog(app, ctx, &mut actions);
    roi_report_options(app, ctx, &mut actions);
    report_viewer(app, ctx);
    analysis_dialog(app, ctx, &mut actions);
    efficiency_dialog(app, ctx, &mut actions);
    library_dialog(app, ctx, &mut actions);
    job_control(app, ctx, &mut actions);
    detector_list_dialog(app, ctx, &mut actions);
    instruments_dialog(app, ctx, &mut actions);
    sample_dialog(app, ctx);
    lock_dialog(app, ctx, &mut actions);
    strip_dialog(app, ctx, &mut actions);
    list_range_dialog(app, ctx, &mut actions);
    preferences_dialog(app, ctx);
    dashboard_dialog(app, ctx, &mut actions);
    qa_dialog(app, ctx);
    batch_dialog(app, ctx, &mut actions);
    insight_dialog(app, ctx, &mut actions);
    shortcuts_dialog(app, ctx);
    about_dialog(app, ctx);
    confirm_close_dialog(app, ctx, &mut actions);
    recover_dialog(app, ctx);
    exit_dialog(app, ctx);
    open_path_dialog(app, ctx, &mut actions);
    actions
}

/// Acquire/InSight: a virtual oscilloscope on the simulated preamplifier.
///
/// The trace is what the amplifier settings really imply - trapezoid shaping
/// from the rise time and flat top, an exponential tail when the pole zero is
/// away from where this detector's preamp needs it - so turning the knobs
/// teaches the same lesson the real screen does, and Optimize visibly flattens
/// the tail.
fn insight_dialog(app: &mut App, ctx: &egui::Context, actions: &mut Vec<Action>) {
    dialog_window(app, ctx, Dialog::Insight, "InSight", |app, ui| {
        let Some(detector) = app.active_detector_index() else {
            ui.label("open a detector window first");
            return;
        };
        let tuning = app.detectors[detector].optimizing() || app.detectors[detector].pole_zeroing();
        let mut amplifier = app.detectors[detector].properties().amplifier;
        // The instrument reports its own analog behaviour; the display draws it.
        let pz_error = app.detectors[detector].pole_zero_error();

        // The trace.
        let (rect, _) = ui.allocate_exact_size(egui::vec2(520.0, 220.0), egui::Sense::hover());
        let painter = ui.painter_at(rect);
        painter.rect_filled(
            rect,
            egui::CornerRadius::same(3),
            app.colors.background.to_color(),
        );
        // Graticule.
        for step in 1..8 {
            let x = rect.left() + rect.width() * step as f32 / 8.0;
            painter.line_segment(
                [
                    egui::Pos2::new(x, rect.top()),
                    egui::Pos2::new(x, rect.bottom()),
                ],
                egui::Stroke::new(1.0, app.colors.axes.with_alpha(0.15)),
            );
        }
        for step in 1..4 {
            let y = rect.top() + rect.height() * step as f32 / 4.0;
            painter.line_segment(
                [
                    egui::Pos2::new(rect.left(), y),
                    egui::Pos2::new(rect.right(), y),
                ],
                egui::Stroke::new(1.0, app.colors.axes.with_alpha(0.15)),
            );
        }
        let baseline = rect.bottom() - rect.height() * 0.25;
        painter.line_segment(
            [
                egui::Pos2::new(rect.left(), baseline),
                egui::Pos2::new(rect.right(), baseline),
            ],
            egui::Stroke::new(1.0, app.colors.axes.with_alpha(0.5)),
        );

        // Two pulses of different heights across a 40 us window, plus noise.
        // The pole-zero error decides the tail after each pulse; Optimize and
        // the auto pole zero drive it to zero.
        let window_us = 40.0;
        let rise = amplifier.rise_time_us.max(0.1);
        let flat = amplifier.flat_top_us.max(0.05);
        let points: Vec<egui::Pos2> = (0..(rect.width() as usize))
            .map(|column| {
                let t = window_us * column as f64 / rect.width() as f64;
                let mut level = 0.0f64;
                for (start, height) in [(4.0, 0.9), (22.0, 0.55)] {
                    let dt = t - start;
                    if dt >= 0.0 {
                        let shaped = if dt < rise {
                            dt / rise
                        } else if dt < rise + flat {
                            1.0
                        } else if dt < rise * 2.0 + flat {
                            1.0 - (dt - rise - flat) / rise
                        } else {
                            0.0
                        };
                        level += height * shaped;
                        // The tail after the pulse: a smooth undershoot when the
                        // pole zero is under-compensated, overshoot when over,
                        // rising over a couple of microseconds and dying with
                        // the preamp's ~45 us time constant.
                        let fall_end = rise * 2.0 + flat;
                        if dt > fall_end {
                            let since = dt - fall_end;
                            let envelope = (-since / 45.0).exp() - (-since / 2.0).exp();
                            level += height * 1.5 * pz_error.clamp(-0.4, 0.4) * envelope;
                        }
                    }
                }
                // Deterministic noise, wiggled by the frame so it looks alive.
                let wiggle = ui.ctx().cumulative_frame_nr() as f64;
                let noise =
                    ((column as f64 * 12.9898 + wiggle * 78.233).sin() * 43758.5453).fract();
                level += (noise - 0.5) * 0.015;
                egui::Pos2::new(
                    rect.left() + column as f32,
                    baseline - (level as f32) * rect.height() * 0.6,
                )
            })
            .collect();
        painter.add(egui::Shape::line(
            points,
            egui::Stroke::new(1.4, app.colors.foreground.to_color()),
        ));
        painter.text(
            egui::Pos2::new(rect.left() + 8.0, rect.top() + 6.0),
            egui::Align2::LEFT_TOP,
            format!(
                "{:.1} us/div   pole zero {}{}",
                window_us / 8.0,
                amplifier.pole_zero,
                if tuning {
                    "   [auto routine running]"
                } else {
                    ""
                }
            ),
            egui::FontId::monospace(10.0),
            app.colors.axes.to_color(),
        );
        ui.ctx()
            .request_repaint_after(std::time::Duration::from_millis(120));

        // The knobs, in a grid so the dialog stays no wider than the trace.
        let mut changed = false;
        egui::Grid::new("insight-knobs")
            .num_columns(2)
            .min_col_width(70.0)
            .show(ui, |ui| {
                ui.label("Rise time");
                changed |= ui
                    .add(egui::Slider::new(&mut amplifier.rise_time_us, 0.8..=12.0).suffix(" us"))
                    .changed();
                ui.end_row();
                ui.label("Flat top");
                changed |= ui
                    .add(egui::Slider::new(&mut amplifier.flat_top_us, 0.3..=2.4).suffix(" us"))
                    .changed();
                ui.end_row();
                ui.label("Pole zero");
                let mut pole_zero = amplifier.pole_zero as i32;
                if ui
                    .add(egui::Slider::new(&mut pole_zero, 0..=4095))
                    .changed()
                {
                    amplifier.pole_zero = pole_zero as u16;
                    changed = true;
                }
                ui.end_row();
            });
        ui.horizontal(|ui| {
            if ui
                .add_enabled(!tuning, egui::Button::new("Start Auto PZ"))
                .clicked()
            {
                actions.push(Action::StartPoleZero);
            }
            if ui
                .add_enabled(!tuning, egui::Button::new("Optimize"))
                .on_hover_text("the full automatic optimisation")
                .clicked()
            {
                actions.push(Action::StartOptimize);
            }
        });
        if changed {
            let mut properties = app.detectors[detector].properties().clone();
            properties.amplifier = amplifier;
            if let Err(error) = app.detectors[detector].set_properties(properties) {
                actions.push(Action::Status(error.to_string()));
            }
        }
        ui.label(
            egui::RichText::new(
                "a flat baseline after the pulse means the pole zero is right; \
                 a dip is overcompensated, a bump under",
            )
            .weak()
            .small(),
        );
    });
}

/// Help/Keyboard Commands: everything the keyboard does, in one place.
fn shortcuts_dialog(app: &mut App, ctx: &egui::Context) {
    dialog_window(app, ctx, Dialog::Shortcuts, "Keyboard Commands", |_, ui| {
        const GROUPS: [(&str, &[(&str, &str)]); 4] = [
            (
                "Acquire",
                &[
                    ("Alt+1", "Start"),
                    ("Alt+2", "Stop"),
                    ("Alt+3", "Clear"),
                    ("Alt+5", "Copy to buffer"),
                ],
            ),
            (
                "Display",
                &[
                    ("\u{2190} \u{2192}", "Move the marker (Shift: ten channels)"),
                    ("Home End", "Marker to the first or last channel"),
                    ("PgUp PgDn", "Scroll half a window"),
                    ("+ \u{2212}", "Zoom in and out"),
                    ("Wheel", "Zoom about the pointer (Shift: scroll)"),
                    ("Mid-drag", "Pan the view"),
                    ("Overview", "Click jumps; drag pans; double-click resets"),
                    ("/", "Logarithmic or linear"),
                    ("A", "Automatic vertical scale"),
                    ("5", "Centre the marker"),
                    ("Ctrl+M", "One window fills the area, and back"),
                    ("Ctrl+Tab", "Next window (Shift: previous)"),
                    ("Ctrl+F1-12", "Select detector 1-12"),
                ],
            ),
            (
                "Regions and peaks",
                &[
                    ("F2", "Marking mode: off, mark, unmark"),
                    ("Ins", "Mark a region at the marker"),
                    ("Del", "Clear the region under the marker"),
                    (
                        "Esc",
                        "Close dialogs; then peak info, comparison, selection",
                    ),
                    ("Ctrl+C", "Copy the open peak card's text"),
                    ("Shift+\u{2191}\u{2193}", "Offset the comparison trace"),
                ],
            ),
            (
                "Files and history",
                &[
                    ("Ctrl+O", "Recall a spectrum"),
                    ("Ctrl+P", "Print via the browser"),
                    ("Ctrl+S", "Save As (Shift: overwrite in place)"),
                    ("Ctrl+Z", "Undo (Shift or Ctrl+Y: Redo)"),
                    ("F1", "This list"),
                ],
            ),
        ];
        ui.horizontal_top(|ui| {
            for chunk in GROUPS.chunks(2) {
                ui.vertical(|ui| {
                    for (title, keys) in chunk {
                        ui.strong(*title);
                        ui.add_space(2.0);
                        egui::Grid::new(*title).min_col_width(72.0).show(ui, |ui| {
                            for (key, what) in *keys {
                                ui.monospace(*key);
                                ui.label(*what);
                                ui.end_row();
                            }
                        });
                        ui.add_space(8.0);
                    }
                });
                ui.add_space(18.0);
            }
        });
        ui.label("A drag selects; the right mouse button opens the command menu.");
    });
}

fn dialog_window<R>(
    app: &mut App,
    ctx: &egui::Context,
    dialog: Dialog,
    title: &str,
    contents: impl FnOnce(&mut App, &mut egui::Ui) -> R,
) {
    if !app.dialogs.flag(dialog) {
        return;
    }
    let mut open = true;
    // Each dialog gets its own default spot, staggered, so opening two does
    // not stack them exactly on top of each other.
    let position = Dialog::all()
        .iter()
        .position(|candidate| *candidate == dialog)
        .unwrap_or(0);
    let offset = (position % 6) as f32 * 48.0;
    egui::Window::new(title)
        .open(&mut open)
        .resizable(true)
        .collapsible(false)
        // Dialogs float above the spectrum windows. Without this they share a
        // layer with them, so clicking a spectrum - which is exactly what
        // calibrating asks you to do - buries the dialog behind it.
        .order(egui::Order::Foreground)
        .default_pos(egui::Pos2::new(150.0 + offset, 100.0 + offset * 0.6))
        .show(ctx, |ui| {
            contents(app, ui);
        });
    app.dialogs.set(dialog, open);
}

fn mcb_properties(app: &mut App, ctx: &egui::Context, actions: &mut Vec<Action>) {
    dialog_window(
        app,
        ctx,
        Dialog::McbProperties,
        "MCB Properties",
        |app, ui| {
            let Some(index) = app.active else {
                ui.label("open a detector window first");
                return;
            };
            let Target::Detector(detector) = app.windows[index].target else {
                ui.label("the buffer has no instrument settings");
                return;
            };
            let Some(mcb) = app.detectors.get_mut(detector) else {
                return;
            };
            let mut properties = mcb.properties().clone();
            let identity = mcb.identity().clone();
            let status = mcb.status();
            let active = mcb.is_active();

            ui.horizontal(|ui| {
                for (index, label) in [
                    (0, "Amplifier"),
                    (1, "ADC"),
                    (2, "Presets"),
                    (3, "High Voltage"),
                    (4, "Stabilizer"),
                    (5, "About"),
                ] {
                    if ui
                        .selectable_label(app.dialogs.properties_tab == index, label)
                        .clicked()
                    {
                        app.dialogs.properties_tab = index;
                    }
                }
            });
            ui.separator();

            let mut changed = false;
            match app.dialogs.properties_tab {
                0 => {
                    egui::Grid::new("amp").num_columns(2).show(ui, |ui| {
                        ui.label("Coarse gain");
                        changed |= ui
                            .add(
                                egui::DragValue::new(&mut properties.amplifier.coarse_gain)
                                    .range(1.0..=1000.0),
                            )
                            .changed();
                        ui.end_row();
                        ui.label("Fine gain");
                        changed |= ui
                            .add(
                                egui::DragValue::new(&mut properties.amplifier.fine_gain)
                                    .speed(0.01)
                                    .range(0.4..=1.0),
                            )
                            .changed();
                        ui.end_row();
                        ui.label("Total gain");
                        ui.label(format!("{:.3}", properties.amplifier.gain()));
                        ui.end_row();
                        ui.label("Rise time (us)");
                        changed |= ui
                            .add(
                                egui::DragValue::new(&mut properties.amplifier.rise_time_us)
                                    .range(0.2..=23.0)
                                    .speed(0.1),
                            )
                            .changed();
                        ui.end_row();
                        ui.label("Flat top (us)");
                        changed |= ui
                            .add(
                                egui::DragValue::new(&mut properties.amplifier.flat_top_us)
                                    .range(0.3..=2.4)
                                    .speed(0.1),
                            )
                            .changed();
                        ui.end_row();
                        ui.label("Pole zero");
                        changed |= ui
                            .add(
                                egui::DragValue::new(&mut properties.amplifier.pole_zero)
                                    .range(0..=4095),
                            )
                            .changed();
                        ui.end_row();
                    });
                    changed |= ui
                        .checkbox(
                            &mut properties.amplifier.pileup_rejection,
                            "Pile-up rejection",
                        )
                        .changed();
                    ui.label(
                        "Rise time sets the trade-off between resolution and throughput (Table 2).",
                    );
                }
                1 => {
                    egui::Grid::new("adc").num_columns(2).show(ui, |ui| {
                        ui.label("Conversion gain");
                        let mut gain = properties.adc.conversion_gain;
                        egui::ComboBox::from_id_salt("conversion-gain")
                            .selected_text(gain.to_string())
                            .show_ui(ui, |ui| {
                                for option in [256usize, 512, 1024, 2048, 4096, 8192, 16384] {
                                    if option <= identity.channels
                                        && ui
                                            .selectable_value(&mut gain, option, option.to_string())
                                            .clicked()
                                    {
                                        changed = true;
                                    }
                                }
                            });
                        properties.adc.conversion_gain = gain;
                        ui.end_row();
                        ui.label("Lower discriminator");
                        changed |= ui
                            .add(
                                egui::DragValue::new(&mut properties.adc.lower_discriminator)
                                    .range(0..=identity.channels as u32),
                            )
                            .changed();
                        ui.end_row();
                        ui.label("Upper discriminator");
                        changed |= ui
                            .add(
                                egui::DragValue::new(&mut properties.adc.upper_discriminator)
                                    .range(0..=identity.channels as u32),
                            )
                            .changed();
                        ui.end_row();
                    });
                }
                2 => {
                    if active {
                        ui.label(
                            egui::RichText::new("stop the acquisition to change presets").weak(),
                        );
                    }
                    // The edit in progress, seeded from the instrument the
                    // first time this tab is drawn for it. Switching detectors
                    // seeds afresh, so one instrument's presets are never
                    // typed into another's.
                    let mut presets = match app.dialogs.presets_edit {
                        Some((owner, presets)) if owner == detector => presets,
                        _ => properties.presets,
                    };
                    let edit = |ui: &mut egui::Ui, label: &str, value: &mut Option<f64>| {
                        ui.label(label);
                        let mut enabled = value.is_some();
                        if ui.checkbox(&mut enabled, "").changed() {
                            *value = enabled.then_some(0.0);
                        }
                        let mut number = value.unwrap_or(0.0);
                        if ui
                            .add_enabled(
                                enabled,
                                egui::DragValue::new(&mut number).range(0.0..=1e9),
                            )
                            .changed()
                        {
                            *value = Some(number);
                        }
                        ui.end_row();
                    };
                    egui::Grid::new("presets-edit")
                        .num_columns(3)
                        .show(ui, |ui| {
                            edit(ui, "Real time (s)", &mut presets.real_time);
                            edit(ui, "Live time (s)", &mut presets.live_time);
                            ui.label("ROI peak");
                            let mut peak_on = presets.roi_peak.is_some();
                            if ui.checkbox(&mut peak_on, "").changed() {
                                presets.roi_peak = peak_on.then_some(0);
                            }
                            let mut peak = presets.roi_peak.unwrap_or(0);
                            if ui
                                .add_enabled(peak_on, egui::DragValue::new(&mut peak))
                                .changed()
                            {
                                presets.roi_peak = Some(peak);
                            }
                            ui.end_row();
                            ui.label("ROI integral");
                            let mut integral_on = presets.roi_integral.is_some();
                            if ui.checkbox(&mut integral_on, "").changed() {
                                presets.roi_integral = integral_on.then_some(0);
                            }
                            let mut integral = presets.roi_integral.unwrap_or(0);
                            if ui
                                .add_enabled(integral_on, egui::DragValue::new(&mut integral))
                                .changed()
                            {
                                presets.roi_integral = Some(integral);
                            }
                            ui.end_row();
                        });
                    ui.separator();
                    let mut uncertainty_on = presets.uncertainty.is_some();
                    if ui
                        .checkbox(&mut uncertainty_on, "Uncertainty preset")
                        .changed()
                    {
                        presets.uncertainty = uncertainty_on.then_some(UncertaintyPreset {
                            limit_percent: 1.0,
                            low_channel: 0,
                            high_channel: 100,
                        });
                    }
                    if let Some(preset) = presets.uncertainty.as_mut() {
                        ui.horizontal(|ui| {
                            ui.label("limit %");
                            ui.add(egui::DragValue::new(&mut preset.limit_percent).speed(0.1));
                            ui.label("channels");
                            ui.add(egui::DragValue::new(&mut preset.low_channel));
                            ui.add(egui::DragValue::new(&mut preset.high_channel));
                        });
                    }
                    let mut mda_on = presets.mda.is_some();
                    if ui.checkbox(&mut mda_on, "MDA preset").changed() {
                        presets.mda = mda_on.then_some(MdaPreset {
                            target: 10.0,
                            region: ortseam_core::Roi::new(0, 100),
                            efficiency: 0.05,
                            yield_fraction: 0.85,
                        });
                    }
                    if let Some(preset) = presets.mda.as_mut() {
                        ui.horizontal(|ui| {
                            ui.label("target Bq");
                            ui.add(egui::DragValue::new(&mut preset.target).speed(0.5));
                            ui.label("efficiency");
                            ui.add(egui::DragValue::new(&mut preset.efficiency).speed(0.001));
                            ui.label("yield");
                            ui.add(egui::DragValue::new(&mut preset.yield_fraction).speed(0.01));
                        });
                    }
                    ui.horizontal(|ui| {
                        if ui.button("Apply presets").clicked() {
                            match app.detectors[detector].set_presets(presets) {
                                Ok(()) => {
                                    // What the instrument now holds is no
                                    // longer an unsaved edit.
                                    app.dialogs.presets_edit = None;
                                    actions.push(Action::Status("presets applied".into()))
                                }
                                Err(error) => actions.push(Action::Status(error.to_string())),
                            }
                        }
                        if ui.button("Clear presets").clicked() {
                            let _ = app.detectors[detector].set_presets(Presets::default());
                            app.dialogs.presets_edit = None;
                            actions.push(Action::Status("presets cleared".into()));
                        }
                    });
                    // Whatever has been typed so far survives to the next
                    // frame; only Apply sends it to the instrument.
                    if app
                        .detectors
                        .get(detector)
                        .is_some_and(|mcb| mcb.properties().presets != presets)
                    {
                        app.dialogs.presets_edit = Some((detector, presets));
                    } else {
                        app.dialogs.presets_edit = None;
                    }
                    return;
                }
                3 => {
                    egui::Grid::new("hv").num_columns(2).show(ui, |ui| {
                        ui.label("Target (V)");
                        changed |= ui
                            .add(
                                egui::DragValue::new(&mut properties.high_voltage.target_volts)
                                    .range(0.0..=5000.0),
                            )
                            .changed();
                        ui.end_row();
                        ui.label("Enabled");
                        changed |= ui
                            .checkbox(&mut properties.high_voltage.enabled, "")
                            .changed();
                        ui.end_row();
                        ui.label("Measured");
                        ui.label(format!("{:.0} V", status.high_voltage));
                        ui.end_row();
                    });
                }
                4 => {
                    changed |= ui
                        .checkbox(&mut properties.stabilizer.gain_enabled, "Gain stabiliser")
                        .changed();
                    changed |= ui
                        .checkbox(&mut properties.stabilizer.zero_enabled, "Zero stabiliser")
                        .changed();
                    egui::Grid::new("stab").num_columns(2).show(ui, |ui| {
                        ui.label("Gain peak channel");
                        changed |= ui
                            .add(egui::DragValue::new(
                                &mut properties.stabilizer.gain_channel,
                            ))
                            .changed();
                        ui.end_row();
                        ui.label("Gain window");
                        changed |= ui
                            .add(egui::DragValue::new(&mut properties.stabilizer.gain_width))
                            .changed();
                        ui.end_row();
                    });
                }
                _ => {
                    egui::Grid::new("about").num_columns(2).show(ui, |ui| {
                        for (label, value) in [
                            ("Detector", identity.number.to_string()),
                            ("Name", identity.name.clone()),
                            ("Model", identity.model.clone()),
                            ("Serial", identity.serial.clone()),
                            ("Firmware", identity.firmware.clone()),
                            ("Channels", identity.channels.to_string()),
                            ("Real time", format!("{:.2} s", status.real_time)),
                            ("Live time", format!("{:.2} s", status.live_time)),
                            ("Dead time", format!("{:.2} %", status.dead_time_percent)),
                            ("Input rate", format!("{:.0} cps", status.input_count_rate)),
                            ("Stored rate", format!("{:.0} cps", status.total_count_rate)),
                            (
                                "Counting",
                                if status.active {
                                    "yes".into()
                                } else {
                                    "no".to_string()
                                },
                            ),
                        ] {
                            ui.label(label);
                            ui.label(value);
                            ui.end_row();
                        }
                    });
                    ui.label("Capabilities:");
                    let capabilities = identity.capabilities;
                    ui.label(format!(
                        "list mode {}, ZDT {}, sample changer {}, bias {}, stabiliser {}",
                        capabilities.list_mode,
                        capabilities.zdt,
                        capabilities.sample_changer,
                        capabilities.high_voltage,
                        capabilities.stabilizer
                    ));
                }
            }

            if changed {
                match app.detectors[detector].set_properties(properties) {
                    Ok(()) => actions.push(Action::Status("settings applied".into())),
                    Err(error) => actions.push(Action::Status(error.to_string())),
                }
            }
        },
    );
}

fn settings_dialog(app: &mut App, ctx: &egui::Context) {
    dialog_window(
        app,
        ctx,
        Dialog::Settings,
        "Calculation Settings",
        |app, ui| {
            let mut settings = app.settings;
            egui::Grid::new("calc").num_columns(2).show(ui, |ui| {
                ui.label("\"x\" for FW(1/x)M");
                let mut fw_x = settings.fw_x;
                if ui
                    .add(
                        egui::DragValue::new(&mut fw_x)
                            .range(CalculationSettings::MIN_FW_X..=CalculationSettings::MAX_FW_X),
                    )
                    .changed()
                {
                    settings = settings.with_fw_x(fw_x);
                }
                ui.end_row();
                ui.label("Sensitivity factor");
                let mut sensitivity = settings.sensitivity;
                if ui
                    .add(egui::Slider::new(&mut sensitivity, 1..=5))
                    .on_hover_text("1 finds the most peaks, 5 only the strongest")
                    .changed()
                {
                    settings = settings.with_sensitivity(sensitivity);
                }
                ui.end_row();
                ui.label("Background points");
                let mut points = settings.background_points;
                if ui.add(egui::Slider::new(&mut points, 1..=5)).changed() {
                    settings = settings.with_background_points(points);
                }
                ui.end_row();
            });
            app.settings = settings;
            ui.label("Background points are used at each end of a region (equations 17-21).");
        },
    );
}

fn calibration_dialog(app: &mut App, ctx: &egui::Context, actions: &mut Vec<Action>) {
    dialog_window(app, ctx, Dialog::Calibration, "Calibrate", |app, ui| {
        let Some(index) = app.active else {
            ui.label("open a spectrum first");
            return;
        };
        let points = app.windows[index].calibration.points().to_vec();
        let fit = app.windows[index].calibration.calibration().cloned();
        // Exactly what pressing Enter will use, so there is no guessing at
        // whether the region's peak or the bare marker is about to be entered.
        let (channel, from_roi) = app.calibration_channel().unwrap_or((0.0, false));

        ui.horizontal(|ui| {
            if from_roi {
                ui.label(format!("Peak in channel: {channel:.2}"))
                    .on_hover_text("the centroid of the region under the marker");
            } else {
                ui.label(format!("Marker channel: {channel:.0}"))
                    .on_hover_text(
                        "no region under the marker, so the marker channel itself \
                         is used - mark the peak with a region for a better figure",
                    );
            }
            ui.label("Calibration (Energy):");
            ui.add(
                egui::TextEdit::singleline(&mut app.dialogs.calibration_energy).desired_width(90.0),
            );
            if ui.button("Enter").clicked() {
                match app.dialogs.calibration_energy.trim().parse::<f64>() {
                    Ok(energy) => {
                        actions.push(Action::AddCalibrationPoint(energy));
                        app.dialogs.calibration_energy.clear();
                    }
                    Err(_) => actions.push(Action::Status("enter a number for the energy".into())),
                }
            }
        });
        ui.horizontal(|ui| {
            ui.label("Units");
            ui.add(
                egui::TextEdit::singleline(&mut app.dialogs.calibration_units).desired_width(60.0),
            );
            if ui.button("Destroy Calibration").clicked() {
                actions.push(Action::DestroyCalibration);
            }
            if ui.button("Recall from file...").clicked() {
                actions.push(Action::RecallCalibration);
            }
        });
        ui.separator();
        // The whole procedure for a known source: pick it, press the button.
        ui.horizontal(|ui| {
            ui.label("From peaks:");
            let names: Vec<String> = app
                .library
                .iter()
                .map(|nuclide| nuclide.name.clone())
                .collect();
            if app.dialogs.auto_nuclide.is_empty() {
                app.dialogs.auto_nuclide = names.first().cloned().unwrap_or_default();
            }
            egui::ComboBox::from_id_salt("auto-cal-nuclide")
                .selected_text(app.dialogs.auto_nuclide.clone())
                .show_ui(ui, |ui| {
                    for name in &names {
                        if ui
                            .selectable_label(*name == app.dialogs.auto_nuclide, name)
                            .clicked()
                        {
                            app.dialogs.auto_nuclide = name.clone();
                        }
                    }
                });
            if ui
                .button("Calibrate")
                .on_hover_text(
                    "search for peaks and fit the calibration that lines them up \
                     with this nuclide's energies",
                )
                .clicked()
            {
                actions.push(Action::AutoCalibrate(app.dialogs.auto_nuclide.clone()));
            }
        });
        ui.separator();
        // A line needs two points. Saying so here is the difference between
        // "it did not work" and "it is not finished yet".
        match (points.len(), fit.is_some()) {
            (0, _) => {
                ui.label("no points yet - put the marker on a peak and enter its energy");
            }
            (1, false) => {
                ui.label(
                    egui::RichText::new(
                        "1 point entered - a second peak is needed before the \
                         spectrum is calibrated",
                    )
                    .strong(),
                );
            }
            (count, false) => {
                ui.label(
                    egui::RichText::new(format!(
                        "{count} points entered, but they do not fit - two \
                         different channels are needed"
                    ))
                    .strong(),
                );
            }
            (count, true) => {
                ui.label(format!("{count} point(s) entered"));
            }
        }
        ui.label(
            egui::RichText::new(
                "Type in a channel or an energy to correct it - the fit follows.                  The last column is how far the fit misses that point by; a number                  much larger than the others is the point to look at.",
            )
            .weak()
            .small(),
        );
        // The table is edited in place rather than rebuilt: a mistyped energy
        // is one digit wrong, and starting a whole calibration again over it is
        // the kind of thing that makes people keep a paper notebook.
        let edits = &mut app.dialogs.calibration_edits;
        edits.resize(points.len(), (String::new(), String::new()));
        egui::ScrollArea::vertical()
            .max_height(200.0)
            .show(ui, |ui| {
                egui::Grid::new("cal-points")
                    .num_columns(4)
                    .striped(true)
                    .show(ui, |ui| {
                        for header in ["channel", "energy", "miss", ""] {
                            ui.label(egui::RichText::new(header).strong());
                        }
                        ui.end_row();
                        for (row, point) in points.iter().enumerate() {
                            // An untouched box shows the stored value; once it
                            // has been typed in, what was typed stays until it
                            // is applied or the dialog is closed.
                            let (channel_text, value_text) = &mut edits[row];
                            if channel_text.is_empty() {
                                *channel_text = format!("{:.2}", point.channel);
                            }
                            if value_text.is_empty() {
                                *value_text = format!("{:.3}", point.value);
                            }
                            let channel = ui
                                .add(egui::TextEdit::singleline(channel_text).desired_width(70.0));
                            let value =
                                ui.add(egui::TextEdit::singleline(value_text).desired_width(80.0));
                            let typed = channel.lost_focus() || value.lost_focus();
                            let parsed = (
                                channel_text.trim().parse::<f64>(),
                                value_text.trim().parse::<f64>(),
                            );
                            match &fit {
                                Some(fit) => {
                                    let miss = fit.energy(point.channel) - point.value;
                                    ui.monospace(format!("{miss:+.3}"));
                                }
                                None => {
                                    ui.monospace("-");
                                }
                            }
                            if ui
                                .button("remove")
                                .on_hover_text("drop this point and fit without it")
                                .clicked()
                            {
                                actions.push(Action::RemoveCalibrationPoint(row));
                            }
                            ui.end_row();
                            if typed {
                                match parsed {
                                    (Ok(new_channel), Ok(new_value)) => {
                                        if new_channel != point.channel || new_value != point.value
                                        {
                                            actions.push(Action::EditCalibrationPoint(
                                                row,
                                                new_channel,
                                                new_value,
                                            ));
                                        }
                                    }
                                    _ => actions.push(Action::Status(
                                        "a calibration point needs a number in both boxes".into(),
                                    )),
                                }
                            }
                        }
                    });
            });
        if let Some(fit) = fit {
            ui.separator();
            ui.monospace(format!(
                "{:.6} + {:.6} ch + {:.4e} ch^2  {}",
                fit.coefficients[0], fit.coefficients[1], fit.coefficients[2], fit.units
            ));
            ui.label(if fit.order() >= 2 {
                "quadratic fit"
            } else {
                "linear fit"
            });
        } else {
            ui.label("two points give a linear fit, three or more a quadratic one");
        }
    });
}

fn roi_report_options(app: &mut App, ctx: &egui::Context, actions: &mut Vec<Action>) {
    dialog_window(
        app,
        ctx,
        Dialog::RoiReportOptions,
        "ROI Report",
        |app, ui| {
            ui.horizontal(|ui| {
                ui.label("Format");
                ui.radio_value(&mut app.dialogs.report_columns, false, "Paragraph");
                ui.radio_value(&mut app.dialogs.report_columns, true, "Column");
            });
            ui.checkbox(
                &mut app.dialogs.report_displayed_only,
                "Only the regions in the expanded view",
            );
            ui.separator();
            ui.horizontal(|ui| {
                if ui.button("Show report").clicked() {
                    actions.push(Action::RoiReport);
                }
                if ui.button("Peak search first").clicked() {
                    actions.push(Action::PeakSearch);
                    actions.push(Action::RoiReport);
                }
            });
        },
    );
}

fn report_viewer(app: &mut App, ctx: &egui::Context) {
    if !app.dialogs.is_open(Dialog::Report) {
        return;
    }
    let mut open = true;
    let text = app.report_text.clone().unwrap_or_default();
    egui::Window::new("Report")
        .open(&mut open)
        // Above the spectrum windows, like every dialog_window - a maximized
        // plot would otherwise bury it on the first click.
        .order(egui::Order::Foreground)
        .default_size([720.0, 460.0])
        .show(ctx, |ui| {
            ui.horizontal(|ui| {
                if ui.button("Copy").clicked() {
                    ui.ctx().copy_text(text.clone());
                }
                if ui.button("Save as...").clicked()
                    && let Some(path) = pick_save_file(&[("Report", &["rpt", "txt"])])
                {
                    let _ = std::fs::write(path, &text);
                }
            });
            ui.separator();
            egui::ScrollArea::both().show(ui, |ui| {
                ui.monospace(&text);
            });
        });
    app.dialogs.set(Dialog::Report, open);
}

/// The centroid of a nuclide's strongest measured line, in channels.
///
/// This is where the analysis table jumps when a nuclide's name is clicked:
/// the line with the largest net area is the one worth looking at.
pub(crate) fn strongest_centroid(nuclide: &ortseam_core::NuclideResult) -> Option<f64> {
    nuclide
        .lines
        .iter()
        .max_by(|a, b| {
            a.net_area
                .partial_cmp(&b.net_area)
                .unwrap_or(std::cmp::Ordering::Equal)
        })
        .map(|line| line.centroid)
}

fn analysis_dialog(app: &mut App, ctx: &egui::Context, actions: &mut Vec<Action>) {
    dialog_window(app, ctx, Dialog::Analysis, "Nuclide analysis", |app, ui| {
        ui.horizontal(|ui| {
            ui.label("Sample quantity");
            ui.add(egui::DragValue::new(&mut app.sample.quantity).speed(0.1));
            ui.add(egui::TextEdit::singleline(&mut app.sample.unit).desired_width(60.0));
            ui.checkbox(
                &mut app.dialogs.report_mda,
                "detection limits for absent nuclides",
            );
        });
        ui.horizontal(|ui| {
            ui.checkbox(
                &mut app.dialogs.correct_during_count,
                "correct for decay during the count",
            );
            ui.add_enabled_ui(app.sample.reference_date.is_some(), |ui| {
                ui.checkbox(
                    &mut app.dialogs.correct_to_reference,
                    "decay-correct to the reference date",
                )
                .on_disabled_hover_text(
                    "set the reference date in Services/Sample Description first",
                );
            });
        });
        ui.horizontal(|ui| {
            if ui.button("Analyse").clicked() {
                actions.push(Action::Analyse);
            }
            if ui.button("Efficiency...").clicked() {
                actions.push(Action::Show(Dialog::Efficiency));
            }
            if ui.button("Export CSV...").clicked()
                && let (Some(report), Some(path)) =
                    (app.analysis.as_ref(), pick_save_file(&[("CSV", &["csv"])]))
            {
                let _ = std::fs::write(path, ortseam_report::results_csv(report));
            }
        });
        ui.separator();
        match &app.analysis {
            Some(report) => {
                ui.label(format!(
                    "activities in {}; {} region(s) analysed",
                    report.activity_unit(),
                    report.peaks.len()
                ));
                egui::ScrollArea::vertical()
                    .max_height(320.0)
                    .show(ui, |ui| {
                        egui::Grid::new("nuclides")
                            .num_columns(5)
                            .striped(true)
                            .show(ui, |ui| {
                                for header in ["nuclide", "activity", "+/-", "MDA", "status"] {
                                    ui.label(egui::RichText::new(header).strong());
                                }
                                ui.end_row();
                                for nuclide in &report.nuclides {
                                    // The name is a link: clicking it jumps the
                                    // display to the nuclide's strongest
                                    // measured line and opens its peak card.
                                    match strongest_centroid(nuclide) {
                                        Some(centroid) => {
                                            if ui
                                                .link(&nuclide.nuclide)
                                                .on_hover_text(
                                                    "jump to this nuclide's strongest line",
                                                )
                                                .clicked()
                                            {
                                                let centre = centroid.round().max(0.0) as usize;
                                                actions.push(Action::Marker(centre));
                                                actions.push(Action::ZoomTo(
                                                    centre.saturating_sub(60),
                                                    centre + 60,
                                                ));
                                                actions.push(Action::PeakInfoAtMarker);
                                            }
                                        }
                                        None => {
                                            ui.label(&nuclide.nuclide);
                                        }
                                    }
                                    // A nuclide that was not detected shows dim
                                    // numbers, and an MDA that was never
                                    // computed shows a dash rather than a
                                    // misleading 0.000.
                                    let number = |text: String| {
                                        if nuclide.detected {
                                            egui::RichText::new(text)
                                        } else {
                                            egui::RichText::new(text).weak()
                                        }
                                    };
                                    ui.label(number(format!("{:.3}", nuclide.activity)));
                                    ui.label(number(format!("{:.3}", nuclide.uncertainty)));
                                    ui.label(if nuclide.mda > 0.0 {
                                        number(format!("{:.3}", nuclide.mda))
                                    } else {
                                        egui::RichText::new("\u{2014}").weak()
                                    });
                                    ui.label(if nuclide.detected {
                                        egui::RichText::new("detected")
                                            .color(egui::Color32::LIGHT_GREEN)
                                    } else {
                                        egui::RichText::new("not detected").weak()
                                    });
                                    ui.end_row();
                                }
                            });
                    });
            }
            None => {
                ui.label("run an analysis to see nuclides and activities");
            }
        }
    });
}

fn efficiency_dialog(app: &mut App, ctx: &egui::Context, actions: &mut Vec<Action>) {
    dialog_window(
        app,
        ctx,
        Dialog::Efficiency,
        "Efficiency calibration",
        |app, ui| {
            ui.label(
                "Without an efficiency curve, results are count rates rather than activities.",
            );
            ui.horizontal(|ui| {
                ui.label("Constant efficiency");
                ui.add(
                    egui::DragValue::new(&mut app.dialogs.efficiency_constant)
                        .speed(0.001)
                        .range(0.0..=1.0),
                );
                if ui.button("Use").clicked() {
                    app.efficiency = Some(EfficiencyCalibration::constant(
                        app.dialogs.efficiency_constant,
                    ));
                    actions.push(Action::Status("constant efficiency in force".into()));
                }
            });
            ui.separator();
            // The way curves are really made: a certified source's spectrum in
            // the active window, the certificate's numbers typed in, one press.
            ui.label(egui::RichText::new("From a certified source").strong());
            ui.label(
                egui::RichText::new(
                    "with the source's spectrum in the active window: one point per \
                     library line found, decay-corrected to the measurement",
                )
                .weak()
                .small(),
            );
            ui.horizontal(|ui| {
                let names: Vec<String> = app
                    .library
                    .iter()
                    .map(|nuclide| nuclide.name.clone())
                    .collect();
                if app.dialogs.source_nuclide.is_empty() {
                    app.dialogs.source_nuclide = names.first().cloned().unwrap_or_default();
                }
                egui::ComboBox::from_id_salt("eff-source-nuclide")
                    .selected_text(app.dialogs.source_nuclide.clone())
                    .show_ui(ui, |ui| {
                        for name in &names {
                            if ui
                                .selectable_label(*name == app.dialogs.source_nuclide, name)
                                .clicked()
                            {
                                app.dialogs.source_nuclide = name.clone();
                            }
                        }
                    });
                ui.add(
                    egui::DragValue::new(&mut app.dialogs.source_activity_bq)
                        .speed(10.0)
                        .range(0.0..=1e12)
                        .suffix(" Bq"),
                );
                ui.label("±");
                ui.add(
                    egui::DragValue::new(&mut app.dialogs.source_uncertainty_pct)
                        .speed(0.1)
                        .range(0.0..=50.0)
                        .suffix(" %"),
                );
            });
            ui.horizontal(|ui| {
                ui.label("certificate age");
                ui.add(
                    egui::DragValue::new(&mut app.dialogs.source_age_days)
                        .speed(1.0)
                        .range(0.0..=100_000.0)
                        .suffix(" days"),
                );
                if ui
                    .button("Measure")
                    .on_hover_text(
                        "search for peaks if none are marked, measure each library \
                         line's rate, and fill the point table below",
                    )
                    .clicked()
                {
                    let nuclide = app.dialogs.source_nuclide.clone();
                    let activity = app.dialogs.source_activity_bq;
                    let uncertainty = app.dialogs.source_uncertainty_pct;
                    let age = app.dialogs.source_age_days;
                    match app.efficiency_points_from_source(&nuclide, activity, uncertainty, age) {
                        Ok(measured) => {
                            app.dialogs.efficiency_points = measured
                                .iter()
                                .map(|point| {
                                    (
                                        format!("{:.1}", point.energy),
                                        format!("{:.6}", point.efficiency),
                                    )
                                })
                                .collect();
                            let order = (measured.len() - 1).min(3);
                            app.dialogs.efficiency_order = order.max(1);
                            match EfficiencyCalibration::fit_log_log(&measured, order.max(1)) {
                                Ok(curve) => {
                                    app.efficiency = Some(curve);
                                    actions.push(Action::Status(format!(
                                        "efficiency measured from {} lines of {nuclide} and fitted",
                                        measured.len()
                                    )));
                                }
                                Err(error) => actions.push(Action::Status(error.to_string())),
                            }
                        }
                        Err(error) => actions.push(Action::Status(error)),
                    }
                }
            });
            ui.separator();
            ui.label("Points (energy in keV, absolute efficiency as a fraction)");
            let mut remove = None;
            for (index, (energy, efficiency)) in
                app.dialogs.efficiency_points.iter_mut().enumerate()
            {
                ui.horizontal(|ui| {
                    ui.add(egui::TextEdit::singleline(energy).desired_width(70.0));
                    ui.add(egui::TextEdit::singleline(efficiency).desired_width(70.0));
                    if ui.button("x").clicked() {
                        remove = Some(index);
                    }
                });
            }
            if let Some(index) = remove {
                app.dialogs.efficiency_points.remove(index);
            }
            ui.horizontal(|ui| {
                if ui.button("Add point").clicked() {
                    app.dialogs
                        .efficiency_points
                        .push((String::new(), String::new()));
                }
                ui.label("fit order");
                ui.add(egui::Slider::new(&mut app.dialogs.efficiency_order, 1..=4));
            });
            let points: Vec<EfficiencyPoint> = app
                .dialogs
                .efficiency_points
                .iter()
                .filter_map(|(energy, efficiency)| {
                    Some(EfficiencyPoint::new(
                        energy.trim().parse().ok()?,
                        efficiency.trim().parse().ok()?,
                    ))
                })
                .collect();
            ui.horizontal(|ui| {
                if ui.button("Interpolate through points").clicked() {
                    app.efficiency = Some(EfficiencyCalibration::interpolated(points.clone()));
                    actions.push(Action::Status("interpolated efficiency in force".into()));
                }
                if ui.button("Fit log-log polynomial").clicked() {
                    match EfficiencyCalibration::fit_log_log(&points, app.dialogs.efficiency_order)
                    {
                        Ok(curve) => {
                            app.efficiency = Some(curve);
                            actions.push(Action::Status("efficiency curve fitted".into()));
                        }
                        Err(error) => actions.push(Action::Status(error.to_string())),
                    }
                }
            });
            if let Some(curve) = &app.efficiency {
                ui.separator();
                // The curve, drawn as every efficiency curve is read: log-log,
                // with the entered points on top of the fit.
                let (response, painter) = ui.allocate_painter(
                    egui::vec2(ui.available_width().min(440.0), 150.0),
                    egui::Sense::hover(),
                );
                let rect = response.rect.shrink(1.0);
                painter.rect_filled(
                    rect,
                    egui::CornerRadius::same(3),
                    app.colors.background.to_color(),
                );
                let (e_low, e_high) = (20.0f64, 3000.0f64);
                let samples: Vec<(f64, f64)> = (0..=120)
                    .map(|step| {
                        let energy = e_low * (e_high / e_low).powf(step as f64 / 120.0);
                        (energy, curve.efficiency(energy).max(1e-9))
                    })
                    .collect();
                let (eff_low, eff_high) = samples
                    .iter()
                    .fold((f64::MAX, f64::MIN), |(low, high), (_, efficiency)| {
                        (low.min(*efficiency), high.max(*efficiency))
                    });
                let (eff_low, eff_high) = (eff_low / 1.5, (eff_high * 1.5).max(eff_low * 2.0));
                let x_of = |energy: f64| {
                    rect.left()
                        + rect.width() * ((energy / e_low).ln() / (e_high / e_low).ln()) as f32
                };
                let y_of = |efficiency: f64| {
                    rect.bottom()
                        - rect.height()
                            * ((efficiency / eff_low).ln() / (eff_high / eff_low).ln()) as f32
                };
                // Decade gridlines on both axes.
                for decade in [100.0, 1000.0] {
                    painter.line_segment(
                        [
                            egui::Pos2::new(x_of(decade), rect.top()),
                            egui::Pos2::new(x_of(decade), rect.bottom()),
                        ],
                        egui::Stroke::new(1.0, app.colors.axes.with_alpha(0.2)),
                    );
                    painter.text(
                        egui::Pos2::new(x_of(decade), rect.bottom() - 2.0),
                        egui::Align2::CENTER_BOTTOM,
                        format!("{decade:.0}"),
                        egui::FontId::monospace(9.0),
                        app.colors.axes.with_alpha(0.8),
                    );
                }
                let curve_points: Vec<egui::Pos2> = samples
                    .iter()
                    .map(|(energy, efficiency)| egui::Pos2::new(x_of(*energy), y_of(*efficiency)))
                    .collect();
                painter.add(egui::Shape::line(
                    curve_points,
                    egui::Stroke::new(1.6, app.colors.foreground.to_color()),
                ));
                for point in &points {
                    if point.energy > 0.0 && point.efficiency > 0.0 {
                        painter.circle_filled(
                            egui::Pos2::new(x_of(point.energy), y_of(point.efficiency)),
                            3.0,
                            app.colors.marker.to_color(),
                        );
                    }
                }
                painter.text(
                    egui::Pos2::new(rect.left() + 6.0, rect.top() + 4.0),
                    egui::Align2::LEFT_TOP,
                    "efficiency vs energy (log-log)",
                    egui::FontId::monospace(9.0),
                    app.colors.axes.with_alpha(0.85),
                );

                egui::Grid::new("eff-curve").num_columns(2).show(ui, |ui| {
                    for energy in [59.5, 122.1, 356.0, 661.7, 1173.2, 1332.5, 1460.8] {
                        ui.label(format!("{energy:.1} keV"));
                        ui.label(format!("{:.5}", curve.efficiency(energy)));
                        ui.end_row();
                    }
                });
            }
        },
    );
}

fn library_dialog(app: &mut App, ctx: &egui::Context, actions: &mut Vec<Action>) {
    dialog_window(app, ctx, Dialog::Library, "Nuclide library", |app, ui| {
        ui.horizontal(|ui| {
            ui.label(format!(
                "{} ({} nuclides)",
                if app.library.name.is_empty() {
                    "library".into()
                } else {
                    app.library.name.clone()
                },
                app.library.len()
            ));
            if ui.button("Load...").clicked() {
                actions.push(Action::LoadLibrary);
            }
            if ui.button("Save as...").clicked()
                && let Some(path) = pick_save_file(&[("Library", &["json", "csv"])])
            {
                match ortseam_formats::library::save(&app.library, &path) {
                    Ok(()) => actions.push(Action::Status(format!("wrote {}", path.display()))),
                    Err(error) => actions.push(Action::Status(error.to_string())),
                }
            }
            if ui.button("Sort by energy").clicked() {
                app.library.sort_by_energy();
            }
            if ui.button("Sort by name").clicked() {
                app.library.sort_by_name();
            }
        });
        ui.separator();
        ui.horizontal_top(|ui| {
            egui::ScrollArea::vertical()
                .id_salt("nuclide-list")
                .max_height(280.0)
                .show(ui, |ui| {
                    ui.set_min_width(140.0);
                    for index in 0..app.library.len() {
                        let name = app.library.nuclides[index].name.clone();
                        if ui
                            .selectable_label(app.dialogs.library_selection == Some(index), name)
                            .clicked()
                        {
                            app.dialogs.library_selection = Some(index);
                        }
                    }
                    if ui.button("+ nuclide").clicked() {
                        app.library.push(Nuclide::new(
                            "New",
                            0.0,
                            vec![LibraryPeak::new(100.0, 100.0)],
                        ));
                        app.dialogs.library_selection = Some(app.library.len() - 1);
                    }
                });
            ui.separator();
            ui.vertical(|ui| {
                let Some(selected) = app.dialogs.library_selection else {
                    ui.label("select a nuclide");
                    return;
                };
                if selected >= app.library.len() {
                    app.dialogs.library_selection = None;
                    return;
                }
                let nuclide = &mut app.library.nuclides[selected];
                ui.horizontal(|ui| {
                    ui.label("Name");
                    ui.add(egui::TextEdit::singleline(&mut nuclide.name).desired_width(100.0));
                    ui.label("Half life (s)");
                    ui.add(egui::DragValue::new(&mut nuclide.half_life_seconds).speed(100.0));
                });
                ui.label(format!("= {}", nuclide.half_life_display()));
                ui.separator();
                egui::Grid::new("peaks")
                    .num_columns(4)
                    .striped(true)
                    .show(ui, |ui| {
                        ui.label(egui::RichText::new("energy").strong());
                        ui.label(egui::RichText::new("yield %").strong());
                        ui.label(egui::RichText::new("key").strong());
                        ui.label("");
                        ui.end_row();
                        let mut remove = None;
                        for (index, peak) in nuclide.peaks.iter_mut().enumerate() {
                            ui.add(egui::DragValue::new(&mut peak.energy).speed(0.1));
                            ui.add(egui::DragValue::new(&mut peak.yield_percent).speed(0.1));
                            ui.checkbox(&mut peak.key_line, "");
                            if ui.button("x").clicked() {
                                remove = Some(index);
                            }
                            ui.end_row();
                        }
                        if let Some(index) = remove {
                            nuclide.peaks.remove(index);
                        }
                    });
                if ui.button("+ line").clicked() {
                    nuclide.peaks.push(LibraryPeak::new(100.0, 10.0));
                }
                if ui.button("Delete nuclide").clicked() {
                    app.library.cut(selected);
                    app.dialogs.library_selection = None;
                }
            });
        });
    });
}

fn job_control(app: &mut App, ctx: &egui::Context, actions: &mut Vec<Action>) {
    dialog_window(app, ctx, Dialog::JobControl, "Run JOB File", |app, ui| {
        ui.horizontal(|ui| {
            ui.label("Directory");
            ui.add(egui::TextEdit::singleline(&mut app.dialogs.job_directory).desired_width(220.0));
            if ui.button("Browse...").clicked()
                && let Some(path) = pick_open_file(&[("Job file", &["job"])])
            {
                app.dialogs.job_file = Some(path.clone());
                if let Some(parent) = path.parent() {
                    app.dialogs.job_directory = parent.display().to_string();
                }
            }
        });
        let files: Vec<PathBuf> = std::fs::read_dir(&app.dialogs.job_directory)
            .map(|entries| {
                entries
                    .flatten()
                    .map(|entry| entry.path())
                    .filter(|path| {
                        path.extension()
                            .is_some_and(|extension| extension.eq_ignore_ascii_case("job"))
                    })
                    .collect()
            })
            .unwrap_or_default();
        egui::ScrollArea::vertical()
            .max_height(140.0)
            .show(ui, |ui| {
                for path in &files {
                    let name = path
                        .file_name()
                        .map(|name| name.to_string_lossy().to_string())
                        .unwrap_or_default();
                    if ui
                        .selectable_label(app.dialogs.job_file.as_ref() == Some(path), name)
                        .clicked()
                    {
                        app.dialogs.job_file = Some(path.clone());
                    }
                }
                if files.is_empty() {
                    ui.label("no .JOB files in that directory");
                }
            });
        if let Some(path) = app.dialogs.job_file.clone()
            && let Ok(text) = std::fs::read_to_string(&path)
        {
            ui.separator();
            ui.label("Contents:");
            egui::ScrollArea::vertical()
                .id_salt("job-text")
                .max_height(160.0)
                .show(ui, |ui| {
                    ui.monospace(text);
                });
        }
        ui.separator();
        ui.horizontal(|ui| {
            let running = app.job.is_some();
            if ui
                .add_enabled(
                    !running && app.dialogs.job_file.is_some(),
                    egui::Button::new("Run"),
                )
                .clicked()
                && let Some(path) = app.dialogs.job_file.clone()
            {
                match crate::jobs::start(app, &path) {
                    Ok(()) => actions.push(Action::Status(format!("running {}", path.display()))),
                    Err(error) => actions.push(Action::Status(error)),
                }
            }
            if ui
                .add_enabled(running, egui::Button::new("Terminate"))
                .clicked()
            {
                app.job = None;
                actions.push(Action::Status("job terminated".into()));
            }
            if let Some(job) = &app.job {
                ui.label(format!("line {} of {}", job.position(), job.len()));
            }
        });
    });
}

/// Instrument setup, in plain words.
///
/// This is the one dialog somebody meets before they know anything about the
/// program, so it says what the situation is, what each button will do, and
/// what to try if it does not work - rather than assuming any of it is
/// obvious. Nothing here needs a manual.
fn instruments_dialog(app: &mut App, ctx: &egui::Context, actions: &mut Vec<Action>) {
    dialog_window(
        app,
        ctx,
        Dialog::Instruments,
        "Set up instruments",
        |app, ui| {
            let scan = app.instruments.clone();

            ui.label("An instrument has to be found before it can be counted with.");
            ui.add_space(6.0);

            if let Some(problem) = &scan.problem {
                ui.colored_label(ui.visuals().warn_fg_color, problem);
                ui.add_space(6.0);
            }

            ui.label(egui::RichText::new("What is connected right now").strong());
            if scan.reachable.is_empty() {
                ui.label(egui::RichText::new("  nothing answered").weak());
            }
            for (number, name) in &scan.reachable {
                ui.label(format!("  {number}: {name} - ready"));
            }

            if !scan.missing.is_empty() {
                ui.add_space(6.0);
                ui.label(egui::RichText::new("Listed, but not answering").strong());
                for (number, name) in &scan.missing {
                    ui.colored_label(ui.visuals().warn_fg_color, format!("  {number}: {name}"));
                }
                ui.add_space(4.0);
                ui.label(
                    egui::RichText::new(
                        "This usually means the instrument is switched off or unplugged, or that \
                     the list was written on a different computer. Instruments are numbered in \
                     the order they are found, so a list from elsewhere can point at the wrong \
                     one. \"Rebuild the list\" fixes that.",
                    )
                    .weak(),
                );
            }

            ui.add_space(10.0);
            ui.separator();
            ui.add_space(6.0);

            ui.horizontal(|ui| {
                if ui.button("Look again").clicked() {
                    actions.push(Action::ScanLocalInstruments);
                }
                ui.label(egui::RichText::new("check what is connected now").weak());
            });
            ui.horizontal(|ui| {
                if ui.button("Rebuild the list").clicked() {
                    actions.push(Action::ConfigureLocalInstruments);
                }
                ui.label(
                    egui::RichText::new("forget the old list and number what is here, from one")
                        .weak(),
                );
            });
            ui.horizontal(|ui| {
                let any = !scan.reachable.is_empty();
                if ui
                    .add_enabled(any, egui::Button::new("Open them"))
                    .clicked()
                {
                    actions.push(Action::OpenLocalInstruments);
                }
                ui.label(egui::RichText::new("show a window for each one that answered").weak());
            });

            ui.add_space(10.0);
            ui.collapsing("It still cannot find anything", |ui| {
                ui.label(
                    "Things to check, in the order worth checking them:\n\
                 \n\
                 1. The instrument is switched on and its cable is in at both ends.\n\
                 2. Unplug the cable and plug it back in. An instrument that was \
                 interrupted mid-conversation stays silent until it is power-cycled.\n\
                 3. Windows can see it: Device Manager should list it without a \
                 warning triangle. If it has one, its driver is not installed.\n\
                 4. Press \"Look again\" after any of the above.",
                );
                ui.add_space(6.0);
                ui.label(
                    egui::RichText::new(
                        "If the instrument's software lives somewhere unusual, name that folder \
                     here. Leave it empty otherwise - it is found automatically.",
                    )
                    .weak(),
                );
                ui.add(
                    egui::TextEdit::singleline(&mut app.dialogs.local_umcbi)
                        .hint_text("only if asked to")
                        .desired_width(380.0),
                );
            });

            if !app.dialogs.local_scan.is_empty() {
                ui.collapsing("What the search reported", |ui| {
                    egui::ScrollArea::vertical()
                        .max_height(200.0)
                        .id_salt("instrument-scan-detail")
                        .show(ui, |ui| {
                            ui.monospace(&app.dialogs.local_scan);
                        });
                });
            }
        },
    );
}

fn detector_list_dialog(app: &mut App, ctx: &egui::Context, actions: &mut Vec<Action>) {
    dialog_window(
        app,
        ctx,
        Dialog::DetectorList,
        "Detector List",
        |app, ui| {
            ui.label(format!("pick list \"{}\"", app.detector_list.name));
            egui::Grid::new("detectors")
                .num_columns(5)
                .striped(true)
                .show(ui, |ui| {
                    for header in ["#", "name", "model", "channels", ""] {
                        ui.label(egui::RichText::new(header).strong());
                    }
                    ui.end_row();
                    let entries = app.detector_list.entries().to_vec();
                    for entry in entries {
                        ui.label(entry.number.to_string());
                        let position = app
                            .detectors
                            .iter()
                            .position(|mcb| mcb.identity().number == entry.number);
                        // The name is the operator's, not the instrument's:
                        // "Bench HPGe" beats the model and serial the scan
                        // printed, and it is what a saved spectrum carries.
                        // Typed over several frames, so what is being typed
                        // lives in the dialog until it is committed.
                        let typing = match &mut app.dialogs.detector_name_edit {
                            Some((number, text)) if *number == entry.number => text,
                            slot => {
                                *slot = Some((entry.number, entry.name.clone()));
                                &mut slot.as_mut().expect("just set").1
                            }
                        };
                        let editor = ui.add(
                            egui::TextEdit::singleline(typing)
                                .desired_width(140.0)
                                .hint_text("name"),
                        );
                        let committed = editor.lost_focus()
                            && ui.input(|input| input.key_pressed(egui::Key::Enter));
                        if committed || (editor.lost_focus() && *typing != entry.name) {
                            let typed = typing.clone();
                            app.dialogs.detector_name_edit = None;
                            if let Some(index) = position {
                                actions.push(Action::RenameDetector(index, typed));
                            }
                        }
                        ui.label(&entry.model);
                        ui.label(entry.channels.to_string());
                        ui.horizontal(|ui| {
                            if ui.button("open").clicked()
                                && let Some(index) = position
                            {
                                actions.push(Action::OpenDetector(index));
                            }
                            if ui
                                .button("remove")
                                .on_hover_text(
                                    "take it out of the list; its windows close \
                                     (a running count refuses)",
                                )
                                .clicked()
                                && let Some(index) = position
                            {
                                actions.push(Action::RemoveDetector(index));
                            }
                        });
                        ui.end_row();
                    }
                });
            #[cfg(feature = "simulator")]
            {
                ui.separator();
                ui.label("Add a simulated detector (test builds only)");
                ui.horizontal(|ui| {
                    ui.add(
                        egui::DragValue::new(&mut app.dialogs.new_detector.number)
                            .range(1..=999)
                            .prefix("#"),
                    );
                    ui.add(
                        egui::TextEdit::singleline(&mut app.dialogs.new_detector.name)
                            .desired_width(90.0),
                    );
                    egui::ComboBox::from_id_salt("new-detector-channels")
                        .selected_text(app.dialogs.new_detector.channels.to_string())
                        .show_ui(ui, |ui| {
                            for option in [1024usize, 2048, 4096, 8192, 16384] {
                                ui.selectable_value(
                                    &mut app.dialogs.new_detector.channels,
                                    option,
                                    option.to_string(),
                                );
                            }
                        });
                    if ui.button("Add").clicked() {
                        let mut entry = app.dialogs.new_detector.clone();
                        entry.kind = DetectorKind::Simulated;
                        app.add_detector(entry);
                        app.dialogs.new_detector.number += 1;
                        app.dialogs.new_detector.name =
                            format!("SIM-{:02}", app.dialogs.new_detector.number);
                    }
                });
            }
            ui.separator();
            ui.label("Add a local instrument");
            ui.label(
                egui::RichText::new(
                    "ORTEC hardware on this machine - USB, DPM or a serial port - reached \
                     through the bridge. The number is the one ORTEC's MCB Configuration \
                     gave it.",
                )
                .weak()
                .small(),
            );
            ui.horizontal(|ui| {
                ui.add(
                    egui::TextEdit::singleline(&mut app.dialogs.local_name)
                        .hint_text("name")
                        .desired_width(90.0),
                );
                ui.add(
                    egui::DragValue::new(&mut app.dialogs.local_detector)
                        .range(1..=999)
                        .prefix("detector #"),
                );
                if ui.button("Open").clicked() {
                    let name = if app.dialogs.local_name.trim().is_empty() {
                        format!("MCB-{}", app.dialogs.local_detector)
                    } else {
                        app.dialogs.local_name.trim().to_string()
                    };
                    let mut description = app.dialogs.local_detector.to_string();
                    if !app.dialogs.local_umcbi.trim().is_empty() {
                        description.push(';');
                        description.push_str(app.dialogs.local_umcbi.trim());
                    }
                    app.add_detector(DetectorEntry {
                        number: app.dialogs.new_detector.number,
                        name,
                        model: "local".into(),
                        kind: DetectorKind::Local,
                        channels: 0,
                        description,
                        // Learned from the instrument on the first open.
                        serial: String::new(),
                    });
                    app.dialogs.new_detector.number += 1;
                }
                if ui.button("Scan").clicked() {
                    actions.push(Action::ScanLocalInstruments);
                }
            });
            ui.add(
                egui::TextEdit::singleline(&mut app.dialogs.local_umcbi)
                    .hint_text("ORTEC library directory (only if not installed system-wide)")
                    .desired_width(360.0),
            );
            if !app.dialogs.local_scan.is_empty() {
                egui::ScrollArea::vertical()
                    .max_height(140.0)
                    .id_salt("local-scan")
                    .show(ui, |ui| {
                        ui.monospace(&app.dialogs.local_scan);
                    });
            }
            ui.separator();
            ui.label("Add a network instrument");
            ui.label(
                egui::RichText::new(
                    "an MCB served over TCP - another ortseam's simulator, a gateway, \
                     or bench hardware speaking the SET_/SHOW_ dialect",
                )
                .weak()
                .small(),
            );
            ui.horizontal(|ui| {
                ui.add(
                    egui::TextEdit::singleline(&mut app.dialogs.network_name)
                        .hint_text("name")
                        .desired_width(90.0),
                );
                ui.add(
                    egui::TextEdit::singleline(&mut app.dialogs.network_address)
                        .hint_text("host:port")
                        .desired_width(150.0),
                );
                if ui.button("Connect").clicked() {
                    let name = if app.dialogs.network_name.trim().is_empty() {
                        "REMOTE".to_string()
                    } else {
                        app.dialogs.network_name.trim().to_string()
                    };
                    let entry = DetectorEntry {
                        number: app.dialogs.new_detector.number,
                        name,
                        model: "remote".into(),
                        kind: DetectorKind::Network,
                        channels: 0,
                        description: app.dialogs.network_address.trim().to_string(),
                        serial: String::new(),
                    };
                    app.add_detector(entry);
                    app.dialogs.new_detector.number += 1;
                }
            });
        },
    );
}

fn sample_dialog(app: &mut App, ctx: &egui::Context) {
    dialog_window(app, ctx, Dialog::Sample, "Sample Description", |app, ui| {
        ui.label("Saved with the spectrum and printed on reports (63 characters in .Chn).");
        ui.add(
            egui::TextEdit::multiline(&mut app.sample.description)
                .desired_rows(3)
                .desired_width(360.0),
        );
        ui.horizontal(|ui| {
            ui.label("Quantity");
            ui.add(egui::DragValue::new(&mut app.sample.quantity).speed(0.1));
            ui.add(egui::TextEdit::singleline(&mut app.sample.unit).desired_width(60.0));
        });
        ui.horizontal(|ui| {
            ui.label("Reference date");
            let field = ui.add(
                egui::TextEdit::singleline(&mut app.dialogs.reference_input)
                    .hint_text("YYYY-MM-DD [HH:MM]")
                    .desired_width(150.0),
            );
            if field.lost_focus() {
                let text = app.dialogs.reference_input.clone();
                match app.set_reference_date(&text) {
                    Ok(()) => {}
                    Err(error) => app.status = error,
                }
            }
            match app.sample.reference_date {
                Some(date) => {
                    ui.label(
                        egui::RichText::new(format!("set: {}", date.format("%Y-%m-%d %H:%M")))
                            .weak()
                            .small(),
                    );
                }
                None => {
                    ui.label(
                        egui::RichText::new("activities decay-correct to this date")
                            .weak()
                            .small(),
                    );
                }
            }
        });
        if ui.button("Apply to the active spectrum").clicked() {
            let description = app.sample.description.clone();
            if let Some(spectrum) = app.active_spectrum_mut() {
                spectrum.sample_description = description;
            }
        }
    });
}

fn lock_dialog(app: &mut App, ctx: &egui::Context, actions: &mut Vec<Action>) {
    dialog_window(
        app,
        ctx,
        Dialog::Lock,
        "Lock / Unlock Detector",
        |app, ui| {
            ui.horizontal(|ui| {
                ui.label("Password");
                ui.add(
                    egui::TextEdit::singleline(&mut app.dialogs.lock_password)
                        .password(true)
                        .desired_width(140.0),
                );
            });
            ui.horizontal(|ui| {
                ui.label("Owner");
                ui.add(
                    egui::TextEdit::singleline(&mut app.dialogs.lock_owner).desired_width(140.0),
                );
            });
            let Some(index) = app.active else { return };
            let Target::Detector(detector) = app.windows[index].target else {
                ui.label("select a detector window");
                return;
            };
            let locked = app.detectors[detector].is_locked();
            ui.horizontal(|ui| {
                if ui.add_enabled(!locked, egui::Button::new("Lock")).clicked() {
                    let (password, owner) = (
                        app.dialogs.lock_password.clone(),
                        app.dialogs.lock_owner.clone(),
                    );
                    match app.detectors[detector].lock(&password, &owner) {
                        Ok(()) => actions.push(Action::Status("detector locked".into())),
                        Err(error) => actions.push(Action::Status(error.to_string())),
                    }
                }
                if ui
                    .add_enabled(locked, egui::Button::new("Unlock"))
                    .clicked()
                {
                    let password = app.dialogs.lock_password.clone();
                    match app.detectors[detector].unlock(&password) {
                        Ok(()) => actions.push(Action::Status("detector unlocked".into())),
                        Err(error) => actions.push(Action::Status(error.to_string())),
                    }
                }
            });
            ui.label(if locked { "locked" } else { "unlocked" });
        },
    );
}

fn strip_dialog(app: &mut App, ctx: &egui::Context, actions: &mut Vec<Action>) {
    dialog_window(app, ctx, Dialog::Strip, "Strip Spectrum", |app, ui| {
        ui.horizontal(|ui| {
            ui.label("File");
            ui.add(egui::TextEdit::singleline(&mut app.dialogs.strip_path).desired_width(240.0));
            if ui.button("Browse...").clicked()
                && let Some(path) = pick_open_file(&[("Spectrum", &["chn", "spc", "spe", "json"])])
            {
                app.dialogs.strip_path = path.display().to_string();
            }
        });
        ui.checkbox(&mut app.dialogs.strip_use_ratio, "Use ratio of live times");
        ui.add_enabled(
            !app.dialogs.strip_use_ratio,
            egui::DragValue::new(&mut app.dialogs.strip_factor).speed(0.01),
        );
        ui.label("A negative factor adds the spectra instead of subtracting them.");
        if ui.button("Strip").clicked() {
            let path = PathBuf::from(app.dialogs.strip_path.clone());
            let factor = (!app.dialogs.strip_use_ratio).then_some(app.dialogs.strip_factor);
            // Queued actions land after strip_from's own status: claiming
            // success here would paper over its failure message.
            if app.strip_from(path, factor) {
                actions.push(Action::Status("stripped; Edit/Undo takes it back".into()));
            }
        }
    });
}

fn list_range_dialog(app: &mut App, ctx: &egui::Context, actions: &mut Vec<Action>) {
    dialog_window(
        app,
        ctx,
        Dialog::ListRange,
        "List Mode Data Range",
        |app, ui| {
            let Some(index) = app.active else { return };
            let Some(list) = app.windows[index].list.clone() else {
                ui.label("this window holds no list-mode data");
                return;
            };
            ui.label(format!(
                "{} events over {:.2} s",
                list.len(),
                list.real_time
            ));
            ui.horizontal(|ui| {
                ui.label("Start (s)");
                ui.add(egui::DragValue::new(&mut app.dialogs.list_start).speed(1.0));
                ui.label("Duration (s)");
                ui.add(egui::DragValue::new(&mut app.dialogs.list_duration).speed(1.0));
            });
            ui.horizontal(|ui| {
                if ui.button("Apply").clicked() {
                    let range = ortseam_formats::ListRange::from_seconds(
                        app.dialogs.list_start,
                        app.dialogs.list_duration,
                    );
                    if let Some(slice) = list.slice(range) {
                        let title = format!(
                            "Slice {:.0}-{:.0} s",
                            range.start,
                            range.start + range.duration
                        );
                        app.open_buffer(title, slice, None);
                        actions.push(Action::Status("time slice opened in a buffer".into()));
                    }
                }
                if ui.button("Increment").clicked() {
                    app.dialogs.list_duration += app.dialogs.list_duration.max(1.0);
                }
                if ui.button("Restore").clicked() {
                    let whole = list.to_spectrum();
                    app.open_buffer("List data".into(), whole, None);
                }
            });

            // Replay: scrub through the acquisition, or let it play. The
            // window's own spectrum follows the slice, so the peaks grow and
            // shift exactly as they did while the events were recorded.
            ui.separator();
            ui.label(egui::RichText::new("Replay").strong());
            let latest = (list.real_time - app.dialogs.list_duration).max(0.0);
            let mut scrubbed = false;
            ui.horizontal(|ui| {
                // ▶ is in egui's fonts; the pause glyph is not, so words do
                // the work.
                let label = if app.dialogs.list_playing {
                    "Pause"
                } else {
                    "\u{25b6} Play"
                };
                if ui.button(label).clicked() {
                    app.dialogs.list_playing = !app.dialogs.list_playing;
                    // Play from the top when the scrubber is already at the end.
                    if app.dialogs.list_playing && app.dialogs.list_start >= latest {
                        app.dialogs.list_start = 0.0;
                    }
                }
                ui.label("speed");
                ui.add(
                    egui::Slider::new(&mut app.dialogs.list_speed, 1.0..=50.0)
                        .logarithmic(true)
                        .suffix("x"),
                );
            });
            scrubbed |= ui
                .add(
                    egui::Slider::new(&mut app.dialogs.list_start, 0.0..=latest.max(0.001))
                        .suffix(" s")
                        .text("window start"),
                )
                .changed();
            if app.dialogs.list_playing {
                let dt = ui.input(|input| input.stable_dt).min(0.1) as f64;
                app.dialogs.list_start += dt * app.dialogs.list_speed;
                if app.dialogs.list_start >= latest {
                    app.dialogs.list_start = latest;
                    app.dialogs.list_playing = false;
                }
                scrubbed = true;
                ui.ctx().request_repaint();
            }
            if scrubbed {
                let range = ortseam_formats::ListRange::from_seconds(
                    app.dialogs.list_start,
                    app.dialogs.list_duration,
                );
                if let Some(mut slice) = list.slice(range) {
                    // The window keeps its regions across the replay, so a
                    // marked peak can be watched growing in.
                    if let Some(existing) = app.windows[index].buffer.as_ref() {
                        slice.rois = existing.rois.clone();
                    }
                    if app.windows[index].is_buffer() {
                        app.windows[index].buffer = Some(slice);
                    }
                }
            }
        },
    );
}

fn preferences_dialog(app: &mut App, ctx: &egui::Context) {
    dialog_window(app, ctx, Dialog::Preferences, "Colours", |app, ui| {
        ui.label("Scheme");
        ui.horizontal_wrapped(|ui| {
            for theme in Theme::all() {
                let colors = theme.colors();
                let selected = app.theme == *theme;
                // Each choice carries a swatch of its own data colours.
                let response = ui.add(
                    egui::Button::new(
                        egui::RichText::new(theme.label()).color(colors.foreground.to_color()),
                    )
                    .fill(colors.background.to_color())
                    .stroke(egui::Stroke::new(
                        if selected { 2.0 } else { 1.0 },
                        if selected {
                            colors.foreground.to_color()
                        } else {
                            colors.axes.with_alpha(0.6)
                        },
                    )),
                );
                if response.clicked() {
                    app.theme = *theme;
                    app.colors = colors;
                    crate::theme::apply(ui.ctx(), *theme);
                }
                // A row of dots showing the palette.
                let (rect, _) =
                    ui.allocate_exact_size(egui::vec2(52.0, 12.0), egui::Sense::hover());
                for (index, colour) in [
                    colors.foreground,
                    colors.roi,
                    colors.compare,
                    colors.library,
                    colors.marker,
                ]
                .iter()
                .enumerate()
                {
                    ui.painter().circle_filled(
                        rect.left_center() + egui::vec2(5.0 + index as f32 * 10.0, 0.0),
                        4.0,
                        colour.to_color(),
                    );
                }
            }
        });

        ui.separator();
        ui.horizontal(|ui| {
            ui.label("Peak Info text size");
            ui.add(egui::Slider::new(&mut app.peak_font, 9.0..=18.0).suffix(" px"));
        });
        ui.horizontal(|ui| {
            ui.label("Default save format");
            egui::ComboBox::from_id_salt("default-save-format")
                .selected_text(format!(".{}", app.default_format))
                .show_ui(ui, |ui| {
                    for extension in ["chn", "spe", "json", "txt", "csv"] {
                        if ui
                            .selectable_label(
                                app.default_format == extension,
                                format!(".{extension}"),
                            )
                            .clicked()
                        {
                            app.default_format = extension.into();
                        }
                    }
                });
            ui.label(
                egui::RichText::new("offered first when saving a new buffer")
                    .weak()
                    .small(),
            );
        });
        ui.checkbox(
            &mut app.reopen_last,
            "Reopen the last spectrum when the application starts",
        );

        ui.separator();
        ui.label("Individual colours");
        let edit = |ui: &mut egui::Ui, label: &str, value: &mut Rgb| {
            ui.horizontal(|ui| {
                let mut color = value.to_color();
                if ui.color_edit_button_srgba(&mut color).changed() {
                    *value = Rgb::from_color(color);
                }
                ui.label(label);
            });
        };
        edit(ui, "Background", &mut app.colors.background);
        edit(ui, "Spectrum", &mut app.colors.foreground);
        edit(ui, "Regions", &mut app.colors.roi);
        edit(ui, "Comparison", &mut app.colors.compare);
        edit(ui, "Composite", &mut app.colors.composite);
        edit(ui, "Axes", &mut app.colors.axes);
        edit(ui, "Marker", &mut app.colors.marker);
        edit(ui, "Library lines", &mut app.colors.library);

        ui.separator();
        ui.label("Contrast against the plot background");
        ui.label(
            egui::RichText::new(
                "4.5:1 is the least a line should have; the spectrum wants 7:1 or more.",
            )
            .weak()
            .small(),
        );
        egui::Grid::new("contrast").num_columns(3).show(ui, |ui| {
            for (role, ratio) in app.colors.contrast_report() {
                ui.label(role);
                ui.monospace(format!("{ratio:.1}:1"));
                let (verdict, colour) = if ratio >= 7.0 {
                    ("good", app.colors.healthy.to_color())
                } else if ratio >= 4.5 {
                    ("readable", app.colors.roi.to_color())
                } else {
                    ("too faint", app.colors.alarm.to_color())
                };
                ui.colored_label(colour, verdict);
                ui.end_row();
            }
        });

        let clashes = app.colors.clashes();
        if clashes.is_empty() {
            ui.colored_label(
                app.colors.healthy.to_color(),
                "every colour is distinguishable from the others",
            );
        } else {
            for (first, second, hue) in clashes {
                ui.colored_label(
                    app.colors.alarm.to_color(),
                    format!("{first} and {second} are only {hue:.0}° apart and equally light"),
                );
            }
        }

        ui.separator();
        if ui.button("Restore this scheme's colours").clicked() {
            app.colors = app.theme.colors();
            crate::theme::apply(ui.ctx(), app.theme);
        }
    });
}

fn dashboard_dialog(app: &mut App, ctx: &egui::Context, actions: &mut Vec<Action>) {
    dialog_window(
        app,
        ctx,
        Dialog::Dashboard,
        "Detector dashboard",
        |app, ui| {
            let tiles: Vec<_> = app.detectors.iter().map(|mcb| tile(mcb)).collect();
            egui::Grid::new("dashboard")
                .num_columns(7)
                .striped(true)
                .show(ui, |ui| {
                    for header in ["#", "name", "state", "live", "real", "dead %", "cps"] {
                        ui.label(egui::RichText::new(header).strong());
                    }
                    ui.end_row();
                    for (index, tile) in tiles.iter().enumerate() {
                        ui.label(tile.number.to_string());
                        if ui.link(&tile.name).clicked() {
                            actions.push(Action::OpenDetector(index));
                        }
                        let state = if tile.alarm {
                            egui::RichText::new(&tile.status_text).color(egui::Color32::LIGHT_RED)
                        } else if tile.active {
                            egui::RichText::new(&tile.status_text).color(egui::Color32::LIGHT_GREEN)
                        } else {
                            egui::RichText::new(&tile.status_text).weak()
                        };
                        ui.label(state);
                        ui.label(format!("{:.1}", tile.live_time));
                        ui.label(format!("{:.1}", tile.real_time));
                        ui.label(format!("{:.2}", tile.dead_time_percent));
                        ui.label(format!("{:.0}", tile.count_rate));
                        ui.end_row();
                    }
                });
            ui.separator();
            ui.horizontal(|ui| {
                if ui.button("Start all").clicked() {
                    for detector in &mut app.detectors {
                        let _ = detector.start();
                    }
                }
                if ui.button("Stop all").clicked() {
                    for detector in &mut app.detectors {
                        let _ = detector.stop();
                    }
                }
            });
            ui.separator();
            ui.label("Alarm limits");
            let mut limits = app.alarms.limits;
            let edit = |ui: &mut egui::Ui, label: &str, value: &mut Option<f64>, speed: f64| {
                ui.horizontal(|ui| {
                    let mut enabled = value.is_some();
                    if ui.checkbox(&mut enabled, label).changed() {
                        *value = enabled.then_some(0.0);
                    }
                    let mut number = value.unwrap_or(0.0);
                    if ui
                        .add_enabled(enabled, egui::DragValue::new(&mut number).speed(speed))
                        .changed()
                    {
                        *value = Some(number);
                    }
                });
            };
            edit(
                ui,
                "max dead time %",
                &mut limits.max_dead_time_percent,
                1.0,
            );
            edit(ui, "max count rate", &mut limits.max_count_rate, 100.0);
            edit(ui, "min count rate", &mut limits.min_count_rate, 1.0);
            app.alarms.limits = limits;
            let tripped: Vec<String> = app
                .alarms
                .latched()
                .iter()
                .map(|kind| kind.label().to_string())
                .collect();
            if !tripped.is_empty() {
                ui.colored_label(
                    egui::Color32::LIGHT_RED,
                    format!("alarms: {}", tripped.join(", ")),
                );
                if ui.button("Reset alarms").clicked() {
                    app.alarms.reset();
                }
            }
        },
    );
}

fn qa_dialog(app: &mut App, ctx: &egui::Context) {
    dialog_window(app, ctx, Dialog::Qa, "Quality assurance", |app, ui| {
        if app.qa.is_empty() {
            ui.label("Mark a check-source peak and use Services/Record QA point.");
            return;
        }
        for chart in &mut app.qa {
            ui.separator();
            ui.horizontal(|ui| {
                ui.label(egui::RichText::new(chart.metric.label()).strong());
                ui.label(format!("{} point(s)", chart.len()));
                let status = chart.status();
                let color = match status {
                    QaStatus::InControl => egui::Color32::LIGHT_GREEN,
                    QaStatus::Warning => egui::Color32::YELLOW,
                    QaStatus::Action => egui::Color32::LIGHT_RED,
                };
                ui.colored_label(color, status.label());
                if ui.button("freeze limits").clicked() {
                    chart.freeze_limits();
                }
            });
            if let Some(stats) = chart.statistics() {
                ui.monospace(format!(
                    "mean {:.4}  sd {:.4}  min {:.4}  max {:.4}",
                    stats.mean, stats.standard_deviation, stats.min, stats.max
                ));
            }
            let values: Vec<f64> = chart.records().iter().map(|record| record.value).collect();
            if values.len() >= 2 {
                // A proper control chart: the centre line, the warning and
                // action bands shaded in the palette's status colours, and each
                // point judged individually.
                let stats = chart.statistics();
                let (center, sigma) = match (chart.limits.center, chart.limits.sigma, &stats) {
                    (Some(center), Some(sigma), _) => (center, sigma),
                    (_, _, Some(stats)) => (stats.mean, stats.standard_deviation.max(1e-12)),
                    _ => (0.0, 1.0),
                };
                let action = chart.limits.action_sigma * sigma;
                let warning = chart.limits.warning_sigma * sigma;
                let (low, high) = values.iter().fold(
                    (center - action * 1.2, center + action * 1.2),
                    |(low, high), value| (low.min(*value), high.max(*value)),
                );
                let span = (high - low).max(1e-12);
                let (response, painter) = ui.allocate_painter(
                    egui::vec2(ui.available_width().min(460.0), 88.0),
                    egui::Sense::hover(),
                );
                let rect = response.rect.shrink(1.0);
                painter.rect_filled(
                    rect,
                    egui::CornerRadius::same(3),
                    app.colors.background.to_color(),
                );
                let y_of = |value: f64| {
                    rect.bottom() - rect.height() * (((value - low) / span) as f32).clamp(0.0, 1.0)
                };
                // Bands: healthy inside the warning limits, amber to the action
                // limits, alarm outside.
                let band = |from: f64, to: f64, colour: egui::Color32| {
                    let rect = egui::Rect::from_min_max(
                        egui::Pos2::new(rect.left(), y_of(to)),
                        egui::Pos2::new(rect.right(), y_of(from)),
                    );
                    painter.rect_filled(rect, egui::CornerRadius::ZERO, colour);
                };
                band(
                    center - warning,
                    center + warning,
                    app.colors.healthy.with_alpha(0.10),
                );
                band(
                    center + warning,
                    center + action,
                    app.colors.roi.with_alpha(0.10),
                );
                band(
                    center - action,
                    center - warning,
                    app.colors.roi.with_alpha(0.10),
                );
                for limit in [center - action, center + action] {
                    painter.line_segment(
                        [
                            egui::Pos2::new(rect.left(), y_of(limit)),
                            egui::Pos2::new(rect.right(), y_of(limit)),
                        ],
                        egui::Stroke::new(1.0, app.colors.alarm.with_alpha(0.6)),
                    );
                }
                painter.line_segment(
                    [
                        egui::Pos2::new(rect.left(), y_of(center)),
                        egui::Pos2::new(rect.right(), y_of(center)),
                    ],
                    egui::Stroke::new(1.0, app.colors.axes.with_alpha(0.8)),
                );
                // The measurements: a line through them, each point coloured by
                // its own verdict.
                let position = |index: usize, value: f64| {
                    egui::Pos2::new(
                        rect.left()
                            + 6.0
                            + (rect.width() - 12.0) * index as f32
                                / (values.len() - 1).max(1) as f32,
                        y_of(value),
                    )
                };
                let line: Vec<egui::Pos2> = values
                    .iter()
                    .enumerate()
                    .map(|(index, value)| position(index, *value))
                    .collect();
                painter.add(egui::Shape::line(
                    line,
                    egui::Stroke::new(1.2, app.colors.foreground.with_alpha(0.7)),
                ));
                for (index, value) in values.iter().enumerate() {
                    let miss = (value - center).abs();
                    let colour = if miss > action {
                        app.colors.alarm.to_color()
                    } else if miss > warning {
                        app.colors.roi.to_color()
                    } else {
                        app.colors.healthy.to_color()
                    };
                    painter.circle_filled(position(index, *value), 2.8, colour);
                }
            }
        }
    });
}

fn batch_dialog(app: &mut App, ctx: &egui::Context, actions: &mut Vec<Action>) {
    dialog_window(app, ctx, Dialog::Batch, "Batch queue", |app, ui| {
        ui.horizontal(|ui| {
            ui.label("Sample id");
            ui.add(egui::TextEdit::singleline(&mut app.dialogs.queue_id).desired_width(90.0));
            ui.label("live time (s)");
            ui.add(egui::DragValue::new(&mut app.dialogs.queue_live_time).speed(10.0));
            if ui.button("Queue").clicked() {
                let job = SampleJob::new(&app.dialogs.queue_id, app.dialogs.queue_live_time);
                app.queue.push(job);
                // Bump the identifier for the next sample.
                app.dialogs.queue_id = next_id(&app.dialogs.queue_id);
            }
        });
        ui.separator();
        egui::Grid::new("queue")
            .num_columns(3)
            .striped(true)
            .show(ui, |ui| {
                for header in ["sample", "preset", "state"] {
                    ui.label(egui::RichText::new(header).strong());
                }
                ui.end_row();
                for job in app.queue.jobs() {
                    ui.label(&job.id);
                    ui.label(
                        job.presets
                            .live_time
                            .map(|live| format!("{live:.0} s live"))
                            .unwrap_or_else(|| "-".into()),
                    );
                    ui.label(match &job.state {
                        JobState::Pending => "pending".to_string(),
                        JobState::Counting => "counting".to_string(),
                        JobState::Complete => "complete".to_string(),
                        JobState::Failed(reason) => format!("failed: {reason}"),
                    });
                    ui.end_row();
                }
            });
        ui.add(
            egui::ProgressBar::new(app.queue.progress() as f32).text(format!(
                "{} of {} done",
                app.queue.completed() + app.queue.failed(),
                app.queue.len()
            )),
        );
        ui.separator();
        ui.label(format!(
            "changer at position {} of {}{}",
            app.changer.position(),
            app.changer.positions(),
            if app.changer.is_ready() {
                ""
            } else {
                ", moving"
            }
        ));
        ui.horizontal(|ui| {
            if ui.button("Run next sample").clicked() {
                actions.extend(crate::jobs::run_next_sample(app));
            }
            if ui.button("Change sample").clicked() {
                app.changer.request_change();
            }
            if ui.button("Reset queue").clicked() {
                app.queue.reset();
                app.changer.reset();
            }
        });
    });
}

fn next_id(current: &str) -> String {
    let digits: String = current
        .chars()
        .rev()
        .take_while(|c| c.is_ascii_digit())
        .collect::<Vec<_>>()
        .into_iter()
        .rev()
        .collect();
    if digits.is_empty() {
        return format!("{current}1");
    }
    let stem = &current[..current.len() - digits.len()];
    let width = digits.len();
    let next = digits.parse::<u64>().unwrap_or(0) + 1;
    format!("{stem}{next:0width$}")
}

fn about_dialog(app: &mut App, ctx: &egui::Context) {
    dialog_window(app, ctx, Dialog::About, "About ortseam", |_app, ui| {
        ui.heading("ortseam");
        ui.label(format!("version {}", env!("CARGO_PKG_VERSION")));
        ui.label("A modern multichannel-analyzer emulator and gamma-spectroscopy workbench.");
        ui.separator();
        ui.label(
            "Analysis follows the published MAESTRO behaviour: equations 17-21 for peak areas, \
             equation 22 for counting activity, equation 23 for smoothing, and a Mariscotti peak \
             search.",
        );
        ui.label("Reads and writes .Chn, .Spc, .Spe, .Roi, ASCII and its own JSON format.");
        ui.label(
            egui::RichText::new(format!(
                "if it ever crashes, a report appears in {}",
                std::env::temp_dir().display()
            ))
            .weak(),
        );
        ui.separator();
        ui.label("Not affiliated with or endorsed by ORTEC / AMETEK.");
    });
}

fn exit_dialog(app: &mut App, ctx: &egui::Context) {
    if !app.dialogs.is_open(Dialog::Exit) {
        return;
    }
    let unsaved: Vec<String> = app
        .windows
        .iter()
        .filter(|window| window.modified)
        .map(|window| window.title.clone())
        .collect();
    let counting = app.detectors.iter().filter(|d| d.is_active()).count();
    let mut open = true;
    let mut cancel = false;
    let mut confirmed = false;
    egui::Window::new("Exit ortseam")
        .open(&mut open)
        // A question the whole application is waiting on must not be buried
        // under a maximized plot - the title-bar X would then appear dead,
        // each close request re-opening an already-hidden dialog.
        .order(egui::Order::Foreground)
        .collapsible(false)
        .show(ctx, |ui| {
            if !unsaved.is_empty() {
                ui.colored_label(
                    egui::Color32::YELLOW,
                    format!("unsaved: {}", unsaved.join(", ")),
                );
            }
            if counting > 0 {
                ui.label(format!("{counting} detector(s) are still counting"));
            }
            ui.horizontal(|ui| {
                if ui.button("Exit").clicked() {
                    confirmed = true;
                    ui.ctx().send_viewport_cmd(egui::ViewportCommand::Close);
                }
                if ui.button("Cancel").clicked() {
                    cancel = true;
                }
            });
        });
    if confirmed {
        // Otherwise the close request this Close raises would reopen the
        // same question.
        app.quit_confirmed = true;
    }
    app.dialogs.set(Dialog::Exit, open && !cancel);
}

/// How the count has behaved while it ran: rate, and dead time under it.
///
/// The readout above says what the rate is now; this says whether it has been
/// that all along. A source that shifted, a detector warming up, a shield left
/// open - all of them are changes over time, invisible in an instantaneous
/// number and obvious in forty pixels of trace.
fn stability_trace(app: &mut App, ui: &mut egui::Ui) {
    let Some(number) = app
        .active_detector_index()
        .and_then(|index| app.detectors.get(index))
        .map(|mcb| ortseam_device::Mcb::identity(mcb).number)
    else {
        return;
    };
    let Some(log) = app.stability.get(&number) else {
        return;
    };
    // Two readings cannot show a trend, and a flat line from nothing invites
    // being read as one.
    if log.len() < 3 {
        return;
    }
    let peak = log.peak_rate();
    if peak <= 0.0 {
        return;
    }

    ui.separator();
    ui.horizontal(|ui| {
        ui.label("Stability");
        if let Some(drift) = log.drift() {
            let percent = drift * 100.0;
            // Counting statistics move any reading around; a few percent
            // across a run is not a finding, and colouring it as one would
            // train the operator to ignore the colour.
            let colour = if percent.abs() >= 20.0 {
                app.colors.alarm.to_color()
            } else if percent.abs() >= 8.0 {
                app.colors.roi.to_color()
            } else {
                app.colors.healthy.to_color()
            };
            ui.label(egui::RichText::new(format!("{percent:+.0}%")).color(colour))
                .on_hover_text(
                    "how the count rate over the last fifth of the run compares with the \
                     first fifth - drift, not a single reading",
                );
        }
    });

    let (rect, response) = ui.allocate_exact_size(
        egui::vec2(ui.available_width().min(180.0), 34.0),
        egui::Sense::hover(),
    );
    let painter = ui.painter();
    painter.rect_filled(
        rect,
        egui::CornerRadius::same(2),
        ui.visuals().extreme_bg_color,
    );
    let samples: Vec<_> = log.samples().collect();
    let last = samples.len().saturating_sub(1).max(1) as f32;
    let point = |index: usize, value: f64, top: f64| -> egui::Pos2 {
        let x = rect.left() + rect.width() * (index as f32 / last);
        let fraction = (value / top).clamp(0.0, 1.0) as f32;
        egui::Pos2::new(x, rect.bottom() - rect.height() * fraction)
    };
    // Dead time first and faint, so the rate reads over the top of it.
    let dead: Vec<egui::Pos2> = samples
        .iter()
        .enumerate()
        .map(|(index, sample)| point(index, sample.dead_time, 100.0))
        .collect();
    painter.add(egui::Shape::line(
        dead,
        egui::Stroke::new(1.0, app.colors.roi.to_color().gamma_multiply(0.5)),
    ));
    let rate: Vec<egui::Pos2> = samples
        .iter()
        .enumerate()
        .map(|(index, sample)| point(index, sample.rate, peak))
        .collect();
    painter.add(egui::Shape::line(
        rate,
        egui::Stroke::new(1.2, app.colors.foreground.to_color()),
    ));
    response.on_hover_text(format!(
        "count rate across the acquisition, peak {}; dead time faint underneath, \
         full height is 100%",
        crate::view::format_rate(peak)
    ));
}

/// What a run that did not finish left behind, offered back.
///
/// Not restored silently: windows appearing unbidden is how somebody ends up
/// working on the wrong data without noticing. The question says what there is
/// and when it was taken, and discarding is a deliberate answer rather than
/// the default.
fn recover_dialog(app: &mut App, ctx: &egui::Context) {
    if !app.dialogs.is_open(Dialog::Recover) {
        return;
    }
    let Some(session) = app.recovered.clone() else {
        app.dialogs.set(Dialog::Recover, false);
        return;
    };
    egui::Window::new("Unfinished work")
        .order(egui::Order::Foreground)
        .collapsible(false)
        .resizable(false)
        .anchor(egui::Align2::CENTER_CENTER, egui::Vec2::ZERO)
        .show(ctx, |ui| {
            ui.label("ortseam did not exit cleanly last time.");
            ui.add_space(4.0);
            if session.saved_at.is_empty() {
                ui.label(format!("{} window(s) were open:", session.windows.len()));
            } else {
                ui.label(format!(
                    "{} window(s) were open, last recorded {}:",
                    session.windows.len(),
                    session.saved_at
                ));
            }
            ui.add_space(2.0);
            for window in session.windows.iter().take(8) {
                let counts = window.spectrum.total_counts();
                ui.label(
                    egui::RichText::new(format!(
                        "   {}  -  {} counts{}",
                        window.title,
                        format_counts(counts),
                        if window.modified { ", unsaved" } else { "" }
                    ))
                    .monospace()
                    .small(),
                );
            }
            if session.windows.len() > 8 {
                ui.label(
                    egui::RichText::new(format!("   and {} more", session.windows.len() - 8))
                        .small()
                        .weak(),
                );
            }
            ui.add_space(6.0);
            ui.horizontal(|ui| {
                if ui.button("Reopen them").clicked() {
                    let session = app.recovered.take().unwrap_or_default();
                    app.restore_session(session);
                    app.dialogs.set(Dialog::Recover, false);
                }
                if ui
                    .button("Discard")
                    .on_hover_text("throw the unfinished work away; this cannot be undone")
                    .clicked()
                {
                    app.recovered = None;
                    crate::session::clear();
                    app.status = "the unfinished work was discarded".into();
                    app.dialogs.set(Dialog::Recover, false);
                }
            });
        });
}

/// The unsaved-changes question a modified buffer asks before its window goes.
fn confirm_close_dialog(app: &mut App, ctx: &egui::Context, actions: &mut Vec<Action>) {
    if !app.dialogs.is_open(Dialog::ConfirmClose) {
        return;
    }
    let Some(id) = app.pending_close else {
        app.dialogs.set(Dialog::ConfirmClose, false);
        return;
    };
    let title = app
        .windows
        .iter()
        .find(|window| window.id == id)
        .map(|window| window.title.clone())
        .unwrap_or_default();
    let mut done = false;
    egui::Window::new("Unsaved changes")
        // The same burial as Exit: this question must stay on top.
        .order(egui::Order::Foreground)
        .collapsible(false)
        .resizable(false)
        .anchor(egui::Align2::CENTER_CENTER, egui::Vec2::ZERO)
        .show(ctx, |ui| {
            ui.label(format!("{title} has changes that are not saved."));
            ui.add_space(6.0);
            ui.horizontal(|ui| {
                if ui.button("Save").clicked() {
                    actions.push(Action::SaveThenClose(id));
                    done = true;
                }
                if ui.button("Discard").clicked() {
                    actions.push(Action::ForceCloseWindow(id));
                    done = true;
                }
                if ui.button("Cancel").clicked() {
                    done = true;
                }
            });
        });
    if done {
        app.pending_close = None;
        app.dialogs.set(Dialog::ConfirmClose, false);
    }
}

fn open_path_dialog(app: &mut App, ctx: &egui::Context, actions: &mut Vec<Action>) {
    dialog_window(
        app,
        ctx,
        Dialog::OpenPath,
        "Open a file by path",
        |app, ui| {
            ui.label("Type or paste a path. Useful when no native file dialog is available.");
            ui.add(egui::TextEdit::singleline(&mut app.dialogs.path_input).desired_width(420.0));
            ui.horizontal(|ui| {
                if ui.button("Open").clicked() {
                    let path = PathBuf::from(app.dialogs.path_input.trim());
                    if path.exists() {
                        app.recall_path(path);
                    } else {
                        actions.push(Action::Status("no such file".into()));
                    }
                }
            });
        },
    );
}

#[cfg(test)]
mod tests {
    use super::{case_variants, strongest_centroid};
    use ortseam_core::{LineResult, NuclideResult};

    #[test]
    fn a_filter_matches_the_capitals_ortec_actually_writes() {
        // The bug this pins: on Linux a lowercase-only filter hid every real
        // instrument file, because ORTEC names them `.Spe` and GTK matches
        // patterns case-sensitively.
        let patterns = case_variants(&["spe"]);
        assert!(patterns.contains(&"Spe".to_string()), "{patterns:?}");
        assert!(patterns.contains(&"spe".to_string()), "{patterns:?}");
        assert!(patterns.contains(&"SPE".to_string()), "{patterns:?}");
    }

    #[test]
    fn a_filter_written_in_capitals_still_offers_the_lowercase_spelling() {
        let patterns = case_variants(&["Chn"]);
        assert!(patterns.contains(&"chn".to_string()), "{patterns:?}");
        assert!(patterns.contains(&"Chn".to_string()), "{patterns:?}");
    }

    #[test]
    fn a_filter_lists_each_spelling_once() {
        // Every extension of every spectrum filter, so a duplicate cannot
        // creep into the pattern list the dialog is given.
        let patterns = case_variants(&["chn", "spc", "spe", "json", "txt", "lis"]);
        let mut sorted = patterns.clone();
        sorted.sort();
        sorted.dedup();
        assert_eq!(sorted.len(), patterns.len(), "{patterns:?}");
    }

    fn line(centroid: f64, net_area: f64) -> LineResult {
        LineResult {
            energy: centroid,
            centroid,
            net_area,
            net_uncertainty: 0.0,
            count_rate: 0.0,
            activity: 0.0,
            activity_uncertainty: 0.0,
            mda: 0.0,
            efficiency: 0.0,
            yield_percent: 0.0,
            used_in_average: true,
        }
    }

    fn nuclide(lines: Vec<LineResult>) -> NuclideResult {
        NuclideResult {
            nuclide: "Eu-152".into(),
            activity: 0.0,
            uncertainty: 0.0,
            mda: 0.0,
            detected: true,
            half_life_seconds: 1.0,
            lines,
        }
    }

    #[test]
    fn the_analysis_table_jumps_to_the_strongest_line() {
        let n = nuclide(vec![
            line(300.0, 40.0),
            line(900.0, 4000.0),
            line(1500.0, 400.0),
        ]);
        assert_eq!(strongest_centroid(&n), Some(900.0));
    }

    #[test]
    fn a_nuclide_without_measured_lines_has_nowhere_to_jump() {
        assert_eq!(strongest_centroid(&nuclide(Vec::new())), None);
    }

    #[test]
    fn a_nan_net_area_cannot_hijack_the_jump() {
        let n = nuclide(vec![line(300.0, f64::NAN), line(900.0, 10.0)]);
        assert_eq!(strongest_centroid(&n), Some(900.0));
    }
}
