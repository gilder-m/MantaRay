//! Quality-assurance control charts: track a detector figure of merit over time
//! and flag drift.

use chrono::NaiveDateTime;
use serde::{Deserialize, Serialize};

/// What a control chart tracks.
#[derive(Clone, Copy, Debug, Default, PartialEq, Eq, Serialize, Deserialize)]
pub enum QaMetric {
    /// Peak centroid, in channels or keV.
    #[default]
    Centroid,
    /// Peak width.
    Fwhm,
    /// Net count rate of a check-source peak.
    NetRate,
    /// Absolute efficiency.
    Efficiency,
    /// Dead time percentage.
    DeadTime,
    /// Background count rate.
    Background,
}

impl QaMetric {
    /// Label for reports and charts.
    pub fn label(&self) -> &'static str {
        match self {
            Self::Centroid => "Centroid",
            Self::Fwhm => "FWHM",
            Self::NetRate => "Net rate",
            Self::Efficiency => "Efficiency",
            Self::DeadTime => "Dead time",
            Self::Background => "Background",
        }
    }
}

/// Where the warning and action limits sit, in standard deviations.
#[derive(Clone, Copy, Debug, PartialEq, Serialize, Deserialize)]
pub struct ControlLimits {
    /// Warning limit, in standard deviations (default 2).
    pub warning_sigma: f64,
    /// Action limit, in standard deviations (default 3).
    pub action_sigma: f64,
    /// Fixed centre line; when absent the mean of the history is used.
    pub center: Option<f64>,
    /// Fixed standard deviation; when absent the history is used.
    pub sigma: Option<f64>,
}

impl Default for ControlLimits {
    fn default() -> Self {
        Self {
            warning_sigma: 2.0,
            action_sigma: 3.0,
            center: None,
            sigma: None,
        }
    }
}

impl ControlLimits {
    /// Fixed limits established from a baseline period.
    pub fn fixed(center: f64, sigma: f64) -> Self {
        Self {
            center: Some(center),
            sigma: Some(sigma),
            ..Self::default()
        }
    }
}

/// Result of comparing a value against the control limits.
#[derive(Clone, Copy, Debug, Default, PartialEq, Eq, Serialize, Deserialize)]
pub enum QaStatus {
    /// Inside the warning limit.
    #[default]
    InControl,
    /// Between the warning and action limits.
    Warning,
    /// Beyond the action limit.
    Action,
}

impl QaStatus {
    /// Label for reports.
    pub fn label(&self) -> &'static str {
        match self {
            Self::InControl => "in control",
            Self::Warning => "warning",
            Self::Action => "action",
        }
    }
}

/// One measurement on a control chart.
#[derive(Clone, Debug, PartialEq, Serialize, Deserialize)]
pub struct QaRecord {
    /// When the measurement was taken.
    pub timestamp: NaiveDateTime,
    /// The measured value.
    pub value: f64,
    /// Optional note (detector, source, operator).
    pub note: Option<String>,
}

impl QaRecord {
    /// A record with no note.
    pub fn new(timestamp: NaiveDateTime, value: f64) -> Self {
        Self {
            timestamp,
            value,
            note: None,
        }
    }

    /// A record with a note.
    pub fn with_note(timestamp: NaiveDateTime, value: f64, note: &str) -> Self {
        Self {
            timestamp,
            value,
            note: Some(note.to_string()),
        }
    }
}

/// Summary statistics of a chart's history.
#[derive(Clone, Copy, Debug, PartialEq, Serialize, Deserialize)]
pub struct QaStatistics {
    /// Arithmetic mean.
    pub mean: f64,
    /// Sample standard deviation (n-1).
    pub standard_deviation: f64,
    /// Number of records.
    pub count: usize,
    /// Smallest value.
    pub min: f64,
    /// Largest value.
    pub max: f64,
}

/// A control chart: a metric, its history and its limits.
#[derive(Clone, Debug, PartialEq, Serialize, Deserialize)]
pub struct QaChart {
    /// What is tracked.
    pub metric: QaMetric,
    /// Limits used to judge new values.
    pub limits: ControlLimits,
    /// Name of the detector or check source, for reports.
    pub subject: String,
    records: Vec<QaRecord>,
}

