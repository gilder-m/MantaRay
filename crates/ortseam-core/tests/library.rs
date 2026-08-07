//! Nuclide library model and lookup (MAESTRO §7.2, Services/Library).

use ortseam_core::{LibraryPeak, Nuclide, NuclideFlags, NuclideLibrary, PhotonKind};

fn library() -> NuclideLibrary {
    let mut lib = NuclideLibrary::new("test");
    lib.push(Nuclide {
        name: "Co-60".into(),
        half_life_seconds: 5.2711 * 365.25 * 86400.0,
        uncertainty_percent: 0.1,
        flags: NuclideFlags::default(),
        peaks: vec![
            LibraryPeak::new(1173.228, 99.85),
            LibraryPeak::new(1332.492, 99.9826),
        ],
    });
    lib.push(Nuclide {
        name: "Cs-137".into(),
        half_life_seconds: 30.08 * 365.25 * 86400.0,
        uncertainty_percent: 0.2,
        flags: NuclideFlags::default(),
        peaks: vec![LibraryPeak::new(661.657, 85.1)],
    });
    lib
}

#[test]
fn best_match_finds_the_nearest_line_within_tolerance() {
    let lib = library();
    let m = lib.best_match(1173.4, 1.0).expect("match");
    assert_eq!(m.nuclide.name, "Co-60");
    assert!((m.peak.energy - 1173.228).abs() < 1e-6);
    assert!((m.delta - 0.172).abs() < 1e-3);
}

#[test]
fn best_match_respects_the_tolerance() {
    let lib = library();
    assert!(lib.best_match(1173.4, 0.05).is_none());
    assert!(lib.best_match(500.0, 1.0).is_none());
}

#[test]
fn peaks_can_be_listed_in_an_energy_window() {
    let lib = library();
    let hits = lib.peaks_in(600.0, 1200.0);
    let names: Vec<&str> = hits.iter().map(|h| h.nuclide.name.as_str()).collect();
    assert_eq!(names, vec!["Cs-137", "Co-60"], "sorted by energy");
}

#[test]
fn nuclide_lookup_by_name_is_case_insensitive() {
    let lib = library();
    assert!(lib.nuclide("co-60").is_some());
    assert!(lib.nuclide("CS-137").is_some());
    assert!(lib.nuclide("Eu-152").is_none());
}

#[test]
fn editor_operations_insert_cut_and_move() {
    let mut lib = library();
    assert_eq!(lib.len(), 2);

    // Insert above the highlighted entry, as the library editor does.
    lib.insert(
        0,
        Nuclide {
            name: "Am-241".into(),
            half_life_seconds: 432.6 * 365.25 * 86400.0,
            uncertainty_percent: 0.5,
            flags: NuclideFlags::default(),
            peaks: vec![LibraryPeak::new(59.541, 35.9)],
        },
    );
    assert_eq!(lib.names(), vec!["Am-241", "Co-60", "Cs-137"]);

    let cut = lib.cut(0).expect("cut the first entry");
    assert_eq!(cut.name, "Am-241");
    assert_eq!(lib.names(), vec!["Co-60", "Cs-137"]);

    lib.push(cut);
    lib.sort_by_name();
    assert_eq!(lib.names(), vec!["Am-241", "Co-60", "Cs-137"]);

    lib.sort_by_energy();
    assert_eq!(
        lib.names(),
        vec!["Am-241", "Cs-137", "Co-60"],
        "sorted by first peak energy"
    );
}

#[test]
fn peak_flags_and_kinds_round_trip() {
    let mut peak = LibraryPeak::new(511.0, 180.0);
    peak.photon = PhotonKind::Positron;
    peak.key_line = true;
    peak.not_in_average = true;
    assert_eq!(peak.photon.code(), 'P');
    assert_eq!(PhotonKind::from_code('S'), Some(PhotonKind::SingleEscape));
    assert_eq!(PhotonKind::from_code('?'), None);

    let flags = NuclideFlags {
        fission_product: true,
        naturally_occurring: true,
        ..NuclideFlags::default()
    };
    // Flag order is T F I N P C M A, a dot meaning "not set".
    assert_eq!(flags.to_code_string(), "..IN....");
    assert_eq!(NuclideFlags::from_code_string("..IN...."), flags);
    assert_eq!(NuclideFlags::default().to_code_string(), "........");
}

#[test]
fn the_standard_library_covers_the_usual_calibration_nuclides() {
    let lib = NuclideLibrary::sample_for_tests();
    for n in [
        "Am-241", "Ba-133", "Co-57", "Co-60", "Cs-137", "Eu-152", "K-40",
    ] {
        assert!(
            lib.nuclide(n).is_some(),
            "{n} missing from the standard library"
        );
    }
    // Every nuclide must carry at least one line with a positive yield.
    for n in lib.iter() {
        assert!(!n.peaks.is_empty(), "{} has no peaks", n.name);
        assert!(
            n.peaks
                .iter()
                .all(|p| p.energy > 0.0 && p.yield_percent > 0.0)
        );
    }
    let m = lib.best_match(1332.5, 1.0).unwrap();
    assert_eq!(m.nuclide.name, "Co-60");
}

