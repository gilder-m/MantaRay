//! Instruments: the multichannel-buffer abstraction, presets, a physics-based
//! simulator, detector monitoring and the batch workflow.
//!
//! ```
//! use mantaray_device::{Mcb, SimulatedMcb, advance};
//!
//! let mut detector = SimulatedMcb::new(1, "SIM-01");
//! detector.presets_mut().live_time = Some(10.0);
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
pub use presets::{MdaPreset, PresetKind, Presets, UncertaintyPreset};
pub use remote::RemoteMcb;
pub use rng::Rng;
pub use simulator::{SimulatedMcb, SimulatedMcbBuilder, SimulatedSource, SourceLine};
pub use transport::{MockTransport, TcpTransport, Transport};

/// Re-exported so callers do not need `mantaray-core` just to name a mode.
pub use mantaray_core::AcquisitionMode;

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
    match mcb.preset_reached() {
        Some(kind) => {
            mcb.stop()?;
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
