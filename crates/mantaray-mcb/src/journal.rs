//! The bridge's half of the debug journal.
//!
//! With `MANTARAY_DEBUG` set - the application passes its environment down,
//! so setting it once covers both processes - the bridge writes what the
//! instrument actually said to `mantaray-mcb-debug.log` in the directory it
//! was started from: what each read asked for and what the driver returned,
//! and each served reply's shape. The application keeps the mirror's half in
//! `mantaray-debug.log`; a fault that only one side can see is named by
//! reading the two against each other.
//!
//! Without `MANTARAY_DEBUG` nothing is opened and nothing is written; call
//! sites format only behind [`on`].

use std::io::Write;
use std::sync::{Mutex, OnceLock};

/// The open journal, or `None` with `MANTARAY_DEBUG` unset. The file is
/// created fresh at the first line of the run.
static JOURNAL: OnceLock<Option<Mutex<std::fs::File>>> = OnceLock::new();

fn journal() -> Option<&'static Mutex<std::fs::File>> {
    JOURNAL
        .get_or_init(|| {
            std::env::var_os("MANTARAY_DEBUG")?;
            let mut file = std::fs::File::create("mantaray-mcb-debug.log").ok()?;
            let _ = writeln!(
                file,
                "mantaray-mcb {} · journal opened {}",
                env!("CARGO_PKG_VERSION"),
                chrono::Local::now().format("%Y-%m-%d %H:%M:%S")
            );
            Some(Mutex::new(file))
        })
        .as_ref()
}

/// Whether the journal is being kept - the guard a call site formats behind.
pub fn on() -> bool {
    journal().is_some()
}

/// Writes one timestamped line, flushed as it goes. A no-op without
/// `MANTARAY_DEBUG`.
pub fn line(text: &str) {
    if let Some(journal) = journal() {
        let mut file = journal
            .lock()
            .unwrap_or_else(|poisoned| poisoned.into_inner());
        let _ = writeln!(
            file,
            "{} {text}",
            chrono::Local::now().format("%H:%M:%S%.3f")
        );
        let _ = file.flush();
    }
}
