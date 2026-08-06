//! The application: windows, menus, the acquisition tick and the undo history.

use std::path::PathBuf;
use std::time::Instant;

use ortseam_core::{
    AnalysisOptions, CalculationSettings, CalibrationPoint, CalibrationTable,
    EfficiencyCalibration, NuclideLibrary, PeakInfo, QaChart, QuantitativeReport, Roi, SampleInfo,
    Spectrum, StripFactor, UndoStack, analyse, mark_peaks, peak_info, smooth, strip,
};
#[cfg(feature = "simulator")]
use ortseam_device::SimulatedMcb;
use ortseam_device::{
    AlarmMonitor, AnyMcb, DetectorEntry, DetectorList, Mcb, PresetKind, SampleChanger, SampleQueue,
    advance,
};
use ortseam_formats::{
    ListModeFile, SpectrumFormat, load_rois, load_spectrum, save_rois, save_spectrum,
};
use ortseam_report::{ReportOptions, nuclide_report, print_spectrum, roi_report};

use crate::dialogs::{Dialog, Dialogs};
use crate::theme::{self, SpectrumColors};
use crate::view::{MarkMode, ViewEvent, ViewInput};
use crate::viewmodel::{DisplayState, FillMode, VerticalScale};

/// How many files the File menu remembers.
const RECENT_FILES: usize = 8;

#[cfg(test)]
mod pin_tests {
    use super::pin_by_serial;

    #[test]
    fn an_adapter_serial_pins_the_connection() {
        // What the Linux bench reports: the adapter's own serial, which is
        // not the number that selected it, so it is worth naming.
        assert_eq!(pin_by_serial("08134079", 1), Some("08134079"));
        assert_eq!(pin_by_serial("  08134079 ", 2), Some("08134079"));
    }

    #[test]
    fn a_detector_number_is_not_a_pin() {
        // Through ORTEC's library an instrument reports the detector number
        // that selected it. Handing that back as --device would ask the
        // helper to match a position or a path against it, which is how the
        // wrong adapter gets opened.
        assert_eq!(pin_by_serial("1", 1), None);
        assert_eq!(pin_by_serial(" 337 ", 337), None);
        // A different number is still an identity worth naming.
        assert_eq!(pin_by_serial("338", 337), Some("338"));
    }

    #[test]
    fn nothing_remembered_pins_nothing() {
        assert_eq!(pin_by_serial("", 1), None);
        assert_eq!(pin_by_serial("   ", 1), None);
    }
}

/// The serial to select an adapter by, when selecting by serial is meaningful.
///
/// Reached through ORTEC's library, an instrument reports its detector number
/// as its identity - the same number that selected it. Handing that back as
/// `--device` would ask the helper to match a *position or path* against it,
/// which is how the wrong adapter gets opened. So an identity that is only the
/// number we already used is not a pin; anything else, an adapter serial, is.
/// Either way the identity is still checked after connecting.
fn pin_by_serial(remembered: &str, detector: i32) -> Option<&str> {
    let remembered = remembered.trim();
    if remembered.is_empty() || remembered == detector.to_string() {
        return None;
    }
    Some(remembered)
}

/// Launches a program from a job's command line (`RUN`, `WAIT "program"`).
///
/// The first quoted or unquoted word is the program, the rest its arguments -
/// the same reading MAESTRO's job engine gives the line.
pub fn spawn_program(command_line: &str) -> Result<std::process::Child, String> {
    let mut words: Vec<String> = Vec::new();
    let mut current = String::new();
    let mut quoted = false;
    for character in command_line.chars() {
        match character {
            '"' => quoted = !quoted,
            ' ' | '\t' if !quoted => {
                if !current.is_empty() {
                    words.push(std::mem::take(&mut current));
                }
            }
            _ => current.push(character),
        }
    }
    if !current.is_empty() {
        words.push(current);
    }
    let (program, arguments) = words
        .split_first()
        .ok_or_else(|| "an empty command line".to_string())?;
    std::process::Command::new(program)
        .args(arguments)
        .spawn()
        .map_err(|error| format!("could not run {program}: {error}"))
}

/// Opens a file with whatever the desktop associates with it (the browser,
/// for the printable report).
fn open_with_default_app(path: &std::path::Path) -> Result<(), String> {
    // Tests set this to keep browsers from popping up mid-suite.
    if std::env::var_os("ORTSEAM_NO_OPEN").is_some() {
        return Ok(());
    }
    let result = if cfg!(target_os = "windows") {
        std::process::Command::new("cmd")
            .args(["/C", "start", ""])
            .arg(path)
            .spawn()
    } else if cfg!(target_os = "macos") {
        std::process::Command::new("open").arg(path).spawn()
    } else {
        std::process::Command::new("xdg-open").arg(path).spawn()
    };
    result.map(drop).map_err(|error| error.to_string())
}

/// Runs `reg.exe` quietly; the console child must not flash a window.
#[cfg(windows)]
fn reg(args: &[&str]) -> Result<(), String> {
    use std::os::windows::process::CommandExt;
    const CREATE_NO_WINDOW: u32 = 0x0800_0000;
    // Tests must never write to the user's registry.
    if std::env::var_os("ORTSEAM_NO_OPEN").is_some() {
        return Ok(());
    }
    let output = std::process::Command::new("reg")
        .args(args)
        .creation_flags(CREATE_NO_WINDOW)
        .output()
        .map_err(|error| error.to_string())?;
    if output.status.success() {
        Ok(())
    } else {
        Err(String::from_utf8_lossy(&output.stderr).trim().to_string())
    }
}

/// What survives closing the application: the look, and the recent files.
///
/// Deliberately small. Spectra belong in spectrum files, not in an interface
/// settings blob, and window layout is cheap to remake.
#[derive(serde::Serialize, serde::Deserialize)]
pub struct Persisted {
    /// The colour scheme in force.
    pub theme: crate::theme::Theme,
    /// The working colours, which may be hand-edited from the scheme.
    pub colors: SpectrumColors,
    /// Files opened lately, most recent first.
    pub recent: Vec<PathBuf>,
    /// Simulation speed.
    pub time_scale: f64,
    /// Whether a peak search clears the existing regions first.
    #[serde(default)]
    pub auto_clear_roi: bool,
    /// Font size of the peak-information card.
    #[serde(default = "default_peak_font")]
    pub peak_font: f32,
    /// Extension offered first when saving.
    #[serde(default = "default_format")]
    pub default_format: String,
    /// Whether the last spectrum reopens at start.
    #[serde(default)]
    pub reopen_last: bool,
}

fn default_peak_font() -> f32 {
    11.0
}

fn default_format() -> String {
    "chn".into()
}

/// The window arrangements (Window menu).
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
enum Tiling {
    /// Full-width strips, stacked.
    Rows,
    /// Full-height strips, side by side.
    Columns,
    /// Staggered, all the same size.
    Cascade,
}

/// Which spectrum a window shows.
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub enum Target {
    /// A configured detector, by index.
    Detector(usize),
    /// A buffer holding a recalled or copied spectrum.
    Buffer,
}

/// One spectrum window.
pub struct SpectrumWindow {
    /// Stable identifier.
    pub id: usize,
    /// Title bar text.
    pub title: String,
    /// What it shows.
    pub target: Target,
    /// The data, for buffer windows.
    pub buffer: Option<Spectrum>,
    /// Where it came from or was last saved.
    pub path: Option<PathBuf>,
    /// What is on screen.
    pub display: DisplayState,
    /// A comparison spectrum.
    pub compare: Option<Spectrum>,
    /// Vertical offset of the comparison trace.
    pub compare_offset: i64,
    /// Points entered while calibrating.
    pub calibration: CalibrationTable,
    /// List-mode events, when the window holds them.
    pub list: Option<ListModeFile>,
    /// The rubber-rectangle selection.
    pub selection: Option<(usize, usize)>,
    /// The last peak-info result, shown in a pop-up.
    pub peak_info: Option<PeakInfo>,
    /// Whether the window is open.
    pub open: bool,
    /// Whether the data has changed since it was saved.
    pub modified: bool,
}

impl SpectrumWindow {
    fn new(id: usize, title: String, target: Target, buffer: Option<Spectrum>) -> Self {
        let length = buffer.as_ref().map(|s| s.len()).unwrap_or(0);
        Self {
            id,
            title,
            target,
            buffer,
            path: None,
            display: DisplayState::for_length(length),
            compare: None,
            compare_offset: 0,
            calibration: CalibrationTable::default(),
            list: None,
            selection: None,
            peak_info: None,
            open: true,
            modified: false,
        }
    }

    /// True for a buffer window.
    pub fn is_buffer(&self) -> bool {
        self.target == Target::Buffer
    }
}

/// A remembered spectrum, for undo.
#[derive(Clone)]
pub struct UndoSnapshot {
    /// Which window the command ran on.
    pub target: Target,
    /// The window's identifier, so a buffer can be restored in place.
    pub window: usize,
    /// The data before the command.
    pub spectrum: Spectrum,
    /// The window's title, for the recovered buffer's name.
    pub title: String,
}

/// Everything the menus and toolbar can ask for.
#[derive(Clone, Debug, PartialEq)]
pub enum Action {
    // File
    Recall,
    /// Open a particular file, from the recent list or a drop.
    RecallFile(PathBuf),
    Save,
    SaveAs,
    Export,
    /// Write the active window's plot as an SVG picture.
    ExportPlot,
    Import,
    Print,
    RoiReport,
    Compare,
    ClearCompare,
    Exit,
    // Edit
    Undo,
    Redo,
    /// Copy the open peak-information card's text to the clipboard.
    CopyPeakInfo,
    /// Copy the region report to the clipboard.
    CopyReport,
    /// Copy the spectrum as CSV to the clipboard.
    CopyCsv,
    // Acquire
    Start,
    Stop,
    Clear,
    CopyToBuffer,
    ToggleListMode,
    ViewZdt,
    /// Snapshot the live spectrum into instrument memory and clear (field mode).
    StoreSpectrum,
    /// Pull every stored spectrum out of the instrument into buffer windows.
    DownloadSpectra,
    /// Begin the automatic optimisation routine.
    StartOptimize,
    /// Begin the automatic pole-zero routine.
    StartPoleZero,
    // Calculate
    PeakSearch,
    PeakInfoAtMarker,
    InputCountRate,
    Sum,
    Smooth,
    Strip,
    ListDataRange,
    // ROI
    MarkMode(MarkMode),
    MarkPeak,
    ClearRoi,
    ClearAllRois,
    SaveRoiFile,
    RecallRoiFile,
    MarkRange(usize, usize),
    UnmarkRange(usize, usize),
    // Display
    ZoomIn,
    ZoomOut,
    /// Zoom one step about a channel, keeping it in place on screen.
    ZoomAbout(usize, i32),
    /// Put the marker at an energy (or a channel, uncalibrated) and centre it.
    GotoEnergy(f64),
    /// Make the next (or previous) visible window active.
    CycleWindow(i32),
    Center,
    FullView,
    ToggleLog,
    AutoScale,
    BaselineZoom,
    SetFill(FillMode),
    ToggleIsotopeMarkers,
    ToggleEnergyAxis,
    // Windows and detectors
    OpenDetector(usize),
    OpenBuffer,
    CloseWindow(usize),
    /// Close without the unsaved-changes question (the Discard button).
    ForceCloseWindow(usize),
    /// Save, and close only if the save went through.
    SaveThenClose(usize),
    /// Focus a window by its id - not its position, which an earlier close in
    /// the same frame's batch can shift.
    Activate(usize),
    TileHorizontally,
    TileVertically,
    Cascade,
    /// Collapse every window to its title bar (Arrange Icons, modernised).
    CollapseAll,
    /// Expand every collapsed window again.
    ExpandAll,
    ToggleMaximize(usize),
    MaximizeActive,
    /// Leave full-area mode, whatever window holds it (JOB ZOOM restore).
    RestoreWindows,
    // Analysis and services
    Analyse,
    /// Fit an energy calibration by matching found peaks to a nuclide's lines.
    AutoCalibrate(String),
    RecallCalibration,
    DestroyCalibration,
    AddCalibrationPoint(f64),
    /// Replace a calibration point with a corrected channel and energy.
    EditCalibrationPoint(usize, f64, f64),
    /// Take a calibration point out of the fit.
    RemoveCalibrationPoint(usize),
    LoadLibrary,
    RunJob,
    RecordQa,
    /// Take a detector out of the pick list and the session (Detector List).
    RemoveDetector(usize),
    /// Rename the detector at this index, in the list and everywhere it shows.
    RenameDetector(usize, String),
    /// Point Explorer's .Spe/.Chn/.Spc at this executable (per-user).
    /// Ask the bridge what ORTEC hardware this machine can reach.
    ScanLocalInstruments,
    /// Have the bridge write a detector list from what it finds.
    ConfigureLocalInstruments,
    /// Open every instrument the bridge reports, as detector windows.
    OpenLocalInstruments,
    RegisterFileTypes,
    /// Take the association back out of the registry.
    UnregisterFileTypes,
    // Dialogs
    Show(Dialog),
    // From the view
    Marker(usize),
    Select(usize, usize),
    ClearSelection,
    ZoomTo(usize, usize),
    Scroll(i64),
    Status(String),
}

/// The instrument section of the empty screen.
///
/// It says what the machine can reach and offers the one action that follows
/// from it, so that connecting an instrument is a button rather than a
/// procedure. What it will not do is guess: a detector that is configured but
/// silent is reported as exactly that, because the alternative - quietly
/// dropping it - loses the only evidence that something was unplugged.
fn instrument_panel(app: &mut App, ui: &mut egui::Ui) -> Vec<Action> {
    let mut actions = Vec::new();
    let scan = app.instruments.clone();

    egui::Frame::group(ui.style())
        .inner_margin(egui::Margin::symmetric(14, 10))
        .show(ui, |ui| {
            ui.set_max_width(520.0);
            ui.vertical_centered(|ui| {
                ui.label(egui::RichText::new("Instruments").strong());
                ui.add_space(4.0);

                if let Some(problem) = &scan.problem {
                    ui.colored_label(ui.visuals().warn_fg_color, problem);
                } else if !scan.scanned {
                    ui.label(
                        egui::RichText::new("Not looked yet. Scan to find what is connected.")
                            .weak(),
                    );
                } else if scan.reachable.is_empty() && scan.missing.is_empty() {
                    ui.label(
                        egui::RichText::new(
                            "Nothing answered. Check the instrument is powered and plugged in.",
                        )
                        .weak(),
                    );
                }

                // Words rather than symbols: the interface font has no bullet
                // or warning sign, and a missing glyph draws as an empty box,
                // which reads as a fault rather than a state.
                for (number, name) in &scan.reachable {
                    ui.horizontal(|ui| {
                        ui.monospace(format!("{number}"));
                        ui.label(name);
                        ui.label(egui::RichText::new("ready").weak());
                    });
                }
                // A configured detector that did not answer is the case worth
                // interrupting for: it usually means unplugged, or a list
                // written on another machine where the numbering differed.
                for (number, name) in &scan.missing {
                    ui.horizontal(|ui| {
                        ui.monospace(format!("{number}"));
                        ui.label(name);
                        ui.colored_label(ui.visuals().warn_fg_color, "not answering");
                    });
                }

                ui.add_space(8.0);
                ui.horizontal(|ui| {
                    if ui
                        .button("Scan")
                        .on_hover_text("ask the machine what instruments are connected")
                        .clicked()
                    {
                        actions.push(Action::ScanLocalInstruments);
                    }
                    if !scan.reachable.is_empty()
                        && ui
                            .button("Open all")
                            .on_hover_text("open a window on every instrument that answered")
                            .clicked()
                    {
                        actions.push(Action::OpenLocalInstruments);
                    }
                    if scan.scanned
                        && (!scan.missing.is_empty() || scan.reachable.is_empty())
                        && ui
                            .button("Set up...")
                            .on_hover_text("rebuild the detector list from what is connected")
                            .clicked()
                    {
                        actions.push(Action::Show(Dialog::Instruments));
                    }
                });
            });
        });
    actions
}

/// What a scan of the machine's own instruments found.
///
/// Two lists rather than one, because they answer different questions: what
/// answered when asked, and what the configuration says should be there. A
/// detector in the second and not the first is the case worth warning about -
/// an instrument that has been unplugged, or a list written on another machine.
#[derive(Clone, Debug, Default, PartialEq, Eq)]
pub struct InstrumentScan {
    /// Instruments that answered, as (detector number, name).
    pub reachable: Vec<(i32, String)>,
    /// Configured detectors that did not answer, as (number, name).
    pub missing: Vec<(i32, String)>,
    /// Set when the scan itself could not be run.
    pub problem: Option<String>,
    /// Whether a scan has been run at all this session.
    pub scanned: bool,
}

