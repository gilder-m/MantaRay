//! ORTEC's own example files, read as they ship.
//!
//! GammaVision and MAESTRO ship a directory of sample data - spectra in three
//! formats, the calibrations and reports that go with them, and files from
//! instruments nobody here owns. They are the closest thing to an acceptance
//! test that exists: real files, written by the vendor, in the shapes the
//! vendor's own software produces.
//!
//! They are not redistributable, so the suite reads them from wherever they are
//! installed and **skips rather than fails** when they are not there. Point it
//! with `ORTSEAM_ORTEC_EXAMPLES`; the default is where the reference copy lives
//! on the machine this was written on.
//!
//! ```text
//! ORTSEAM_ORTEC_EXAMPLES="D:\V9 Examples" cargo test -p ortseam-formats --test ortec_examples
//! ```

use std::path::PathBuf;

/// Where the examples are, if they are anywhere.
fn examples() -> Option<PathBuf> {
    let directory = std::env::var("ORTSEAM_ORTEC_EXAMPLES")
        .map(PathBuf::from)
        .unwrap_or_else(|_| PathBuf::from(r"C:\Users\gilde\ORTEC-reference\V9 Examples"));
    directory.is_dir().then_some(directory)
}

/// Every spectrum file in the examples, by extension.
fn spectra(extensions: &[&str]) -> Vec<PathBuf> {
    let Some(directory) = examples() else {
        return Vec::new();
    };
    let mut found: Vec<PathBuf> = std::fs::read_dir(directory)
        .into_iter()
        .flatten()
        .flatten()
        .map(|entry| entry.path())
        .filter(|path| {
            path.extension()
                .and_then(|value| value.to_str())
                .map(|value| {
                    extensions
                        .iter()
                        .any(|wanted| value.eq_ignore_ascii_case(wanted))
                })
                .unwrap_or(false)
        })
        .collect();
    found.sort();
    found
}

/// Names the skip out loud, so a green run that tested nothing cannot be
/// mistaken for a green run that tested something.
fn skip_unless_present() -> bool {
    if examples().is_none() {
        eprintln!(
            "skipping: ORTEC's example files are not installed. \
             Set ORTSEAM_ORTEC_EXAMPLES to the V9 Examples directory to run this."
        );
        return true;
    }
    false
}

#[test]
fn every_example_spectrum_loads() {
    if skip_unless_present() {
        return;
    }
    let files = spectra(&["spc", "chn", "n42"]);
    assert!(
        files.len() >= 20,
        "the examples directory should hold the shipped spectra, found {}",
        files.len()
    );
    let mut failures = Vec::new();
    for path in &files {
        if let Err(error) = ortseam_formats::load_spectrum(path) {
            failures.push(format!("{}: {error}", path.display()));
        }
    }
    assert!(
        failures.is_empty(),
        "{} of {} example spectra would not load:\n  {}",
        failures.len(),
        files.len(),
        failures.join("\n  ")
    );
}

#[test]
fn every_example_spectrum_holds_counts_and_a_clock() {
    if skip_unless_present() {
        return;
    }
    for path in spectra(&["spc", "chn", "n42"]) {
        let spectrum = ortseam_formats::load_spectrum(&path).expect("it loaded a moment ago");
        let name = path.file_name().unwrap_or_default().to_string_lossy();
        assert!(
            !spectrum.channels.is_empty(),
            "{name} has no channels at all"
        );
        assert!(
            spectrum.total_counts() > 0,
            "{name} holds no counts, which no shipped example does"
        );
        assert!(
            spectrum.live_time >= 0.0 && spectrum.real_time >= 0.0,
            "{name} has a negative clock"
        );
        assert!(
            spectrum.live_time <= spectrum.real_time + 1e-6,
            "{name}: live time {} exceeds real time {}",
            spectrum.live_time,
            spectrum.real_time
        );
    }
}

