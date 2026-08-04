//! The `.JOB` command set (MAESTRO §6.5).

/// How `BEEP` was called.
#[derive(Clone, Debug, PartialEq, Eq)]
pub enum BeepSpec {
    /// `BEEP <freq>,<duration>`: a tone in hertz and milliseconds.
    Tone {
        /// Pitch in hertz.
        frequency: u32,
        /// Length in milliseconds.
        duration: u32,
    },
    /// `BEEP <id>`: a numbered system event.
    Event(u32),
    /// `BEEP "name"`: a sound file or named system event.
    Sound(String),
}

/// A time slice of list-mode data, as `SET_RANGE` expresses it.
#[derive(Clone, Debug, PartialEq)]
pub enum ListSlice {
    /// `SET_RANGE "M/dd/yyyy","hh:mm:ss",<t>`.
    Absolute {
        /// Date as written in the job.
        date: String,
        /// Time of day as written in the job.
        time: String,
        /// Slice length in seconds.
        duration: f64,
    },
    /// `SET_RANGE "r","t"`: a detector real time and a duration.
    Relative {
        /// Start, in seconds of real time.
        start: f64,
        /// Slice length in seconds.
        duration: f64,
    },
}

/// One `.JOB` command.
#[derive(Clone, Debug, PartialEq)]
pub enum Command {
    /// Ask for the password and owner used by `LOCK`.
    AskPassword,
    /// Make a noise.
    Beep(BeepSpec),
    /// Run the sample-changer handshake.
    ChangeSample,
    /// Erase the data and times of the selected detector.
    Clear,
    /// Close every buffer window.
    CloseBuffers,
    /// Close every detector window.
    CloseMcbs,
    /// Set the sample description.
    DescribeSample(String),
    /// Run the export program on a file.
    Export(String),
    /// Copy the detector data into a buffer.
    FillBuffer,
    /// Run the import program on a file.
    Import(String),
    /// Load a nuclide library.
    LoadLibrary(String),
    /// Lock the detector.
    Lock {
        /// Password.
        password: String,
        /// Owner recorded with the lock.
        owner: String,
        /// Optional lock name.
        name: Option<String>,
    },
    /// Begin a loop of `n` repetitions (0 and 1 both run once).
    Loop(u32),
    /// Begin a loop over the spectra stored in the instrument.
    LoopSpectra,
    /// End of a loop body.
    EndLoop,
    /// Peak search, marking each peak as a region.
    MarkPeaks,
    /// Leave the program.
    Quit,
    /// Read a spectrum file into a buffer.
    Recall(String),
    /// Read only the calibration from a spectrum file.
    RecallCalibration(String),
    /// Read a region file.
    RecallRoi(String),
    /// A comment.
    Rem(String),
    /// Write an ROI report to a file, or to the printer when absent.
    Report(Option<String>),
    /// Run a program.
    Run {
        /// Program to run.
        program: String,
        /// Start it minimised.
        minimized: bool,
    },
    /// Save the active spectrum.
    Save(Option<String>),
    /// Save the region table.
    SaveRoi(String),
    /// Send a raw command to the instrument.
    SendMessage(String),
    /// Select the buffer (the same as detector 0).
    SetBuffer,
    /// Select a detector, or the previous one when absent.
    SetDetector(Option<u16>),
    /// Switch the detector to list mode.
    SetList,
    /// Remember a file name for a later `STRIP`.
    SetNameStrip(String),
    /// Switch the detector to pulse-height analysis.
    SetPha,
    /// Clear every preset.
    SetPresetClear,
    /// Set the ROI peak-count preset.
    SetPresetCount(u64),
    /// Set the ROI integral preset.
    SetPresetIntegral(u64),
    /// Set the live-time preset, in seconds.
    SetPresetLive(f64),
    /// Set the real-time preset, in seconds.
    SetPresetReal(f64),
    /// Set the statistical uncertainty preset.
    SetPresetUncertainty {
        /// Uncertainty limit in percent.
        limit: f64,
        /// First channel of the region used.
        low: usize,
        /// Last channel of the region used.
        high: usize,
    },
    /// Show a time slice of recalled list-mode data.
    SetRange(ListSlice),
    /// Smooth the buffer.
    Smooth,
    /// Start data collection.
    Start,
    /// Start the optimise function.
    StartOptimize,
    /// Start the pole-zero function.
    StartPz,
    /// Stop data collection.
    Stop,
    /// Stop the pole-zero function.
    StopPz,
    /// Subtract a disk spectrum from the buffer.
    Strip {
        /// Scale factor; zero means "use the live-time ratio".
        factor: f64,
        /// File to subtract, when given here rather than by `SET_NAME_STRIP`.
        file: Option<String>,
    },
    /// Unlock the detector.
    Unlock(String),
    /// Move a stored spectrum to position 0.
    View(usize),
    /// Wait for the detector to stop, or for a number of seconds.
    Wait(Option<f64>),
    /// Wait until the optimise function finishes.
    WaitAuto,
    /// Wait for the sample-ready signal.
    WaitChanger,
    /// Wait until a program exits.
    WaitProgram(String),
    /// Wait until the pole-zero function finishes.
    WaitPz,
    /// Resize the window (0 minimised, 1 normal, 2 maximised).
    Zoom(i32),
    /// Move and resize the window.
    ZoomRect {
        /// Left edge.
        x: i32,
        /// Top edge.
        y: i32,
        /// Width.
        width: i32,
        /// Height.
        height: i32,
    },
}

