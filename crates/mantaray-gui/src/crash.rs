//! A crash reporter: a panic writes what happened to a file worth sending.
//!
//! A desktop application that dies takes its console with it - on Windows
//! there is no console at all - so a panic must leave something behind. The
//! hook writes the panic, its location and a backtrace to a timestamped file
//! in the temporary directory, prints the path to standard error for whoever
//! can see it, and then lets the default hook run.

use std::path::PathBuf;

/// The report text: enough to reproduce without asking follow-up questions.
pub fn report_text(message: &str, location: &str, backtrace: &str) -> String {
    format!(
        "mantaray {} crashed.\n\
         \n\
         panic:    {message}\n\
         location: {location}\n\
         platform: {} / {}\n\
         \n\
         Please attach this file to a bug report.\n\
         \n\
         backtrace:\n{backtrace}\n",
        env!("CARGO_PKG_VERSION"),
        std::env::consts::OS,
        std::env::consts::ARCH,
    )
}

/// Where a crash report goes: the temporary directory, named by process id so
/// parallel instances cannot clobber each other.
pub fn report_path() -> PathBuf {
    std::env::temp_dir().join(format!("mantaray-crash-{}.txt", std::process::id()))
}

/// Installs the panic hook. Call once, before the event loop starts.
pub fn install() {
    let default_hook = std::panic::take_hook();
    std::panic::set_hook(Box::new(move |info| {
        let message = info
            .payload()
            .downcast_ref::<&str>()
            .map(|s| s.to_string())
            .or_else(|| info.payload().downcast_ref::<String>().cloned())
            .unwrap_or_else(|| "unknown panic".into());
        let location = info
            .location()
            .map(|l| l.to_string())
            .unwrap_or_else(|| "unknown location".into());
        let backtrace = std::backtrace::Backtrace::force_capture().to_string();
        let path = report_path();
        let text = report_text(&message, &location, &backtrace);
        if std::fs::write(&path, &text).is_ok() {
            eprintln!("crash report written to {}", path.display());
        }
        default_hook(info);
    }));
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn the_report_says_what_where_and_on_what() {
        let text = report_text(
            "index out of bounds",
            "crates/mantaray-gui/src/view.rs:123:9",
            "   0: rust_begin_unwind",
        );
        assert!(text.contains("index out of bounds"));
        assert!(text.contains("view.rs:123"));
        assert!(text.contains(env!("CARGO_PKG_VERSION")));
        assert!(text.contains(std::env::consts::OS));
        assert!(text.contains("rust_begin_unwind"));
        assert!(text.contains("bug report"));
    }

    #[test]
    fn the_report_path_is_per_process() {
        let path = report_path();
        assert!(path.to_string_lossy().contains("mantaray-crash-"));
        assert!(
            path.to_string_lossy()
                .contains(&std::process::id().to_string())
        );
    }

    #[test]
    fn a_panic_in_a_thread_leaves_the_report_behind() {
        install();
        let _ = std::thread::spawn(|| panic!("deliberate test panic"))
            .join()
            .expect_err("the thread must panic");
        let written = std::fs::read_to_string(report_path()).expect("the report file");
        assert!(written.contains("deliberate test panic"));
        let _ = std::fs::remove_file(report_path());
    }
}