impl InstrumentScan {
    /// Reads the bridge's probe output.
    pub fn parse(text: &str) -> Self {
        let mut scan = Self {
            scanned: true,
            ..Default::default()
        };
        let mut pending = None;
        for line in text.lines() {
            let trimmed = line.trim();
            // "  1: Rayleigh" opens a detector's block; the lines under it say
            // whether it answered.
            if let Some((head, name)) = trimmed.split_once(": ")
                && let Ok(number) = head.trim().parse::<i32>()
            {
                pending = Some((number, name.trim().to_string()));
                continue;
            }
            let Some((number, name)) = pending.clone() else {
                continue;
            };
            if trimmed.starts_with("model") {
                scan.reachable.push((number, name));
                pending = None;
            } else if trimmed.starts_with('<') {
                scan.missing.push((number, name));
                pending = None;
            }
        }
        scan
    }

    /// A scan that could not be run at all.
    pub fn failed(problem: &str) -> Self {
        Self {
            problem: Some(problem.to_string()),
            scanned: true,
            ..Default::default()
        }
    }

    /// One line for the status bar.
    pub fn summary(&self) -> String {
        if let Some(problem) = &self.problem {
            return problem.clone();
        }
        match (self.reachable.len(), self.missing.len()) {
            (0, 0) => "no instruments found".into(),
            (found, 0) => format!("{found} instrument(s) answered"),
            (0, absent) => format!("{absent} configured detector(s), none answering"),
            (found, absent) => {
                format!("{found} instrument(s) answered, {absent} configured but not answering")
            }
        }
    }
}

/// The application.
pub struct App {
    /// What the last scan of local instruments found.
    pub instruments: InstrumentScan,
    /// Whether the automatic first look for instruments has happened.
    ///
    /// Done on the first drawn frame rather than at startup, so the window is
    /// on screen while it happens rather than the program appearing to hang.
    pub looked_for_instruments: bool,
    /// Whether the window has been shown yet.
    ///
    /// It is created hidden and revealed after the first frame is drawn, so
    /// start-up does not flash an empty window while it loads.
    pub shown: bool,
    /// Configured detectors. Simulated instruments stand in for hardware.
    pub detectors: Vec<AnyMcb>,
    /// The detector pick list.
    pub detector_list: DetectorList,
    /// Open spectrum windows.
    pub windows: Vec<SpectrumWindow>,
    /// Index of the active window.
    pub active: Option<usize>,
    next_id: usize,
    /// Calculation settings.
    pub settings: CalculationSettings,
    /// Working nuclide library.
    pub library: NuclideLibrary,
    /// Where the library came from.
    pub library_path: Option<PathBuf>,
    /// Display colours.
    pub colors: SpectrumColors,
    /// The colour scheme in force.
    pub theme: crate::theme::Theme,
    /// Undo history.
    pub history: UndoStack<UndoSnapshot>,
    /// The supplementary information line.
    pub status: String,
    /// Region-marking mode.
    pub mark_mode: MarkMode,
    /// Dialog visibility and scratch state.
    pub dialogs: Dialogs,
    /// Efficiency curve used for activities.
    pub efficiency: Option<EfficiencyCalibration>,
    /// Sample description and quantity.
    pub sample: SampleInfo,
    /// Alarm limits and latch.
    pub alarms: AlarmMonitor,
    /// Batch queue.
    pub queue: SampleQueue,
    /// Simulated sample changer.
    pub changer: SampleChanger,
    /// Quality-assurance charts.
    pub qa: Vec<QaChart>,
    /// The last analysis.
    pub analysis: Option<QuantitativeReport>,
    /// Text of the last report, shown in a window.
    pub report_text: Option<String>,
    /// A running job.
    pub job: Option<crate::jobs::JobRun>,
    /// A wait the job is in, checked once per frame instead of blocking one.
    pub job_wait: Option<crate::jobs::JobWait>,
    /// Variables a job can read.
    pub job_variables: ortseam_jobs::JobVariables,
    /// Directory a job's relative file names resolve against.
    pub job_directory: PathBuf,
    /// File remembered by `SET_NAME_STRIP`.
    pub strip_name: Option<String>,
    /// How many simulated seconds pass per real second.
    pub time_scale: f64,
    /// Files opened lately, most recent first.
    recent: Vec<PathBuf>,
    /// An arrangement to apply on the next frame (Window/Tile and Cascade).
    pending_tiling: Option<Tiling>,
    /// A window whose close is waiting on the unsaved-changes question.
    pub pending_close: Option<usize>,
    /// Collapse (or expand) every window on the next frame.
    pending_collapse: Option<bool>,
    /// A window rect forced on the next frame (the JOB `ZOOM:` command).
    pending_place: Option<(usize, egui::Rect)>,
    /// Text bound for the clipboard on the next frame.
    pub pending_clipboard: Option<String>,
    /// The theme egui's own widgets were last dressed in, so a programmatic
    /// switch (restore, demo states) restyles the chrome on the next frame.
    applied_theme: Option<crate::theme::Theme>,
    /// Whether a peak search clears the existing regions first (ROI/Auto Clear).
    pub auto_clear_roi: bool,
    /// Font size of the in-plot peak-information card.
    pub peak_font: f32,
    /// Extension offered first when saving a spectrum without a path.
    pub default_format: String,
    /// Whether the last spectrum reopens at start (Preferences).
    pub reopen_last: bool,
    /// The area spectrum windows are kept inside.
    window_area: egui::Rect,
    /// Whether the user has answered the exit question; lets the next
    /// close request through instead of asking again.
    pub quit_confirmed: bool,
    /// The window filling the spectrum area, if any (MAESTRO's one-window mode).
    maximized: Option<usize>,
    last_tick: Instant,
}

impl App {
    /// Builds the application, restores what an earlier session saved, and
    /// dresses the interface in its theme.
    pub fn new(cc: &eframe::CreationContext<'_>) -> Self {
        let mut app = Self::headless();
        if let Some(text) = cc.storage.and_then(|storage| storage.get_string("ortseam"))
            && let Ok(persisted) = serde_json::from_str::<Persisted>(&text)
        {
            app.restore(persisted);
        }
        theme::apply(&cc.egui_ctx, app.theme);
        app
    }

    /// What to remember for the next session.
    pub fn persisted(&self) -> Persisted {
        Persisted {
            theme: self.theme,
            colors: self.colors,
            recent: self.recent.clone(),
            time_scale: self.time_scale,
            auto_clear_roi: self.auto_clear_roi,
            peak_font: self.peak_font,
            default_format: self.default_format.clone(),
            reopen_last: self.reopen_last,
        }
    }

    /// Applies an earlier session's settings. Files that no longer exist drop
    /// off the recent list rather than being offered.
    pub fn restore(&mut self, persisted: Persisted) {
        self.theme = persisted.theme;
        self.colors = persisted.colors;
        self.recent = persisted
            .recent
            .into_iter()
            .filter(|path| path.exists())
            .take(RECENT_FILES)
            .collect();
        // Without the simulator the clock runs at real speed and there is no
        // control to put it back, so a scale saved by a simulator build is
        // ignored rather than silently stretching a real instrument's timing.
        #[cfg(feature = "simulator")]
        if persisted.time_scale > 0.0 {
            self.time_scale = persisted.time_scale;
        }
        self.auto_clear_roi = persisted.auto_clear_roi;
        self.peak_font = persisted.peak_font.clamp(9.0, 18.0);
        if !persisted.default_format.is_empty() {
            self.default_format = persisted.default_format;
        }
        self.reopen_last = persisted.reopen_last;
    }

    /// Picks up where the last session left off, when the preference asks for
    /// it and nothing was named on the command line.
    pub fn maybe_reopen_last(&mut self, had_arguments: bool) {
        if !self.reopen_last || had_arguments {
            return;
        }
        if let Some(path) = self.recent.first().cloned() {
            self.recall_path(path);
        }
    }

    /// Builds the application without touching egui.
    ///
    /// Everything except drawing goes through this, so the tests drive exactly
    /// the state machine the interface drives.
    pub fn headless() -> Self {
        let theme = crate::theme::Theme::default();
        // A shipped build starts with no instruments at all: the only counts it
        // can ever show came off real hardware or out of a file. The simulator
        // exists so the test suites have something to drive.
        #[cfg(feature = "simulator")]
        let detector_list = DetectorList::with_simulator();
        #[cfg(not(feature = "simulator"))]
        let detector_list = DetectorList::default();
        #[cfg(feature = "simulator")]
        let detectors: Vec<AnyMcb> = detector_list
            .entries()
            .iter()
            .map(|entry| {
                AnyMcb::from(
                    SimulatedMcb::builder(entry.number, &entry.name)
                        .channels(entry.channels)
                        .build(),
                )
            })
            .collect();
        #[cfg(not(feature = "simulator"))]
        let detectors: Vec<AnyMcb> = Vec::new();
        let opening_advice = if detectors.is_empty() {
            "ortseam ready. File/Recall opens a spectrum; \
             Services/Detector List connects an instrument."
        } else {
            "ortseam ready. Acquire/Start begins a count; File/Recall opens a spectrum."
        };
        let mut app = Self {
            instruments: InstrumentScan::default(),
            looked_for_instruments: false,
            // Headless construction never shows a window, and revealing one
            // that does not exist is harmless, so this starts false either way.
            shown: false,
            detectors,
            detector_list,
            windows: Vec::new(),
            active: None,
            next_id: 1,
            settings: CalculationSettings::default(),
            library: NuclideLibrary::standard(),
            library_path: None,
            colors: theme.colors(),
            theme,
            history: UndoStack::default(),
            status: opening_advice.to_string(),
            mark_mode: MarkMode::Off,
            dialogs: Dialogs::default(),
            efficiency: None,
            sample: SampleInfo::default(),
            alarms: AlarmMonitor::default(),
            queue: SampleQueue::default(),
            changer: SampleChanger::new(8),
            qa: Vec::new(),
            analysis: None,
            report_text: None,
            job: None,
            job_wait: None,
            job_variables: ortseam_jobs::JobVariables::new(),
            job_directory: std::env::current_dir().unwrap_or_else(|_| PathBuf::from(".")),
            strip_name: None,
            time_scale: 1.0,
            recent: Vec::new(),
            pending_tiling: None,
            pending_close: None,
            pending_collapse: None,
            pending_place: None,
            pending_clipboard: None,
            applied_theme: None,
            auto_clear_roi: false,
            peak_font: 11.0,
            default_format: "chn".into(),
            reopen_last: false,
            window_area: egui::Rect::from_min_size(egui::Pos2::ZERO, egui::vec2(1000.0, 700.0)),
            quit_confirmed: false,
            maximized: None,
            last_tick: Instant::now(),
        };
        if !app.detectors.is_empty() {
            app.open_detector(0);
        }
        app
    }