#[test]
fn a_channel_count_is_never_a_strange_number() {
    if skip_unless_present() {
        return;
    }
    // Instrument memory comes in powers of two. A file that reads as 7027
    // channels is a decompression that went wrong, which is exactly the fault
    // this catches - the first N42 reader produced that number.
    for path in spectra(&["spc", "chn", "n42"]) {
        let spectrum = ortseam_formats::load_spectrum(&path).expect("it loaded");
        let length = spectrum.len();
        assert!(
            length.is_power_of_two(),
            "{}: {length} channels is not a power of two",
            path.file_name().unwrap_or_default().to_string_lossy()
        );
    }
}

#[test]
fn a_calibrated_example_puts_its_peaks_at_real_energies() {
    if skip_unless_present() {
        return;
    }
    // Reading a calibration is easy to get subtly wrong - the wrong one of two
    // blocks, coefficients in the wrong order - and the spectrum still looks
    // reasonable. What does not survive a mistake is a peak landing on a line
    // that exists. These are the strongest lines in the files named.
    let checks: &[(&str, f64, &str)] = &[
        ("NaI Source 97279 - 1024ch.N42", 661.66, "Cs-137"),
        ("NaI Source 97279 - 2048ch.N42", 661.66, "Cs-137"),
        ("NaI Source 97279 - 512ch.N42", 661.66, "Cs-137"),
        ("NaI StandFast Calibration.N42", 661.66, "Cs-137"),
        ("Detective-Remote.N42", 1460.82, "K-40"),
        ("Portal.n42", 1460.82, "K-40"),
    ];
    let Some(directory) = examples() else { return };
    for (name, energy, line) in checks {
        let path = directory.join(name);
        if !path.exists() {
            continue;
        }
        let mut spectrum = ortseam_formats::load_spectrum(&path).expect("it loads");
        let calibration = spectrum
            .energy_calibration
            .clone()
            .unwrap_or_else(|| panic!("{name} should carry a calibration"));
        let settings = ortseam_core::CalculationSettings::default();
        ortseam_core::mark_peaks(&mut spectrum, &settings);
        let near = spectrum
            .rois
            .iter()
            .filter_map(|roi| ortseam_core::peak_info(&spectrum, *roi, &settings).ok())
            .map(|info| calibration.energy(info.centroid))
            .any(|found| (found - energy).abs() < 3.0);
        assert!(
            near,
            "{name}: no peak within 3 keV of the {line} line at {energy} keV, \
             so the calibration is probably being read wrongly"
        );
    }
}

#[test]
fn an_spc_and_its_chn_agree_where_both_exist() {
    if skip_unless_present() {
        return;
    }
    // Several examples ship as both. The formats carry different amounts of
    // detail, but the counts are the counts.
    let Some(directory) = examples() else { return };
    let mut compared = 0;
    for path in spectra(&["spc"]) {
        let twin = path.with_extension("Chn");
        if !twin.exists() {
            continue;
        }
        let (Ok(spc), Ok(chn)) = (
            ortseam_formats::load_spectrum(&path),
            ortseam_formats::load_spectrum(&twin),
        ) else {
            continue;
        };
        let name = path.file_name().unwrap_or_default().to_string_lossy();
        assert_eq!(spc.len(), chn.len(), "{name}: different channel counts");
        assert_eq!(
            spc.total_counts(),
            chn.total_counts(),
            "{name}: the two formats disagree about the total"
        );
        compared += 1;
    }
    let _ = directory;
    eprintln!("compared {compared} spectra that ship in both formats");
}

#[test]
fn every_example_spectrum_survives_a_native_round_trip() {
    if skip_unless_present() {
        return;
    }
    // Whatever ortseam reads, it must be able to write and read back without
    // losing anything - that is what makes the native format worth having.
    let directory = std::env::temp_dir().join("ortseam-ortec-examples");
    std::fs::create_dir_all(&directory).expect("a scratch directory");
    for path in spectra(&["spc", "chn", "n42"]) {
        let original = ortseam_formats::load_spectrum(&path).expect("it loads");
        let name = path.file_name().unwrap_or_default().to_string_lossy();
        let out = directory.join(format!("{name}.json"));
        ortseam_formats::save_spectrum(&original, &out).expect("it writes");
        let again = ortseam_formats::load_spectrum(&out).expect("it reads back");
        assert_eq!(
            original.channels, again.channels,
            "{name}: channels changed"
        );
        assert_eq!(
            original.live_time, again.live_time,
            "{name}: live time changed"
        );
        assert_eq!(
            original.energy_calibration, again.energy_calibration,
            "{name}: calibration changed"
        );
    }
}