#[test]
fn half_life_is_formatted_in_human_units() {
    let lib = NuclideLibrary::sample_for_tests();
    let cs = lib.nuclide("Cs-137").unwrap();
    let text = cs.half_life_display();
    assert!(text.contains("y"), "expected years, got {text}");
}

/// However an operator writes a nuclide name, it should find the nuclide.
#[test]
fn a_name_is_read_however_it_is_written() {
    use ortseam_core::parse_nuclide_name as parse;
    for written in [
        "Na22", "22Na", "Na-22", "Na 22", "na22", "22na", "NA-22", "22-NA", " na 22 ", "Na_22",
    ] {
        assert_eq!(
            parse(written).as_deref(),
            Some("Na-22"),
            "{written:?} should read as Na-22"
        );
    }
    // A one-letter symbol, either way round.
    assert_eq!(parse("k40").as_deref(), Some("K-40"));
    assert_eq!(parse("40K").as_deref(), Some("K-40"));
}

/// The metastable suffix travels with the name, wherever it is written.
#[test]
fn a_metastable_state_keeps_its_suffix() {
    use ortseam_core::parse_nuclide_name as parse;
    for written in [
        "Ba137m", "Ba-137m", "137mBa", "137m-Ba", "ba 137 m", "BA-137M",
    ] {
        assert_eq!(
            parse(written).as_deref(),
            Some("Ba-137m"),
            "{written:?} should read as Ba-137m"
        );
    }
    assert_eq!(parse("Tc99m").as_deref(), Some("Tc-99m"));
    assert_eq!(parse("99mTc").as_deref(), Some("Tc-99m"));
    // A second isomer keeps its number.
    assert_eq!(parse("Ir194m2").as_deref(), Some("Ir-194m2"));
    assert_eq!(parse("194m2Ir").as_deref(), Some("Ir-194m2"));
}

/// `22mg` is Mg-22 to one reader and G-22m to another. The library decides.
#[test]
fn a_symbol_beginning_with_m_is_not_mistaken_for_a_metastable_state() {
    use ortseam_core::{LibraryPeak, Nuclide, NuclideLibrary, parse_nuclide_name as parse};

    // Read on its own, the element wins - it is much the likelier reading.
    assert_eq!(parse("22mg").as_deref(), Some("Mg-22"));
    assert_eq!(parse("22Mg").as_deref(), Some("Mg-22"));
    assert_eq!(parse("54mn").as_deref(), Some("Mn-54"));
    // And where the leading letters cannot be a symbol, the state reading is
    // the only one left.
    assert_eq!(parse("137mBa").as_deref(), Some("Ba-137m"));

    // Against a library, whichever reading it actually holds is the one found.
    let mut library = NuclideLibrary::new("test");
    library.push(Nuclide::new(
        "Mn-54",
        2.7e7,
        vec![LibraryPeak::new(834.8, 99.9)],
    ));
    library.push(Nuclide::new(
        "N-13m",
        1.0,
        vec![LibraryPeak::new(100.0, 50.0)],
    ));
    assert_eq!(
        library.find_typed("54mn").map(|n| n.name.as_str()),
        Some("Mn-54")
    );
    assert_eq!(
        library.find_typed("13mn").map(|n| n.name.as_str()),
        Some("N-13m")
    );
}

/// A typo is refused rather than turned into a lookup for something else.
#[test]
fn nonsense_is_not_read_as_a_nuclide() {
    use ortseam_core::parse_nuclide_name as parse;
    for written in [
        "", "  ", "22", "Na", "Cs137Ba", "Na-22-3", "Na*22", "Cs-13 7x", "Abc-137", "-", "137m",
        "Na-022", "Cs137q", "12Na34",
    ] {
        assert_eq!(
            parse(written),
            None,
            "{written:?} should not read as a nuclide"
        );
    }
}

/// The typed lookup goes through the parser, and still accepts an exact name.
#[test]
fn a_typed_name_finds_the_nuclide() {
    let library = ortseam_core::NuclideLibrary::sample_for_tests();
    let found = library.find_typed("cs137").expect("Cs-137 by any spelling");
    assert_eq!(found.name, "Cs-137");
    assert_eq!(
        library.find_typed("137-cs").map(|n| n.name.as_str()),
        Some("Cs-137")
    );
    assert!(library.find_typed("Xe-999").is_none());
    // A library whose names follow no convention is still searchable by
    // typing one of its names exactly.
    let mut odd = ortseam_core::NuclideLibrary::new("odd");
    odd.push(ortseam_core::Nuclide::new(
        "BACKGROUND",
        0.0,
        vec![ortseam_core::LibraryPeak::new(1460.8, 10.66)],
    ));
    assert!(odd.find_typed("background").is_some());
}