    /// The spectrum a window shows.
    fn spectrum_of<'a>(
        windows: &'a [SpectrumWindow],
        detectors: &'a [AnyMcb],
        index: usize,
    ) -> Option<&'a Spectrum> {
        let window = windows.get(index)?;
        match window.target {
            Target::Buffer => window.buffer.as_ref(),
            Target::Detector(detector) => detectors.get(detector).map(|mcb| mcb.spectrum()),
        }
    }

    /// The active window's spectrum.
    pub fn active_spectrum(&self) -> Option<&Spectrum> {
        Self::spectrum_of(&self.windows, &self.detectors, self.active?)
    }

    /// The active window's spectrum, mutably.
    pub fn active_spectrum_mut(&mut self) -> Option<&mut Spectrum> {
        let index = self.active?;
        let window = self.windows.get_mut(index)?;
        match window.target {
            Target::Buffer => window.buffer.as_mut(),
            Target::Detector(detector) => self
                .detectors
                .get_mut(detector)
                .map(|mcb| mcb.spectrum_mut()),
        }
    }

    /// The active detector's index, when a detector window is active.
    pub fn active_detector_index(&self) -> Option<usize> {
        match self.windows.get(self.active?)?.target {
            Target::Detector(detector) => Some(detector),
            Target::Buffer => None,
        }
    }

    /// The active detector, when a detector window is active.
    fn active_detector(&mut self) -> Option<&mut AnyMcb> {
        let index = self.active?;
        match self.windows.get(index)?.target {
            Target::Detector(detector) => self.detectors.get_mut(detector),
            Target::Buffer => None,
        }
    }

    /// Arranges every visible window (Window/Tile Horizontally, Tile Vertically,
    /// Cascade).
    ///
    /// The rects are applied on the next frame and then released, so the windows
    /// land where asked and stay free to move afterwards. Tiling leaves
    /// full-area mode: arranging windows only means something when more than one
    /// is on screen.
    fn tile(&mut self, tiling: Tiling) {
        self.maximized = None;
        let count = self.visible_windows().len();
        if count == 0 {
            self.status = "no windows to arrange".into();
            return;
        }
        // The rects themselves are worked out at drawing time, from the area the
        // windows are actually drawn into that frame.
        self.pending_tiling = Some(tiling);
        self.status = match tiling {
            Tiling::Rows => format!("{count} window(s) tiled horizontally"),
            Tiling::Columns => format!("{count} window(s) tiled vertically"),
            Tiling::Cascade => format!("{count} window(s) cascaded"),
        };
    }

    /// The rect each of `count` windows gets under an arrangement of `area`.
    fn layout_rects(tiling: Tiling, area: egui::Rect, count: usize) -> Vec<egui::Rect> {
        (0..count)
            .map(|position| match tiling {
                Tiling::Rows => {
                    let height = area.height() / count as f32;
                    egui::Rect::from_min_size(
                        area.min + egui::vec2(0.0, position as f32 * height),
                        egui::vec2(area.width(), height - 4.0),
                    )
                }
                Tiling::Columns => {
                    let width = area.width() / count as f32;
                    egui::Rect::from_min_size(
                        area.min + egui::vec2(position as f32 * width, 0.0),
                        egui::vec2(width - 4.0, area.height()),
                    )
                }
                Tiling::Cascade => {
                    let step = 30.0;
                    let offset = position as f32 * step;
                    let size = egui::vec2(
                        (area.width() * 0.62).max(360.0),
                        (area.height() * 0.62).max(260.0),
                    );
                    egui::Rect::from_min_size(
                        (area.min + egui::vec2(offset, offset)).min(area.max - size * 0.5),
                        size,
                    )
                }
            })
            .collect()
    }

    /// Records what an instrument called itself, so the next open can check.
    fn remember_serial(&mut self, number: u16, serial: &str) {
        let Some(mut entry) = self.detector_list.get(number).cloned() else {
            return;
        };
        entry.serial = serial.to_string();
        self.detector_list.push(entry);
    }

    /// Renames a detector: the saved entry, the open instrument, its windows.
    ///
    /// All three, or the rename is only half true - the pick list would say
    /// one thing, the window title another, and a spectrum saved afterwards a
    /// third.
    fn rename_detector(&mut self, index: usize, name: &str) {
        let name = name.trim();
        if name.is_empty() {
            self.status = "a detector needs a name".into();
            return;
        }
        let Some(detector) = self.detectors.get_mut(index) else {
            return;
        };
        let number = detector.identity().number;
        let was = detector.identity().name.clone();
        if was == name {
            return;
        }
        detector.set_name(name);
        if let Some(mut entry) = self.detector_list.get(number).cloned() {
            entry.name = name.to_string();
            self.detector_list.push(entry);
        }
        for window in &mut self.windows {
            if window.target == Target::Detector(index) {
                window.title = name.to_string();
            }
        }
        self.status = format!("detector {number} is now {name}");
    }

    /// Removes a window and hands its roles - active, filling the area - to
    /// whatever is left.
    fn close_window(&mut self, id: usize) {
        // What was active stays active by identity, not by position: closing
        // a background window must not silently retarget later commands.
        let active_id = self.active.and_then(|index| {
            self.windows
                .get(index)
                .map(|window| window.id)
                .filter(|active| *active != id)
        });
        let before = self.windows.len();
        self.windows.retain(|window| window.id != id);
        if self.windows.len() == before {
            // Nothing by that id: nothing to retarget either.
            return;
        }
        self.active = active_id
            .and_then(|id| self.windows.iter().position(|window| window.id == id))
            .or(if self.windows.is_empty() {
                None
            } else {
                Some(self.windows.len() - 1)
            });
        // Closing the window that filled the area hands the area to
        // whatever is left, rather than leaving it blank.
        if self.maximized == Some(id) {
            self.maximized = self.active.map(|index| self.windows[index].id);
        }
        self.validate_maximized();
        if self.pending_close == Some(id) {
            self.pending_close = None;
        }
    }

    /// Puts the marker at an energy - or at a channel, when the spectrum is
    /// uncalibrated - and centres the view on it.
    fn goto_energy(&mut self, value: f64) {
        let Some(spectrum) = self.active_spectrum() else {
            self.status = "open a spectrum first".into();
            return;
        };
        let channel = match &spectrum.energy_calibration {
            Some(cal) => match cal.channel(value) {
                Some(channel) if channel >= 0.0 && (channel as usize) < spectrum.len() => {
                    channel.round() as usize
                }
                _ => {
                    self.status = format!("{value:.1} {} is outside the spectrum", cal.units);
                    return;
                }
            },
            None => {
                let channel = value.round().max(0.0) as usize;
                if channel >= spectrum.len() {
                    self.status = format!("channel {channel} is outside the spectrum");
                    return;
                }
                channel
            }
        };
        self.apply_one(Action::Marker(channel));
        self.apply_one(Action::Center);
        self.status = match self
            .active_spectrum()
            .and_then(|spectrum| spectrum.energy_calibration.as_ref())
        {
            Some(cal) => format!("marker at {value:.1} {} (channel {channel})", cal.units),
            None => format!("marker at channel {channel}"),
        };
    }

    /// Makes the next (or previous) open window the active one; in full-area
    /// mode it flips which window fills the area, so Ctrl+Tab pages through
    /// maximised spectra.
    fn cycle_window(&mut self, direction: i32) {
        // Every open window with something to show - deliberately not
        // `visible_windows`, which in full-area mode holds only the one window
        // filling the area; the cycle must reach the others.
        let candidates: Vec<usize> = (0..self.windows.len())
            .filter(|index| {
                self.windows[*index].open
                    && Self::spectrum_of(&self.windows, &self.detectors, *index).is_some()
            })
            .collect();
        if candidates.len() < 2 {
            return;
        }
        let position = self
            .active
            .and_then(|active| candidates.iter().position(|index| *index == active))
            .unwrap_or(0);
        let count = candidates.len() as i32;
        let next = candidates[(position as i32 + direction).rem_euclid(count) as usize];
        self.focus_window(next);
        self.status = format!("window: {}", self.windows[next].title);
    }

    /// Which windows the window pass paints, in drawing order.
    ///
    /// The rule lives here rather than inline in the drawing code so that it can
    /// be tested: one window may fill the spectrum area, hiding the rest, and a
    /// window with nothing to show is not drawn at all.
    pub fn visible_windows(&self) -> Vec<usize> {
        (0..self.windows.len())
            .filter(|index| {
                let window = &self.windows[*index];
                if !window.open || self.maximized.is_some_and(|id| id != window.id) {
                    return false;
                }
                Self::spectrum_of(&self.windows, &self.detectors, *index).is_some()
            })
            .collect()
    }

    /// Makes a window the active one, and the visible one when the display is in
    /// full-area mode.
    ///
    /// Without the second half, opening a spectrum while another window filled
    /// the area would load the data into a window nothing ever drew.
    fn focus_window(&mut self, index: usize) {
        let Some(window) = self.windows.get(index) else {
            return;
        };
        let id = window.id;
        self.active = Some(index);
        if self.maximized.is_some() {
            self.maximized = Some(id);
        }
    }

    /// Forgets a full-area window that no longer exists, so the spectrum area
    /// can never end up blank.
    fn validate_maximized(&mut self) {
        if let Some(id) = self.maximized
            && !self.windows.iter().any(|window| window.id == id)
        {
            self.maximized = None;
        }
    }

    fn open_detector(&mut self, detector: usize) {
        let Some(mcb) = self.detectors.get(detector) else {
            return;
        };
        if let Some(existing) = self
            .windows
            .iter()
            .position(|window| window.target == Target::Detector(detector))
        {
            self.windows[existing].open = true;
            self.focus_window(existing);
            return;
        }
        let title = format!(
            "Detector {} - {}",
            mcb.identity().number,
            mcb.identity().name
        );
        let length = mcb.spectrum().len();
        let id = self.next_id;
        self.next_id += 1;
        let mut window = SpectrumWindow::new(id, title, Target::Detector(detector), None);
        window.display = DisplayState::for_length(length);
        self.windows.push(window);
        self.focus_window(self.windows.len() - 1);
    }

    /// Opens a buffer window holding a spectrum.
    pub fn open_buffer(&mut self, title: String, spectrum: Spectrum, path: Option<PathBuf>) {
        let id = self.next_id;
        self.next_id += 1;
        let mut window = SpectrumWindow::new(id, title, Target::Buffer, Some(spectrum));
        window.path = path;
        self.windows.push(window);
        self.focus_window(self.windows.len() - 1);
    }

    /// Records the active spectrum before a command changes it.
    pub fn push_undo(&mut self, label: &str) {
        let Some(index) = self.active else { return };
        let Some(spectrum) = Self::spectrum_of(&self.windows, &self.detectors, index).cloned()
        else {
            return;
        };
        let window = &self.windows[index];
        self.history.push(
            label,
            UndoSnapshot {
                target: window.target,
                window: window.id,
                spectrum,
                title: window.title.clone(),
            },
        );
    }

    /// Takes back the last destructive command.
    ///
    /// A buffer is restored in place. A detector is never written back to, so its
    /// data is recovered into a new buffer window, exactly as a recall would open
    /// one.
    fn undo(&mut self) {
        let current = self.active.and_then(|index| {
            Self::spectrum_of(&self.windows, &self.detectors, index)
                .cloned()
                .map(|spectrum| {
                    let window = &self.windows[index];
                    UndoSnapshot {
                        target: window.target,
                        window: window.id,
                        spectrum,
                        title: window.title.clone(),
                    }
                })
        });
        let Some(current) = current else {
            self.status = "nothing to undo".into();
            return;
        };
        let Some((label, snapshot)) = self.history.undo(current) else {
            self.status = "nothing to undo".into();
            return;
        };

        // A buffer is restored in place; a detector snapshot is recovered into a
        // new buffer window, because instrument memory is not written back to.
        let existing = match snapshot.target {
            Target::Buffer => self
                .windows
                .iter()
                .position(|window| window.id == snapshot.window && window.is_buffer()),
            Target::Detector(_) => None,
        };
        match existing {
            Some(index) => {
                let length = snapshot.spectrum.len();
                self.windows[index].buffer = Some(snapshot.spectrum);
                self.windows[index].display.set_length(length);
                self.windows[index].modified = true;
                self.active = Some(index);
                self.status = format!("undid {label}");
            }
            None => {
                let title = format!("Recovered - {} ({label})", snapshot.title);
                self.open_buffer(title, snapshot.spectrum, None);
                self.status = format!(
                    "undid {label}: the data was recovered into a buffer window, because \
                     instrument memory is not written back to"
                );
            }
        }
    }

    fn redo(&mut self) {
        let current = self.active.and_then(|index| {
            Self::spectrum_of(&self.windows, &self.detectors, index)
                .cloned()
                .map(|spectrum| {
                    let window = &self.windows[index];
                    UndoSnapshot {
                        target: window.target,
                        window: window.id,
                        spectrum,
                        title: window.title.clone(),
                    }
                })
        });
        let Some(current) = current else { return };
        match self.history.redo(current) {
            Some((label, snapshot)) => {
                if let Some(index) = match snapshot.target {
                    Target::Buffer => self
                        .windows
                        .iter()
                        .position(|window| window.id == snapshot.window && window.is_buffer()),
                    Target::Detector(_) => None,
                } {
                    let length = snapshot.spectrum.len();
                    self.windows[index].buffer = Some(snapshot.spectrum);
                    self.windows[index].display.set_length(length);
                    self.active = Some(index);
                }
                self.status = format!("redid {label}");
            }
            None => self.status = "nothing to redo".into(),
        }
    }

    /// Advances every running detector by the elapsed wall-clock time.
    fn tick(&mut self) {
        // Look for instruments once, on the first frame after the window is up,
        // and only when there is nothing configured. Somebody with an
        // instrument plugged in should find it waiting for them rather than
        // have to know that a button exists. `shown` gates it to the second
        // frame: the probe is synchronous, and running it before the first
        // frame was on screen held the window invisible for its whole
        // duration - the launch appeared to hang.
        if !self.looked_for_instruments && self.shown {
            self.looked_for_instruments = true;
            if self.detectors.is_empty() && std::env::var("ORTSEAM_NO_OPEN").is_err() {
                self.scan_local_instruments();
            }
        }
        let elapsed = self.last_tick.elapsed().as_secs_f64() * self.time_scale;
        self.last_tick = Instant::now();
        self.advance_by(elapsed);
    }

    /// Advances the session by a given number of seconds.
    ///
    /// Every frame does this with the time that really passed; tests pass a fixed
    /// number so that a whole count can be run without waiting for it.
    pub fn advance_by(&mut self, elapsed: f64) {
        if elapsed <= 0.0 {
            return;
        }
        let mut stopped: Vec<(u16, PresetKind)> = Vec::new();
        let mut troubled: Vec<(u16, String)> = Vec::new();
        for detector in &mut self.detectors {
            // Idle instruments still advance: the tuning routines run on the
            // instrument's clock whether or not a count does.
            match advance(detector, elapsed) {
                Ok(Some(kind)) => stopped.push((detector.identity().number, kind)),
                Ok(None) => {}
                // A network instrument that stops answering must be named,
                // not silently frozen on screen.
                Err(error) => troubled.push((detector.identity().number, error.to_string())),
            }
        }
        for (number, error) in troubled {
            self.status = format!("Detector {number}: {error}");
        }
        for (number, kind) in stopped {
            self.status = format!("Detector {number} stopped: {} preset reached", kind.label());
            // A queued sample that was counting is finished, and the changer
            // moves on, so a batch runs unattended.
            if self
                .queue
                .current()
                .is_some_and(|job| job.state == ortseam_device::JobState::Counting)
            {
                crate::jobs::finish_current_sample(self);
                self.status = format!(
                    "{self_status} - sample complete, {} left in the queue",
                    self.queue.remaining(),
                    self_status = self.status
                );
            }
        }
        // Keep the changer moving.
        if !self.changer.is_ready() {
            self.changer.poll(elapsed);
        }
        // Watch the detectors against the alarm limits.
        for detector in &self.detectors {
            self.alarms.evaluate(&detector.status());
        }
        // Keep every window's view in step with its spectrum length.
        for index in 0..self.windows.len() {
            let length = Self::spectrum_of(&self.windows, &self.detectors, index)
                .map(|spectrum| spectrum.len())
                .unwrap_or(0);
            self.windows[index].display.set_length(length);
        }
    }

    /// Applies a batch of actions.
    fn apply(&mut self, actions: Vec<Action>) {
        for action in actions {
            self.apply_one(action);
        }
    }

    fn apply_one(&mut self, action: Action) {
        match action {
            Action::Recall => self.recall_dialog(),
            Action::RecallFile(path) => self.recall_path(path),
            Action::Save => self.save(false),
            Action::SaveAs => self.save(true),
            Action::Export => self.export(),
            Action::ExportPlot => self.export_plot(),
            Action::Import => self.recall_dialog(),
            Action::Print => self.print_to_browser(),
            Action::RoiReport => self.build_roi_report(),
            Action::Compare => self.compare_dialog(),
            Action::ClearCompare => {
                if let Some(index) = self.active {
                    self.windows[index].compare = None;
                    self.status = "comparison cleared".into();
                }
            }
            Action::Exit => self.dialogs.open(Dialog::Exit),
            Action::Undo => self.undo(),
            Action::Redo => self.redo(),
            Action::CopyPeakInfo => {
                let text = self.active.and_then(|index| {
                    let info = self.windows[index].peak_info.as_ref()?;
                    let spectrum = Self::spectrum_of(&self.windows, &self.detectors, index)?;
                    Some(
                        crate::view::peak_info_lines(spectrum, info, Some(&self.library))
                            .join("\n"),
                    )
                });
                match text {
                    Some(text) => {
                        self.pending_clipboard = Some(text);
                        self.status = "peak information copied".into();
                    }
                    None => self.status = "open a peak's information first (Peak Info)".into(),
                }
            }
            Action::CopyReport => {
                if let Some(spectrum) = self.active_spectrum() {
                    let options = ReportOptions {
                        format: ortseam_report::ReportFormat::Column,
                        settings: self.settings,
                        library: Some(&self.library),
                        match_tolerance: 2.0,
                        displayed: None,
                    };
                    self.pending_clipboard = Some(roi_report(spectrum, &options));
                    self.status = "region report copied".into();
                } else {
                    self.status = "open a spectrum first".into();
                }
            }
            Action::CopyCsv => {
                if let Some(spectrum) = self.active_spectrum() {
                    self.pending_clipboard = Some(ortseam_report::spectrum_csv(spectrum));
                    self.status = "spectrum copied as CSV - paste into a spreadsheet".into();
                } else {
                    self.status = "open a spectrum first".into();
                }
            }
            Action::Start => match self.active_detector() {
                Some(detector) => match detector.start() {
                    Ok(()) => self.status = "acquisition started".into(),
                    Err(error) => self.status = error.to_string(),
                },
                None => self.status = "select a detector window to start a count".into(),
            },
            Action::Stop => match self.active_detector() {
                Some(detector) => {
                    let _ = detector.stop();
                    self.status = "acquisition stopped".into();
                }
                None => self.status = "select a detector window to stop a count".into(),
            },
            Action::Clear => self.clear_active(),
            Action::CopyToBuffer => self.copy_to_buffer(),
            Action::ToggleListMode => self.toggle_list_mode(),
            Action::ViewZdt => self.toggle_zdt(),
            Action::StoreSpectrum => {
                let result = self
                    .active_detector()
                    .map(|detector| detector.store_spectrum());
                self.status = match result {
                    Some(Ok(count)) => {
                        format!("spectrum stored in the instrument ({count} held)")
                    }
                    Some(Err(error)) => format!("could not store: {error}"),
                    None => "storing needs a detector window".into(),
                };
            }
            Action::DownloadSpectra => self.download_spectra(),
            Action::StartOptimize => {
                let result = self
                    .active_detector()
                    .map(|detector| detector.start_optimize());
                self.status = match result {
                    Some(Ok(())) => "optimising: watch the pole zero settle in InSight".into(),
                    Some(Err(error)) => format!("could not optimise: {error}"),
                    None => "optimising needs a detector window".into(),
                };
            }
            Action::StartPoleZero => {
                let result = self
                    .active_detector()
                    .map(|detector| detector.set_pole_zero_running(true));
                self.status = match result {
                    Some(Ok(())) => "automatic pole zero running".into(),
                    Some(Err(error)) => format!("could not start the pole zero: {error}"),
                    None => "the pole zero needs a detector window".into(),
                };
            }
            Action::PeakSearch => {
                self.push_undo("Peak Search");
                let settings = self.settings;
                let auto_clear = self.auto_clear_roi;
                if let Some(spectrum) = self.active_spectrum_mut() {
                    // ROI/Auto Clear: the search starts from a clean slate
                    // instead of adding to whatever was marked before.
                    if auto_clear {
                        spectrum.rois = ortseam_core::RoiSet::default();
                    }
                    let found = mark_peaks(spectrum, &settings);
                    self.status = format!("{found} peak(s) marked");
                }
            }
            Action::PeakInfoAtMarker => self.peak_info_at_marker(),
            Action::InputCountRate => {
                let rate = self
                    .active_detector()
                    .map(|detector| detector.status().input_count_rate);
                self.status = match rate {
                    Some(rate) => format!("Input Count Rate: {rate:.0} cps"),
                    None => "the input count rate is a detector reading".into(),
                };
            }
            Action::Sum => self.sum_selection(),
            Action::Smooth => {
                if self.active_is_buffer() {
                    self.push_undo("Smooth");
                    if let Some(spectrum) = self.active_spectrum_mut() {
                        smooth(spectrum);
                    }
                    self.mark_modified();
                    self.status = "smoothed (Edit/Undo takes it back)".into();
                } else {
                    self.status = "Smooth works on a buffer; use Acquire/Copy to Buffer".into();
                }
            }
            Action::Strip => self.dialogs.open(Dialog::Strip),
            Action::ListDataRange => self.dialogs.open(Dialog::ListRange),
            Action::MarkMode(mode) => {
                self.mark_mode = mode;
                self.status = format!("region marking: {}", mode.label());
            }
            Action::MarkPeak => self.mark_peak_at_marker(),
            Action::ClearRoi => {
                let marker = self.active_marker();
                self.push_undo("Clear ROI");
                if let Some(spectrum) = self.active_spectrum_mut() {
                    if spectrum.rois.clear_at(marker) {
                        self.status = "region cleared".into();
                    } else {
                        self.status = "no region at the marker".into();
                    }
                }
            }
            Action::ClearAllRois => {
                self.push_undo("Clear All ROIs");
                if let Some(spectrum) = self.active_spectrum_mut() {
                    spectrum.rois.clear_all();
                }
                self.status = "all regions cleared".into();
            }
            Action::SaveRoiFile => self.save_roi_file(),
            Action::RecallRoiFile => self.recall_roi_file(),
            Action::MarkRange(low, high) => {
                self.push_undo("Mark ROI");
                if let Some(spectrum) = self.active_spectrum_mut() {
                    spectrum.rois.mark(Roi::new(low, high));
                }
                self.status = format!("marked channels {low}-{high}");
            }
            Action::UnmarkRange(low, high) => {
                self.push_undo("UnMark ROI");
                if let Some(spectrum) = self.active_spectrum_mut() {
                    spectrum.rois.unmark(Roi::new(low, high));
                }
                self.status = format!("unmarked channels {low}-{high}");
            }
            Action::ZoomIn => self.with_display(|display| display.zoom_in()),
            Action::ZoomOut => self.with_display(|display| display.zoom_out()),
            Action::ZoomAbout(channel, direction) => self.with_display(|display| {
                let factor = if direction > 0 {
                    1.0 / crate::viewmodel::ZOOM_STEP
                } else {
                    crate::viewmodel::ZOOM_STEP
                };
                display.zoom_about(channel, factor);
            }),
            Action::GotoEnergy(value) => self.goto_energy(value),
            Action::CycleWindow(direction) => self.cycle_window(direction),
            Action::Center => self.with_display(|display| display.center_on_marker()),
            Action::FullView => self.with_display(|display| display.full_view()),
            Action::ToggleLog => {
                if let Some(index) = self.active
                    && let Some(spectrum) =
                        Self::spectrum_of(&self.windows, &self.detectors, index).cloned()
                {
                    self.windows[index].display.toggle_log(&spectrum);
                }
            }
            Action::AutoScale => {
                self.with_display(|display| display.vertical = VerticalScale::Automatic)
            }
            Action::BaselineZoom => {
                if let Some(index) = self.active {
                    let baseline = self.windows[index].display.baseline;
                    self.windows[index].display.baseline = if baseline == 0 {
                        Self::spectrum_of(&self.windows, &self.detectors, index)
                            .map(|spectrum| {
                                let (start, end) = self.windows[index].display.visible();
                                spectrum.channels[start.min(spectrum.len().saturating_sub(1))
                                    ..=end.min(spectrum.len().saturating_sub(1))]
                                    .iter()
                                    .copied()
                                    .min()
                                    .unwrap_or(0)
                            })
                            .unwrap_or(0)
                    } else {
                        0
                    };
                }
            }
            Action::SetFill(fill) => self.with_display(|display| display.fill = fill),
            Action::ToggleIsotopeMarkers => {
                self.with_display(|display| display.isotope_markers = !display.isotope_markers)
            }
            Action::ToggleEnergyAxis => {
                self.with_display(|display| display.energy_axis = !display.energy_axis)
            }
            Action::OpenDetector(detector) => self.open_detector(detector),
            Action::OpenBuffer => {
                self.open_buffer("Buffer".into(), Spectrum::new(0), None);
                self.status = "empty buffer opened".into();
            }
            Action::CloseWindow(id) => {
                // A buffer with unsaved changes is not closed silently: the
                // window stays, and a Save / Discard / Cancel dialog opens.
                let unsaved = self
                    .windows
                    .iter()
                    .position(|window| window.id == id)
                    .filter(|index| {
                        self.windows[*index].is_buffer() && self.windows[*index].modified
                    });
                match unsaved {
                    Some(index) => {
                        self.windows[index].open = true;
                        self.pending_close = Some(id);
                        self.dialogs.open(Dialog::ConfirmClose);
                    }
                    None => self.close_window(id),
                }
            }
            Action::ForceCloseWindow(id) => self.close_window(id),
            Action::SaveThenClose(id) => {
                if let Some(index) = self.windows.iter().position(|window| window.id == id) {
                    self.active = Some(index);
                    self.save(false);
                    // Only close once the save really happened - cancelling the
                    // file dialog leaves the window (and the data) alone.
                    if !self.windows[index].modified {
                        self.close_window(id);
                    }
                }
            }
            Action::Activate(id) => {
                if let Some(index) = self.windows.iter().position(|window| window.id == id) {
                    self.focus_window(index);
                }
            }
            Action::TileHorizontally => self.tile(Tiling::Rows),
            Action::TileVertically => self.tile(Tiling::Columns),
            Action::Cascade => self.tile(Tiling::Cascade),
            Action::CollapseAll => {
                self.pending_collapse = Some(true);
                self.status = "windows collapsed to their title bars".into();
            }
            Action::ExpandAll => {
                self.pending_collapse = Some(false);
                self.status = "windows expanded".into();
            }
            Action::ToggleMaximize(id) => {
                self.maximized = if self.maximized == Some(id) {
                    self.status = "window restored".into();
                    None
                } else {
                    self.status = "window fills the spectrum area (Ctrl+M restores it)".into();
                    if let Some(index) = self.windows.iter().position(|window| window.id == id) {
                        self.active = Some(index);
                    }
                    Some(id)
                };
            }
            Action::MaximizeActive => {
                if let Some(index) = self.active {
                    let id = self.windows[index].id;
                    // Maximise outright: if another window held the area, the
                    // active one takes it over rather than toggling it away.
                    if self.maximized == Some(id) {
                        self.apply_one(Action::ToggleMaximize(id));
                    } else {
                        self.maximized = Some(id);
                        self.status = "window fills the spectrum area (Ctrl+M restores it)".into();
                    }
                }
            }
            Action::RestoreWindows => {
                if self.maximized.take().is_some() {
                    self.status = "window restored".into();
                }
            }
            Action::Analyse => self.run_analysis(),
            Action::AutoCalibrate(nuclide) => self.auto_calibrate(&nuclide),
            Action::RecallCalibration => self.recall_calibration(),
            Action::DestroyCalibration => {
                if let Some(index) = self.active {
                    self.windows[index].calibration.destroy();
                }
                if let Some(spectrum) = self.active_spectrum_mut() {
                    spectrum.energy_calibration = None;
                }
                self.status = "calibration destroyed".into();
            }
            Action::AddCalibrationPoint(energy) => self.add_calibration_point(energy),
            Action::EditCalibrationPoint(index, channel, value) => {
                self.edit_calibration_point(index, channel, value)
            }
            Action::RemoveCalibrationPoint(index) => self.remove_calibration_point(index),
            Action::LoadLibrary => self.load_library(),
            Action::RunJob => self.dialogs.open(Dialog::JobControl),
            Action::RecordQa => self.record_qa(),
            Action::RemoveDetector(index) => self.remove_detector(index),
            Action::RenameDetector(index, name) => self.rename_detector(index, &name),
            Action::ScanLocalInstruments => self.scan_local_instruments(),
            Action::ConfigureLocalInstruments => self.configure_local_instruments(),
            Action::OpenLocalInstruments => self.open_local_instruments(),
            Action::RegisterFileTypes => self.register_file_types(),
            Action::UnregisterFileTypes => self.unregister_file_types(),
            Action::Show(dialog) => self.dialogs.open(dialog),
            Action::Marker(channel) => {
                if let Some(index) = self.active {
                    self.windows[index].display.set_marker(channel);
                }
            }
            Action::Select(low, high) => {
                if let Some(index) = self.active {
                    self.windows[index].selection = Some((low, high));
                }
            }
            Action::ClearSelection => {
                if let Some(index) = self.active {
                    self.windows[index].selection = None;
                }
            }
            Action::ZoomTo(low, high) => {
                if let Some(index) = self.active {
                    self.windows[index].display.zoom_to(low, high);
                }
            }
            Action::Scroll(delta) => self.with_display(|display| display.scroll(delta)),
            Action::Status(message) => self.status = message,
        }
    }

    fn with_display(&mut self, edit: impl FnOnce(&mut DisplayState)) {
        if let Some(index) = self.active {
            edit(&mut self.windows[index].display);
        }
    }

    fn active_marker(&self) -> usize {
        self.active
            .and_then(|index| self.windows.get(index))
            .map(|window| window.display.marker)
            .unwrap_or(0)
    }

    fn active_is_buffer(&self) -> bool {
        self.active
            .and_then(|index| self.windows.get(index))
            .map(|window| window.is_buffer())
            .unwrap_or(false)
    }

    fn mark_modified(&mut self) {
        if let Some(index) = self.active {
            self.windows[index].modified = true;
        }
    }

    fn clear_active(&mut self) {
        self.push_undo("Clear");
        let is_buffer = self.active_is_buffer();
        if is_buffer {
            if let Some(spectrum) = self.active_spectrum_mut() {
                spectrum.clear();
            }
        } else if let Some(detector) = self.active_detector()
            && let Err(error) = detector.clear()
        {
            self.status = error.to_string();
            return;
        }
        self.mark_modified();
        self.status = "cleared - Edit/Undo recovers the data".into();
    }

    fn copy_to_buffer(&mut self) {
        let Some(index) = self.active else { return };
        let Some(spectrum) = Self::spectrum_of(&self.windows, &self.detectors, index).cloned()
        else {
            return;
        };
        let list = self
            .active_detector()
            .and_then(|detector| detector.list_data().cloned());
        let title = format!("Buffer - {}", self.windows[index].title);
        self.open_buffer(title, spectrum, None);
        if let Some(active) = self.active {
            self.windows[active].list = list;
        }
        self.status = "copied to a buffer window".into();
    }

    fn toggle_list_mode(&mut self) {
        let Some(detector) = self.active_detector() else {
            self.status = "list mode is a detector setting".into();
            return;
        };
        let mode = if detector.mode() == ortseam_device::AcquisitionMode::List {
            ortseam_device::AcquisitionMode::Pha
        } else {
            ortseam_device::AcquisitionMode::List
        };
        match detector.set_mode(mode) {
            Ok(()) => self.status = format!("collection mode: {}", mode.label()),
            Err(error) => self.status = error.to_string(),
        }
    }

    fn toggle_zdt(&mut self) {
        let Some(detector) = self.active_detector() else {
            self.status = "zero dead time is a detector setting".into();
            return;
        };
        let mode = match detector.mode() {
            ortseam_device::AcquisitionMode::Zdt => ortseam_device::AcquisitionMode::Pha,
            _ => ortseam_device::AcquisitionMode::Zdt,
        };
        match detector.set_mode(mode) {
            Ok(()) => self.status = format!("collection mode: {}", mode.label()),
            Err(error) => self.status = error.to_string(),
        }
    }

    fn peak_info_at_marker(&mut self) {
        let Some(index) = self.active else { return };
        let marker = self.windows[index].display.marker;
        let settings = self.settings;
        let Some(spectrum) = Self::spectrum_of(&self.windows, &self.detectors, index) else {
            return;
        };
        let region = spectrum.rois.at(marker).copied().or_else(|| {
            // No region: use three FWHM about the peak, as Peak Info does. The
            // width comes from the shape calibration when there is one;
            // otherwise the peak search measures the peak under the marker
            // from the counts themselves, so an uncalibrated spectrum gets
            // its whole peak - a NaI(Tl) line can be a hundred channels wide -
            // rather than a guessed sliver of it.
            let (centre, fwhm) = match spectrum.fwhm_at(marker as f64) {
                Some(fwhm) => (marker as f64, fwhm),
                None => ortseam_core::peak_search(spectrum, &settings)
                    .into_iter()
                    .map(|peak| ((peak.centroid - marker as f64).abs(), peak))
                    .filter(|(distance, peak)| *distance <= peak.width.max(2.0))
                    .min_by(|a, b| a.0.partial_cmp(&b.0).unwrap_or(std::cmp::Ordering::Equal))
                    .map(|(_, peak)| (peak.centroid, peak.width))
                    .unwrap_or((marker as f64, 8.0)),
            };
            // Never narrower than the background channels need, or the
            // figures cannot be computed at all.
            let width = (fwhm * 1.5).max(settings.background_points as f64 + 2.0);
            let low = (centre - width).max(0.0) as usize;
            let high = ((centre + width) as usize).min(spectrum.len().saturating_sub(1));
            (high > low).then(|| Roi::new(low, high))
        });
        let Some(region) = region else {
            self.status = "could not fit a peak here".into();
            return;
        };
        match peak_info(spectrum, region, &settings) {
            Ok(info) => {
                let calibration = spectrum.energy_calibration.clone();
                let live_time = spectrum.live_time;
                let identification = calibration.as_ref().and_then(|cal| {
                    self.library
                        .best_match(info.centroid_energy(cal), 2.0)
                        .map(|matched| {
                            format!(
                                "{} at {:.2} keV, {:.1} cA",
                                matched.nuclide.name,
                                matched.peak.energy,
                                info.activity(matched.peak.yield_percent, live_time)
                            )
                        })
                });
                self.status = match (&calibration, identification) {
                    (Some(cal), Some(identification)) => format!(
                        "Peak: {:.2} {} | FWHM {:.2} | net {:.0} +/- {:.0} | {identification}",
                        info.centroid_energy(cal),
                        cal.units,
                        info.fwhm_energy(cal),
                        info.net_area,
                        info.net_area_uncertainty
                    ),
                    (Some(cal), None) => format!(
                        "Peak: {:.2} {} | FWHM {:.2} | net {:.0} +/- {:.0}",
                        info.centroid_energy(cal),
                        cal.units,
                        info.fwhm_energy(cal),
                        info.net_area,
                        info.net_area_uncertainty
                    ),
                    _ => format!(
                        "Peak: channel {:.2} | FWHM {:.2} ch | net {:.0} +/- {:.0}",
                        info.centroid, info.fwhm, info.net_area, info.net_area_uncertainty
                    ),
                };
                self.windows[index].peak_info = Some(info);
            }
            Err(error) => self.status = format!("Could Not Properly Fit Peak: {error}"),
        }
    }

    fn mark_peak_at_marker(&mut self) {
        let Some(index) = self.active else { return };
        let marker = self.windows[index].display.marker;
        let settings = self.settings;
        let width = Self::spectrum_of(&self.windows, &self.detectors, index)
            .and_then(|spectrum| spectrum.fwhm_at(marker as f64))
            .unwrap_or(8.0);
        let half = (1.5 * width).max((settings.background_points * 2 + 1) as f64 / 2.0);
        self.push_undo("Mark Peak");
        if let Some(spectrum) = self.active_spectrum_mut() {
            let low = (marker as f64 - half).max(0.0) as usize;
            let high = ((marker as f64 + half) as usize).min(spectrum.len().saturating_sub(1));
            if high > low {
                spectrum.rois.mark(Roi::new(low, high));
            }
        }
        self.status = "peak marked".into();
    }

    fn sum_selection(&mut self) {
        let Some(index) = self.active else { return };
        let window = &self.windows[index];
        let marker = window.display.marker;
        let selection = window.selection;
        let Some(spectrum) = Self::spectrum_of(&self.windows, &self.detectors, index) else {
            return;
        };
        let (low, high) = match selection {
            Some(range) => range,
            None => match spectrum.rois.at(marker) {
                Some(roi) => (roi.start, roi.end),
                None => (0, spectrum.len().saturating_sub(1)),
            },
        };
        let sum = spectrum.sum_range(low, high);
        self.status = format!("Sum of Counts from Channel {low} to {high} is: {sum}");
    }

    /// The channel a calibration point would be entered at, and whether it came
    /// from a region's peak rather than the marker itself.
    ///
    /// The manual (§4.3.2.2): "if no ROI is present at the marker, the marker
    /// channel itself will be used. However, that is less accurate than using
    /// the ROI method." The dialog shows what this returns, so what is
    /// displayed and what is entered cannot drift apart.
    pub fn calibration_channel(&self) -> Option<(f64, bool)> {
        let index = self.active?;
        let marker = self.windows[index].display.marker;
        let settings = self.settings;
        let centroid =
            Self::spectrum_of(&self.windows, &self.detectors, index).and_then(|spectrum| {
                spectrum
                    .rois
                    .at(marker)
                    .copied()
                    .and_then(|roi| peak_info(spectrum, roi, &settings).ok())
                    .map(|info| info.centroid)
            });
        Some(match centroid {
            Some(centroid) => (centroid, true),
            None => (marker as f64, false),
        })
    }

    fn add_calibration_point(&mut self, energy: f64) {
        let Some(index) = self.active else { return };
        let (channel, _from_roi) = self.calibration_channel().unwrap_or((0.0, false));
        self.windows[index]
            .calibration
            .add(CalibrationPoint::new(channel, energy));
        let points = self.windows[index].calibration.len();
        let fit = self.windows[index].calibration.calibration().cloned();
        match fit {
            Some(calibration) => {
                let order = calibration.order();
                if let Some(spectrum) = self.active_spectrum_mut() {
                    spectrum.energy_calibration = Some(calibration);
                }
                self.status = format!(
                    "calibration point {points} entered at channel {channel:.2}; {} fit in force",
                    if order >= 2 { "quadratic" } else { "linear" }
                );
            }
            None => {
                // A line through one point is not a line. The manual is
                // explicit - "if two points are entered, a linear calibration
                // is done" - so say which one is missing rather than leaving
                // the spectrum reading "uncalibrated" with no explanation.
                self.status = format!(
                    "point {points} entered at channel {channel:.2}; \
                     mark a second peak and enter its energy to calibrate"
                );
            }
        }
    }

    fn run_analysis(&mut self) {
        let Some(index) = self.active else { return };
        let settings = self.settings;
        let library = self.library.clone();
        let efficiency = self.efficiency.clone();
        let sample = self.sample.clone();
        let Some(spectrum) = Self::spectrum_of(&self.windows, &self.detectors, index).cloned()
        else {
            return;
        };
        let options = AnalysisOptions {
            settings,
            efficiency,
            sample,
            report_mda_for_absent_nuclides: self.dialogs.report_mda,
            correct_decay_during_count: self.dialogs.correct_during_count,
            decay_correct_to_reference: self.dialogs.correct_to_reference,
            ..AnalysisOptions::default()
        };
        let report = analyse(&spectrum, &library, &options);
        self.status = format!(
            "{} nuclide(s) reported, {} detected",
            report.nuclides.len(),
            report.detected().count()
        );
        self.report_text = Some(nuclide_report(&report));
        self.analysis = Some(report);
        self.dialogs.open(Dialog::Analysis);
    }

    fn build_roi_report(&mut self) {
        let Some(index) = self.active else { return };
        let settings = self.settings;
        let library = self.library.clone();
        let column = self.dialogs.report_columns;
        let displayed = self.dialogs.report_displayed_only;
        let visible = self.windows[index].display.visible();
        let Some(spectrum) = Self::spectrum_of(&self.windows, &self.detectors, index).cloned()
        else {
            return;
        };
        let mut options = if column {
            ReportOptions::column()
        } else {
            ReportOptions::paragraph()
        }
        .with_settings(settings)
        .with_library(&library);
        if displayed {
            options = options.displayed(visible.0, visible.1);
        }
        self.report_text = Some(roi_report(&spectrum, &options));
        self.dialogs.open(Dialog::Report);
    }

    fn record_qa(&mut self) {
        let Some(index) = self.active else { return };
        let marker = self.windows[index].display.marker;
        let settings = self.settings;
        let Some(spectrum) = Self::spectrum_of(&self.windows, &self.detectors, index) else {
            return;
        };
        let Some(roi) = spectrum.rois.at(marker).copied() else {
            self.status = "mark the check-source peak first".into();
            return;
        };
        let Ok(info) = peak_info(spectrum, roi, &settings) else {
            self.status = "could not measure the peak".into();
            return;
        };
        let timestamp = spectrum
            .start_time
            .unwrap_or_else(|| chrono::Local::now().naive_local());
        let live_time = spectrum.live_time;
        let (centroid, fwhm, net) = (info.centroid, info.fwhm, info.net_area);
        for (metric, value) in [
            (ortseam_core::QaMetric::Centroid, centroid),
            (ortseam_core::QaMetric::Fwhm, fwhm),
            (
                ortseam_core::QaMetric::NetRate,
                if live_time > 0.0 {
                    net / live_time
                } else {
                    0.0
                },
            ),
        ] {
            let chart = match self.qa.iter_mut().find(|chart| chart.metric == metric) {
                Some(chart) => chart,
                None => {
                    self.qa
                        .push(QaChart::new(metric, ortseam_core::ControlLimits::default()));
                    self.qa.last_mut().expect("just pushed")
                }
            };
            chart.push(ortseam_core::QaRecord::new(timestamp, value));
        }
        self.status = "quality-assurance point recorded".into();
        self.dialogs.open(Dialog::Qa);
    }

    // File operations.

    /// Services/Register: point Explorer's spectrum extensions at this binary.
    fn register_file_types(&mut self) {
        #[cfg(windows)]
        {
            let Ok(exe) = std::env::current_exe() else {
                self.status = "could not find the executable to register".into();
                return;
            };
            let exe = exe.display().to_string();
            for (key, value) in crate::assoc::registration(&exe) {
                let path = format!(r"HKCU\Software\Classes\{key}");
                if let Err(error) = reg(&["add", &path, "/ve", "/d", &value, "/f"]) {
                    self.status = format!("registration failed at {key}: {error}");
                    return;
                }
            }
            self.status = "double-clicking .Spe, .Chn or .Spc now opens ortseam".into();
        }
        #[cfg(not(windows))]
        {
            self.status = "file-type registration is a Windows feature".into();
        }
    }

    /// Services/Unregister: take the association back out, leaving other
    /// programs' claims on the extensions alone.
    fn unregister_file_types(&mut self) {
        #[cfg(windows)]
        {
            let (prog_id, extensions) = crate::assoc::removal();
            let _ = reg(&["delete", &format!(r"HKCU\Software\Classes\{prog_id}"), "/f"]);
            for extension in extensions {
                let _ = reg(&[
                    "delete",
                    &format!(r"HKCU\Software\Classes\{extension}"),
                    "/ve",
                    "/f",
                ]);
            }
            self.status = "spectrum file types unregistered".into();
        }
        #[cfg(not(windows))]
        {
            self.status = "file-type registration is a Windows feature".into();
        }
    }

    fn recall_dialog(&mut self) {
        let Some(path) = crate::dialogs::pick_open_file(&self.spectrum_filters()) else {
            return;
        };
        self.recall_path(path);
    }

    /// Files opened lately, most recent first (File/Recent).
    pub fn recent_files(&self) -> &[PathBuf] {
        &self.recent
    }

    /// Remembers a file in the recent list, without repeats.
    fn remember_file(&mut self, path: &std::path::Path) {
        self.recent.retain(|existing| existing != path);
        self.recent.insert(0, path.to_path_buf());
        self.recent.truncate(RECENT_FILES);
    }

    /// Opens a file into a buffer window.
    pub fn recall_path(&mut self, path: PathBuf) {
        match load_spectrum(&path) {
            Ok(spectrum) => {
                self.remember_file(&path);
                let title = path
                    .file_name()
                    .map(|name| name.to_string_lossy().to_string())
                    .unwrap_or_else(|| "Buffer".into());
                let list = if SpectrumFormat::from_path(&path) == Some(SpectrumFormat::List) {
                    std::fs::read(&path)
                        .ok()
                        .and_then(|bytes| ortseam_formats::list_mode::read(&bytes).ok())
                } else {
                    None
                };
                self.status = format!("{}: {}", title, ortseam_report::summary(&spectrum));
                self.open_buffer(title, spectrum, Some(path));
                if let Some(active) = self.active {
                    self.windows[active].list = list;
                }
            }
            Err(error) => self.status = format!("could not read the file: {error}"),
        }
    }

    fn save(&mut self, force_dialog: bool) {
        let Some(index) = self.active else { return };
        let existing = self.windows[index].path.clone();
        let path = match (existing, force_dialog) {
            (Some(path), false) => Some(path),
            _ => {
                // The suggestion carries the preferred default format, so Save
                // on a fresh buffer lands on the right extension immediately.
                let stem = self
                    .windows
                    .get(index)
                    .map(|window| {
                        std::path::Path::new(&window.title)
                            .file_stem()
                            .map(|stem| stem.to_string_lossy().into_owned())
                            .unwrap_or_else(|| window.title.clone())
                    })
                    .filter(|stem| !stem.is_empty())
                    .unwrap_or_else(|| "spectrum".into());
                crate::dialogs::pick_save_file_named(
                    &self.spectrum_filters(),
                    Some(&format!("{stem}.{}", self.default_format)),
                )
            }
        };
        let Some(path) = path else { return };
        let Some(mut spectrum) = Self::spectrum_of(&self.windows, &self.detectors, index).cloned()
        else {
            return;
        };
        if !self.sample.description.is_empty() {
            spectrum.sample_description = self.sample.description.clone();
        }
        match save_spectrum(&spectrum, &path) {
            Ok(()) => {
                self.status = format!("saved {}", path.display());
                self.windows[index].path = Some(path);
                self.windows[index].modified = false;
            }
            Err(error) => self.status = format!("could not save: {error}"),
        }
    }

    fn export(&mut self) {
        let Some(spectrum) = self.active_spectrum().cloned() else {
            return;
        };
        let Some(path) =
            crate::dialogs::pick_save_file(&[("ASCII", &["txt", "asc"]), ("CSV", &["csv"])])
        else {
            return;
        };
        let text = if path
            .extension()
            .is_some_and(|e| e.eq_ignore_ascii_case("csv"))
        {
            ortseam_report::spectrum_csv(&spectrum)
        } else {
            ortseam_formats::ascii::write(&spectrum, &Default::default())
        };
        match std::fs::write(&path, text) {
            Ok(()) => self.status = format!("exported {}", path.display()),
            Err(error) => self.status = format!("could not export: {error}"),
        }
    }

    /// Writes the active window's plot as an SVG picture (File/Export Plot).
    fn export_plot(&mut self) {
        let Some(index) = self.active else {
            self.status = "open a spectrum first".into();
            return;
        };
        let Some(spectrum) = Self::spectrum_of(&self.windows, &self.detectors, index).cloned()
        else {
            self.status = "nothing to draw".into();
            return;
        };
        let Some(path) = crate::dialogs::pick_save_file(&[("SVG picture", &["svg"])]) else {
            return;
        };
        self.write_plot_svg(&spectrum, index, &path);
    }

    /// The dialog-free half of the plot export, for tests and jobs.
    pub fn write_plot_svg(&mut self, spectrum: &Spectrum, index: usize, path: &std::path::Path) {
        let svg = crate::snapshot::plot_svg(
            spectrum,
            &self.windows[index].display,
            &self.colors,
            Some(&self.library),
            1280.0,
            720.0,
        );
        match std::fs::write(path, svg) {
            Ok(()) => self.status = format!("plot written to {}", path.display()),
            Err(error) => self.status = format!("could not write the picture: {error}"),
        }
    }

    fn compare_dialog(&mut self) {
        let Some(index) = self.active else { return };
        let Some(path) = crate::dialogs::pick_open_file(&self.spectrum_filters()) else {
            return;
        };
        match load_spectrum(&path) {
            Ok(spectrum) => {
                self.windows[index].compare = Some(spectrum);
                self.status =
                    "comparing; Shift+Up and Shift+Down offset the comparison, Esc clears it"
                        .into();
            }
            Err(error) => self.status = format!("could not read the file: {error}"),
        }
    }

    fn save_roi_file(&mut self) {
        let Some(spectrum) = self.active_spectrum() else {
            return;
        };
        let rois = spectrum.rois.clone();
        let Some(path) = crate::dialogs::pick_save_file(&[("Region file", &["roi"])]) else {
            return;
        };
        match save_rois(&rois, &path) {
            Ok(()) => self.status = format!("wrote {}", path.display()),
            Err(error) => self.status = format!("could not write the region file: {error}"),
        }
    }

    /// The printable document for the active spectrum: a self-contained HTML
    /// page - the plot on the Paper palette, the region report, and the
    /// channel printout - ready for the browser's print dialog.
    pub fn print_document(&self) -> Option<String> {
        let index = self.active?;
        let spectrum = Self::spectrum_of(&self.windows, &self.detectors, index)?;
        let colors = crate::theme::Theme::Paper.colors();
        let svg = crate::snapshot::plot_svg(
            spectrum,
            &self.windows[index].display,
            &colors,
            Some(&self.library),
            1000.0,
            560.0,
        );
        let options = ReportOptions {
            format: ortseam_report::ReportFormat::Column,
            settings: self.settings,
            library: Some(&self.library),
            match_tolerance: 2.0,
            displayed: None,
        };
        let report = ortseam_report::roi_report(spectrum, &options);
        let channels = print_spectrum(spectrum, None);
        let escape = |text: &str| {
            text.replace('&', "&amp;")
                .replace('<', "&lt;")
                .replace('>', "&gt;")
        };
        // The provenance a lab book wants next to the picture.
        let mut provenance = String::new();
        if !spectrum.sample_description.trim().is_empty() {
            provenance.push_str(&format!(
                "<p>Sample: {}</p>",
                escape(spectrum.sample_description.trim())
            ));
        }
        if let Some(start) = spectrum.start_time {
            provenance.push_str(&format!(
                "<p>Acquired {}</p>",
                start.format("%Y-%m-%d %H:%M:%S")
            ));
        }
        // The nuclide analysis belongs on the same page when one was run:
        // plot, regions and activities are one lab-book entry, not three.
        let analysis = self
            .analysis
            .as_ref()
            .map(|report| format!("<pre>{}</pre>", escape(&nuclide_report(report))))
            .unwrap_or_default();
        Some(format!(
            "<!DOCTYPE html><html><head><meta charset=\"utf-8\">\
             <title>{title}</title>\
             <style>body{{font-family:monospace;margin:24px;color:#111}}\
             h1{{font-size:16px}}p{{font-size:12px;margin:2px 0}}\
             pre{{font-size:11px;line-height:1.35}}\
             svg{{max-width:100%;height:auto}}\
             .plot{{margin:12px 0}}@media print{{.hint{{display:none}}}}</style>\
             </head><body>\
             <h1>{title}</h1>{provenance}\
             <p class=\"hint\">Use the browser's Print command for paper or PDF.</p>\
             <div class=\"plot\">{svg}</div>\
             <pre>{report}</pre>\
             {analysis}\
             <pre>{channels}</pre>\
             </body></html>",
            title = escape(&self.windows[index].title),
            report = escape(&report),
            channels = escape(&channels),
        ))
    }

    /// File/Print: writes the printable document and hands it to the browser,
    /// whose print dialog does the spooling (§4.1.6, modernised).
    fn print_to_browser(&mut self) {
        let Some(document) = self.print_document() else {
            self.status = "open a spectrum first".into();
            return;
        };
        let path = std::env::temp_dir().join("ortseam-print.html");
        match std::fs::write(&path, document) {
            Ok(()) => match open_with_default_app(&path) {
                Ok(()) => {
                    self.status =
                        "printable report opened in the browser - print from there".into();
                }
                Err(error) => self.status = format!("could not open the browser: {error}"),
            },
            Err(error) => self.status = format!("could not write the report: {error}"),
        }
    }

    /// Places a window at a rect on the next frame (the JOB `ZOOM:` command).
    pub fn place_window(&mut self, id: usize, rect: egui::Rect) {
        self.pending_place = Some((id, rect));
    }

    /// Sets the sample's reference date from typed text (Sample Description).
    ///
    /// Accepts `YYYY-MM-DD` or `YYYY-MM-DD HH:MM`; empty clears it. Activities
    /// decay-correct to this date when the analysis asks for it.
    pub fn set_reference_date(&mut self, text: &str) -> Result<(), String> {
        let trimmed = text.trim();
        if trimmed.is_empty() {
            self.sample.reference_date = None;
            return Ok(());
        }
        let parsed = chrono::NaiveDateTime::parse_from_str(trimmed, "%Y-%m-%d %H:%M")
            .or_else(|_| {
                chrono::NaiveDate::parse_from_str(trimmed, "%Y-%m-%d")
                    .map(|date| date.and_hms_opt(0, 0, 0).expect("midnight exists"))
            })
            .map_err(|_| format!("{trimmed:?} is not YYYY-MM-DD or YYYY-MM-DD HH:MM"))?;
        self.sample.reference_date = Some(parsed);
        Ok(())
    }

    /// Measures efficiency points from a certified source's spectrum: one point
    /// per library line of the nuclide that lands in a marked region, decay-
    /// corrected to the measurement (Efficiency dialog, "From a certified
    /// source").
    ///
    /// Runs a peak search first when nothing is marked, so the whole procedure
    /// is: open the source's spectrum, type the certificate, press Measure.
    pub fn efficiency_points_from_source(
        &mut self,
        nuclide_name: &str,
        activity_bq: f64,
        uncertainty_percent: f64,
        elapsed_days: f64,
    ) -> Result<Vec<ortseam_core::EfficiencyPoint>, String> {
        let nuclide = self
            .library
            .iter()
            .find(|n| n.name == nuclide_name)
            .cloned()
            .ok_or_else(|| format!("{nuclide_name} is not in the working library"))?;
        if activity_bq <= 0.0 {
            return Err("the certified activity must be positive".into());
        }
        if self
            .active_spectrum()
            .is_some_and(|spectrum| spectrum.rois.is_empty())
        {
            self.apply_one(Action::PeakSearch);
        }
        let settings = self.settings;
        let spectrum = self
            .active_spectrum()
            .ok_or_else(|| "open the source's spectrum first".to_string())?;
        let calibration = spectrum
            .energy_calibration
            .clone()
            .ok_or_else(|| "the spectrum needs an energy calibration".to_string())?;
        if spectrum.live_time <= 0.0 {
            return Err("the spectrum carries no live time".into());
        }
        let source = ortseam_core::CertifiedSource {
            nuclide: nuclide.name.clone(),
            activity_bq,
            uncertainty_percent,
            elapsed_seconds: elapsed_days * 86_400.0,
            half_life_seconds: nuclide.half_life_seconds,
        };
        let mut points = Vec::new();
        for line in &nuclide.peaks {
            let Some(channel) = calibration.channel(line.energy) else {
                continue;
            };
            let Some(roi) = spectrum.rois.at(channel.round().max(0.0) as usize) else {
                continue;
            };
            let Ok(info) = ortseam_core::peak_info(spectrum, *roi, &settings) else {
                continue;
            };
            let rate = info.net_count_rate(spectrum.live_time);
            if rate <= 0.0 || line.yield_percent <= 0.0 {
                continue;
            }
            points.push(source.efficiency_point(line.energy, rate, line.yield_percent));
        }
        if points.len() < 2 {
            return Err(format!(
                "only {} of {}'s lines were found in marked regions - two are the least a curve needs",
                points.len(),
                nuclide.name
            ));
        }
        points.sort_by(|a, b| {
            a.energy
                .partial_cmp(&b.energy)
                .unwrap_or(std::cmp::Ordering::Equal)
        });
        Ok(points)
    }

    /// Pulls every spectrum out of the active instrument's memory into buffer
    /// windows (Acquire/Download Spectra, §4.2.6).
    ///
    /// The copies stay in the instrument too; erasing its memory is its own
    /// decision, not a side effect of looking.
    fn download_spectra(&mut self) {
        let Some(detector) = self.active_detector() else {
            self.status = "downloading needs a detector window".into();
            return;
        };
        let name = detector.identity().name.clone();
        let count = detector.stored_spectra();
        if count == 0 {
            self.status = format!("{name} holds no stored spectra");
            return;
        }
        let spectra: Vec<Spectrum> = (0..count)
            .filter_map(|index| detector.stored_spectrum(index).cloned())
            .collect();
        for (index, spectrum) in spectra.into_iter().enumerate() {
            self.open_buffer(format!("{name} store {}", index + 1), spectrum, None);
        }
        self.status = format!("{count} spectrum(s) downloaded from {name}");
    }

    /// Fits an energy calibration by matching found peaks against a library
    /// nuclide's lines (Calculate/Calibration, "from peaks").
    ///
    /// Runs a peak search first when nothing is marked, so "open the Eu-152
    /// spectrum, pick Eu-152, press the button" is the whole procedure.
    pub fn auto_calibrate(&mut self, nuclide: &str) {
        let Some(nuclide) = self.library.iter().find(|n| n.name == nuclide).cloned() else {
            self.status = format!("{nuclide} is not in the working library");
            return;
        };
        if self
            .active_spectrum()
            .is_some_and(|spectrum| spectrum.rois.is_empty())
        {
            self.apply_one(Action::PeakSearch);
        }
        let settings = self.settings;
        let Some(spectrum) = self.active_spectrum() else {
            self.status = "open a spectrum first".into();
            return;
        };
        // Centroids strongest first, which is the order the matcher wants.
        let mut peaks: Vec<(f64, f64)> = spectrum
            .rois
            .iter()
            .filter_map(|roi| ortseam_core::peak_info(spectrum, *roi, &settings).ok())
            .map(|info| (info.centroid, info.net_area))
            .collect();
        peaks.sort_by(|a, b| b.1.partial_cmp(&a.1).unwrap_or(std::cmp::Ordering::Equal));
        let centroids: Vec<f64> = peaks.into_iter().map(|(centroid, _)| centroid).collect();
        let lines: Vec<f64> = nuclide.peaks.iter().map(|peak| peak.energy).collect();

        match ortseam_core::auto_calibrate(&centroids, &lines, "keV") {
            Ok(result) => {
                self.push_undo("Auto Calibrate");
                let matched = result.matches.len();
                let gain = result.calibration.coefficients[1];
                if let Some(index) = self.active {
                    // The matched points go into the table, so the Calibration
                    // dialog shows exactly what the fit stands on.
                    self.windows[index].calibration.destroy();
                    for point in &result.matches {
                        self.windows[index].calibration.add(*point);
                    }
                }
                if let Some(spectrum) = self.active_spectrum_mut() {
                    spectrum.energy_calibration = Some(result.calibration);
                }
                self.status = format!(
                    "calibrated from {matched} {} lines: {gain:.4} keV/ch",
                    nuclide.name
                );
            }
            Err(error) => self.status = format!("could not calibrate: {error}"),
        }
    }

    fn recall_roi_file(&mut self) {
        let Some(path) = crate::dialogs::pick_open_file(&[("Region file", &["roi"])]) else {
            return;
        };
        self.apply_roi_file(&path);
    }

    /// Replaces the active spectrum's regions with those in a `.Roi` file.
    pub fn apply_roi_file(&mut self, path: &std::path::Path) {
        match load_rois(path) {
            Ok(rois) => {
                self.push_undo("Recall ROI File");
                if let Some(spectrum) = self.active_spectrum_mut() {
                    spectrum.rois = rois;
                }
                self.status = format!("regions read from {}", path.display());
            }
            Err(error) => self.status = format!("could not read the region file: {error}"),
        }
    }

    fn recall_calibration(&mut self) {
        let Some(path) = crate::dialogs::pick_open_file(&self.spectrum_filters()) else {
            return;
        };
        self.apply_calibration_from(&path);
    }

    /// Copies another spectrum's energy and shape calibration onto the active one.
    pub fn apply_calibration_from(&mut self, path: &std::path::Path) {
        match load_spectrum(path) {
            Ok(other) => {
                let (energy, shape) = (other.energy_calibration, other.shape_calibration);
                if let Some(spectrum) = self.active_spectrum_mut() {
                    spectrum.energy_calibration = energy;
                    spectrum.shape_calibration = shape;
                }
                self.status = format!("calibration read from {}", path.display());
            }
            Err(error) => self.status = format!("could not read the file: {error}"),
        }
    }

    fn load_library(&mut self) {
        let Some(path) = crate::dialogs::pick_open_file(&[("Library", &["json", "csv", "lib"])])
        else {
            return;
        };
        match ortseam_formats::library::load(&path) {
            Ok(library) => {
                self.status = format!("library {} with {} nuclides", path.display(), library.len());
                self.library = library;
                self.library_path = Some(path);
            }
            Err(error) => self.status = format!("could not read the library: {error}"),
        }
    }

    /// Applies a strip operation from the Strip dialog.
    /// Whether it stripped: the caller's status must not claim success over
    /// the failure message this leaves behind.
    pub fn strip_from(&mut self, path: PathBuf, factor: Option<f64>) -> bool {
        let Some(disk) = load_spectrum(&path).ok() else {
            self.status = format!("could not read {}", path.display());
            return false;
        };
        self.push_undo("Strip");
        let mode = match factor {
            Some(value) => StripFactor::Fixed(value),
            None => StripFactor::LiveTimeRatio,
        };
        let outcome = match self.active_spectrum_mut() {
            Some(spectrum) => strip(spectrum, &disk, mode),
            None => return false,
        };
        match outcome {
            Ok(()) => {
                self.mark_modified();
                self.status = format!("stripped {}", path.display());
                true
            }
            Err(error) => {
                self.status = format!("could not strip: {error}");
                false
            }
        }
    }

    fn spectrum_filters(&self) -> Vec<(&'static str, &'static [&'static str])> {
        vec![
            (
                "Spectrum files",
                &["chn", "spc", "spe", "json", "txt", "lis"],
            ),
            ("Integer .Chn", &["chn"]),
            ("ORTEC .Spc", &["spc"]),
            ("IAEA .Spe", &["spe"]),
            ("ortseam native", &["json"]),
            ("List mode", &["lis"]),
        ]
    }

    /// Carries out one action; jobs and tests use this so they do exactly what
    /// the menus do.
    pub fn apply_action(&mut self, action: Action) {
        self.apply_one(action);
    }

    /// The detector a job command applies to: the active detector window's, or
    /// the first configured detector.
    pub(crate) fn job_detector(&self) -> Option<usize> {
        if let Some(index) = self.active
            && let Target::Detector(detector) = self.windows[index].target
        {
            return Some(detector);
        }
        (!self.detectors.is_empty()).then_some(0)
    }

    /// Edits the job's detector, reporting when there is none.
    pub(crate) fn with_job_detector(
        &mut self,
        edit: impl FnOnce(&mut AnyMcb),
    ) -> Result<(), ortseam_jobs::JobError> {
        match self.job_detector() {
            Some(detector) => {
                edit(&mut self.detectors[detector]);
                Ok(())
            }
            None => Err(ortseam_jobs::JobError::host("no detector is configured")),
        }
    }

    /// Saves the active spectrum to a path, for jobs.
    pub fn save_active_to(&mut self, path: &std::path::Path) -> Result<(), String> {
        let Some(index) = self.active else {
            return Err("no window is active".into());
        };
        let Some(mut spectrum) = Self::spectrum_of(&self.windows, &self.detectors, index).cloned()
        else {
            return Err("nothing to save".into());
        };
        if !self.sample.description.is_empty() {
            spectrum.sample_description = self.sample.description.clone();
        }
        save_spectrum(&spectrum, path).map_err(|error| error.to_string())?;
        self.windows[index].path = Some(path.to_path_buf());
        self.windows[index].modified = false;
        self.status = format!("saved {}", path.display());
        Ok(())
    }

    /// Settles the layout once the files named on the command line are open.
    ///
    /// A single spectrum fills the area, because that is what someone opening one
    /// file wants to see. `ORTSEAM_DEMO=1` additionally marks the peaks and opens
    /// the peak information on the strongest one, which is how the screenshots in
    /// the documentation are made.
    pub fn settle_layout(&mut self) {
        // One window worth looking at fills the screen: either it is the only
        // window, or it is the only one holding data (a recalled file next to
        // idle, empty detectors).
        let with_data: Vec<usize> = (0..self.windows.len())
            .filter(|index| {
                Self::spectrum_of(&self.windows, &self.detectors, *index)
                    .is_some_and(|spectrum| spectrum.total_counts() > 0)
            })
            .collect();
        if self.windows.len() == 1 {
            self.maximized = Some(self.windows[0].id);
        } else if let [only] = with_data[..] {
            self.maximized = Some(self.windows[only].id);
            self.active = Some(only);
        }
        let mut demo = std::env::var("ORTSEAM_DEMO").unwrap_or_default();
        // "amber:peak" switches the theme, then runs the named demo state.
        if let Some((theme_name, rest)) = demo.split_once(':') {
            if let Some(theme) = crate::theme::Theme::all()
                .iter()
                .find(|theme| theme.label().to_lowercase().contains(theme_name))
            {
                self.theme = *theme;
                self.colors = theme.colors();
            }
            demo = rest.to_string();
        }
        // Named demo states, for the documentation screenshots.
        match demo.as_str() {
            "tile" => {
                self.apply_one(Action::TileVertically);
                return;
            }
            "rows" => {
                self.apply_one(Action::TileHorizontally);
                return;
            }
            "help" => {
                self.dialogs.open(Dialog::Shortcuts);
                return;
            }
            "analyse" => {
                self.apply_one(Action::PeakSearch);
                self.apply_one(Action::Analyse);
                self.dialogs.open(Dialog::Analysis);
                return;
            }
            "dialogs" => {
                // The service dialogs, for the visual sweep.
                self.queue
                    .push(ortseam_device::SampleJob::new("S001", 300.0));
                self.dialogs.open(Dialog::Dashboard);
                self.dialogs.open(Dialog::Batch);
                self.dialogs.open(Dialog::McbProperties);
                return;
            }
            "charts" => {
                // Filled QA and efficiency charts, for the screenshots.
                let mut chart = QaChart::new(
                    ortseam_core::QaMetric::Centroid,
                    ortseam_core::ControlLimits::default(),
                );
                for (day, value) in [
                    661.9, 662.0, 661.95, 662.05, 661.9, 662.0, 661.85, 662.4, 661.95, 662.7,
                ]
                .iter()
                .enumerate()
                {
                    chart.push(ortseam_core::QaRecord::new(
                        chrono::NaiveDate::from_ymd_opt(2026, 7, 1)
                            .expect("date")
                            .and_hms_opt(8, 0, 0)
                            .expect("time")
                            + chrono::Duration::days(day as i64),
                        *value,
                    ));
                }
                self.qa.push(chart);
                self.dialogs.open(Dialog::Qa);
                let points = vec![
                    ortseam_core::EfficiencyPoint::new(59.5, 0.012),
                    ortseam_core::EfficiencyPoint::new(122.1, 0.058),
                    ortseam_core::EfficiencyPoint::new(356.0, 0.035),
                    ortseam_core::EfficiencyPoint::new(661.7, 0.021),
                    ortseam_core::EfficiencyPoint::new(1173.2, 0.013),
                    ortseam_core::EfficiencyPoint::new(1332.5, 0.0115),
                ];
                self.dialogs.efficiency_points = points
                    .iter()
                    .map(|point| (point.energy.to_string(), point.efficiency.to_string()))
                    .collect();
                if let Ok(curve) = ortseam_core::EfficiencyCalibration::fit_log_log(&points, 3) {
                    self.efficiency = Some(curve);
                }
                self.dialogs.open(Dialog::Efficiency);
                return;
            }
            "hardware" => {
                // A real instrument through the bridge. ORTSEAM_UMCBI_DIR names
                // ORTEC's library directory when it is not installed
                // system-wide, which is the usual case on a machine that only
                // has the files copied across.
                let umcbi = std::env::var("ORTSEAM_UMCBI_DIR").unwrap_or_default();
                let number = std::env::var("ORTSEAM_MCB_DETECTOR")
                    .ok()
                    .unwrap_or_else(|| "1".into());
                let description = if umcbi.is_empty() {
                    number.clone()
                } else {
                    format!("{number};{umcbi}")
                };
                self.add_detector(ortseam_device::DetectorEntry {
                    number: 1,
                    name: format!("MCB-{number}"),
                    model: "local".into(),
                    kind: ortseam_device::DetectorKind::Local,
                    channels: 0,
                    description,
                    // Learned from the instrument on the first open.
                    serial: String::new(),
                });
                if !self.detectors.is_empty() {
                    self.apply_one(Action::OpenDetector(0));
                    self.apply_one(Action::MaximizeActive);
                }
                return;
            }
            "roi" => {
                // Regions marked, nothing overlaying them: the view for judging
                // how region shading looks.
                self.apply_one(Action::PeakSearch);
                return;
            }
            "empty" => {
                // A buffer with nothing in it, on the logarithmic scale: the
                // corner the monkey crashed, so it is worth looking at.
                self.apply_one(Action::OpenBuffer);
                self.apply_one(Action::ToggleLog);
                return;
            }
            "insight" => {
                // A visibly wrong pole zero, so the tail shows.
                if let Some(detector) = self.detectors.first_mut() {
                    let mut properties = detector.properties().clone();
                    properties.amplifier.pole_zero = 300;
                    let _ = detector.set_properties(properties);
                }
                self.dialogs.open(Dialog::Insight);
                return;
            }
            "select" => {
                // A marked band with its midpoint line, for the visual sweep.
                if let Some(index) = self.active {
                    let length = Self::spectrum_of(&self.windows, &self.detectors, index)
                        .map(|spectrum| spectrum.len())
                        .unwrap_or(0);
                    if length > 0 {
                        let low = length * 40 / 100;
                        let high = length * 60 / 100;
                        self.windows[index].selection = Some((low, high));
                    }
                }
                return;
            }
            "cal" => {
                // The auto-calibration story, as a picture: strip the
                // calibration, then recover it from the peaks.
                if let Some(spectrum) = self.active_spectrum_mut() {
                    spectrum.energy_calibration = None;
                    spectrum.rois = ortseam_core::RoiSet::default();
                }
                self.auto_calibrate("Eu-152");
                self.dialogs.auto_nuclide = "Eu-152".into();
                self.dialogs.open(Dialog::Calibration);
                return;
            }
            _ => {}
        }
        if !demo.is_empty() {
            self.apply_one(Action::PeakSearch);
            let settings = self.settings;
            let strongest = self.active.and_then(|index| {
                let spectrum = Self::spectrum_of(&self.windows, &self.detectors, index)?;
                spectrum
                    .rois
                    .iter()
                    .filter_map(|roi| peak_info(spectrum, *roi, &settings).ok())
                    .max_by(|a, b| {
                        a.net_area
                            .partial_cmp(&b.net_area)
                            .unwrap_or(std::cmp::Ordering::Equal)
                    })
            });
            if let Some(info) = strongest {
                let centre = info.centroid.round().max(0.0) as usize;
                let width = info.roi.len() * 8;
                self.apply_one(Action::Marker(centre));
                self.apply_one(Action::ZoomTo(centre.saturating_sub(width), centre + width));
                self.apply_one(Action::PeakInfoAtMarker);
            }
        }
    }

    /// Opens the files named on the command line.
    ///
    /// Spectra open in buffer windows, a nuclide library becomes the working
    /// library and a `.JOB` file starts running, as MAESTRO's command line does.
    pub fn open_arguments(&mut self, files: &[PathBuf]) {
        for path in files {
            let extension = path
                .extension()
                .map(|value| value.to_string_lossy().to_ascii_lowercase())
                .unwrap_or_default();
            match extension.as_str() {
                "job" => {
                    if let Err(error) = crate::jobs::start(self, path) {
                        self.status = error;
                    }
                }
                "lib" | "csv" => match ortseam_formats::library::load(path) {
                    Ok(library) => {
                        self.status = format!("library {} loaded", path.display());
                        self.library = library;
                        self.library_path = Some(path.clone());
                    }
                    Err(error) => self.status = error.to_string(),
                },
                _ => self.recall_path(path.clone()),
            }
        }
    }

    /// Adds a detector from the detector-list editor.
    /// Removes a detector: its windows close, the pick list forgets it, and
    /// windows on later detectors follow their instruments' new positions.
    /// A running count protects its detector - stop it first.
    fn remove_detector(&mut self, index: usize) {
        let Some(detector) = self.detectors.get(index) else {
            return;
        };
        if detector.is_active() {
            self.status = "stop the count before removing the detector".into();
            return;
        }
        let number = detector.identity().number;
        let doomed: Vec<usize> = self
            .windows
            .iter()
            .filter(|window| matches!(window.target, Target::Detector(d) if d == index))
            .map(|window| window.id)
            .collect();
        for id in doomed {
            self.close_window(id);
        }
        for window in &mut self.windows {
            if let Target::Detector(d) = window.target
                && d > index
            {
                window.target = Target::Detector(d - 1);
            }
        }
        self.detectors.remove(index);
        self.detector_list.remove(number);
        self.status = format!("Detector {number} removed");
    }

    /// Runs the bridge and returns what it printed.
    fn run_bridge(&mut self, arguments: &[&str]) -> Result<String, String> {
        let executable = ortseam_device::BridgeTransport::find_executable().ok_or_else(|| {
            format!(
                "{} is not beside ortseam - the bridge to instruments is missing",
                ortseam_device::BRIDGE_EXECUTABLE
            )
        })?;
        let mut command = std::process::Command::new(&executable);
        command.args(arguments);
        ortseam_device::no_console(&mut command);
        let directory = self.dialogs.local_umcbi.trim().to_string();
        if !directory.is_empty() {
            command.arg("--umcbi-dir").arg(&directory);
        }
        let output = command
            .output()
            .map_err(|error| format!("could not run the bridge: {error}"))?;
        Ok(format!(
            "{}{}",
            String::from_utf8_lossy(&output.stdout),
            String::from_utf8_lossy(&output.stderr)
        ))
    }

    /// Asks the bridge what instruments this machine can reach.
    ///
    /// The bridge answers with what the transports report and what the
    /// configuration claims, separately, because the interesting case is when
    /// they disagree.
    pub fn scan_local_instruments(&mut self) {
        match self.run_bridge(&["probe"]) {
            Ok(text) => {
                self.instruments = InstrumentScan::parse(&text);
                self.dialogs.local_scan = text;
                self.status = self.instruments.summary();
            }
            Err(error) => {
                self.instruments = InstrumentScan::failed(&error);
                self.dialogs.local_scan = error.clone();
                self.status = error;
            }
        }
    }

    /// Has the bridge write a detector list from what is actually connected.
    pub fn configure_local_instruments(&mut self) {
        match self.run_bridge(&["configure"]) {
            Ok(text) => {
                self.dialogs.local_scan = text.clone();
                self.status = text
                    .lines()
                    .find(|line| line.starts_with("wrote") && line.contains("instrument"))
                    .unwrap_or("the bridge configured what it found")
                    .to_string();
                self.scan_local_instruments();
            }
            Err(error) => {
                self.status = error.clone();
                self.dialogs.local_scan = error;
            }
        }
    }

    /// Opens a detector window for every instrument the bridge reports.
    pub fn open_local_instruments(&mut self) {
        if self.instruments.reachable.is_empty() {
            self.scan_local_instruments();
        }
        let reachable: Vec<(i32, String)> = self.instruments.reachable.clone();
        if reachable.is_empty() {
            self.status = "no instrument answered, so there is nothing to open".into();
            return;
        }
        let umcbi = self.dialogs.local_umcbi.trim().to_string();
        for (number, name) in reachable {
            // Already open is not an error; it is the usual case on a second
            // press, and silently doing nothing is the friendly answer.
            if self
                .detector_list
                .entries()
                .iter()
                .any(|entry| entry.description.split(';').next() == Some(&number.to_string()))
            {
                continue;
            }
            let description = if umcbi.is_empty() {
                number.to_string()
            } else {
                format!("{number};{umcbi}")
            };
            let next = self.next_detector_number();
            self.add_detector(DetectorEntry {
                number: next,
                name,
                model: "local".into(),
                kind: ortseam_device::DetectorKind::Local,
                channels: 0,
                description,
                // Learned from the instrument on the first open.
                serial: String::new(),
            });
        }
        for index in 0..self.detectors.len() {
            if !self
                .windows
                .iter()
                .any(|window| window.target == Target::Detector(index))
            {
                self.apply_one(Action::OpenDetector(index));
            }
        }
    }

    /// The lowest detector number not already taken.
    fn next_detector_number(&self) -> u16 {
        (1..=ortseam_device::MAX_DETECTOR_NUMBER)
            .find(|candidate| {
                !self
                    .detector_list
                    .entries()
                    .iter()
                    .any(|entry| entry.number == *candidate)
            })
            .unwrap_or(1)
    }

    /// Puts the active window's fitted calibration onto its spectrum.
    ///
    /// Adding, correcting and removing a point all end here, so a change to one
    /// of them cannot leave the table and the spectrum disagreeing.
    fn apply_fitted_calibration(&mut self) {
        let Some(index) = self.active else { return };
        let fit = self.windows[index].calibration.calibration().cloned();
        if let Some(spectrum) = self.active_spectrum_mut() {
            spectrum.energy_calibration = fit;
        }
    }

    /// Corrects a calibration point in place and applies the new fit.
    fn edit_calibration_point(&mut self, index: usize, channel: f64, value: f64) {
        let Some(window) = self.active.map(|active| &mut self.windows[active]) else {
            return;
        };
        if !channel.is_finite() || !value.is_finite() {
            self.status = "a calibration point needs two numbers".into();
            return;
        }
        if window
            .calibration
            .set(index, ortseam_core::CalibrationPoint::new(channel, value))
        {
            self.apply_fitted_calibration();
            self.status = format!("point {} is now {channel:.2} = {value:.3}", index + 1);
        }
    }

    /// Drops a calibration point and refits without it.
    fn remove_calibration_point(&mut self, index: usize) {
        let Some(window) = self.active.map(|active| &mut self.windows[active]) else {
            return;
        };
        if window.calibration.remove(index).is_some() {
            // The edit texts are keyed by row: the row under the removed one
            // moves up, and must not inherit the removed point's typing.
            if index < self.dialogs.calibration_edits.len() {
                self.dialogs.calibration_edits.remove(index);
            }
            self.apply_fitted_calibration();
            self.status = format!("point {} removed", index + 1);
        }
    }

    pub fn add_detector(&mut self, entry: DetectorEntry) {
        let mcb: AnyMcb = match entry.kind {
            ortseam_device::DetectorKind::Network => {
                // The description carries the address ("192.168.0.40:2000").
                let address = entry.description.trim().to_string();
                if address.is_empty() {
                    self.status = "a network detector needs host:port in its description".into();
                    return;
                }
                let connected = ortseam_device::TcpTransport::connect(
                    &address,
                    std::time::Duration::from_secs(3),
                )
                .and_then(|transport| {
                    ortseam_device::RemoteMcb::connect(
                        Box::new(transport),
                        entry.number,
                        &entry.name,
                    )
                });
                match connected {
                    Ok(remote) => {
                        self.status = format!(
                            "connected to {} ({} channels)",
                            address,
                            remote.identity().channels
                        );
                        remote.into()
                    }
                    Err(error) => {
                        self.status = format!("{error}");
                        return;
                    }
                }
            }
            // Anything this build cannot actually reach is refused by name.
            // Handing back a simulator would put invented counts on screen
            // under the name of an instrument, which is the one mistake a
            // spectroscopy program must not make.
            // A local instrument is reached through the 32-bit bridge, which
            // owns ORTEC's library because a 64-bit process cannot. The
            // description carries the detector number ORTEC's configuration
            // gave it, and optionally the directory holding that library.
            ortseam_device::DetectorKind::Local => {
                let mut parts = entry.description.split(';');
                let detector: i32 = match parts.next().unwrap_or("").trim().parse() {
                    Ok(number) => number,
                    Err(_) => {
                        self.status = "a local instrument needs its ORTEC detector number in \
                                       the description"
                            .into();
                        return;
                    }
                };
                let umcbi = parts.next().map(str::trim).filter(|text| !text.is_empty());
                let Some(executable) = ortseam_device::BridgeTransport::find_executable() else {
                    self.status = format!(
                        "{} is not beside ortseam - the bridge to ORTEC hardware is missing",
                        ortseam_device::BRIDGE_EXECUTABLE
                    );
                    return;
                };
                // The detector number says where to look; the serial says
                // which instrument is meant. Naming it keeps a second adapter,
                // or a replug that reorders the bus, from opening the wrong
                // one - and the check after connecting catches it even when
                // the helper cannot select by serial at all.
                let remembered = entry.serial.trim().to_string();
                let connected = ortseam_device::BridgeTransport::start_pinned(
                    &executable,
                    detector,
                    pin_by_serial(&remembered, detector),
                    umcbi.map(std::path::Path::new),
                )
                .and_then(|transport| {
                    ortseam_device::RemoteMcb::connect_expecting(
                        Box::new(transport),
                        entry.number,
                        &entry.name,
                        Some(remembered.as_str()),
                    )
                });
                match connected {
                    Ok(instrument) => {
                        self.status = format!(
                            "opened {} ({} channels)",
                            instrument.identity().model,
                            instrument.identity().channels
                        );
                        // Learned on the first open, so every open after this
                        // one has something to check against.
                        let reported = instrument.identity().serial.trim().to_string();
                        if remembered.is_empty() && !reported.is_empty() {
                            self.remember_serial(entry.number, &reported);
                        }
                        instrument.into()
                    }
                    Err(error) => {
                        self.status = format!("{error}");
                        return;
                    }
                }
            }
            ortseam_device::DetectorKind::FileReplay => {
                self.status =
                    "file replay is not built into this version - use File/Recall instead".into();
                return;
            }
            #[cfg(feature = "simulator")]
            ortseam_device::DetectorKind::Simulated => {
                SimulatedMcb::builder(entry.number, &entry.name)
                    .channels(entry.channels)
                    .build()
                    .into()
            }
            #[cfg(not(feature = "simulator"))]
            ortseam_device::DetectorKind::Simulated => {
                self.status = "this build has no simulator: counts come from real instruments \
                               or from files, never from nowhere"
                    .into();
                return;
            }
        };
        self.detector_list.push(entry);
        self.detectors.push(mcb);
        if !self.status.starts_with("connected") {
            self.status = "detector added".into();
        }
    }
}

