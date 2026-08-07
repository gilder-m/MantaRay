//! Nuclide library files: ORTEC's binary `.Lib`, native JSON and a
//! spreadsheet-friendly CSV.

use ortseam_core::{LibraryPeak, Nuclide, NuclideFlags, NuclideLibrary, PhotonKind};
use ortseam_formats::library;

/// Writes word `number` (1-based, as the File Structure Manual counts) of
/// 128-byte record `record` (1-based, record 1 is the header).
fn set_word(bytes: &mut [u8], record: usize, number: usize, value: u16) {
    let at = (record - 1) * 128 + (number - 1) * 2;
    bytes[at..at + 2].copy_from_slice(&value.to_le_bytes());
}

/// Writes an R*4 starting at word `number` of record `record`.
fn set_real(bytes: &mut [u8], record: usize, number: usize, value: f32) {
    let at = (record - 1) * 128 + (number - 1) * 2;
    bytes[at..at + 4].copy_from_slice(&value.to_le_bytes());
}

/// Writes an eight-character space-padded name at word `number`.
fn set_name(bytes: &mut [u8], record: usize, number: usize, name: &str) {
    let at = (record - 1) * 128 + (number - 1) * 2;
    let padded = format!("{name:<8}");
    bytes[at..at + 8].copy_from_slice(&padded.as_bytes()[..8]);
}

/// A two-nuclide ORTEC binary library whose chains run against file order,
/// the way GammaVision leaves a library after edits: the nuclide chain starts
/// at the second stored record, and Co-60's peaks list 1332 keV before 1173.
///
/// Layout: record 1 header, record 2 nuclides (ordinals 1-3), record 3 peaks
/// (ordinals 1-4).
fn ortec_binary_library() -> Vec<u8> {
    let mut bytes = vec![0u8; 3 * 128];
    // Header.
    set_word(&mut bytes, 1, 1, 4); // LIBID
    set_word(&mut bytes, 1, 2, 0x4000); // sorted flag
    set_word(&mut bytes, 1, 3, 64); // RECLEN
    set_word(&mut bytes, 1, 4, 3); // RECTOT
    set_word(&mut bytes, 1, 5, 2);
    set_word(&mut bytes, 1, 9, 2); // ISOMAX
    set_word(&mut bytes, 1, 10, 21); // ISOLEN
    set_word(&mut bytes, 1, 11, 2); // ISOSTR: the chain starts at ordinal 2
    set_word(&mut bytes, 1, 12, 1); // ISOEND
    set_word(&mut bytes, 1, 14, 1);
    set_word(&mut bytes, 1, 15, 1);
    set_word(&mut bytes, 1, 16, 2); // ISOBAS
    set_word(&mut bytes, 1, 17, 4); // PKMAX
    set_word(&mut bytes, 1, 18, 16); // PKLEN
    set_word(&mut bytes, 1, 22, 2);
    set_word(&mut bytes, 1, 24, 3); // PKBAS
    set_word(&mut bytes, 1, 52, 2); // NUCUSE
    set_word(&mut bytes, 1, 53, 4); // PKUSE

    // Nuclide ordinal 1 (words 1-21 of record 2): Cs-137, second in the chain.
    set_name(&mut bytes, 2, 1, "CS-137");
    set_real(&mut bytes, 2, 5, 30.08); // half-life
    set_real(&mut bytes, 2, 7, 1.3); // uncertainty, percent
    set_word(&mut bytes, 2, 9, 5); // years
    set_word(&mut bytes, 2, 12, 0b0000_0100); // fission product
    set_word(&mut bytes, 2, 17, 1); // pointer to itself
    set_word(&mut bytes, 2, 19, 2); // first peak: ordinal 2
    set_word(&mut bytes, 2, 21, 0); // end of the nuclide chain

    // Nuclide ordinal 2 (words 22-42 of record 2): Co-60, FIRST in the chain.
    let word = |n: usize| 21 + n;
    set_name(&mut bytes, 2, word(1), "CO-60");
    set_real(&mut bytes, 2, word(5), 5.2711);
    set_real(&mut bytes, 2, word(7), 0.1);
    set_word(&mut bytes, 2, word(9), 5); // years
    set_word(&mut bytes, 2, word(12), 0b0000_0011); // thermal + fast neutron
    set_word(&mut bytes, 2, word(17), 2);
    set_word(&mut bytes, 2, word(19), 3); // first peak: ordinal 3
    set_word(&mut bytes, 2, word(21), 1); // then Cs-137

    // Peak ordinal 1 (words 1-16 of record 3): Co-60 1173, second in its chain.
    set_real(&mut bytes, 3, 1, 1173.228);
    set_real(&mut bytes, 3, 3, 99.85);
    set_word(&mut bytes, 3, 5, 0b10_0001); // gamma, key line
    set_word(&mut bytes, 3, 10, 2); // owner: Co-60
    set_word(&mut bytes, 3, 16, 0); // end of Co-60's peaks

    // Peak ordinal 2 (words 17-32): Cs-137 661.657.
    set_real(&mut bytes, 3, 17, 661.657);
    set_real(&mut bytes, 3, 19, 85.1);
    set_word(&mut bytes, 3, 21, 0b10_0001); // gamma, key line
    set_word(&mut bytes, 3, 26, 1);
    set_word(&mut bytes, 3, 32, 4); // then the x-ray

    // Peak ordinal 3 (words 33-48): Co-60 1332, FIRST in its chain.
    set_real(&mut bytes, 3, 33, 1332.492);
    set_real(&mut bytes, 3, 35, 99.9826);
    set_word(&mut bytes, 3, 37, 0b10_0001);
    set_word(&mut bytes, 3, 42, 2);
    set_word(&mut bytes, 3, 48, 1); // then 1173

    // Peak ordinal 4 (words 49-64): a Cs-137 Ba x-ray, kept out of the average.
    set_real(&mut bytes, 3, 49, 32.194);
    set_real(&mut bytes, 3, 51, 3.6);
    set_word(&mut bytes, 3, 53, 0b100_0010); // x-ray, not in average
    set_word(&mut bytes, 3, 58, 1);
    set_word(&mut bytes, 3, 64, 0);

    bytes
}

