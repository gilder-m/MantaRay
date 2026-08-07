//! Fixture-driven checks against real instrument files.
//!
//! Drop genuine `.Chn`, `.Spe`, `.Spc`, `.Roi` or `.Lis` files into
//! `crates/mantaray-formats/tests/fixtures/` and these tests will exercise the
//! readers against them. With an empty fixture directory the tests report what
//! they skipped and pass, so the suite stays green in a clean checkout.

use std::path::{Path, PathBuf};

use mantaray_formats::{SpectrumFormat, load_spectrum};

fn fixture_dir() -> PathBuf {
    Path::new(env!("CARGO_MANIFEST_DIR")).join("tests/fixtures")
}

fn fixtures() -> Vec<PathBuf> {
    let dir = fixture_dir();
    let Ok(entries) = std::fs::read_dir(&dir) else {
        return Vec::new();
    };
    entries
        .flatten()
        .map(|entry| entry.path())
        .filter(|path| path.is_file())
        .filter(|path| SpectrumFormat::from_path(path).is_some())
        .collect()
}

#[test]
fn every_fixture_spectrum_loads() {
    let files = fixtures();
    if files.is_empty() {
        eprintln!(
            "no fixtures in {} - skipping (see the README in that directory)",
            fixture_dir().display()
        );
        return;
    }
    for path in files {
        let spectrum = load_spectrum(&path).unwrap_or_else(|e| panic!("{}: {e}", path.display()));
        assert!(
            !spectrum.is_empty(),
            "{}: loaded zero channels",
            path.display()
        );
        assert!(
            spectrum.real_time >= spectrum.live_time - 0.05,
            "{}: live time {} exceeds real time {}",
            path.display(),
            spectrum.live_time,
            spectrum.real_time
        );
        eprintln!(
            "{}: {} channels, {:.2} s live, {} counts{}",
            path.file_name().unwrap().to_string_lossy(),
            spectrum.len(),
            spectrum.live_time,
            spectrum.total_counts(),
            match &spectrum.energy_calibration {
                Some(cal) => format!(", calibrated {:.4} {}/ch", cal.coefficients[1], cal.units),
                None => String::new(),
            }
        );
    }
}

#[test]
fn fixture_spectra_survive_a_native_round_trip() {
    let files = fixtures();
    if files.is_empty() {
        return;
    }
    for path in files {
        let original = load_spectrum(&path).unwrap();
        let text = mantaray_formats::native::write(&original).unwrap();
        let back = mantaray_formats::native::read(&text).unwrap();
        assert_eq!(back, original, "{}", path.display());
    }
}

#[test]
fn fixture_chn_files_survive_a_chn_round_trip() {
    for path in fixtures() {
        if SpectrumFormat::from_path(&path) != Some(SpectrumFormat::Chn) {
            continue;
        }
        let original = load_spectrum(&path).unwrap();
        let bytes = mantaray_formats::chn::write(&original).unwrap();
        let back = mantaray_formats::chn::read(&bytes).unwrap();
        assert_eq!(back.channels, original.channels, "{}", path.display());
        assert!(
            (back.live_time - original.live_time).abs() < 0.02,
            "{}",
            path.display()
        );
        assert_eq!(back.start_time, original.start_time, "{}", path.display());
    }
}