impl eframe::App for App {
    fn ui(&mut self, ui: &mut egui::Ui, _frame: &mut eframe::Frame) {
        self.tick();
        self.draw(ui);
        // The window is created hidden so that start-up does not flash an empty
        // frame; now that one has been drawn, there is something worth showing.
        if !self.shown {
            self.shown = true;
            ui.ctx()
                .send_viewport_cmd(egui::ViewportCommand::Visible(true));
        }
    }

    fn save(&mut self, storage: &mut dyn eframe::Storage) {
        if let Ok(text) = serde_json::to_string(&self.persisted()) {
            storage.set_string("ortseam", text);
        }
    }
}

impl App {
    /// Draws one frame: menus, toolbar, sidebar, spectrum windows and dialogs.
    ///
    /// Separate from [`eframe::App::ui`] so a test can run frames against a bare
    /// [`egui::Context`], with no window and no event loop.
    pub fn draw(&mut self, ui: &mut egui::Ui) {
        crate::jobs::step(self);
        self.take_dropped_files(ui.ctx());
        // Keep egui's own widgets dressed in the theme, however it was chosen -
        // Preferences, a restored session, or a demo state.
        if self.applied_theme != Some(self.theme) {
            theme::apply(ui.ctx(), self.theme);
            self.applied_theme = Some(self.theme);
        }

        let ctx = ui.ctx().clone();
        let mut actions: Vec<Action> = Vec::new();
        actions.extend(self.keyboard(&ctx));

        // The title-bar close button goes through the same exit question as
        // File/Exit: unsaved spectra and running counts deserve a look before
        // the process dies. Answering "Exit" lets the next request through.
        if ctx.input(|input| input.viewport().close_requested()) && !self.quit_confirmed {
            let unsaved = self.windows.iter().any(|window| window.modified);
            let counting = self.detectors.iter().any(|detector| detector.is_active());
            if unsaved || counting {
                ctx.send_viewport_cmd(egui::ViewportCommand::CancelClose);
                self.dialogs.open(Dialog::Exit);
            }
        }

        egui::Panel::top("menu").show(ui, |ui| {
            actions.extend(crate::dialogs::menu_bar(self, ui));
        });
        egui::Panel::top("toolbar").show(ui, |ui| {
            ui.add_space(2.0);
            actions.extend(crate::dialogs::toolbar(self, ui));
            ui.add_space(2.0);
        });

        // The width bounds live on the panel itself so that a persisted or
        // dragged width can never crush the sidebar into an unreadable sliver.
        egui::Panel::right("status")
            .min_size(220.0)
            .max_size(340.0)
            .default_size(240.0)
            .show(ui, |ui| {
                actions.extend(crate::dialogs::status_sidebar(self, ui));
            });

        egui::Panel::bottom("supplementary").show(ui, |ui| {
            ui.horizontal(|ui| {
                ui.label(&self.status);
            });
        });

        egui::CentralPanel::default().show(ui, |ui| {
            // Spectrum windows float, but stay inside the spectrum area so they
            // cannot hide the status sidebar.
            self.window_area = ui.max_rect();
            actions.extend(self.draw_windows(ui));
        });

        actions.extend(crate::dialogs::windows(self, &ctx));
        self.apply(actions);

        self.drop_target_overlay(&ctx);

        if let Some(text) = self.pending_clipboard.take() {
            ctx.copy_text(text);
        }

        // Redraw while anything is counting, tuning or running a job - the
        // clock only advances on frames, so a stalled screen would stall an
        // Optimize left running with its dialog closed.
        let busy = self.job.is_some()
            || self.detectors.iter().any(|detector| {
                detector.is_active() || detector.optimizing() || detector.pole_zeroing()
            });
        if busy {
            ctx.request_repaint_after(std::time::Duration::from_millis(100));
        }
    }