#[test]
fn ortec_binary_reads_by_chain_not_by_file_order() {
    let library = library::read_ortec(&ortec_binary_library()).unwrap();
    assert_eq!(
        library.names(),
        ["CO-60", "CS-137"],
        "the nuclide chain starts at the second stored record"
    );

    let co = library.nuclide("CO-60").unwrap();
    let year = 365.25 * 86_400.0;
    // The file stores the half-life as an f32, so compare in years, not seconds.
    assert!((co.half_life_seconds / year - 5.2711).abs() < 1e-4);
    assert!((co.uncertainty_percent - 0.1).abs() < 1e-6);
    assert!(co.flags.thermal_neutron_activation && co.flags.fast_neutron_activation);
    let energies: Vec<f64> = co.peaks.iter().map(|p| p.energy).collect();
    assert!(
        (energies[0] - 1332.492).abs() < 1e-3 && (energies[1] - 1173.228).abs() < 1e-3,
        "peaks follow the peak chain, which lists 1332 first: {energies:?}"
    );
    assert!(co.peaks.iter().all(|p| p.key_line));
    assert!((co.peaks[0].yield_percent - 99.9826).abs() < 1e-3);

    let cs = library.nuclide("CS-137").unwrap();
    assert!(cs.flags.fission_product);
    assert_eq!(cs.peaks.len(), 2);
    assert_eq!(cs.peaks[0].photon, PhotonKind::Gamma);
    assert_eq!(cs.peaks[1].photon, PhotonKind::XRay);
    assert!(cs.peaks[1].not_in_average && !cs.peaks[1].key_line);
}

#[test]
fn a_truncated_binary_library_is_an_error_not_a_panic() {
    let whole = ortec_binary_library();
    for keep in [0, 1, 100, 127, 128, 200, 300] {
        let cut = &whole[..keep];
        assert!(
            library::read_ortec(cut).is_err(),
            "{keep} bytes of a 384-byte library read as something"
        );
    }
}

#[test]
fn a_looping_chain_terminates() {
    // Point the two nuclides at each other, and a peak at itself.
    let mut bytes = ortec_binary_library();
    set_word(&mut bytes, 2, 21, 2); // Cs-137's fore pointer back to Co-60
    set_word(&mut bytes, 3, 48, 3); // the 1332 peak chains to itself
    let library = library::read_ortec(&bytes).unwrap();
    assert_eq!(
        library.names(),
        ["CO-60", "CS-137"],
        "each record is visited once, then the walk stops"
    );
    assert_eq!(library.nuclide("CO-60").unwrap().peaks.len(), 1);
}

