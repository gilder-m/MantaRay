//! The second-opinion review of the 0.2.5 release fixes (Lucius,
//! runs/release-0.2.5/review-fix-batches-lucius.md): the files and core
//! batches. Each test failed on audit/release-fix-save d445ada.

use mantaray_core::{LibraryPeak, Nuclide, NuclideLibrary, Spectrum};
use mantaray_formats::{chn, library, spc, spe};

/// Lucius f3: a `$DATA:` range at the edge of `usize` is refused, in a test
/// build (no overflow panic) and in a release build (no wrap to an empty
/// spectrum), as 59e3230's checked arithmetic did.
#[test]
fn a_range_ending_at_usize_max_is_refused_not_wrapped() {
    let text = format!("$SPEC_ID:\nx\n$DATA:\n0 {}\n1\n2\n", usize::MAX);
    let result = std::panic::catch_unwind(|| spe::read(&text)).expect("no overflow panic");
    assert!(
        result.is_err(),
        "opened as {:?} channels",
        result.map(|s| s.len())
    );
}

/// Lucius f3, the 0.2.4-offset branch: a huge first channel in a file that
/// names this program.
#[test]
fn a_0_2_4_looking_range_with_a_huge_first_channel_is_refused_not_wrapped() {
    let text = format!(
        "$SPEC_REM:\nAP# mantaray 0.2.4\n$DATA:\n{} 0\n7\n",
        usize::MAX
    );
    let result = std::panic::catch_unwind(|| spe::read(&text)).expect("no overflow panic");
    assert!(result.is_err());
}

/// The bytes 0.2.4 stored for a .Chn description: every byte of the UTF-8
/// text from 0x20 up, the first 63 (`d5b`).
fn chn_with_0_2_4_description(text: &str) -> Vec<u8> {
    let stored: Vec<u8> = text.bytes().filter(|b| *b >= 0x20).take(63).collect();
    let mut bytes = chn::write(&Spectrum::from_counts(vec![1; 4])).expect("write");
    let at = 32 + 4 * 4 + 320; // sample description: length byte, then 63 bytes
    bytes[at] = stored.len() as u8;
    bytes[at + 1..at + 1 + stored.len()].copy_from_slice(&stored);
    bytes
}

/// Lucius f2: a 0.2.4 .Chn description cut inside a UTF-8 character reads its
/// whole characters as UTF-8 (the partial one is dropped), not the whole field
/// as Windows-1252 mojibake, and a re-save stores the micro sign MAESTRO
/// reads, not the mojibake.
#[test]
fn a_0_2_4_description_cut_inside_a_character_keeps_its_micro_sign() {
    let full = format!("1 \u{b5}Ci {}\u{b5}Ci", "x".repeat(55));
    assert_eq!(
        full.as_bytes()[62],
        0xc2,
        "the cut falls inside the second micro sign"
    );
    let back = chn::read(&chn_with_0_2_4_description(&full))
        .expect("read")
        .sample_description;
    assert_eq!(back, format!("1 \u{b5}Ci {}", "x".repeat(55)));

    let mut spectrum = Spectrum::from_counts(vec![1; 4]);
    spectrum.sample_description = back;
    let saved = chn::write(&spectrum).expect("re-save");
    let at = 32 + 4 * 4 + 320;
    let field = &saved[at + 1..at + 1 + saved[at] as usize];
    assert_eq!(&field[..5], b"1 \xb5Ci", "Windows-1252 micro sign");
    assert!(!field.contains(&0xc2), "no mojibake written: {field:?}");
}

/// The guard the fix must keep: Windows-1252 text whose last byte happens to
/// be a UTF-8 lead byte ("café", E9) is still read as Windows-1252, not cut.
#[test]
fn a_windows_1252_description_ending_in_a_lead_byte_keeps_its_last_letter() {
    let mut bytes = chn::write(&Spectrum::from_counts(vec![1; 4])).expect("write");
    let at = 32 + 4 * 4 + 320;
    let stored = b"caf\xe9";
    bytes[at] = stored.len() as u8;
    bytes[at + 1..at + 1 + stored.len()].copy_from_slice(stored);
    let back = chn::read(&bytes).expect("read").sample_description;
    assert_eq!(back, "caf\u{e9}");
}

fn library_with(peak: LibraryPeak, half_life: f64) -> NuclideLibrary {
    let mut lib = NuclideLibrary::new("t");
    lib.push(Nuclide::new("Bad-1", half_life, vec![peak]));
    lib
}