    /// Opens anything dropped on the window.
    ///
    /// Dragging a file in is the shortest way to look at a spectrum, and it works
    /// when a system file dialog will not - inside a container, over a remote
    /// session, or on a desktop without a portal.
    /// A "drop to open" veil while a file is dragged over the window, so the
    /// drag-and-drop support is discoverable rather than a hidden feature.
    fn drop_target_overlay(&self, ctx: &egui::Context) {
        let names: Vec<String> = ctx.input(|input| {
            input
                .raw
                .hovered_files
                .iter()
                .filter_map(|file| file.path.as_ref())
                .filter_map(|path| path.file_name())
                .map(|name| name.to_string_lossy().into_owned())
                .collect()
        });
        let hovering = ctx.input(|input| !input.raw.hovered_files.is_empty());
        if !hovering {
            return;
        }
        let painter = ctx.layer_painter(egui::LayerId::new(
            egui::Order::Foreground,
            egui::Id::new("drop_target"),
        ));
        let screen = ctx.content_rect();
        painter.rect_filled(screen, 0.0, egui::Color32::from_black_alpha(110));
        painter.rect_stroke(
            screen.shrink(12.0),
            egui::CornerRadius::same(8),
            egui::Stroke::new(2.0, self.colors.foreground.to_color()),
            egui::StrokeKind::Inside,
        );
        let label = match names.len() {
            0 => "drop to open".to_string(),
            1 => format!("drop to open {}", names[0]),
            n => format!("drop to open {n} files"),
        };
        painter.text(
            screen.center(),
            egui::Align2::CENTER_CENTER,
            label,
            egui::FontId::proportional(22.0),
            self.colors.foreground.to_color(),
        );
    }