#[test]
fn a_pbc_background_file_is_refused_by_its_name() {
    let mut bytes = ortec_binary_library();
    set_word(&mut bytes, 1, 1, 16); // LIBID 16 is the .PBC layout
    let error = library::read_ortec(&bytes).unwrap_err().to_string();
    assert!(
        error.contains("PBC"),
        "the refusal should say what the file is: {error}"
    );
}

#[test]
fn binary_and_text_libraries_load_through_the_same_door() {
    let dir = std::env::temp_dir().join(format!("ortseam-binlib-{}", std::process::id()));
    std::fs::create_dir_all(&dir).unwrap();

    let binary = dir.join("shipped.lib");
    std::fs::write(&binary, ortec_binary_library()).unwrap();
    let library = library::load(&binary).unwrap();
    assert_eq!(library.names(), ["CO-60", "CS-137"]);
    assert_eq!(library.name, "shipped", "the name comes from the file stem");

    // A JSON library renamed to .lib still loads as before.
    let renamed = dir.join("renamed.lib");
    library::save(&NuclideLibrary::sample_for_tests(), &renamed).unwrap();
    let text = library::load(&renamed).unwrap();
    assert_eq!(text.names(), NuclideLibrary::sample_for_tests().names());
}

#[test]
fn json_round_trips_the_standard_library() {
    let original = NuclideLibrary::sample_for_tests();
    let text = library::write_json(&original).unwrap();
    let back = library::read_json(&text).unwrap();
    assert_eq!(back, original);
}

#[test]
fn csv_round_trips_nuclides_peaks_and_flags() {
    let mut original = NuclideLibrary::new("csv");
    original.push(Nuclide {
        name: "Co-60".into(),
        half_life_seconds: 166_344_000.0,
        uncertainty_percent: 0.1,
        flags: NuclideFlags {
            thermal_neutron_activation: true,
            ..NuclideFlags::default()
        },
        peaks: vec![
            LibraryPeak::new(1173.228, 99.85).key(),
            LibraryPeak::new(1332.492, 99.9826).key(),
        ],
    });
    original.push(Nuclide {
        name: "Na-22".into(),
        half_life_seconds: 82_100_000.0,
        uncertainty_percent: 0.2,
        flags: NuclideFlags::default(),
        peaks: vec![LibraryPeak {
            photon: PhotonKind::Positron,
            not_in_average: true,
            ..LibraryPeak::new(511.0, 180.7)
        }],
    });

    let text = library::write_csv(&original);
    assert!(text.starts_with("nuclide,"), "a header row is written");
    let back = library::read_csv(&text).unwrap();

    assert_eq!(back.names(), original.names());
    let co = back.nuclide("Co-60").unwrap();
    assert_eq!(co.peaks.len(), 2);
    assert!(co.peaks.iter().all(|p| p.key_line));
    assert!(co.flags.thermal_neutron_activation);
    assert!((co.half_life_seconds - 166_344_000.0).abs() < 1.0);

    let na = back.nuclide("Na-22").unwrap();
    assert_eq!(na.peaks[0].photon, PhotonKind::Positron);
    assert!(na.peaks[0].not_in_average);
}

#[test]
fn csv_accepts_a_minimal_hand_written_table() {
    let text = "\
# nuclide, half life (s), energy, yield
Cs-137, 949252608, 661.657, 85.1
Cs-137, 949252608, 31.817, 1.99
K-40, 39390336000000000, 1460.822, 10.66
";
    let lib = library::read_csv(text).unwrap();
    assert_eq!(lib.len(), 2, "rows are grouped by nuclide");
    assert_eq!(lib.nuclide("Cs-137").unwrap().peaks.len(), 2);
    assert!((lib.nuclide("K-40").unwrap().peaks[0].energy - 1460.822).abs() < 1e-9);
}

#[test]
fn csv_rejects_rows_without_an_energy() {
    assert!(library::read_csv("Co-60\n").is_err());
    assert!(library::read_csv("").is_err());
}

#[test]
fn libraries_can_be_saved_and_loaded_by_extension() {
    let dir = std::env::temp_dir().join(format!("ortseam-lib-{}", std::process::id()));
    std::fs::create_dir_all(&dir).unwrap();
    let original = NuclideLibrary::sample_for_tests();
    for name in ["lib.json", "lib.csv"] {
        let path = dir.join(name);
        library::save(&original, &path).unwrap();
        let back = library::load(&path).unwrap();
        assert_eq!(back.names(), original.names(), "{name}");
        assert_eq!(
            back.nuclide("Co-60").unwrap().peaks.len(),
            2,
            "{name} lost peaks"
        );
    }
}
