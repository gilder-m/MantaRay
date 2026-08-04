//! Opening a spectrum whose name says nothing about its format.
//!
//! Real collections are full of `SPECTRUM.001`, `run3.dat` and files with no
//! extension at all, and refusing to read one on the strength of its name is the
//! difference between "recall works" and "recall does not work".

use std::path::{Path, PathBuf};

use ortseam_formats::{SpectrumFormat, load_spectrum, sniff};

/// A named file in the fixture directory.
fn fixture(name: &str) -> PathBuf {
    Path::new(env!("CARGO_MANIFEST_DIR"))
        .join("tests/fixtures")
        .join(name)
}

/// Whether a fixture is present, so a clean checkout still passes.
fn have(name: &str) -> bool {
    let there = fixture(name).is_file();
    if !there {
        eprintln!("fixture {name} is not present - skipping");
    }
    there
}

/// Copies a fixture to a scratch file with a different, meaningless name.
fn renamed(fixture_name: &str, new_name: &str) -> PathBuf {
    let source = fixture(fixture_name);
    let target = std::env::temp_dir()
        .join("ortseam-sniff-tests")
        .join(new_name);
    std::fs::create_dir_all(target.parent().expect("has a parent")).expect("scratch directory");
    std::fs::copy(&source, &target).expect("copy the fixture");
    target
}

#[test]
fn an_iaea_spectrum_is_recognised_by_its_contents() {
    if !have("eu152_spectra.Spe") {
        return;
    }
    let bytes = std::fs::read(fixture("eu152_spectra.Spe")).expect("read the fixture");
    assert_eq!(sniff(&bytes), Some(SpectrumFormat::Spe));
}

#[test]
fn an_ortec_binary_spectrum_is_recognised_by_its_contents() {
    if !have("Alcatraz14.Spc") {
        return;
    }
    let bytes = std::fs::read(fixture("Alcatraz14.Spc")).expect("read the fixture");
    assert_eq!(sniff(&bytes), Some(SpectrumFormat::Spc));
}

#[test]
fn a_chn_spectrum_is_recognised_by_its_leading_marker() {
    let spectrum = ortseam_core::Spectrum::new(256);
    let bytes = ortseam_formats::chn::write(&spectrum).expect("write .Chn");
    assert_eq!(sniff(&bytes), Some(SpectrumFormat::Chn));
}

#[test]
fn native_json_is_recognised_by_its_contents() {
    let spectrum = ortseam_core::Spectrum::new(16);
    let text = ortseam_formats::native::write(&spectrum).expect("write native");
    assert_eq!(sniff(text.as_bytes()), Some(SpectrumFormat::Native));
}

#[test]
fn an_ascii_channel_dump_is_recognised_by_its_columns() {
    let text = "0 12\n1 15\n2 19\n3 22\n";
    assert_eq!(sniff(text.as_bytes()), Some(SpectrumFormat::Ascii));
}

#[test]
fn a_list_mode_file_is_recognised_by_its_magic() {
    let file = ortseam_formats::ListModeFile::new(64);
    let bytes = ortseam_formats::list_mode::write(&file).expect("write list mode");
    assert_eq!(sniff(&bytes), Some(SpectrumFormat::List));
}

#[test]
fn prose_is_not_mistaken_for_a_spectrum() {
    assert_eq!(sniff(b"This is a report, not a spectrum.\n"), None);
    assert_eq!(sniff(&[]), None);
    assert_eq!(sniff(&[0xff; 64]), None);
}

#[test]
fn a_spectrum_with_a_meaningless_extension_still_opens() {
    for (source, name) in [
        ("eu152_spectra.Spe", "spectrum.001"),
        ("Alcatraz14.Spc", "binary.dat"),
    ] {
        if !have(source) {
            continue;
        }
        let path = renamed(source, name);
        let spectrum = load_spectrum(&path)
            .unwrap_or_else(|error| panic!("{name} should have opened, but {error}"));
        assert!(
            spectrum.total_counts() > 0,
            "{name} opened but held no counts"
        );
    }
}

#[test]
fn a_spectrum_with_no_extension_at_all_still_opens() {
    if !have("eu152_spectra.Spe") {
        return;
    }
    let path = renamed("eu152_spectra.Spe", "SPECTRUM");
    let spectrum = load_spectrum(&path).expect("a file with no extension should still be read");
    assert_eq!(spectrum.len(), 8192);
}

#[test]
fn the_extension_still_wins_when_it_is_meaningful() {
    // A `.Spe` file keeps being read as one even though sniffing would agree:
    // the point is that the extension path is not disturbed by the fallback.
    if !have("eu152_spectra.Spe") {
        return;
    }
    let path = fixture("eu152_spectra.Spe");
    assert_eq!(SpectrumFormat::from_path(&path), Some(SpectrumFormat::Spe));
    assert!(load_spectrum(&path).is_ok());
}

#[test]
fn an_unreadable_file_reports_what_was_wrong() {
    let target = std::env::temp_dir()
        .join("ortseam-sniff-tests")
        .join("nonsense.dat");
    std::fs::create_dir_all(target.parent().expect("has a parent")).expect("scratch directory");
    std::fs::write(&target, "this is not a spectrum at all").expect("write the file");
    let error = load_spectrum(&target).expect_err("nonsense should not load");
    let message = error.to_string();
    assert!(
        message.contains("dat") || message.to_lowercase().contains("format"),
        "unhelpful message: {message}"
    );
}
