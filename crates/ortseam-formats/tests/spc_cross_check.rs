//! Cross-checks the `.Spc` reader against a `.Spe` conversion of the same
//! spectrum.
//!
//! Put `Alcatraz14.Spc` and `Alcatraz14.spe` (from LBNL's becquerel test
//! samples, or any other matching pair named `X.Spc` / `X.spe`) into
//! `tests/fixtures/` and this test compares the two readings field by field.

use std::path::{Path, PathBuf};

use ortseam_formats::{SpectrumFormat, load_spectrum, spc};

fn fixture_dir() -> PathBuf {
    Path::new(env!("CARGO_MANIFEST_DIR")).join("tests/fixtures")
}

/// Every `.Spc` file that has a `.Spe` file of the same stem beside it.
fn pairs() -> Vec<(PathBuf, PathBuf)> {
    let Ok(entries) = std::fs::read_dir(fixture_dir()) else {
        return Vec::new();
    };
    let files: Vec<PathBuf> = entries.flatten().map(|entry| entry.path()).collect();
    files
        .iter()
        .filter(|path| SpectrumFormat::from_path(path) == Some(SpectrumFormat::Spc))
        .filter_map(|spc_path| {
            let stem = spc_path.file_stem()?.to_string_lossy().to_ascii_lowercase();
            let twin = files.iter().find(|candidate| {
                SpectrumFormat::from_path(candidate) == Some(SpectrumFormat::Spe)
                    && candidate
                        .file_stem()
                        .map(|s| s.to_string_lossy().to_ascii_lowercase() == stem)
                        .unwrap_or(false)
            })?;
            Some((spc_path.clone(), twin.clone()))
        })
        .collect()
}

#[test]
fn spc_and_spe_readings_of_the_same_spectrum_agree() {
    let pairs = pairs();
    if pairs.is_empty() {
        eprintln!("no .Spc/.Spe pairs in fixtures - skipping");
        return;
    }
    for (spc_path, spe_path) in pairs {
        let from_spc = load_spectrum(&spc_path).unwrap();
        let from_spe = load_spectrum(&spe_path).unwrap();
        let name = spc_path.file_name().unwrap().to_string_lossy();

        assert_eq!(from_spc.len(), from_spe.len(), "{name}: channel count");
        assert_eq!(
            from_spc.channels, from_spe.channels,
            "{name}: channel data differs"
        );
        assert!(
            (from_spc.live_time - from_spe.live_time).abs() < 0.01,
            "{name}: live time {} against {}",
            from_spc.live_time,
            from_spe.live_time
        );
        assert!(
            (from_spc.real_time - from_spe.real_time).abs() < 0.01,
            "{name}: real time {} against {}",
            from_spc.real_time,
            from_spe.real_time
        );
        assert_eq!(
            from_spc.start_time, from_spe.start_time,
            "{name}: start time"
        );

        match (&from_spc.energy_calibration, &from_spe.energy_calibration) {
            (Some(a), Some(b)) => {
                for index in 0..3 {
                    let (left, right) = (a.coefficients[index], b.coefficients[index]);
                    let tolerance = 1e-4 * right.abs().max(1e-6);
                    assert!(
                        (left - right).abs() <= tolerance,
                        "{name}: calibration coefficient {index}: {left} against {right}"
                    );
                }
            }
            (a, b) => panic!("{name}: calibration present in .Spc {a:?} and .Spe {b:?}"),
        }
        assert!(
            from_spe
                .sample_description
                .to_lowercase()
                .contains(&from_spc.sample_description.to_lowercase()),
            "{name}: sample description {:?} against {:?}",
            from_spc.sample_description,
            from_spe.sample_description
        );

        eprintln!(
            "{name}: {} channels, {} counts, live {:.2} s, real {:.2} s, gain {:.6} keV/ch - matches its .Spe",
            from_spc.len(),
            from_spc.total_counts(),
            from_spc.live_time,
            from_spc.real_time,
            from_spc.energy_calibration.as_ref().unwrap().coefficients[1]
        );
    }
}

#[test]
fn fixture_spc_files_survive_a_write_and_reread() {
    let Ok(entries) = std::fs::read_dir(fixture_dir()) else {
        return;
    };
    for path in entries
        .flatten()
        .map(|entry| entry.path())
        .filter(|path| SpectrumFormat::from_path(path) == Some(SpectrumFormat::Spc))
    {
        let original = load_spectrum(&path).unwrap();
        let bytes = spc::write(&original).unwrap();
        let back = spc::read(&bytes).unwrap();
        let name = path.file_name().unwrap().to_string_lossy();
        assert_eq!(back.channels, original.channels, "{name}");
        assert!((back.live_time - original.live_time).abs() < 0.01, "{name}");
        assert_eq!(back.start_time, original.start_time, "{name}");
        assert_eq!(
            back.sample_description, original.sample_description,
            "{name}"
        );
    }
}
