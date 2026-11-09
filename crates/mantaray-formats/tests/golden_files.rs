//! Files saved by released versions of mantaray, which every later version
//! must still open exactly.
//!
//! `golden/v0.2.4/` was written by the code at the `v0.2.4-alpha` tag, from a
//! test added to a copy of that tree for the purpose (the content is a made-up
//! Cs-137 count and a three-nuclide library, not real lab data). These files
//! are never regenerated or edited: a release that changes a format adds a new
//! directory beside them, and the old ones keep proving the old files open.
//!
//! Each format is checked four ways: the released file reads back field for
//! field; a field this version does not know is ignored; a field added after
//! the file was written may be missing; and a file from a newer format version
//! is refused with a message that names both versions, never guessed at.

use mantaray_core::{AcquisitionMode, PhotonKind, RoiSet};
use mantaray_formats::{FormatError, library, native, roi};
use serde_json::Value;

const SPECTRUM: &str = include_str!("golden/v0.2.4/spectrum.json");
const LIBRARY: &str = include_str!("golden/v0.2.4/library.json");
const REGIONS: &[u8] = include_bytes!("golden/v0.2.4/regions.roi");

// `golden/v0.1.0/` was written by the code at `v0.1.0-alpha`, when the program
// was called ortseam (v0.1.1-alpha's formats are the same code). Same content
// as the v0.2.4 files. Its spectrum's tag sits under the key `ortseam`; 0.1
// wrote no crash snapshot.
const SPECTRUM_0_1: &str = include_str!("golden/v0.1.0/spectrum.json");
const LIBRARY_0_1: &str = include_str!("golden/v0.1.0/library.json");

#[test]
fn a_v0_1_spectrum_from_ortseam_reads_back_field_for_field() {
    let old = native::read(SPECTRUM_0_1).expect("the ortseam 0.1 spectrum opens");
    // 0.1's spectrum layout is 0.2.4's: only the tag's key differs.
    assert_eq!(old, native::read(SPECTRUM).expect("golden"));
    let again = json(&native::write(&old).expect("writes"));
    assert_eq!(again["mantaray"], "spectrum/1", "written under today's key");
    assert!(again.get("ortseam").is_none());
    let mut expected = json(SPECTRUM_0_1);
    object(&mut expected).remove("ortseam");
    assert_kept(&expected, &again, "spectrum file");
}

#[test]
fn an_ortseam_spectrum_from_a_newer_format_is_still_refused() {
    let text = SPECTRUM_0_1.replace("\"spectrum/1\"", "\"spectrum/2\"");
    let message = native::read(&text).expect_err("spectrum/2").to_string();
    assert!(message.contains("spectrum/2"), "{message}");
}

#[test]
fn a_v0_1_library_from_ortseam_reads_back_field_for_field() {
    let old = library::read_json(LIBRARY_0_1).expect("the ortseam 0.1 library opens");
    assert_eq!(old, library::read_json(LIBRARY).expect("golden"));
    let again = library::write_json(&old).expect("writes");
    assert_kept(&json(LIBRARY_0_1), &json(&again), "library file");
}

/// Every field of `old` is in `new` with the same value. `new` may carry
/// more: a later version adds fields, but must never drop or change one.
fn assert_kept(old: &Value, new: &Value, at: &str) {
    match (old, new) {
        (Value::Object(old), Value::Object(new)) => {
            for (key, value) in old {
                let Some(other) = new.get(key) else {
                    panic!("{at}.{key} was in the old file and is gone");
                };
                assert_kept(value, other, &format!("{at}.{key}"));
            }
        }
        (Value::Array(old), Value::Array(new)) => {
            assert_eq!(old.len(), new.len(), "{at}: length changed");
            for (index, (a, b)) in old.iter().zip(new).enumerate() {
                assert_kept(a, b, &format!("{at}[{index}]"));
            }
        }
        _ => assert_eq!(old, new, "{at} changed"),
    }
}

fn json(text: &str) -> Value {
    serde_json::from_str(text).expect("valid JSON")
}

/// Edits a JSON document as a value and writes it back out.
fn edited(text: &str, edit: impl FnOnce(&mut Value)) -> String {
    let mut value = json(text);
    edit(&mut value);
    serde_json::to_string_pretty(&value).expect("JSON")
}