#[test]
fn every_shipped_binary_library_reads_whole_and_sane() {
    if skip_unless_present() {
        return;
    }
    // ORTEC's `.Lib` is binary, and a library silently read as empty or as
    // nonsense would quietly change what every analysis identifies. So every
    // shipped library must read, and what it reads must look like nuclides:
    // named, with lines at gamma energies and yields that are percentages.
    let Some(directory) = examples() else { return };
    let mut checked = 0;
    for entry in std::fs::read_dir(&directory)
        .into_iter()
        .flatten()
        .flatten()
    {
        let path = entry.path();
        if path
            .extension()
            .and_then(|value| value.to_str())
            .map(|value| value.eq_ignore_ascii_case("lib"))
            != Some(true)
        {
            continue;
        }
        checked += 1;
        let name = path.file_name().unwrap_or_default().to_string_lossy();
        let library = ortseam_formats::library::load(&path)
            .unwrap_or_else(|error| panic!("{name} would not read: {error}"));
        assert!(!library.is_empty(), "{name} read as an empty library");
        for nuclide in library.iter() {
            assert!(!nuclide.name.is_empty(), "{name}: a nameless nuclide");
            assert!(
                nuclide.half_life_seconds >= 0.0,
                "{name}: {} has a negative half-life",
                nuclide.name
            );
            for peak in &nuclide.peaks {
                assert!(
                    peak.energy > 0.0 && peak.energy < 20_000.0,
                    "{name}: {} has a line at {} keV, which is not gamma spectroscopy",
                    nuclide.name,
                    peak.energy
                );
                assert!(
                    peak.yield_percent >= 0.0 && peak.yield_percent <= 400.0,
                    "{name}: {} yields {} per hundred decays",
                    nuclide.name,
                    peak.yield_percent
                );
            }
        }
    }
    assert!(checked > 0, "the examples should include some libraries");
}

#[test]
fn the_shipped_suspect_library_reads_back_its_known_contents() {
    if skip_unless_present() {
        return;
    }
    // Suspect.Lib is GammaVision's stock identification library. Its contents
    // are physics, so a few well-known entries pin the field layout: get an
    // offset wrong anywhere and Co-60 stops being Co-60.
    let Some(directory) = examples() else { return };
    let path = directory.join("Suspect.Lib");
    if !path.exists() {
        return;
    }
    let library = ortseam_formats::library::load(&path).expect("Suspect.Lib reads");
    assert_eq!(library.len(), 99, "Suspect.Lib holds 99 nuclides");
    assert_eq!(library.name, "Suspect", "the library is named after the file");

    let co = library.nuclide("CO-60").expect("Co-60 is in it");
    let year = 365.25 * 86_400.0;
    assert!(
        (co.half_life_seconds / year - 5.27).abs() < 0.01,
        "Co-60's half-life reads as {} years",
        co.half_life_seconds / year
    );
    assert!(
        co.peaks
            .iter()
            .any(|p| (p.energy - 1332.5).abs() < 0.1 && (p.yield_percent - 99.99).abs() < 0.1),
        "Co-60's 1332 keV line is missing or misread"
    );

    let cs = library.nuclide("CS-137").expect("Cs-137 is in it");
    assert!(
        cs.peaks
            .iter()
            .any(|p| (p.energy - 661.66).abs() < 0.1
                && p.photon == ortseam_core::PhotonKind::Gamma),
        "Cs-137's 662 keV gamma is missing"
    );
    assert!(
        cs.peaks
            .iter()
            .any(|p| (p.energy - 32.194).abs() < 0.1
                && p.photon == ortseam_core::PhotonKind::XRay),
        "Cs-137's barium x-ray should read as an x-ray"
    );

    let na = library.nuclide("NA-22").expect("Na-22 is in it");
    assert!(
        na.peaks
            .iter()
            .any(|p| (p.energy - 511.0).abs() < 0.1
                && p.photon == ortseam_core::PhotonKind::Positron),
        "Na-22's annihilation line should read as positron decay"
    );
}
