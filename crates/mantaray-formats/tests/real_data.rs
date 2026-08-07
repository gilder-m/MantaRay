//! End-to-end checks against genuine instrument files.
//!
//! These run only when the matching fixture is present in `tests/fixtures`
//! (see the README there), so a clean checkout skips them.

use std::path::{Path, PathBuf};

use mantaray_core::{CalculationSettings, NuclideLibrary, Roi, mark_peaks, peak_info, peak_search};
use mantaray_formats::load_spectrum;

/// Lines every one of these sources must show, in keV.
const EXPECTED_LINES: &[(&str, &[f64])] = &[
    ("am241", &[59.541]),
    ("am-241", &[59.541]),
    ("cs137", &[661.657]),
    ("cs-137", &[661.657]),
    ("co-60", &[1173.228, 1332.492]),
    ("ba133", &[80.998, 356.017]),
    ("ba-133", &[80.998, 356.017]),
    ("eu152", &[121.782, 344.279, 1408.013]),
    // The Th-232 fixture stops at channel 6700 (about 2450 keV), so Tl-208's
    // 2614.5 keV line is outside the recorded range; these three chain lines are.
    ("th-232", &[238.632, 583.191, 911.204]),
];

fn fixtures() -> Vec<PathBuf> {
    let dir = Path::new(env!("CARGO_MANIFEST_DIR")).join("tests/fixtures");
    let Ok(entries) = std::fs::read_dir(dir) else {
        return Vec::new();
    };
    entries
        .flatten()
        .map(|entry| entry.path())
        .filter(|path| {
            path.extension()
                .and_then(|e| e.to_str())
                .is_some_and(|e| e.eq_ignore_ascii_case("spe"))
        })
        .collect()
}

fn expectation(path: &Path) -> Option<&'static [f64]> {
    let name = path.file_name()?.to_string_lossy().to_ascii_lowercase();
    EXPECTED_LINES
        .iter()
        .find(|(key, _)| name.contains(key))
        .map(|(_, lines)| *lines)
}

#[test]
fn peak_search_finds_the_expected_lines_in_real_spectra() {
    let mut checked = 0;
    for path in fixtures() {
        let Some(expected) = expectation(&path) else {
            continue;
        };
        let spectrum = load_spectrum(&path).unwrap();
        let Some(cal) = spectrum.energy_calibration.clone() else {
            continue;
        };
        let found = peak_search(&spectrum, &CalculationSettings::default());
        let energies: Vec<f64> = found.iter().map(|p| cal.energy(p.centroid)).collect();
        for line in expected {
            // Real files carry real calibrations: the Co-60 spectrum in this set
            // has a pure-gain calibration that is 0.2 % low at 1332 keV, putting
            // that line at 1329.7 keV. The tolerance therefore allows a small
            // proportional calibration error on top of the centroid resolution.
            let tolerance = 1.5 + 0.004 * line;
            assert!(
                energies.iter().any(|e| (e - line).abs() <= tolerance),
                "{}: no peak within {tolerance:.2} keV of {line} keV.\nfound: {:?}",
                path.file_name().unwrap().to_string_lossy(),
                energies
                    .iter()
                    .map(|e| format!("{e:.1}"))
                    .collect::<Vec<_>>()
            );
        }
        checked += 1;
    }
    if checked == 0 {
        eprintln!("no recognised source fixtures - skipping");
    } else {
        eprintln!("checked {checked} real spectra");
    }
}

#[test]
fn library_identification_names_the_source_in_real_spectra() {
    let library = NuclideLibrary::sample_for_tests();
    for path in fixtures() {
        let name = path
            .file_name()
            .unwrap()
            .to_string_lossy()
            .to_ascii_lowercase();
        // One clean, unambiguous case per nuclide.
        let expected = if name.starts_with("cs137_spectra") {
            "Cs-137"
        } else if name.starts_with("co-60-25cm") {
            "Co-60"
        } else if name.starts_with("ba133_spectra") {
            "Ba-133"
        } else {
            continue;
        };
        let mut spectrum = load_spectrum(&path).unwrap();
        let cal = spectrum.energy_calibration.clone().unwrap();
        mark_peaks(&mut spectrum, &CalculationSettings::default());
        assert!(!spectrum.rois.is_empty(), "{name}: no regions marked");

        let mut identified: Vec<String> = Vec::new();
        for roi in spectrum.rois.iter() {
            let Ok(info) = peak_info(&spectrum, *roi, &CalculationSettings::default()) else {
                continue;
            };
            if let Some(matched) = library.best_match(info.centroid_energy(&cal), 2.5) {
                identified.push(matched.nuclide.name.clone());
            }
        }
        assert!(
            identified.iter().any(|found| found == expected),
            "{name}: expected {expected} among {identified:?}"
        );
    }
}

#[test]
fn peak_info_on_a_real_cs137_peak_is_physically_sensible() {
    let Some(path) = fixtures().into_iter().find(|p| {
        p.file_name()
            .unwrap()
            .to_string_lossy()
            .eq_ignore_ascii_case("cs137_spectra.Spe")
    }) else {
        eprintln!("cs137_spectra.Spe not present - skipping");
        return;
    };
    let spectrum = load_spectrum(&path).unwrap();
    let cal = spectrum.energy_calibration.clone().unwrap();
    let shape = spectrum.shape_calibration.expect("a shape calibration");
    let channel = cal.channel(661.657).unwrap();
    let fwhm = shape.fwhm(channel);
    let roi = Roi::new(
        (channel - 3.0 * fwhm).max(0.0) as usize,
        (channel + 3.0 * fwhm) as usize,
    );
    let info = peak_info(&spectrum, roi, &CalculationSettings::default()).unwrap();

    // The centroid lands on the line.
    assert!(
        (info.centroid_energy(&cal) - 661.657).abs() < 1.5,
        "centroid {:.2} keV",
        info.centroid_energy(&cal)
    );
    // The measured width agrees with the stored shape calibration.
    assert!(
        (info.fwhm - fwhm).abs() < 0.35 * fwhm,
        "measured {:.2} channels against calibrated {fwhm:.2}",
        info.fwhm
    );
    // A real HPGe resolution at 662 keV is between 1 and 3 keV.
    let fwhm_kev = info.fwhm_energy(&cal);
    assert!(
        (1.0..3.0).contains(&fwhm_kev),
        "resolution {fwhm_kev:.2} keV is not HPGe-like"
    );
    // Net area is positive, smaller than gross, and well determined.
    assert!(info.net_area > 0.0);
    assert!(info.net_area < info.gross_area);
    assert!(
        info.net_uncertainty_percent() < 1.0,
        "uncertainty {:.2} %",
        info.net_uncertainty_percent()
    );
    eprintln!(
        "Cs-137 661.657 keV: centroid {:.2} keV, FWHM {:.2} keV, net {:.0} +/- {:.0} ({:.2} %)",
        info.centroid_energy(&cal),
        fwhm_kev,
        info.net_area,
        info.net_area_uncertainty,
        info.net_uncertainty_percent()
    );
}