impl Command {
    /// The command word, for messages.
    pub fn word(&self) -> &'static str {
        match self {
            Self::AskPassword => "ASK_PASSWORD",
            Self::Beep(_) => "BEEP",
            Self::ChangeSample => "CHANGE_SAMPLE",
            Self::Clear => "CLEAR",
            Self::CloseBuffers => "CLOSEBUFFERS",
            Self::CloseMcbs => "CLOSEMCBS",
            Self::DescribeSample(_) => "DESCRIBE_SAMPLE",
            Self::Export(_) => "EXPORT",
            Self::FillBuffer => "FILL_BUFFER",
            Self::Import(_) => "IMPORT",
            Self::LoadLibrary(_) => "LOAD_LIBRARY",
            Self::Lock { .. } => "LOCK",
            Self::Loop(_) => "LOOP",
            Self::LoopSpectra => "LOOP SPECTRA",
            Self::EndLoop => "END_LOOP",
            Self::MarkPeaks => "MARK_PEAKS",
            Self::Quit => "QUIT",
            Self::Recall(_) => "RECALL",
            Self::RecallCalibration(_) => "RECALL_CALIB",
            Self::RecallRoi(_) => "RECALL_ROI",
            Self::Rem(_) => "REM",
            Self::Report(_) => "REPORT",
            Self::Run {
                minimized: false, ..
            } => "RUN",
            Self::Run {
                minimized: true, ..
            } => "RUN_MINIMIZED",
            Self::Save(_) => "SAVE",
            Self::SaveRoi(_) => "SAVE_ROI",
            Self::SendMessage(_) => "SEND_MESSAGE",
            Self::SetBuffer => "SET_BUFFER",
            Self::SetDetector(_) => "SET_DETECTOR",
            Self::SetList => "SET_LIST",
            Self::SetNameStrip(_) => "SET_NAME_STRIP",
            Self::SetPha => "SET_PHA",
            Self::SetPresetClear => "SET_PRESET_CLEAR",
            Self::SetPresetCount(_) => "SET_PRESET_COUNT",
            Self::SetPresetIntegral(_) => "SET_PRESET_INTEG",
            Self::SetPresetLive(_) => "SET_PRESET_LIVE",
            Self::SetPresetReal(_) => "SET_PRESET_REAL",
            Self::SetPresetUncertainty { .. } => "SET_PRESET_UNCERTAINTY",
            Self::SetRange(_) => "SET_RANGE",
            Self::Smooth => "SMOOTH",
            Self::Start => "START",
            Self::StartOptimize => "START_OPTIMIZE",
            Self::StartPz => "START_PZ",
            Self::Stop => "STOP",
            Self::StopPz => "STOP_PZ",
            Self::Strip { .. } => "STRIP",
            Self::Unlock(_) => "UNLOCK",
            Self::View(_) => "VIEW",
            Self::Wait(_) => "WAIT",
            Self::WaitAuto => "WAIT_AUTO",
            Self::WaitChanger => "WAIT_CHANGER",
            Self::WaitProgram(_) => "WAIT",
            Self::WaitPz => "WAIT_PZ",
            Self::Zoom(_) => "ZOOM",
            Self::ZoomRect { .. } => "ZOOM:",
        }
    }
}
