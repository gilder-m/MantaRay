//! Error types for calibration and analysis.

use thiserror::Error;

/// Failures while establishing a calibration.
#[derive(Debug, Error, Clone, PartialEq, Eq)]
pub enum CalibrationError {
    /// Fewer points than the requested fit needs.
    #[error("a calibration needs at least {needed} points, got {got}")]
    NotEnoughPoints {
        /// Minimum number of points required.
        needed: usize,
        /// Number of points supplied.
        got: usize,
    },
    /// The points do not determine a fit (e.g. all at the same channel).
    #[error("the calibration points are degenerate")]
    Degenerate,
    /// Automatic calibration found no assignment of peaks to lines it trusts.
    #[error(
        "no calibration matched: {matched} of {peaks} peaks lined up with the reference energies"
    )]
    NoMatch {
        /// Peaks offered for matching.
        peaks: usize,
        /// The most that any candidate calibration explained.
        matched: usize,
    },
}

/// Failures while analysing a spectrum.
#[derive(Debug, Error, Clone, PartialEq, Eq)]
pub enum AnalysisError {
    /// The region of interest reaches past the end of the spectrum.
    #[error("region {start}-{end} lies outside a spectrum of {length} channels")]
    RoiOutOfRange {
        /// First channel of the region.
        start: usize,
        /// Last channel of the region.
        end: usize,
        /// Length of the spectrum.
        length: usize,
    },
    /// The region cannot hold the requested number of background points.
    #[error(
        "region is {width} channels wide but {needed} are needed for {background_points} background points"
    )]
    RoiTooNarrow {
        /// Width of the supplied region.
        width: usize,
        /// Minimum width required.
        needed: usize,
        /// Configured number of background points.
        background_points: usize,
    },
    /// Two spectra that must line up channel-for-channel do not.
    #[error("spectrum lengths differ: {left} vs {right}")]
    LengthMismatch {
        /// Length of the spectrum being modified.
        left: usize,
        /// Length of the other spectrum.
        right: usize,
    },
    /// A live-time ratio was requested but a live time is missing or zero.
    #[error("both spectra need a non-zero live time for a live-time ratio strip")]
    NoLiveTime,
    /// The spectrum holds no data.
    #[error("the spectrum is empty")]
    EmptySpectrum,
}
