//! MAESTRO-compatible `.JOB` automation.
//!
//! A job is a plain-text list of commands (§6 of the MAESTRO manual). Parse one
//! with [`Job::parse`] and run it against anything that implements [`JobHost`]:
//!
//! ```
//! use mantaray_jobs::{Job, JobError, JobHost, JobVariables, run};
//!
//! #[derive(Default)]
//! struct Log {
//!     lines: Vec<String>,
//!     variables: JobVariables,
//! }
//!
//! impl JobHost for Log {
//!     fn variables(&mut self) -> &mut JobVariables {
//!         &mut self.variables
//!     }
//!     fn save(&mut self, path: Option<&str>) -> Result<(), JobError> {
//!         self.lines.push(path.unwrap_or_default().to_string());
//!         Ok(())
//!     }
//! }
//!
//! let job = Job::parse("LOOP 2\nSAVE \"RUN???.CHN\"\nEND_LOOP\n").unwrap();
//! let mut host = Log::default();
//! run(&job, &mut host).unwrap();
//! assert_eq!(host.lines, ["RUN001.CHN", "RUN002.CHN"]);
//! ```
//!
//! Commands the host does not implement fail with [`JobError::Unsupported`],
//! naming the line, rather than being silently skipped.

#![forbid(unsafe_code)]

pub mod command;
pub mod error;
pub mod exec;
pub mod host;
pub mod parse;
pub mod vars;

pub use command::{BeepSpec, Command, ListSlice};
pub use error::JobError;
pub use exec::{JobOutcome, Runner, run};
pub use host::JobHost;
pub use parse::{Job, JobLine};
pub use vars::JobVariables;
