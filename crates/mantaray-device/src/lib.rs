//! Instruments: the multichannel-buffer abstraction, presets, a physics-based
//! simulator, detector monitoring and the batch workflow.
//!
//! ```
//! use mantaray_device::{Mcb, Presets, SimulatedMcb, advance};
//!
//! let mut detector = SimulatedMcb::new(1, "SIM-01");
//! detector.apply_presets(Presets::live(10.0)).unwrap();
//! detector.start().unwrap();
//! while detector.is_active() {
//!     advance(&mut detector, 1.0).unwrap();
//! }
//! assert!(detector.status().live_time >= 10.0);
//! assert!(detector.spectrum().total_counts() > 0);
//! ```

#![forbid(unsafe_code)]

pub mod alarm;
pub mod any;
pub mod bridge;
pub mod changer;
mod courier;
pub mod detector;
pub mod error;
pub mod journal;
pub mod mcb;
pub mod presets;
pub mod remote;
#[cfg(feature = "rig")]
pub mod rig;
pub mod rng;
pub mod server;
pub mod simulator;
pub mod transport;

pub use alarm::{AlarmKind, AlarmLimits, AlarmMonitor};
pub use any::AnyMcb;
pub use bridge::{BRIDGE_EXECUTABLE, BridgeTransport, HubTransport, no_console};
pub use changer::{JobState, SampleChanger, SampleJob, SampleQueue};
pub use detector::{
    DetectorEntry, DetectorKind, DetectorList, MAX_DETECTOR_NUMBER, MIN_DETECTOR_NUMBER,
};
pub use error::DeviceError;
pub use mcb::{
    AdcSettings, AmplifierSettings, BaselineRestorer, Capabilities, GateMode, HighVoltageSettings,
    HvShutdown, Mcb, McbIdentity, McbProperties, McbStatus, Polarity, StabilizerSettings,
};
pub use presets::{MdaPreset, PRESET_TICK_SECONDS, PresetKind, Presets, UncertaintyPreset};
pub use remote::RemoteMcb;
pub use rng::Rng;
pub use simulator::{SimulatedMcb, SimulatedMcbBuilder, SimulatedSource, SourceLine};
pub use transport::{MockTransport, TcpTransport, Transport};

/// Re-exported so callers do not need `mantaray-core` just to name a mode.
pub use mantaray_core::AcquisitionMode;

/// Where a watched count stands, as the readings say.
#[derive(Clone, Debug, PartialEq, Eq)]
pub enum CountState {
    /// Still counting, or no reading taken since the watch began says
    /// otherwise yet.
    Counting,
    /// A believed reading taken after the watch began says it is not
    /// counting: the count is over (or never ran).
    Over,
    /// The readings have not been believed for [`UNREADABLE_READINGS`] in a
    /// row, with the last one's reason: whether it counts cannot be told.
    Unreadable(String),
    /// A START was accepted, and [`NEVER_STARTED_READINGS`] believed
    /// readings since have all said idle with the clocks where they stood:
    /// it did not count (the 926 accepts START and does not count when a
    /// preset is already satisfied).
    NeverStarted,
}

/// How many readings in a row may be unbelievable before a watched count is
/// given up as unreadable: about ten seconds of fetches at two a second.
pub const UNREADABLE_READINGS: u64 = 20;

/// How many believed readings after an accepted START may say idle with the
/// clocks unmoved before the count is called never started: about ten
/// seconds of fetches at two a second. Nobody has measured how long the 926
/// takes to raise ACTIVE after START (the bench only showed it lagging after
/// STOP), so the bound is wide: only a detector that truly does not count
/// trips it, never a slow one mid-start. Counted in readings, not in time.
pub const NEVER_STARTED_READINGS: u64 = 20;

/// Watches one count for its end, for a job's `WAIT` and the batch queue
/// (the one count-over rule, shared by the interface and the command line).
///
/// A START is accepted on its reply (amended rule 9), so right after it the
/// mirror's flag is still the reading from before it. The watch therefore
/// judges only readings taken after it began ([`Mcb::readings`]). Watching
/// a count it started ([`Self::started`]), a believed idle reading ends the
/// count once it has been seen counting or its clocks have moved; idle with
/// the clocks unmoved for [`NEVER_STARTED_READINGS`] readings, it never
/// started. Watching with no START of its own ([`Self::begin`], a job's WAIT
/// on a detector it did not start), the first believed idle reading ends
/// it, so a WAIT on an idle detector returns on the next fetch. Nothing here
/// reads a clock of this computer's, so no count is cut short or waited out
/// by a timer.
#[derive(Clone, Copy, Debug, PartialEq)]
pub struct CountWatch {
    /// The readings taken when the watch began.
    after: u64,
    /// The real time the mirror read when the watch began.
    real_time: f64,
    /// Whether the watch began on a START it saw accepted.
    started: bool,
    /// Whether a reading during the watch said counting.
    seen_counting: bool,
    /// The readings count when the current run of unbelieved ones began.
    untrusted_from: Option<u64>,
    /// Believed readings seen idle with the clocks unmoved since the START.
    idle_unmoved: u64,
    /// The readings count of the last reading added to `idle_unmoved`, so a
    /// second look at the same reading is not a second reading.
    idle_seen_at: u64,
}

