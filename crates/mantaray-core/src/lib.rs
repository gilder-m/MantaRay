//! Core data model and analysis mathematics for [mantaray], an open multichannel
//! analyzer (MCA) workbench for gamma spectroscopy.
//!
//! The algorithms here follow the published behaviour of the ORTEC MAESTRO MCA
//! emulator (A65-B32 v7 user manual) so that results are directly comparable:
//!
//! | Concept | Reference |
//! |---|---|
//! | ROI background / areas / uncertainty | §4.3.5.1 equations (17)-(21) |
//! | Counting activity `cA` | §4.3.5.1 equation (22) |
//! | Five-point binomial smoothing | §4.3.8 equation (23) |
//! | Peak search | §4.3.4 (Mariscotti-style second difference) |
//! | Energy calibration | §4.3.2 (2 points linear, 3+ quadratic, max 96) |
//!
//! Channels are zero-based everywhere in this crate.
//!
//! [mantaray]: https://github.com/gilder-m/MantaRay

#![forbid(unsafe_code)]

pub mod analysis;
pub mod calibration;
pub mod efficiency;
pub mod elements;
pub mod error;
pub mod library;
pub mod qa;
pub mod quant;
pub mod roi;
pub mod settings;
pub mod spectrum;
pub mod undo;

pub use analysis::{
    FoundPeak, GaussianFit, PeakInfo, StripFactor, counting_activity, mark_peaks, mda_currie,
    peak_info, peak_search, smooth, smoothed, statistical_uncertainty, strip, sum_channels,
};
pub use calibration::{
    AutoCalibration, CalibrationPoint, CalibrationTable, EnergyCalibration, MAX_CALIBRATION_POINTS,
    ShapeCalibration, auto_calibrate,
};
pub use efficiency::{
    CertifiedSource, EfficiencyCalibration, EfficiencyModel, EfficiencyPoint, decay_factor,
    efficiency_from_source, source_activity_at,
};
pub use error::{AnalysisError, CalibrationError};
pub use library::{
    LibraryHit, LibraryMatch, LibraryPeak, Nuclide, NuclideFlags, NuclideLibrary, PhotonKind,
    parse_nuclide_name, significant,
};
pub use qa::{ControlLimits, QaChart, QaMetric, QaRecord, QaStatistics, QaStatus};
pub use quant::{
    AnalysisOptions, Identification, LineResult, NuclideResult, PeakResult, QuantitativeReport,
    SampleInfo, analyse, bq_to_uci, decay_during_acquisition, decay_to_reference,
};
pub use roi::{Roi, RoiSet};
pub use settings::CalculationSettings;
pub use spectrum::{AcquisitionMode, Spectrum};
pub use undo::{DEFAULT_CAPACITY as UNDO_CAPACITY, Snapshot, UndoStack};

/// Conversion constant between the standard deviation and the full width at
/// half maximum of a Gaussian: `2 * sqrt(2 * ln 2)`.
pub const FWHM_PER_SIGMA: f64 = 2.354_820_045_030_949;
