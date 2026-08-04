//! The detector pick list (MAESTRO Services/Edit Detector List).

use serde::{Deserialize, Serialize};

use crate::error::DeviceError;

/// Lowest detector number MAESTRO allows (0 means "the buffer").
pub const MIN_DETECTOR_NUMBER: u16 = 1;
/// Highest detector number MAESTRO allows.
pub const MAX_DETECTOR_NUMBER: u16 = 999;

/// How a detector entry is served.
#[derive(Clone, Copy, Debug, Default, PartialEq, Eq, Serialize, Deserialize)]
pub enum DetectorKind {
    /// The built-in simulator.
    #[default]
    Simulated,
    /// A file replayed as if it were live data.
    FileReplay,
    /// A real instrument reached over the network.
    Network,
    /// A real instrument on a local port.
    Local,
}

impl DetectorKind {
    /// Label for the detector list editor.
    pub fn label(&self) -> &'static str {
        match self {
            Self::Simulated => "Simulated",
            Self::FileReplay => "File replay",
            Self::Network => "Network",
            Self::Local => "Local",
        }
    }
}

/// One row of the detector list.
#[derive(Clone, Debug, PartialEq, Eq, Serialize, Deserialize)]
pub struct DetectorEntry {
    /// Detector number, 1..=999.
    pub number: u16,
    /// Display name.
    pub name: String,
    /// Model name.
    pub model: String,
    /// How it is served.
    pub kind: DetectorKind,
    /// Memory size in channels.
    pub channels: usize,
    /// Free-form description.
    pub description: String,
}

impl DetectorEntry {
    /// A simulated detector with sensible defaults.
    pub fn simulated(number: u16, name: &str) -> Self {
        Self {
            number,
            name: name.to_string(),
            model: "SIM-HPGe".to_string(),
            kind: DetectorKind::Simulated,
            channels: 8_192,
            description: "simulated detector".to_string(),
        }
    }
}

/// A named pick list of detectors.
#[derive(Clone, Debug, Default, PartialEq, Eq, Serialize, Deserialize)]
pub struct DetectorList {
    /// Pick-list name (MAESTRO limits this to five characters).
    pub name: String,
    entries: Vec<DetectorEntry>,
}

impl DetectorList {
    /// An empty list.
    pub fn new(name: &str) -> Self {
        Self {
            name: name.chars().take(5).collect(),
            entries: Vec::new(),
        }
    }

    /// A list holding one simulated detector, for a first run.
    pub fn with_simulator() -> Self {
        let mut list = Self::new("M32MC");
        list.push(DetectorEntry::simulated(1, "SIM-01"));
        list
    }

    /// Adds or replaces an entry, keeping the list ordered by number.
    pub fn push(&mut self, entry: DetectorEntry) {
        self.entries
            .retain(|existing| existing.number != entry.number);
        self.entries.push(entry);
        self.entries.sort_by_key(|entry| entry.number);
    }

    /// Adds an entry, rejecting numbers outside the allowed range.
    pub fn try_push(&mut self, entry: DetectorEntry) -> Result<(), DeviceError> {
        if !(MIN_DETECTOR_NUMBER..=MAX_DETECTOR_NUMBER).contains(&entry.number) {
            return Err(DeviceError::OutOfRange {
                setting: "detector number".into(),
                detail: format!(
                    "{} is outside {MIN_DETECTOR_NUMBER}..={MAX_DETECTOR_NUMBER}",
                    entry.number
                ),
            });
        }
        self.push(entry);
        Ok(())
    }

    /// Looks an entry up by number.
    pub fn get(&self, number: u16) -> Option<&DetectorEntry> {
        self.entries.iter().find(|entry| entry.number == number)
    }

    /// Removes an entry by number, reporting whether one was there.
    pub fn remove(&mut self, number: u16) -> bool {
        let before = self.entries.len();
        self.entries.retain(|entry| entry.number != number);
        self.entries.len() != before
    }

    /// Every entry, in number order.
    pub fn entries(&self) -> &[DetectorEntry] {
        &self.entries
    }

    /// The configured numbers, in order.
    pub fn numbers(&self) -> Vec<u16> {
        self.entries.iter().map(|entry| entry.number).collect()
    }

    /// Number of entries.
    pub fn len(&self) -> usize {
        self.entries.len()
    }

    /// True when nothing is configured.
    pub fn is_empty(&self) -> bool {
        self.entries.is_empty()
    }

    /// Serialises the list.
    pub fn to_json(&self) -> Result<String, serde_json::Error> {
        serde_json::to_string_pretty(self)
    }

    /// Parses a serialised list.
    pub fn from_json(text: &str) -> Result<Self, serde_json::Error> {
        serde_json::from_str(text)
    }
}
