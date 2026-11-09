//! A save that had to change the text it was given says so.

use mantaray_core::Spectrum;
use mantaray_formats::save_spectrum_noting;

fn scratch(name: &str) -> std::path::PathBuf {
    let dir = std::env::temp_dir().join(format!("mantaray-save-notes-{}", std::process::id()));
    std::fs::create_dir_all(&dir).expect("scratch dir");
    dir.join(name)
}

fn notes_for(spectrum: &Spectrum, name: &str) -> Vec<String> {
    let path = scratch(name);
    let notes = save_spectrum_noting(spectrum, &path).expect("save");
    std::fs::remove_file(&path).expect("clean");
    notes
}

#[test]
fn text_windows_1252_holds_saves_without_a_note() {
    let mut s = Spectrum::from_counts(vec![1; 4]);
    s.sample_description = "137Cs 1 \u{b5}Ci, 22 \u{b0}C".into();
    for name in ["ok.Chn", "ok.Spe", "ok.Spc", "ok.json"] {
        assert!(notes_for(&s, name).is_empty(), "{name}");
    }
}

#[test]
fn a_character_replaced_by_a_question_mark_is_named() {
    let mut s = Spectrum::from_counts(vec![1; 4]);
    s.sample_description = "\u{3b2} source".into();
    for name in ["beta.Chn", "beta.Spe", "beta.Spc"] {
        let notes = notes_for(&s, name);
        assert_eq!(notes.len(), 1, "{name}: {notes:?}");
        assert!(notes[0].contains("sample description"), "{notes:?}");
        assert!(notes[0].contains('\u{3b2}') && notes[0].contains("'?'"), "{notes:?}");
    }
    assert!(notes_for(&s, "beta.json").is_empty(), "native holds any text");
}

#[test]
fn a_description_cut_to_the_field_is_named() {
    let mut s = Spectrum::from_counts(vec![1; 4]);
    s.detector_description = "y".repeat(80);
    let notes = notes_for(&s, "long.Chn");
    assert_eq!(notes.len(), 1, "{notes:?}");
    assert!(notes[0].contains("detector description") && notes[0].contains("63"), "{notes:?}");
    assert!(notes_for(&s, "long.Spe").is_empty(), ".Spe has no limit");
}