/// Lucius c1: the JSON refusal for a non-finite line energy or yield does not
/// advise .csv, because the CSV reader refuses that line too; where it does
/// advise .csv (a half-life), the .csv opens again.
#[test]
fn the_json_refusal_advises_csv_only_where_the_csv_opens() {
    for (field, bad) in [
        ("energy", f64::NAN),
        ("energy", f64::INFINITY),
        ("yield", f64::NAN),
        ("yield", f64::NEG_INFINITY),
    ] {
        let peak = if field == "energy" {
            LibraryPeak::new(bad, 50.0)
        } else {
            LibraryPeak::new(661.657, bad)
        };
        let lib = library_with(peak, 100.0);
        let refusal = library::write_json(&lib)
            .expect_err("a non-finite line is refused as JSON")
            .to_string();
        assert!(library::read_csv(&library::write_csv(&lib)).is_err());
        assert!(!refusal.contains(".csv"), "{field} {bad}: {refusal}");
        assert!(refusal.contains("finite"), "{field} {bad}: {refusal}");
    }
    for half_life in [f64::INFINITY, f64::NAN] {
        let lib = library_with(LibraryPeak::new(661.657, 85.1), half_life);
        let refusal = library::write_json(&lib)
            .expect_err("a non-finite half-life is refused as JSON")
            .to_string();
        assert!(refusal.contains(".csv"), "{refusal}");
        assert!(
            library::read_csv(&library::write_csv(&lib)).is_ok(),
            "the .csv the refusal points to opens again"
        );
    }
}

/// Lucius c2: the .Spc ASCII time fallback (header times 0) refuses "inf"
/// and "NaN" as the header path does, rather than opening an infinite live
/// time.
#[test]
fn an_spc_ascii_time_that_is_not_finite_is_refused() {
    let mut spectrum = Spectrum::from_counts(vec![5; 1024]);
    spectrum.live_time = 10.0;
    spectrum.real_time = 12.0;
    // Record 3 is the acquisition record; live time ASCII at byte 38, real
    // time at byte 48, ten characters each.
    let acquisition = 2 * spc::RECORD_SIZE;
    for (ascii_at, text) in [
        (38usize, b"       inf"),
        (48usize, b"       NaN"),
        (38usize, b"      -inf"),
    ] {
        let mut bytes = spc::write(&spectrum).expect("write");
        bytes[90..98].copy_from_slice(&[0u8; 8]);
        bytes[acquisition + ascii_at..acquisition + ascii_at + 10].copy_from_slice(text);
        let read = spc::read(&bytes);
        assert!(
            read.is_err(),
            "ASCII field at {ascii_at}: read as {:?}",
            read.map(|s| (s.live_time, s.real_time))
        );
    }
    // A finite ASCII time is still the fallback.
    let mut bytes = spc::write(&spectrum).expect("write");
    bytes[90..98].copy_from_slice(&[0u8; 8]);
    let back = spc::read(&bytes).expect("finite ASCII times read");
    assert!((back.live_time - 10.0).abs() < 1e-6, "{}", back.live_time);
    assert!((back.real_time - 12.0).abs() < 1e-6, "{}", back.real_time);
}

/// A fresh, empty directory for one test.
fn scratch_dir(test: &str) -> std::path::PathBuf {
    let dir = std::env::temp_dir().join(format!(
        "mantaray-release-review-{test}-{}",
        std::process::id()
    ));
    let _ = std::fs::remove_dir_all(&dir);
    std::fs::create_dir_all(&dir).expect("a scratch directory");
    dir
}

/// Lucius a3: a file name of about 250 bytes, which an in-place save (0.2.4)
/// accepts, still saves: the temporary name is no longer 11+ bytes longer
/// than a name the filesystem allows.
#[test]
fn a_long_name_that_saved_before_still_saves() {
    let dir = scratch_dir("long-name");
    let path = dir.join(format!("{}.Spe", "x".repeat(246))); // 250 bytes
    if std::fs::write(&path, "old").is_err() {
        eprintln!("skipped: this filesystem refuses a 250-byte name outright");
        std::fs::remove_dir_all(&dir).ok();
        return;
    }
    mantaray_formats::write_atomic(&path, "new")
        .expect("a name the old in-place save accepted must still save");
    assert_eq!(std::fs::read_to_string(&path).expect("read"), "new");
    let left = std::fs::read_dir(&dir).expect("list").count();
    assert_eq!(left, 1, "no temporary file left");
    std::fs::remove_dir_all(&dir).ok();
}

/// Lucius a4: a link to a file that is not there yet is followed (the file is
/// made where the link points, as 0.2.4 did) and survives.
#[cfg(unix)]
#[test]
fn a_link_to_a_file_not_yet_there_is_followed_and_survives() {
    let dir = scratch_dir("dangling-link");
    std::fs::create_dir_all(dir.join("data")).expect("dirs");
    let real = dir.join("data").join("today.Chn");
    let link = dir.join("today.Chn");
    std::os::unix::fs::symlink(&real, &link).expect("link");
    // And a relative one, resolved from the link's own folder.
    let relative = dir.join("relative.Chn");
    std::os::unix::fs::symlink("data/relative.Chn", &relative).expect("relative link");

    mantaray_formats::write_atomic(&link, "new").expect("save");
    mantaray_formats::write_atomic(&relative, "rel").expect("save through a relative link");

    for path in [&link, &relative] {
        assert!(
            std::fs::symlink_metadata(path)
                .expect("meta")
                .file_type()
                .is_symlink(),
            "the link {} must survive",
            path.display()
        );
    }
    assert_eq!(
        std::fs::read_to_string(&real).expect("the file the link names"),
        "new"
    );
    assert_eq!(
        std::fs::read_to_string(dir.join("data").join("relative.Chn")).expect("relative"),
        "rel"
    );
    std::fs::remove_dir_all(&dir).ok();
}
