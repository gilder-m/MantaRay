//! The debug journal: what the instrument road did, written down.
//!
//! The render-diagnostics overlay says what a frame is doing; it cannot say
//! what the numbers underneath it did between frames. When a bench
//! misbehaves (a spectrum that flashes, a count that will not settle), the
//! question is always "what did the last fetch actually carry?", and the
//! only machine that can answer is the one misbehaving. So with
//! `MANTARAY_DEBUG` set, the
//! mirror writes what it does to `mantaray-debug.log` in the directory the
//! program was started from - or in the system's temporary directory when
//! that cannot be written, saying so on stderr, because a desktop launcher
//! starts a program at a directory nobody can write and a journal that
//! silently fails to open on the very machine being diagnosed is worse than
//! no journal feature at all. It records every fetch's declared count, its
//! sum, its clocks, and what the mirror did about it; every command and its
//! answer.
//! The bridge keeps its own journal (`mantaray-mcb-debug.log`) on the other
//! side of the pipe, so the two can be read against each other.
//!
//! Without `MANTARAY_DEBUG` nothing is opened and nothing is written; the
//! guard is [`on`], and call sites format only behind it, so the journal
//! costs nothing when it is not wanted.

use std::io::Write;
use std::sync::{Mutex, OnceLock};

/// The open journal, or `None` with `MANTARAY_DEBUG` unset. Decided once:
/// the file is created fresh at the first line of the run, so each run's
/// journal starts at its own beginning.
static JOURNAL: OnceLock<Option<Mutex<std::fs::File>>> = OnceLock::new();

fn journal() -> Option<&'static Mutex<std::fs::File>> {
    JOURNAL
        .get_or_init(|| {
            std::env::var_os("MANTARAY_DEBUG")?;
            let mut file = open_journal_file("mantaray-debug.log")?;
            let _ = writeln!(
                file,
                "mantaray {} · journal opened {}",
                env!("CARGO_PKG_VERSION"),
                chrono::Local::now().format("%Y-%m-%d %H:%M:%S")
            );
            Some(Mutex::new(file))
        })
        .as_ref()
}

/// The journal file, in the working directory or the temporary one.
///
/// The working directory is the documented home, so a run from a terminal
/// finds its journal beside it. But a program launched from the desktop
/// starts somewhere unwritable - `/`, on macOS - and the journal failing to
/// open *silently there*, on the machine being diagnosed, while the overlay
/// works, is the most confusing possible outcome. So it falls back to the
/// temporary directory and says so on stderr, and if even that refuses, says
/// that instead of nothing.
fn open_journal_file(name: &str) -> Option<std::fs::File> {
    if let Ok(file) = std::fs::File::create(name) {
        return Some(file);
    }
    let fallback = std::env::temp_dir().join(name);
    match std::fs::File::create(&fallback) {
        Ok(file) => {
            eprintln!(
                "journal: the working directory is not writable; writing {}",
                fallback.display()
            );
            Some(file)
        }
        Err(error) => {
            eprintln!("journal: could not open {name} anywhere: {error}");
            None
        }
    }
}

/// Whether the journal is being kept - the guard a call site formats behind.
pub fn on() -> bool {
    journal().is_some()
}

/// Writes one timestamped line. A no-op without `MANTARAY_DEBUG`.
///
/// Flushed as it is written: the journal exists for runs that end in a
/// window being force-closed, and a line held in a buffer then is a line
/// that was never written.
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
