//! Instrument error type.

use thiserror::Error;

/// Why an instrument refused a request.
#[derive(Clone, Debug, Error, PartialEq, Eq)]
pub enum DeviceError {
    /// The instrument is locked (MAESTRO Services/Lock-Unlock Detector).
    #[error("the detector is locked by {owner}")]
    Locked {
        /// Who holds the lock.
        owner: String,
    },
    /// The request cannot be honoured in the current state.
    #[error("the detector is busy: {detail}")]
    Busy {
        /// What is in the way.
        detail: String,
    },
    /// A preset change was attempted while the detector is counting.
    ///
    /// MAESTRO's error list: "Presets can't be changed during acquisition.
    /// Changes in the preset condition(s) are not allowed while the Detector
    /// is actively acquiring." The text is MAESTRO's, word for word.
    #[error("Presets can't be changed during acquisition.")]
    PresetsWhileCounting,
    /// The instrument could not say whether it is counting: the status read
    /// failed, or its reply was not a status line with the counting flag.
    /// The change that needed the answer is refused rather than guessed.
    #[error("cannot tell whether the detector is counting ({reason}); nothing was sent")]
    CountingUnknown {
        /// Why the answer could not be had.
        reason: String,
    },
    /// A setting was outside the instrument's range.
    #[error("{setting} out of range: {detail}")]
    OutOfRange {
        /// Which setting.
        setting: String,
        /// Why it was rejected.
        detail: String,
    },
    /// The instrument does not have this feature.
    #[error("this detector does not support {feature}")]
    NotSupported {
        /// The missing feature.
        feature: String,
    },
    /// A raw command was rejected.
    #[error("command {command:?} failed: {detail}")]
    Command {
        /// The command sent.
        command: String,
        /// Why it failed.
        detail: String,
    },
    /// Some of a set of changes reached the instrument and a later one did
    /// not. The instrument is not rolled back; the message says what it now
    /// holds and what it does not, e.g. `bias set to 1200 V; presets not set:
    /// ...`.
    #[error("{applied}; {failed}")]
    PartlyApplied {
        /// What was applied before the failure.
        applied: String,
        /// What was not, and why.
        failed: String,
    },
    /// Communication with the instrument failed.
    #[error("communication failure: {detail}")]
    Communication {
        /// What went wrong.
        detail: String,
    },
    /// Reaching the instrument failed.
    #[error("could not reach {address}: {detail}")]
    Connection {
        /// The address that was tried.
        address: String,
        /// What went wrong.
        detail: String,
    },
    /// No detector with that number is configured.
    #[error("no detector numbered {number}")]
    UnknownDetector {
        /// The number asked for.
        number: u16,
    },
}