impl CountWatch {
    /// Begins watching a count that may or may not be running.
    pub fn begin(mcb: &dyn Mcb) -> Self {
        Self {
            after: mcb.readings(),
            real_time: mcb.status().real_time,
            started: false,
            seen_counting: false,
            untrusted_from: None,
            idle_unmoved: 0,
            idle_seen_at: mcb.readings(),
        }
    }

    /// Begins watching a count whose START was just accepted.
    pub fn started(mcb: &dyn Mcb) -> Self {
        Self {
            started: true,
            ..Self::begin(mcb)
        }
    }

    /// Where the count stands now.
    pub fn check(&mut self, mcb: &dyn Mcb) -> CountState {
        let now = mcb.readings();
        if let Some(reason) = mcb.reading_untrusted() {
            let from = *self.untrusted_from.get_or_insert(now);
            if now.saturating_sub(from) >= UNREADABLE_READINGS {
                return CountState::Unreadable(reason.to_string());
            }
            return CountState::Counting;
        }
        self.untrusted_from = None;
        if now <= self.after {
            return CountState::Counting;
        }
        if mcb.is_active() {
            self.seen_counting = true;
            return CountState::Counting;
        }
        if !self.started || self.seen_counting || mcb.status().real_time > self.real_time {
            return CountState::Over;
        }
        // Only believed readings count towards never started: one taken
        // since the last look is counted once, and the ones between two
        // looks that were not seen are not counted at all (the call can come
        // later than NEVER_STARTED_READINGS fetches, never sooner).
        if now > self.idle_seen_at {
            self.idle_seen_at = now;
            self.idle_unmoved += 1;
        }
        if self.idle_unmoved >= NEVER_STARTED_READINGS {
            return CountState::NeverStarted;
        }
        CountState::Counting
    }
}

/// Advances an instrument and stops it if a preset has been satisfied.
///
/// This is the acquisition loop: call it on a timer with the elapsed real time.
/// It returns which preset stopped the run, if any.
pub fn advance(mcb: &mut dyn Mcb, elapsed_seconds: f64) -> Result<Option<PresetKind>, DeviceError> {
    // Polling happens whether or not a count is running: the automatic tuning
    // routines (Optimize, pole zero) run on the instrument's clock while idle.
    mcb.poll(elapsed_seconds)?;
    if !mcb.is_active() {
        return Ok(None);
    }
    // Presets read the clocks and the spectrum, which a remote mirror moves
    // only when a poll integrates a fetch - every half second. The interface
    // calls this every frame, and an ROI or MDA preset walks the marked
    // channels (or runs a whole fit) per evaluation; between fetches those
    // walks re-derived the same answer from the same numbers.
    if !mcb.poll_freshened() {
        return Ok(None);
    }
    // A reached preset stops the count even on a locked detector: the lock
    // guards against the operator's commands, not the preset (and without
    // this an uncertainty or MDA preset on a locked detector never stops).
    // A stop that fails is tried again at the next fresh reading, never per
    // frame, and its error is the caller's to show.
    match mcb.preset_reached() {
        Some(kind) => {
            mcb.stop_at_preset()?;
            Ok(Some(kind))
        }
        None => Ok(None),
    }
}

/// A one-line summary of a detector, for the multi-detector dashboard.
#[derive(Clone, Debug, PartialEq)]
pub struct DetectorTile {
    /// Detector number.
    pub number: u16,
    /// Display name.
    pub name: String,
    /// Whether it is collecting.
    pub active: bool,
    /// Live time in seconds.
    pub live_time: f64,
    /// Real time in seconds.
    pub real_time: f64,
    /// Dead time in percent.
    pub dead_time_percent: f64,
    /// Stored counts per second.
    pub count_rate: f64,
    /// Progress towards the nearest preset, when one is set.
    pub preset_progress: Option<f64>,
    /// Whether the instrument looks unhealthy from its status alone.
    pub alarm: bool,
    /// Short status phrase.
    pub status_text: String,
}

/// Summarises an instrument for the dashboard.
pub fn tile(mcb: &dyn Mcb) -> DetectorTile {
    let status = mcb.status();
    let progress = mcb.preset_progress();
    let status_text = if status.active {
        match progress {
            Some(fraction) => format!("Acquiring ({:.0} %)", fraction * 100.0),
            None => "Acquiring".to_string(),
        }
    } else if status.real_time > 0.0 {
        "Stopped".to_string()
    } else {
        "Idle".to_string()
    };
    DetectorTile {
        number: mcb.identity().number,
        name: mcb.identity().name.clone(),
        active: status.active,
        live_time: status.live_time,
        real_time: status.real_time,
        dead_time_percent: status.dead_time_percent,
        count_rate: status.total_count_rate,
        preset_progress: progress,
        alarm: status.dead_time_percent > 90.0,
        status_text,
    }
}
