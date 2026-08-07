//! Per-user Explorer file associations (Windows).
//!
//! Registration is nothing but values under `HKCU\Software\Classes` - no
//! administrator rights, no effect on other users, and reversible. The keys
//! are built here as plain data so tests can hold them to account; only the
//! menu action actually runs `reg.exe`.

/// The programmatic identifier the extensions point at.
pub const PROG_ID: &str = "mantaray.spectrum";

/// Extensions worth double-clicking.
pub const EXTENSIONS: [&str; 3] = [".spe", ".chn", ".spc"];

/// The keys registration writes: `(subkey under HKCU\Software\Classes,
/// default value)`.
pub fn registration(exe: &str) -> Vec<(String, String)> {
    let mut keys = vec![
        (PROG_ID.to_string(), "Gamma spectrum (mantaray)".to_string()),
        (format!("{PROG_ID}\\DefaultIcon"), format!("\"{exe}\",0")),
        (
            format!("{PROG_ID}\\shell\\open\\command"),
            format!("\"{exe}\" \"%1\""),
        ),
    ];
    for extension in EXTENSIONS {
        keys.push((extension.to_string(), PROG_ID.to_string()));
    }
    keys
}

/// What unregistration removes: the ProgID tree, and only the *default
/// value* of each extension key - another program's OpenWithProgIDs there
/// are none of mantaray's business.
pub fn removal() -> (String, Vec<String>) {
    (
        PROG_ID.to_string(),
        EXTENSIONS.iter().map(|e| e.to_string()).collect(),
    )
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn every_spectrum_extension_points_at_the_prog_id() {
        let keys = registration(r"C:\tools\mantaray-gui.exe");
        for extension in EXTENSIONS {
            assert!(
                keys.iter()
                    .any(|(key, value)| key == extension && value == PROG_ID),
                "{extension} should map to {PROG_ID}"
            );
        }
    }

    #[test]
    fn the_open_command_quotes_both_the_exe_and_the_file() {
        let keys = registration(r"C:\Program Files\mantaray\mantaray-gui.exe");
        let (_, command) = keys
            .iter()
            .find(|(key, _)| key.ends_with("shell\\open\\command"))
            .expect("an open command");
        assert_eq!(
            command, "\"C:\\Program Files\\mantaray\\mantaray-gui.exe\" \"%1\"",
            "spaces in the install path must survive"
        );
    }

    #[test]
    fn unregistration_never_deletes_a_whole_extension_key() {
        let (prog_id, extension_defaults) = removal();
        assert_eq!(prog_id, PROG_ID);
        assert_eq!(extension_defaults.len(), EXTENSIONS.len());
    }
}