fn object(value: &mut Value) -> &mut serde_json::Map<String, Value> {
    value.as_object_mut().expect("an object")
}

// --- native spectrum (`.json`) ---

#[test]
fn a_v0_2_4_spectrum_reads_back_field_for_field() {
    let spectrum = native::read(SPECTRUM).expect("the v0.2.4 file opens");
    assert_eq!(spectrum.len(), 512);
    assert_eq!(spectrum.channels[0], 4012);
    assert_eq!(spectrum.channels[331], 2661);
    assert_eq!(spectrum.channels.iter().sum::<u64>(), 185_715);
    assert_eq!(spectrum.live_time, 1798.42);
    assert_eq!(spectrum.real_time, 1800.0);
    assert_eq!(
        spectrum.start_time.map(|time| time.to_string()).as_deref(),
        Some("2026-10-14 09:31:05")
    );
    assert_eq!(spectrum.detector_id, 1);
    assert_eq!(spectrum.detector_name, "DET-01");
    assert_eq!(
        spectrum.detector_description,
        "3x3 NaI(Tl), shelf 2, lead castle"
    );
    assert_eq!(
        spectrum.sample_description,
        "Cs-137 check source S/N 4417, 10 cm"
    );
    assert_eq!(spectrum.segment, 1);
    assert_eq!(spectrum.channel_offset, 0);
    assert_eq!(spectrum.mode, AcquisitionMode::Pha);
    let energy = spectrum.energy_calibration.as_ref().expect("calibrated");
    assert_eq!(energy.coefficients, [0.21, 1.9985, 1.2e-6]);
    assert_eq!(energy.units, "keV");
    let shape = spectrum.shape_calibration.expect("shape calibrated");
    assert_eq!(shape.coefficients, [8.4, 0.0215, -3.1e-6]);
    assert_eq!(spectrum.rois.to_pairs(), vec![(150, 229), (318, 344)]);
    assert_eq!(
        spectrum.source.as_deref(),
        Some("C:/Lab/2026-10-14/cs137_shelf2.Spe")
    );
    // And nothing the file holds is lost on the way back out.
    let again = native::write(&spectrum).expect("writes");
    assert_kept(&json(SPECTRUM), &json(&again), "spectrum file");
}

#[test]
fn a_spectrum_with_fields_this_version_does_not_know_still_reads() {
    let text = edited(SPECTRUM, |value| {
        object(value).insert("added_later".into(), Value::from("anything"));
        let spectrum = object(value.get_mut("spectrum").expect("spectrum"));
        spectrum.insert("dead_time_model".into(), Value::from(3));
        let energy = object(spectrum.get_mut("energy_calibration").expect("energy"));
        energy.insert("uncertainty".into(), serde_json::json!([0.01, 0.0001, 0.0]));
    });
    let spectrum = native::read(&text).expect("unknown fields are ignored");
    assert_eq!(spectrum, native::read(SPECTRUM).expect("golden"));
}

#[test]
fn a_spectrum_missing_its_descriptive_fields_reads_with_defaults() {
    // Everything but the measurement itself: what a file from a version
    // without some later field looks like, for every such field at once.
    let text = edited(SPECTRUM, |value| {
        let spectrum = object(value.get_mut("spectrum").expect("spectrum"));
        spectrum.retain(|key, _| matches!(key.as_str(), "channels" | "live_time" | "real_time"));
    });
    let spectrum = native::read(&text).expect("missing descriptive fields default");
    assert_eq!(spectrum.channels[331], 2661);
    assert_eq!(spectrum.live_time, 1798.42);
    assert_eq!(spectrum.real_time, 1800.0);
    assert_eq!(spectrum.start_time, None);
    assert_eq!(spectrum.detector_id, 0);
    assert_eq!(spectrum.detector_name, "");
    assert_eq!(spectrum.segment, 0);
    assert_eq!(spectrum.mode, AcquisitionMode::Pha);
    assert_eq!(spectrum.energy_calibration, None);
    assert!(spectrum.rois.is_empty());
    assert_eq!(spectrum.source, None);
}

