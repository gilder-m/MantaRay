//! List-mode (event-by-event) data.
//!
//! MAESTRO's `.Lis` layout is instrument specific, so ortseam defines its own
//! container and can slice it by time exactly as the List Data Range dialog and
//! the `SET_RANGE` job command do.
//!
//! ```text
//! 0   8    magic "ORTSLIST"
//! 8   2    u16 version (1)
//! 10  2    u16 flags (reserved, 0)
//! 12  4    u32 channel count of the histogram
//! 16  8    f64 live time in seconds
//! 24  8    f64 real time in seconds
//! 32  4    u32 event count
//! 36  2    u16 detector number
//! 38  2    u16 length of the sample description
//! 40  ...  sample description, UTF-8
//! then     events: f64 time in seconds, u32 channel
//! ```

use chrono::NaiveDateTime;
use ortseam_core::{AcquisitionMode, EnergyCalibration, Spectrum};
use serde::{Deserialize, Serialize};

use crate::{Cursor, FormatError};

const MAGIC: &[u8; 8] = b"ORTSLIST";
const VERSION: u16 = 1;

/// One detected event.
#[derive(Clone, Copy, Debug, PartialEq, Serialize, Deserialize)]
pub struct ListEvent {
    /// Seconds since the start of acquisition.
    pub time: f64,
    /// Channel the event was sorted into.
    pub channel: u32,
}

/// A time slice of list-mode data.
#[derive(Clone, Copy, Debug, PartialEq, Serialize, Deserialize)]
pub struct ListRange {
    /// Start of the slice, in seconds of real time.
    pub start: f64,
    /// Length of the slice, in seconds.
    pub duration: f64,
}

impl ListRange {
    /// A slice from a start time and a duration, both in seconds.
    pub fn from_seconds(start: f64, duration: f64) -> Self {
        Self { start, duration }
    }

    /// End of the slice, in seconds.
    pub fn end(&self) -> f64 {
        self.start + self.duration
    }

    /// The same slice with `extra` seconds added, as the Increment button does.
    pub fn incremented(&self, extra: f64) -> Self {
        Self {
            start: self.start,
            duration: self.duration + extra,
        }
    }

    /// True when the slice covers a positive span.
    pub fn is_valid(&self) -> bool {
        self.duration > 0.0 && self.start >= 0.0
    }
}

/// A list-mode data set.
#[derive(Clone, Debug, Default, PartialEq, Serialize, Deserialize)]
pub struct ListModeFile {
    /// Number of channels the events histogram into.
    pub channel_count: usize,
    /// Live time of the whole acquisition, in seconds.
    pub live_time: f64,
    /// Real time of the whole acquisition, in seconds.
    pub real_time: f64,
    /// Wall-clock start of acquisition.
    pub start_time: Option<NaiveDateTime>,
    /// Detector number.
    pub detector_id: u16,
    /// Detector description.
    pub detector_description: String,
    /// Sample description.
    pub sample_description: String,
    /// Energy calibration in force during acquisition.
    pub energy_calibration: Option<EnergyCalibration>,
    events: Vec<ListEvent>,
}

impl ListModeFile {
    /// An empty file that histograms into `channel_count` channels.
    pub fn new(channel_count: usize) -> Self {
        Self {
            channel_count,
            ..Self::default()
        }
    }

    /// Appends an event.
    pub fn push(&mut self, time: f64, channel: u32) {
        self.events.push(ListEvent { time, channel });
    }

    /// The events, in the order they were recorded.
    pub fn events(&self) -> &[ListEvent] {
        &self.events
    }

    /// Number of events.
    pub fn len(&self) -> usize {
        self.events.len()
    }

    /// True when no events were recorded.
    pub fn is_empty(&self) -> bool {
        self.events.is_empty()
    }

    /// Time span covered by the events.
    pub fn duration(&self) -> f64 {
        match (self.events.first(), self.events.last()) {
            (Some(first), Some(last)) => last.time - first.time,
            _ => 0.0,
        }
    }

    /// Histograms every event.
    pub fn to_spectrum(&self) -> Spectrum {
        let mut spectrum = self.empty_spectrum();
        for event in &self.events {
            spectrum.add_count(event.channel as usize);
        }
        spectrum.live_time = self.live_time;
        spectrum.real_time = self.real_time;
        spectrum
    }