    fn take_dropped_files(&mut self, ctx: &egui::Context) {
        let dropped: Vec<PathBuf> = ctx.input(|input| {
            input
                .raw
                .dropped_files
                .iter()
                .filter_map(|file| file.path.clone())
                .collect()
        });
        if dropped.is_empty() {
            return;
        }
        let count = dropped.len();
        self.open_arguments(&dropped);
        if count > 1 {
            self.status = format!("{count} files opened");
        }
    }

    /// Keyboard commands (MAESTRO §5).
    fn keyboard(&mut self, ctx: &egui::Context) -> Vec<Action> {
        let mut actions = Vec::new();
        let events = ctx.input(|input| input.clone());
        let input = &events;
        {
            use egui::Key;
            let shift = input.modifiers.shift;
            let ctrl = input.modifiers.command || input.modifiers.ctrl;
            let alt = input.modifiers.alt;
            let step = if shift { 10 } else { 1 };
            // A focused text field owns the plain keys: Delete is deleting a
            // typo, not the ROI under the marker; arrows move the caret, not
            // the marker; "511" being typed into an energy box must not
            // recentre the view. Every unmodified binding stands down while
            // something has the keyboard. (The F-keys and Ctrl/Alt chords
            // type nothing, so they keep working.)
            let typing = ctx.egui_wants_keyboard_input();

            if !typing
                && input.key_pressed(Key::ArrowRight)
                && let Some(index) = self.active
            {
                self.windows[index].display.move_marker(step as i64);
            }
            if !typing
                && input.key_pressed(Key::ArrowLeft)
                && let Some(index) = self.active
            {
                self.windows[index].display.move_marker(-(step as i64));
            }
            if !typing && input.key_pressed(Key::Home) {
                actions.push(Action::Marker(0));
            }
            if !typing && input.key_pressed(Key::End) {
                actions.push(Action::Marker(usize::MAX));
            }
            if !typing
                && input.key_pressed(Key::PageUp)
                && let Some(index) = self.active
            {
                let width = self.windows[index].display.view_width as i64;
                actions.push(Action::Scroll(width / 2));
            }
            if !typing
                && input.key_pressed(Key::PageDown)
                && let Some(index) = self.active
            {
                let width = self.windows[index].display.view_width as i64;
                actions.push(Action::Scroll(-width / 2));
            }
            if !typing && (input.key_pressed(Key::Plus) || input.key_pressed(Key::Equals)) {
                actions.push(Action::ZoomIn);
            }
            if !typing && input.key_pressed(Key::Minus) {
                actions.push(Action::ZoomOut);
            }
            if !typing && input.key_pressed(Key::Slash) {
                actions.push(Action::ToggleLog);
            }
            // egui has no keypad-* key, so automatic scaling is on A.
            if !typing && input.key_pressed(Key::A) && !ctrl && !alt {
                actions.push(Action::AutoScale);
            }
            if !typing && input.key_pressed(Key::Num5) && !ctrl {
                actions.push(Action::Center);
            }
            if !typing && input.key_pressed(Key::Insert) {
                actions.push(Action::MarkPeak);
            }
            if !typing && input.key_pressed(Key::Delete) {
                actions.push(Action::ClearRoi);
            }
            if input.key_pressed(Key::F2) {
                let next = match self.mark_mode {
                    MarkMode::Off => MarkMode::Mark,
                    MarkMode::Mark => MarkMode::UnMark,
                    MarkMode::UnMark => MarkMode::Off,
                };
                actions.push(Action::MarkMode(next));
            }
            if input.key_pressed(Key::F3) {
                actions.push(Action::ViewZdt);
            }
            if input.key_pressed(Key::F4) {
                actions.push(Action::OpenBuffer);
            }
            // A focused text field owns Escape (it releases focus); otherwise
            // Escape peels the topmost layer: open dialogs close first, and
            // only then does it clear the plot's own overlays.
            if input.key_pressed(Key::Escape) && !typing {
                if self.dialogs.any_open() {
                    self.dialogs.close_all();
                    self.pending_close = None;
                } else {
                    actions.push(Action::ClearCompare);
                    actions.push(Action::ClearSelection);
                    if let Some(index) = self.active {
                        self.windows[index].peak_info = None;
                    }
                }
            }
            if ctrl && input.key_pressed(Key::M) {
                actions.push(Action::MaximizeActive);
            }
            // Shift with the up and down arrows offsets the comparison trace.
            if !typing
                && shift
                && (input.key_pressed(Key::ArrowUp) || input.key_pressed(Key::ArrowDown))
                && let Some(index) = self.active
                && self.windows[index].compare.is_some()
            {
                let scale = self
                    .windows
                    .get(index)
                    .and_then(|window| match window.display.vertical {
                        VerticalScale::Linear(max) => Some(max),
                        _ => None,
                    })
                    .unwrap_or(1_000);
                let step = (scale / 20).max(1) as i64;
                self.windows[index].compare_offset += if input.key_pressed(Key::ArrowUp) {
                    step
                } else {
                    -step
                };
            }
            if input.key_pressed(Key::F1) {
                actions.push(Action::Show(Dialog::Shortcuts));
            }
            if ctrl && input.key_pressed(Key::Tab) {
                actions.push(Action::CycleWindow(if shift { -1 } else { 1 }));
            }
            // Ctrl+F1..F12 select detectors 1-12 (MAESTRO §5).
            for (offset, key) in [
                Key::F1,
                Key::F2,
                Key::F3,
                Key::F4,
                Key::F5,
                Key::F6,
                Key::F7,
                Key::F8,
                Key::F9,
                Key::F10,
                Key::F11,
                Key::F12,
            ]
            .into_iter()
            .enumerate()
            {
                if ctrl && input.key_pressed(key) && offset < self.detectors.len() {
                    actions.push(Action::OpenDetector(offset));
                }
            }
            if ctrl && input.key_pressed(Key::Z) {
                actions.push(if shift { Action::Redo } else { Action::Undo });
            }
            if ctrl && input.key_pressed(Key::Y) {
                actions.push(Action::Redo);
            }
            if ctrl && input.key_pressed(Key::O) {
                actions.push(Action::Recall);
            }
            if ctrl && input.key_pressed(Key::P) {
                actions.push(Action::Print);
            }
            // Ctrl+C copies the open peak card - unless a text field has the
            // keyboard, whose own copy must win.
            if ctrl && input.key_pressed(Key::C) && !typing {
                let card_open = self
                    .active
                    .and_then(|index| self.windows.get(index))
                    .is_some_and(|window| window.peak_info.is_some());
                if card_open {
                    actions.push(Action::CopyPeakInfo);
                }
            }
            if ctrl && input.key_pressed(Key::S) {
                actions.push(if shift { Action::SaveAs } else { Action::Save });
            }
            if alt && input.key_pressed(Key::Num1) {
                actions.push(Action::Start);
            }
            if alt && input.key_pressed(Key::Num2) {
                actions.push(Action::Stop);
            }
            if alt && input.key_pressed(Key::Num3) {
                actions.push(Action::Clear);
            }
            if alt && input.key_pressed(Key::Num5) {
                actions.push(Action::CopyToBuffer);
            }
        }
        actions
    }