impl QaChart {
    /// An empty chart.
    pub fn new(metric: QaMetric, limits: ControlLimits) -> Self {
        Self {
            metric,
            limits,
            subject: String::new(),
            records: Vec::new(),
        }
    }

    /// Adds a measurement.
    pub fn push(&mut self, record: QaRecord) {
        self.records.push(record);
        self.records.sort_by_key(|record| record.timestamp);
    }

    /// The history, oldest first.
    pub fn records(&self) -> &[QaRecord] {
        &self.records
    }

    /// Number of records.
    pub fn len(&self) -> usize {
        self.records.len()
    }

    /// True when nothing has been recorded.
    pub fn is_empty(&self) -> bool {
        self.records.is_empty()
    }

    /// Forgets the history.
    pub fn clear(&mut self) {
        self.records.clear();
    }

    /// Statistics over the whole history; `None` with fewer than two records.
    pub fn statistics(&self) -> Option<QaStatistics> {
        statistics(self.records.iter().map(|record| record.value))
    }

    /// Judges a value against the limits.
    ///
    /// Fixed limits are used when set, otherwise the statistics of the recorded
    /// history.
    pub fn evaluate(&self, value: f64) -> QaStatus {
        let (center, sigma) = match (self.limits.center, self.limits.sigma) {
            (Some(center), Some(sigma)) => (center, sigma),
            _ => match self.statistics() {
                Some(stats) => (
                    self.limits.center.unwrap_or(stats.mean),
                    self.limits.sigma.unwrap_or(stats.standard_deviation),
                ),
                None => return QaStatus::InControl,
            },
        };
        classify(value, center, sigma, &self.limits)
    }

    /// Judges the most recent record against the records before it, so that an
    /// out-of-control point cannot hide inside its own statistics.
    pub fn status(&self) -> QaStatus {
        let Some(last) = self.records.last() else {
            return QaStatus::InControl;
        };
        let history = &self.records[..self.records.len() - 1];
        let (center, sigma) = match (self.limits.center, self.limits.sigma) {
            (Some(center), Some(sigma)) => (center, sigma),
            _ => match statistics(history.iter().map(|record| record.value)) {
                Some(stats) => (
                    self.limits.center.unwrap_or(stats.mean),
                    self.limits.sigma.unwrap_or(stats.standard_deviation),
                ),
                None => return QaStatus::InControl,
            },
        };
        classify(last.value, center, sigma, &self.limits)
    }

    /// Freezes the current statistics as fixed control limits.
    pub fn freeze_limits(&mut self) -> bool {
        match self.statistics() {
            Some(stats) => {
                self.limits.center = Some(stats.mean);
                self.limits.sigma = Some(stats.standard_deviation);
                true
            }
            None => false,
        }
    }
}

fn classify(value: f64, center: f64, sigma: f64, limits: &ControlLimits) -> QaStatus {
    if sigma <= 0.0 {
        return QaStatus::InControl;
    }
    let z = (value - center).abs() / sigma;
    // A measurement that is not a number cannot be in control: NaN fails
    // every comparison, and without this it would sail through as fine.
    if !z.is_finite() || z >= limits.action_sigma {
        QaStatus::Action
    } else if z >= limits.warning_sigma {
        QaStatus::Warning
    } else {
        QaStatus::InControl
    }
}

fn statistics<I: Iterator<Item = f64>>(values: I) -> Option<QaStatistics> {
    let values: Vec<f64> = values.collect();
    if values.len() < 2 {
        return None;
    }
    let count = values.len();
    let mean = values.iter().sum::<f64>() / count as f64;
    let variance = values
        .iter()
        .map(|value| (value - mean) * (value - mean))
        .sum::<f64>()
        / (count as f64 - 1.0);
    Some(QaStatistics {
        mean,
        standard_deviation: variance.sqrt(),
        count,
        min: values.iter().copied().fold(f64::INFINITY, f64::min),
        max: values.iter().copied().fold(f64::NEG_INFINITY, f64::max),
    })
}
