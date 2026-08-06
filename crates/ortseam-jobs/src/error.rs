//! Job errors.

use thiserror::Error;

/// Why a job failed to parse or run.
#[derive(Clone, Debug, Error, PartialEq, Eq)]
pub enum JobError {
    /// The line does not start with a known command.
    #[error("line {line}: {word} is not a job command")]
    UnknownCommand {
        /// One-based line number.
        line: usize,
        /// The word found.
        word: String,
    },
    /// A command's arguments could not be read.
    #[error("line {line}: {command}: {detail}")]
    BadArgument {
        /// One-based line number.
        line: usize,
        /// The command.
        command: String,
        /// What was wrong.
        detail: String,
    },
    /// The application cannot carry this command out.
    #[error("line {line}: {command} is not supported here")]
    Unsupported {
        /// One-based line number, or 0 before the executor fills it in.
        line: usize,
        /// The command.
        command: String,
    },
    /// A loop appeared inside another loop.
    #[error("line {line}: loops cannot be nested")]
    NestedLoop {
        /// One-based line number of the inner loop.
        line: usize,
    },
    /// The host reported a failure.
    #[error("line {line}: {detail}")]
    HostFailed {
        /// One-based line number.
        line: usize,
        /// What the host said.
        detail: String,
    },
    /// The runaway guard tripped: far more steps than any real job takes.
    #[error(
        "the job was stopped after a million steps - a LOOP that large is \
         almost certainly a runaway"
    )]
    Aborted,
}

impl JobError {
    /// A "not supported" error without a line number yet.
    pub fn unsupported(command: &str) -> Self {
        Self::Unsupported {
            line: 0,
            command: command.to_string(),
        }
    }

    /// A host failure without a line number yet.
    pub fn host(detail: impl Into<String>) -> Self {
        Self::HostFailed {
            line: 0,
            detail: detail.into(),
        }
    }

    /// Fills in the line number when it is still unknown.
    pub fn at_line(self, line: usize) -> Self {
        match self {
            Self::Unsupported { line: 0, command } => Self::Unsupported { line, command },
            Self::HostFailed { line: 0, detail } => Self::HostFailed { line, detail },
            other => other,
        }
    }

    /// The line the failure happened on, when known.
    pub fn line(&self) -> Option<usize> {
        match self {
            Self::UnknownCommand { line, .. }
            | Self::BadArgument { line, .. }
            | Self::Unsupported { line, .. }
            | Self::NestedLoop { line }
            | Self::HostFailed { line, .. } => Some(*line),
            Self::Aborted => None,
        }
    }
}
