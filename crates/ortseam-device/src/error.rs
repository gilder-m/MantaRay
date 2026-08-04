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