    /// Histograms the events inside `range`.
    ///
    /// The slice's real time is the requested duration and its live time is
    /// scaled by the duty cycle of the whole acquisition.
    pub fn slice(&self, range: ListRange) -> Option<Spectrum> {
        if range.duration < 0.0 {
            return None;
        }
        let mut spectrum = self.empty_spectrum();
        let end = range.end();
        for event in &self.events {
            if event.time >= range.start && event.time < end {
                spectrum.add_count(event.channel as usize);
            }
        }
        spectrum.real_time = range.duration;
        spectrum.live_time = if self.real_time > 0.0 {
            range.duration * (self.live_time / self.real_time)
        } else {
            range.duration
        };
        if let Some(start) = self.start_time {
            spectrum.start_time =
                Some(start + chrono::Duration::milliseconds((range.start * 1000.0) as i64));
        }
        Some(spectrum)
    }

    fn empty_spectrum(&self) -> Spectrum {
        let mut spectrum = Spectrum::new(self.channel_count);
        spectrum.detector_id = self.detector_id;
        spectrum.detector_description = self.detector_description.clone();
        spectrum.sample_description = self.sample_description.clone();
        spectrum.energy_calibration = self.energy_calibration.clone();
        spectrum.start_time = self.start_time;
        spectrum.mode = AcquisitionMode::List;
        spectrum
    }
}

/// Writes a list-mode file.
pub fn write(file: &ListModeFile) -> Result<Vec<u8>, FormatError> {
    let description = file.sample_description.as_bytes();
    let mut out = Vec::with_capacity(40 + description.len() + 12 * file.len());
    out.extend_from_slice(MAGIC);
    out.extend_from_slice(&VERSION.to_le_bytes());
    out.extend_from_slice(&0u16.to_le_bytes());
    out.extend_from_slice(&(file.channel_count as u32).to_le_bytes());
    out.extend_from_slice(&file.live_time.to_le_bytes());
    out.extend_from_slice(&file.real_time.to_le_bytes());
    out.extend_from_slice(&(file.len() as u32).to_le_bytes());
    out.extend_from_slice(&file.detector_id.to_le_bytes());
    let description = &description[..description.len().min(u16::MAX as usize)];
    out.extend_from_slice(&(description.len() as u16).to_le_bytes());
    out.extend_from_slice(description);
    for event in file.events() {
        out.extend_from_slice(&event.time.to_le_bytes());
        out.extend_from_slice(&event.channel.to_le_bytes());
    }
    Ok(out)
}

/// Reads a list-mode file.
pub fn read(bytes: &[u8]) -> Result<ListModeFile, FormatError> {
    let mut cursor = Cursor::new(bytes);
    let magic = cursor.take(8)?;
    if magic != MAGIC {
        return Err(FormatError::BadMagic {
            expected: "ORTSLIST".into(),
            found: String::from_utf8_lossy(magic).to_string(),
        });
    }
    let version = cursor.u16()?;
    if version != VERSION {
        return Err(FormatError::Unsupported {
            detail: format!("list-mode file version {version}"),
        });
    }
    cursor.skip(2)?;
    // Sizes histograms later (`to_spectrum`), so a corrupt field must not be
    // stored to abort there: ~4.29e9 channels is ~34 GB of zeros.
    let channel_count = cursor.u32()? as usize;
    if channel_count > crate::MOST_CHANNELS {
        return Err(FormatError::invalid(
            "channel count",
            channel_count.to_string(),
        ));
    }
    let live_time = cursor.f64()?;
    let real_time = cursor.f64()?;
    let event_count = cursor.u32()? as usize;
    let detector_id = cursor.u16()?;
    let description_len = cursor.u16()? as usize;
    let description = String::from_utf8_lossy(cursor.take(description_len)?).to_string();

    let mut file = ListModeFile::new(channel_count);
    file.live_time = live_time;
    file.real_time = real_time;
    file.detector_id = detector_id;
    file.sample_description = description;
    for _ in 0..event_count {
        let time = cursor.f64()?;
        let channel = cursor.u32()?;
        file.push(time, channel);
    }
    Ok(file)
}