#[test]
fn a_spectrum_missing_its_counts_or_times_is_refused() {
    // The measurement is never defaulted: a spectrum that silently reads as
    // empty, or as counted for zero seconds, is a wrong answer, not an old file.
    for field in ["channels", "live_time", "real_time"] {
        let text = edited(SPECTRUM, |value| {
            object(value.get_mut("spectrum").expect("spectrum")).remove(field);
        });
        assert!(native::read(&text).is_err(), "{field} missing was accepted");
    }
}

#[test]
fn a_spectrum_from_a_newer_format_is_refused_naming_both_versions() {
    let text = SPECTRUM.replace("\"spectrum/1\"", "\"spectrum/2\"");
    let error = native::read(&text).expect_err("a spectrum/2 file must not be guessed at");
    let message = error.to_string();
    assert!(message.contains("spectrum/2"), "{message}");
    assert!(message.contains("spectrum/1"), "{message}");
}

#[test]
fn a_newer_spectrum_names_the_version_that_wrote_it() {
    let text = edited(SPECTRUM, |value| {
        let document = object(value);
        document.insert("mantaray".into(), Value::from("spectrum/3"));
        document.insert("written_by".into(), Value::from("mantaray 9.1.0"));
    });
    let message = native::read(&text).expect_err("newer").to_string();
    assert!(message.contains("mantaray 9.1.0"), "{message}");
}

#[test]
fn a_spectrum_with_a_garbled_version_is_refused() {
    for tag in ["spectrum/", "spectrum/two", "spectrum/0", "spectrum/-1"] {
        let text = SPECTRUM.replace("\"spectrum/1\"", &format!("\"{tag}\""));
        assert!(native::read(&text).is_err(), "{tag} was accepted");
    }
}

#[test]
fn a_spectrum_written_now_carries_its_format_and_writer() {
    let spectrum = native::read(SPECTRUM).expect("golden");
    let value = json(&native::write(&spectrum).expect("writes"));
    assert_eq!(value["mantaray"], "spectrum/1");
    assert_eq!(
        value["written_by"],
        format!("mantaray {}", env!("CARGO_PKG_VERSION"))
    );
}

// --- nuclide library (`.json`, and `.lib` holding JSON) ---

#[test]
fn a_v0_2_4_library_reads_back_field_for_field() {
    let library = library::read_json(LIBRARY).expect("the v0.2.4 library opens");
    assert_eq!(library.name, "lab-check-sources");
    let names: Vec<&str> = library.iter().map(|n| n.name.as_str()).collect();
    assert_eq!(names, ["Am-241", "Cs-137", "Co-60"]);
    let cs = &library.nuclides[1];
    assert_eq!(cs.half_life_seconds, 9.4925e8);
    assert_eq!(cs.uncertainty_percent, 0.2);
    assert!(cs.flags.fission_product);
    assert!(!cs.flags.thermal_neutron_activation);
    assert_eq!(cs.peaks.len(), 2);
    assert_eq!(cs.peaks[0].energy, 32.19);
    assert_eq!(cs.peaks[0].photon, PhotonKind::XRay);
    assert!(cs.peaks[0].not_in_average);
    assert!(!cs.peaks[0].key_line);
    assert_eq!(cs.peaks[1].energy, 661.657);
    assert_eq!(cs.peaks[1].yield_percent, 85.1);
    assert!(cs.peaks[1].key_line);
    let co = &library.nuclides[2];
    assert!(co.flags.thermal_neutron_activation);
    assert_eq!(co.peaks[1].energy, 1332.492);
    assert_eq!(co.peaks[1].yield_percent, 99.9826);
    let again = library::write_json(&library).expect("writes");
    assert_kept(&json(LIBRARY), &json(&again), "library file");
}

#[test]
fn a_v0_2_4_library_opens_through_load_under_both_extensions() {
    let dir = std::env::temp_dir().join(format!("mantaray-golden-{}", std::process::id()));
    std::fs::create_dir_all(&dir).expect("scratch");
    let expected = library::read_json(LIBRARY).expect("golden");
    for name in ["lab-check-sources.json", "lab-check-sources.lib"] {
        let path = dir.join(name);
        std::fs::write(&path, LIBRARY).expect("write");
        assert_eq!(library::load(&path).expect(name), expected, "{name}");
    }
    let _ = std::fs::remove_dir_all(&dir);
}

