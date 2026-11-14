//! Where this program keeps its per-user files.
//!
//! The window learns the directory from eframe; the USB helper, which has no
//! window and no eframe, learns it here. The rule is eframe's
//! (`eframe::storage_dir`), and a test in the application holds the two to
//! the same answer.

use std::path::PathBuf;

/// What the operating system knows this program as: the name of its
/// settings directory.
pub const APPLICATION_ID: &str = "mantaray";

/// This program's settings directory: `$XDG_DATA_HOME/mantaray` or
/// `~/.local/share/mantaray` on Linux, `~/Library/Application Support/mantaray`
/// on macOS, `%APPDATA%\mantaray\data` on Windows.
pub fn directory() -> Option<PathBuf> {
    storage_dir(APPLICATION_ID)
}

/// The directory eframe keeps `app_id`'s files in.
pub fn storage_dir(app_id: &str) -> Option<PathBuf> {
    use std::env::{home_dir, var_os};
    if cfg!(windows) {
        var_os("APPDATA").map(|p| PathBuf::from(p).join(app_id).join("data"))
    } else if cfg!(target_os = "macos") {
        home_dir().map(|p| {
            p.join("Library")
                .join("Application Support")
                .join(app_id.replace(|c: char| c.is_ascii_whitespace(), "-"))
        })
    } else {
        var_os("XDG_DATA_HOME")
            .map(PathBuf::from)
            .filter(|p| p.is_absolute())
            .or_else(|| home_dir().map(|p| p.join(".local").join("share")))
            .map(|p| {
                p.join(
                    app_id
                        .to_lowercase()
                        .replace(|c: char| c.is_ascii_whitespace(), ""),
                )
            })
    }
}
