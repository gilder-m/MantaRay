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
    let lib = NuclideLibrary::standard();
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
    let lib = NuclideLibrary::standard();
    let cs = lib.nuclide("Cs-137").unwrap();
    let text = cs.half_life_display();
    assert!(text.contains("y"), "expected years, got {text}");
}
