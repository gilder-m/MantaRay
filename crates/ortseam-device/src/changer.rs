//! Sample changer and batch queue.
//!
//! MAESTRO drives a changer through the CHANGE SAMPLE output and SAMPLE READY
//! input lines (the `CHANGE_SAMPLE` and `WAIT_CHANGER` job commands). ortseam
//! adds a queue of samples with their own presets, so a rack can be counted
//! unattended.

use serde::{Deserialize, Serialize};

use crate::presets::Presets;

/// Where a queued sample stands.
#[derive(Clone, Debug, Default, PartialEq, Eq, Serialize, Deserialize)]
pub enum JobState {
    /// Waiting its turn.
    #[default]
    Pending,
    /// Being counted now.
    Counting,
    /// Counted and analysed.
    Complete,
    /// Abandoned, with a reason.
    Failed(String),
}

/// One sample in a batch.
#[derive(Clone, Debug, PartialEq, Serialize, Deserialize)]
pub struct SampleJob {
    /// Sample identifier, used in file names.
    pub id: String,
    /// Description saved with the spectrum.
    pub description: String,
    /// Presets to count this sample with.
    pub presets: Presets,
    /// Mass, volume or item count.
    pub quantity: f64,
    /// Unit of `quantity`.
    pub unit: String,
    /// Current state.
    pub state: JobState,
}

impl SampleJob {
    /// A pending sample with a live-time preset.
    pub fn new(id: &str, live_time_seconds: f64) -> Self {
        Self {
            id: id.to_string(),
            description: String::new(),
            presets: Presets::live(live_time_seconds),
            quantity: 1.0,
            unit: String::new(),
            state: JobState::Pending,
        }
    }
}

/// A queue of samples worked through in order.
#[derive(Clone, Debug, Default, PartialEq, Serialize, Deserialize)]
pub struct SampleQueue {
    jobs: Vec<SampleJob>,
    position: usize,
}

impl SampleQueue {
    /// Appends a sample.
    pub fn push(&mut self, job: SampleJob) {
        self.jobs.push(job);
    }

    /// Every sample.
    pub fn jobs(&self) -> &[SampleJob] {
        &self.jobs
    }

    /// Total number of samples.
    pub fn len(&self) -> usize {
        self.jobs.len()
    }

    /// True when the queue holds nothing.
    pub fn is_empty(&self) -> bool {
        self.jobs.is_empty()
    }

    /// Samples not yet finished.
    pub fn remaining(&self) -> usize {
        self.jobs.len().saturating_sub(self.position)
    }

    /// The sample being worked on.
    pub fn current(&self) -> Option<&SampleJob> {
        self.jobs.get(self.position)
    }

    /// Mutable access to the sample being worked on.
    pub fn current_mut(&mut self) -> Option<&mut SampleJob> {
        self.jobs.get_mut(self.position)
    }

    /// Marks the current sample as counting.
    pub fn begin_current(&mut self) {
        if let Some(job) = self.jobs.get_mut(self.position) {
            job.state = JobState::Counting;
        }
    }

    /// Marks the current sample complete and moves on.
    pub fn complete_current(&mut self) {
        if let Some(job) = self.jobs.get_mut(self.position) {
            job.state = JobState::Complete;
            self.position += 1;
        }
    }

    /// Marks the current sample failed and moves on.
    pub fn fail_current(&mut self, reason: &str) {
        if let Some(job) = self.jobs.get_mut(self.position) {
            job.state = JobState::Failed(reason.to_string());
            self.position += 1;
        }
    }

    /// True when every sample has been dealt with.
    pub fn is_complete(&self) -> bool {
        self.position >= self.jobs.len()
    }

    /// Fraction of the queue dealt with.
    pub fn progress(&self) -> f64 {
        if self.jobs.is_empty() {
            return 1.0;
        }
        self.position as f64 / self.jobs.len() as f64
    }

    /// How many samples completed successfully.
    pub fn completed(&self) -> usize {
        self.jobs
            .iter()
            .filter(|job| job.state == JobState::Complete)
            .count()
    }

    /// How many samples failed.
    pub fn failed(&self) -> usize {
        self.jobs
            .iter()
            .filter(|job| matches!(job.state, JobState::Failed(_)))
            .count()
    }

    /// Puts the queue back to the start, clearing every state.
    pub fn reset(&mut self) {
        self.position = 0;
        for job in &mut self.jobs {
            job.state = JobState::Pending;
        }
    }
}

/// A simulated sample changer with the MAESTRO handshake.
#[derive(Clone, Debug, PartialEq, Serialize, Deserialize)]
pub struct SampleChanger {
    positions: usize,
    position: usize,
    moving: bool,
    elapsed: f64,
    /// Seconds a change takes.
    pub move_seconds: f64,
}

impl SampleChanger {
    /// A changer with `positions` slots, parked at slot 0.
    pub fn new(positions: usize) -> Self {
        Self {
            positions,
            position: 0,
            moving: false,
            elapsed: 0.0,
            move_seconds: 2.0,
        }
    }

    /// Asserts CHANGE SAMPLE: the ready line drops until the changer arrives.
    pub fn request_change(&mut self) {
        if self.is_exhausted() {
            return;
        }
        self.moving = true;
        self.elapsed = 0.0;
    }

    /// Advances the changer, returning whether it is ready.
    pub fn poll(&mut self, elapsed_seconds: f64) -> bool {
        if self.moving {
            self.elapsed += elapsed_seconds;
            if self.elapsed >= self.move_seconds {
                self.moving = false;
                self.elapsed = 0.0;
                self.position = (self.position + 1).min(self.positions);
            }
        }
        self.is_ready()
    }

    /// Whether the SAMPLE READY line is asserted.
    pub fn is_ready(&self) -> bool {
        !self.moving
    }

    /// Current slot.
    pub fn position(&self) -> usize {
        self.position
    }

    /// Number of slots.
    pub fn positions(&self) -> usize {
        self.positions
    }

    /// True once every slot has been presented.
    pub fn is_exhausted(&self) -> bool {
        self.position >= self.positions
    }

    /// Parks the changer at slot 0.
    pub fn reset(&mut self) {
        self.position = 0;
        self.moving = false;
        self.elapsed = 0.0;
    }
}
