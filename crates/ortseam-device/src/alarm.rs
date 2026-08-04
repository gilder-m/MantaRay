//! Alarm limits on detector status.

use serde::{Deserialize, Serialize};

use crate::mcb::McbStatus;

/// What tripped.
#[derive(Clone, Copy, Debug, PartialEq, Eq, Serialize, Deserialize)]
pub enum AlarmKind {
    /// Dead time above its limit.
    DeadTime,
    /// Count rate above its limit.
    HighCountRate,
    /// Count rate below its limit, which usually means a dead detector.
    LowCountRate,
    /// Detector bias away from its setting.
    HighVoltage,
    /// Measured activity above its limit.
    Activity,
}

impl AlarmKind {
    /// Label for the dashboard.
    pub fn label(&self) -> &'static str {
        match self {
            Self::DeadTime => "dead time",
            Self::HighCountRate => "count rate high",
            Self::LowCountRate => "count rate low",
            Self::HighVoltage => "bias fault",
            Self::Activity => "activity",
        }
    }
}

/// Thresholds a detector is watched against. `None` disables a check.
#[derive(Clone, Copy, Debug, Default, PartialEq, Serialize, Deserialize)]
pub struct AlarmLimits {
    /// Highest acceptable dead time, in percent.
    pub max_dead_time_percent: Option<f64>,
    /// Highest acceptable stored count rate, per second.
    pub max_count_rate: Option<f64>,
    /// Lowest acceptable stored count rate, per second.
    pub min_count_rate: Option<f64>,
    /// Highest acceptable activity, in the report's activity unit.
    pub max_activity: Option<f64>,
}

/// Watches a detector's status and latches what tripped.
#[derive(Clone, Debug, Default, PartialEq, Serialize, Deserialize)]
pub struct AlarmMonitor {
    /// The thresholds in force.
    pub limits: AlarmLimits,
    active: Vec<AlarmKind>,
    latched: Vec<AlarmKind>,
}

impl AlarmMonitor {
    /// A monitor with the given limits.
    pub fn new(limits: AlarmLimits) -> Self {
        Self {
            limits,
            active: Vec::new(),
            latched: Vec::new(),
        }
    }

    /// Checks a status snapshot, returning what is tripped right now.
    ///
    /// Anything that trips stays latched until [`AlarmMonitor::reset`], so a
    /// transient fault cannot be missed between glances at the screen.
    pub fn evaluate(&mut self, status: &McbStatus) -> &[AlarmKind] {
        self.active.clear();
        if let Some(limit) = self.limits.max_dead_time_percent
            && status.dead_time_percent > limit
        {
            self.active.push(AlarmKind::DeadTime);
        }
        if let Some(limit) = self.limits.max_count_rate
            && status.total_count_rate > limit
        {
            self.active.push(AlarmKind::HighCountRate);
        }
        if let Some(limit) = self.limits.min_count_rate {
            // Only meaningful once the detector has been counting.
            if status.real_time > 0.0 && status.total_count_rate < limit {
                self.active.push(AlarmKind::LowCountRate);
            }
        }
        for kind in &self.active {
            if !self.latched.contains(kind) {
                self.latched.push(*kind);
            }
        }
        &self.active
    }

    /// Records an activity alarm from an analysis result.
    pub fn check_activity(&mut self, activity: f64) -> bool {
        match self.limits.max_activity {
            Some(limit) if activity > limit => {
                if !self.latched.contains(&AlarmKind::Activity) {
                    self.latched.push(AlarmKind::Activity);
                }
                if !self.active.contains(&AlarmKind::Activity) {
                    self.active.push(AlarmKind::Activity);
                }
                true
            }
            _ => false,
        }
    }

    /// What is tripped right now.
    pub fn active(&self) -> &[AlarmKind] {
        &self.active
    }

    /// Everything that has tripped since the last reset.
    pub fn latched(&self) -> &[AlarmKind] {
        &self.latched
    }

    /// Whether anything is latched.
    pub fn is_tripped(&self) -> bool {
        !self.latched.is_empty()
    }

    /// Clears the latch.
    pub fn reset(&mut self) {
        self.active.clear();
        self.latched.clear();
    }
}
