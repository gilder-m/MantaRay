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

use std::path::{Path, PathBuf};

use mantaray_core::{CalibrationTable, Spectrum};
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
    directory()
        .unwrap_or_else(std::env::temp_dir)
        .join("session.json")
}

/// The name this program stored things under before it was renamed.
pub(crate) const FORMER_NAME: &str = "ortseam";

/// Where settings and the snapshot live, carrying the old name's across once.
///
/// The rename would otherwise be silently destructive. Everything the operator
/// has chosen - the theme, the colours they tuned, the recent files, and any
/// snapshot of work a crash left behind - is keyed on the program's name, so a
/// renamed build looks at an empty directory and starts as though it had never
/// run. Losing a tuned palette is an annoyance; not finding a crash snapshot
/// means losing a count that exists nowhere else.
///
/// So the old directory's contents are moved across the first time, and only
/// into a directory that does not already hold them - a copy made after the
/// move must never be overwritten by a staler one.
pub fn directory() -> Option<PathBuf> {
    let current = eframe::storage_dir(crate::APPLICATION_ID)?;
    if let Some(former) = eframe::storage_dir(FORMER_NAME) {
        carry_across(&former, &current);
    }
    Some(current)
}

/// Moves what an earlier name left behind into the directory used now.
///
/// Split out from [`directory`] so it can be tested against real directories
/// without reaching for the environment: a migration that silently does
/// nothing looks exactly like one that worked, right up until somebody
/// notices their settings are gone.
///
/// Nothing already in `current` is overwritten. A file there is either newer
/// than the one being carried across or is the same file already moved, and
/// in both cases the older copy is the wrong one to keep.
pub(crate) fn carry_across(former: &Path, current: &Path) {
    if former == current || !former.is_dir() {
        return;
    }
    if std::fs::create_dir_all(current).is_err() {
        return;
    }
    for entry in std::fs::read_dir(former).into_iter().flatten().flatten() {
        let destination = current.join(entry.file_name());
        if destination.exists() {
            continue;
        }
        // A rename within one filesystem is atomic and costs nothing; the copy
        // is for the rare case the two sit on different ones.
        if std::fs::rename(entry.path(), &destination).is_err() {
            let _ = std::fs::copy(entry.path(), &destination);
        }
    }
    // The old directory is left in place rather than removed. An empty
    // directory costs nothing, and a half-finished move is recoverable where
    // a deletion is not.
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

    /// A rename must not cost the operator what the last name saved.
    #[test]
    fn what_the_former_name_saved_is_carried_across() {
        let root = std::env::temp_dir().join("mantaray-migration-test");
        let _ = std::fs::remove_dir_all(&root);
        let former = root.join("ortseam");
        let current = root.join("mantaray");
        std::fs::create_dir_all(&former).expect("the old directory");
        std::fs::write(former.join("app.ron"), "the tuned colours").expect("settings");
        std::fs::write(former.join("session.json"), "an unfinished count").expect("a snapshot");

        carry_across(&former, &current);

        // Both arrive, contents intact - the snapshot especially, because it
        // is the only copy of work a crash interrupted.
        assert_eq!(
            std::fs::read_to_string(current.join("app.ron")).expect("settings carried across"),
            "the tuned colours"
        );
        assert_eq!(
            std::fs::read_to_string(current.join("session.json")).expect("snapshot carried across"),
            "an unfinished count"
        );
        let _ = std::fs::remove_dir_all(&root);
    }

    /// Anything already saved under the new name is the copy to keep.
    #[test]
    fn a_newer_file_is_not_overwritten_by_the_one_being_carried_across() {
        let root = std::env::temp_dir().join("mantaray-migration-newer");
        let _ = std::fs::remove_dir_all(&root);
        let former = root.join("ortseam");
        let current = root.join("mantaray");
        std::fs::create_dir_all(&former).expect("the old directory");
        std::fs::create_dir_all(&current).expect("the new directory");
        std::fs::write(former.join("app.ron"), "stale").expect("old settings");
        std::fs::write(current.join("app.ron"), "current").expect("new settings");

        carry_across(&former, &current);

        assert_eq!(
            std::fs::read_to_string(current.join("app.ron")).expect("still there"),
            "current",
            "the older copy must not displace the one in use"
        );
        let _ = std::fs::remove_dir_all(&root);
    }

    /// Nothing to carry across, and nothing that goes wrong because of it.
    #[test]
    fn a_fresh_install_has_nothing_to_carry_and_does_not_mind() {
        let root = std::env::temp_dir().join("mantaray-migration-fresh");
        let _ = std::fs::remove_dir_all(&root);
        let current = root.join("mantaray");
        // The old directory never existed, which is every new installation.
        carry_across(&root.join("ortseam"), &current);
        assert!(
            !current.exists(),
            "no directory should be created for nothing"
        );
        // And the same path for both names is not a migration at all.
        std::fs::create_dir_all(&current).expect("a directory");
        std::fs::write(current.join("app.ron"), "settings").expect("settings");
        carry_across(&current, &current);
        assert_eq!(
            std::fs::read_to_string(current.join("app.ron")).expect("untouched"),
            "settings"
        );
        let _ = std::fs::remove_dir_all(&root);
    }
}