#[test]
fn a_library_with_fields_this_version_does_not_know_still_reads() {
    let text = edited(LIBRARY, |value| {
        object(value).insert("evaluated_from".into(), Value::from("ENSDF 2026"));
        let nuclide = object(&mut value["nuclides"][1]);
        nuclide.insert("decay_mode".into(), Value::from("B-"));
        object(&mut nuclide["peaks"][1]).insert("yield_uncertainty".into(), Value::from(0.2));
    });
    assert_eq!(
        library::read_json(&text).expect("unknown fields are ignored"),
        library::read_json(LIBRARY).expect("golden")
    );
}

#[test]
fn a_library_missing_its_optional_fields_reads_with_defaults() {
    let text = edited(LIBRARY, |value| {
        for nuclide in value["nuclides"].as_array_mut().expect("nuclides") {
            let nuclide = object(nuclide);
            nuclide.remove("uncertainty_percent");
            nuclide.remove("flags");
            for peak in nuclide["peaks"].as_array_mut().expect("peaks") {
                let peak = object(peak);
                peak.remove("photon");
                peak.remove("key_line");
                peak.remove("not_in_average");
            }
        }
    });
    let library = library::read_json(&text).expect("missing optional fields default");
    let cs = &library.nuclides[1];
    assert_eq!(cs.half_life_seconds, 9.4925e8);
    assert_eq!(cs.uncertainty_percent, 0.0);
    assert!(!cs.flags.fission_product);
    assert_eq!(cs.peaks[1].energy, 661.657);
    assert_eq!(cs.peaks[1].yield_percent, 85.1);
    assert_eq!(cs.peaks[0].photon, PhotonKind::Gamma);
    assert!(!cs.peaks[1].key_line);
}

#[test]
fn a_library_missing_a_line_energy_or_yield_is_refused() {
    for field in ["energy", "yield_percent"] {
        let text = edited(LIBRARY, |value| {
            object(&mut value["nuclides"][0]["peaks"][0]).remove(field);
        });
        assert!(
            library::read_json(&text).is_err(),
            "{field} missing was accepted"
        );
    }
}

#[test]
fn a_library_from_a_newer_format_is_refused_naming_both_versions() {
    let text = edited(LIBRARY, |value| {
        object(value).insert("mantaray".into(), Value::from("library/2"));
    });
    let message = library::read_json(&text)
        .expect_err("library/2")
        .to_string();
    assert!(message.contains("library/2"), "{message}");
    assert!(message.contains("library/1"), "{message}");
    // Not mistaken for CSV either, when it arrives under the `.lib` name.
    let path = std::env::temp_dir().join(format!("mantaray-newer-{}.lib", std::process::id()));
    std::fs::write(&path, &text).expect("write");
    let message = library::load(&path)
        .expect_err("library/2 as .lib")
        .to_string();
    let _ = std::fs::remove_file(&path);
    assert!(message.contains("library/2"), "{message}");
}

#[test]
fn a_spectrum_file_offered_as_a_library_is_named_for_what_it_is() {
    let error = library::read_json(SPECTRUM).expect_err("not a library");
    assert!(matches!(error, FormatError::BadMagic { .. }), "{error}");
}

#[test]
fn a_library_written_now_carries_its_format_and_writer() {
    let library = library::read_json(LIBRARY).expect("golden");
    let value = json(&library::write_json(&library).expect("writes"));
    assert_eq!(value["mantaray"], "library/1");
    assert_eq!(
        value["written_by"],
        format!("mantaray {}", env!("CARGO_PKG_VERSION"))
    );
}

// --- regions (`.Roi`, ORTEC's binary table) ---
//
// ORTEC's layout has no room for a version or a stray field, and it is
// MAESTRO's format, not ours to extend; the golden file pins what mantaray
// reads and writes.

#[test]
fn a_v0_2_4_region_file_reads_back_exactly() {
    let set = roi::read(REGIONS).expect("the v0.2.4 region file opens");
    assert_eq!(set.to_pairs(), vec![(150, 229), (318, 344), (578, 596)]);
    assert_eq!(roi::write(&set), REGIONS, "written byte for byte as before");
    assert_eq!(
        set,
        RoiSet::from_pairs([(150, 229), (318, 344), (578, 596)])
    );
}
