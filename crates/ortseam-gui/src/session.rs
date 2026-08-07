//! A snapshot of the work in progress, so an unexpected exit does not take it.
//!
//! Instrument data survives anything that happens here, because the MCB owns
//! its own memory. What only exists in this process is everything else: a
//! recalled file, a spectrum that has been smoothed or stripped, the buffer
//! recovered from a Clear, calibration points half entered. None of that is
//! anywhere on disk until somebody saves it, and nobody saves before a crash.
//!
//! The mechanism is deliberately dull. A snapshot is written every so often
//! while the application runs, and deleted when it exits cleanly. A snapshot
//! found at start-up therefore means the last run did not finish - the file's
//! existence *is* the crash flag - and its contents are offered back.

use std::path::PathBuf;

use ortseam_core::{CalibrationTable, Spectrum};
use serde::{Deserialize, Serialize};

use crate::viewmodel::DisplayState;

/// One buffer window, as it will come back.
#[derive(Clone, Debug, Serialize, Deserialize)]
pub struct SavedWindow {
    /// Window title, which is how the operator recognises it.
    pub title: String,
    /// Where it came from or was last saved, when it has been either.
    #[serde(default)]
    pub path: Option<PathBuf>,
    /// The data. This is the part that exists nowhere else.
    pub spectrum: Spectrum,
    /// What was on screen: zoom, marker, scale.
    #[serde(default)]
    pub display: DisplayState,
    /// Calibration points entered but not yet applied.
    #[serde(default)]
    pub calibration: CalibrationTable,
    /// Whether it held unsaved changes.
    #[serde(default)]
    pub modified: bool,
}

/// Everything worth bringing back from an unfinished run.
#[derive(Clone, Debug, Default, Serialize, Deserialize)]
pub struct Session {
    /// When it was taken, for the prompt to quote.
    #[serde(default)]
    pub saved_at: String,
    /// The buffer windows that were open.
    #[serde(default)]
    pub windows: Vec<SavedWindow>,
}

impl Session {
    /// Whether there is anything worth offering back.
    pub fn is_empty(&self) -> bool {
        self.windows.is_empty()
    }
}

/// Where the snapshot lives: beside the settings, so it travels with them.
///
/// The temporary directory is the fallback, because a snapshot somewhere is
/// worth more than no snapshot at all - and a crash is exactly when the
/// well-behaved path may be the one that failed.
pub fn path() -> PathBuf {
    eframe::storage_dir("ortseam")
        .unwrap_or_else(std::env::temp_dir)
        .join("session.json")
}

/// Writes a snapshot, replacing any earlier one.
///
/// Written to a neighbouring temporary file and renamed over the old one, so a
/// crash *during* the write cannot leave a half-written snapshot where a whole
/// one used to be - the failure this exists to survive must not be able to
/// destroy the thing that survives it.
pub fn write(session: &Session) -> std::io::Result<()> {
    let path = path();
    if let Some(parent) = path.parent() {
        std::fs::create_dir_all(parent)?;
    }
    let text = serde_json::to_string(session)
        .map_err(|error| std::io::Error::new(std::io::ErrorKind::InvalidData, error))?;
    let scratch = path.with_extension("json.part");
    std::fs::write(&scratch, text)?;
    std::fs::rename(&scratch, &path)
}

/// Reads the snapshot, if one was left behind.
///
/// An unreadable one is treated as none: a snapshot from an older version, or
/// a truncated file, is not worth an error message on start-up.
pub fn read() -> Option<Session> {
    let text = std::fs::read_to_string(path()).ok()?;
    serde_json::from_str(&text).ok()
}

/// Removes the snapshot, which is what a clean exit does.
pub fn clear() {
    let _ = std::fs::remove_file(path());
}

#[cfg(test)]
mod tests {
    use super::*;

    fn a_window(title: &str) -> SavedWindow {
        let mut spectrum = Spectrum::new(8);
        spectrum.channels[3] = 42;
        spectrum.live_time = 100.0;
        SavedWindow {
            title: title.into(),
            path: None,
            spectrum,
            display: DisplayState::for_length(8),
            calibration: CalibrationTable::default(),
            modified: true,
        }
    }

    #[test]
    fn a_session_survives_the_round_trip() {
        let session = Session {
            saved_at: "2026-08-06 21:14".into(),
            windows: vec![a_window("first.chn"), a_window("Recovered")],
        };
        let text = serde_json::to_string(&session).expect("write");
        let back: Session = serde_json::from_str(&text).expect("read");
        assert_eq!(back.windows.len(), 2);
        assert_eq!(back.windows[0].title, "first.chn");
        assert_eq!(back.windows[0].spectrum.channels[3], 42);
        assert!(back.windows[0].modified);
        assert_eq!(back.saved_at, "2026-08-06 21:14");
    }

    #[test]
    fn a_snapshot_from_another_version_is_treated_as_none() {
        // Not an error on start-up: an older or truncated snapshot is simply
        // nothing to offer, and saying so loudly would be worse than silence.
        let text = r#"{"saved_at":"then","windows":[{"title":"x"}]}"#;
        assert!(serde_json::from_str::<Session>(text).is_err());
    }

    #[test]
    fn an_empty_session_offers_nothing() {
        assert!(Session::default().is_empty());
        assert!(
            !Session {
                saved_at: String::new(),
                windows: vec![a_window("one")],
            }
            .is_empty()
        );
    }
}