    /// Draws every open spectrum window.
    fn draw_windows(&mut self, ui: &mut egui::Ui) -> Vec<Action> {
        let mut actions = Vec::new();
        if self.windows.is_empty() {
            let recent: Vec<PathBuf> = self.recent.clone();
            ui.vertical_centered(|ui| {
                ui.add_space(80.0);
                ui.heading("No spectrum windows are open");
                ui.label("File/Recall opens a spectrum, or drop a file anywhere in this window.");
                if !self.detectors.is_empty() {
                    ui.label("Display/Detector opens an instrument.");
                }
                // Connecting hardware belongs here rather than three menus
                // down: it is the first thing somebody with an instrument on
                // the bench wants, and until it is done the rest of the
                // program has nothing to show them.
                ui.add_space(18.0);
                actions.extend(instrument_panel(self, ui));
                if !recent.is_empty() {
                    ui.add_space(16.0);
                    ui.label(egui::RichText::new("Pick up where you left off").weak());
                    for path in &recent {
                        let name = path
                            .file_name()
                            .map(|name| name.to_string_lossy().to_string())
                            .unwrap_or_else(|| path.display().to_string());
                        if ui
                            .link(egui::RichText::new(name).monospace())
                            .on_hover_text(path.display().to_string())
                            .clicked()
                        {
                            actions.push(Action::RecallFile(path.clone()));
                        }
                    }
                }
            });
            return actions;
        }

        self.validate_maximized();
        let drawn = self.visible_windows();
        // Collapse or expand every window, by editing the state egui's own
        // title-bar triangle edits.
        if let Some(open) = self.pending_collapse.take() {
            for window in &self.windows {
                let area_id = egui::Id::new(("spectrum", window.id));
                let collapse_id = area_id.with("collapsing");
                let mut state = egui::collapsing_header::CollapsingState::load_with_default_open(
                    ui.ctx(),
                    collapse_id,
                    true,
                );
                state.set_open(open);
                state.store(ui.ctx());
            }
        }
        // An arrangement asked for this frame: rects from the area as it is now.
        let mut forced: std::collections::HashMap<usize, egui::Rect> =
            match self.pending_tiling.take() {
                Some(tiling) => {
                    let rects =
                        Self::layout_rects(tiling, self.window_area.shrink(4.0), drawn.len());
                    drawn
                        .iter()
                        .zip(rects)
                        .map(|(index, rect)| (self.windows[*index].id, rect))
                        .collect()
                }
                None => std::collections::HashMap::new(),
            };
        // A single placement asked for by a job's ZOOM: command.
        if let Some((id, rect)) = self.pending_place.take() {
            forced.insert(id, rect.intersect(self.window_area));
        }
        let App {
            windows,
            detectors,
            colors,
            mark_mode,
            library,
            active,
            window_area,
            maximized,
            peak_font,
            ..
        } = self;
        let area = *window_area;
        let maximized_id = *maximized;
        let peak_font = *peak_font;
        // Windows are staggered so several are usable at once, and sized to fit
        // the spectrum area.
        let default_size = egui::vec2(
            (area.width() - 40.0).clamp(360.0, 900.0),
            (area.height() * 0.78).clamp(260.0, 720.0),
        );

        for index in drawn {
            // Split the window out so its display can be borrowed mutably while
            // the spectrum - which may live in the detector - is borrowed shared.
            let (_before, rest) = windows.split_at_mut(index);
            let (window, _after) = rest.split_first_mut().expect("index is in range");
            let SpectrumWindow {
                id,
                title: window_title,
                target,
                buffer,
                display,
                compare,
                compare_offset,
                selection,
                peak_info,
                open: window_open,
                modified,
                ..
            } = window;
            let is_maximized = maximized_id == Some(*id);

            let spectrum = match *target {
                Target::Buffer => buffer.as_ref(),
                Target::Detector(detector) => detectors.get(detector).map(|mcb| mcb.spectrum()),
            };
            let Some(spectrum) = spectrum else { continue };

            let status = match *target {
                Target::Detector(detector) => detectors.get(detector).map(|mcb| mcb.status()),
                Target::Buffer => None,
            };
            let header = crate::view::ViewHeader {
                source: window_title,
                counting: status.map(|status| status.active).unwrap_or(false),
                dead_time: status
                    .map(|status| status.dead_time_percent)
                    .or_else(|| spectrum.dead_time_percent()),
                active: *active == Some(index),
                maximized: is_maximized,
            };

            let mut open = *window_open;
            let title = format!("{}{}", window_title, if *modified { " *" } else { "" });
            let offset = 26.0 * (index % 6) as f32;
            let mut window = egui::Window::new(title)
                .id(egui::Id::new(("spectrum", *id)))
                .constrain_to(area)
                .open(&mut open);
            window = if is_maximized {
                window.fixed_rect(area.shrink(2.0))
            } else if let Some(rect) = forced.remove(id) {
                // A tile or cascade: forced for this one frame, which egui keeps,
                // so the window stays free to move and resize afterwards.
                // Constraining is off for the frame because egui would clamp the
                // new position against the window's old size - the new size is
                // not known until the frame ends - shoving tiled windows left.
                // The rects are inside the area by construction.
                window.fixed_rect(rect.intersect(area)).constrain(false)
            } else {
                window
                    .default_size(default_size)
                    .default_pos(area.min + egui::vec2(8.0 + offset, 8.0 + offset))
            };
            let response = window.show(ui.ctx(), |ui| {
                let events = crate::view::draw(
                    ui,
                    ViewInput {
                        spectrum,
                        compare: compare.as_ref(),
                        compare_offset: *compare_offset,
                        display,
                        colors,
                        mark_mode: *mark_mode,
                        library: Some(library),
                        selection: *selection,
                        peak_info: peak_info.as_ref(),
                        peak_font,
                        header,
                    },
                );
                for event in events {
                    match event {
                        ViewEvent::Marker(channel) => actions.push(Action::Marker(channel)),
                        ViewEvent::Selected(low, high) => actions.push(Action::Select(low, high)),
                        ViewEvent::SelectionCleared => actions.push(Action::ClearSelection),
                        ViewEvent::MarkRoi(low, high) => actions.push(Action::MarkRange(low, high)),
                        ViewEvent::UnmarkRoi(low, high) => {
                            actions.push(Action::UnmarkRange(low, high))
                        }
                        ViewEvent::PeakInfo(channel) => {
                            actions.push(Action::Marker(channel));
                            actions.push(Action::PeakInfoAtMarker);
                        }
                        ViewEvent::ZoomTo(low, high) => actions.push(Action::ZoomTo(low, high)),
                        ViewEvent::Zoom(direction) => actions.push(if direction > 0 {
                            Action::ZoomIn
                        } else {
                            Action::ZoomOut
                        }),
                        ViewEvent::ZoomAt(channel, direction) => {
                            actions.push(Action::ZoomAbout(channel, direction))
                        }
                        ViewEvent::Scroll(delta) => actions.push(Action::Scroll(delta)),
                        ViewEvent::Menu(command) => {
                            use crate::view::MenuCommand;
                            actions.push(match command {
                                MenuCommand::Start => Action::Start,
                                MenuCommand::Stop => Action::Stop,
                                MenuCommand::Clear => Action::Clear,
                                MenuCommand::CopyToBuffer => Action::CopyToBuffer,
                                MenuCommand::ZoomIn => Action::ZoomIn,
                                MenuCommand::ZoomOut => Action::ZoomOut,
                                MenuCommand::UndoZoom => Action::FullView,
                                MenuCommand::MarkRoi => match *selection {
                                    Some((low, high)) => Action::MarkRange(low, high),
                                    None => Action::MarkPeak,
                                },
                                MenuCommand::ClearRoi => Action::ClearRoi,
                                MenuCommand::PeakInfo => Action::PeakInfoAtMarker,
                                MenuCommand::InputCountRate => Action::InputCountRate,
                                MenuCommand::Sum => Action::Sum,
                                MenuCommand::McbProperties => {
                                    Action::Show(crate::dialogs::Dialog::McbProperties)
                                }
                            });
                        }
                        ViewEvent::Activate => actions.push(Action::Activate(*id)),
                        ViewEvent::ToggleMaximize => actions.push(Action::ToggleMaximize(*id)),
                        ViewEvent::ClosePeakInfo => *peak_info = None,
                    }
                }
            });
            if response.is_some() {
                *window_open = open;
            }
            if !open {
                actions.push(Action::CloseWindow(*id));
            }
        }
        actions
    }
}
