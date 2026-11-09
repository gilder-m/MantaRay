//! The lab-flow faults found in the review before the first bench day
//! (swarm 2026-11-08: the GUI flow review S2-S4 and test gaps GUI-1, GUI-2,
//! GUI-5). Each test is a step a student takes at the bench.

use std::path::{Path, PathBuf};

use mantaray_core::Spectrum;
use mantaray_formats::SpectrumFormat;
use mantaray_gui::app::{Action, App, SAVE_FORMATS, save_path_for, writable_save_path};
use mantaray_gui::dialogs::Dialog;

fn scratch(name: &str) -> PathBuf {
    let dir = std::env::temp_dir().join(format!("mantaray-monday-{}-{name}", std::process::id()));
    let _ = std::fs::remove_dir_all(&dir);
    std::fs::create_dir_all(&dir).expect("a scratch directory");
    dir
}

fn spectrum() -> Spectrum {
    let mut spectrum = Spectrum::from_counts((0..64).map(|channel| 10 + channel % 7).collect());
    spectrum.live_time = 30.0;
    spectrum.real_time = 31.0;
    spectrum
}

/// A reopened file: a buffer window with a path, nothing changed yet.
fn reopened() -> App {
    let mut app = App::headless();
    app.open_buffer(
        "Co60.Spe".into(),
        spectrum(),
        Some(PathBuf::from("Co60.Spe")),
    );
    let index = app.active.expect("the buffer is active");
    assert!(!app.windows[index].modified, "a file just opened is not modified");
    app
}

fn active_modified(app: &App) -> bool {
    app.windows[app.active.expect("a window is active")].modified
}

// ------------------------------------------------- S3: unsaved regions, calibration

#[test]
fn marking_a_region_on_a_reopened_file_makes_it_unsaved() {
    let mut app = reopened();
    app.apply_action(Action::MarkRange(10, 20));
    assert!(active_modified(&app), "a new region is not saved anywhere yet");
}

#[test]
fn every_region_change_makes_the_window_unsaved() {
    for action in [
        Action::UnmarkRange(10, 20),
        Action::ClearRoi,
        Action::ClearAllRois,
        Action::PeakSearch,
    ] {
        let mut app = reopened();
        app.apply_action(action.clone());
        assert!(active_modified(&app), "{action:?} left the window unsaved-free");
    }
}

#[test]
fn closing_a_reopened_file_after_marking_a_region_asks_first() {
    let mut app = reopened();
    app.apply_action(Action::MarkRange(10, 20));
    let id = app.windows[app.active.unwrap()].id;
    app.apply_action(Action::CloseWindow(id));
    assert!(
        app.windows.iter().any(|window| window.id == id),
        "the window closed and took the region with it"
    );
    assert!(app.dialogs.is_open(Dialog::ConfirmClose));
}

#[test]
fn a_calibration_point_makes_the_window_unsaved() {
    let mut app = reopened();
    app.apply_action(Action::AddCalibrationPoint(661.7));
    assert!(active_modified(&app));
}

#[test]
fn destroying_the_calibration_makes_the_window_unsaved() {
    let mut app = reopened();
    app.apply_action(Action::DestroyCalibration);
    assert!(active_modified(&app));
}

#[test]
fn a_region_marked_on_a_detector_window_after_a_save_is_unsaved_again() {
    let dir = scratch("detector-roi");
    let mut app = App::headless();
    app.apply_action(Action::OpenDetector(0));
    app.save_active_to(&dir.join("count.Chn")).expect("the save");
    assert!(!active_modified(&app));
    app.apply_action(Action::MarkRange(5, 9));
    assert!(active_modified(&app), "Exit would not ask, and the region would be lost");
}

// --------------------------------------------- S4, GUI-5: the name decides the format

#[test]
fn a_name_with_a_dot_in_it_gets_the_default_extension() {
    assert_eq!(
        writable_save_path(Path::new("Co60_10.5cm"), "chn"),
        PathBuf::from("Co60_10.5cm.Chn")
    );
}

#[test]
fn a_name_with_no_extension_gets_the_default_extension() {
    assert_eq!(writable_save_path(Path::new("run1"), "spe"), PathBuf::from("run1.Spe"));
    assert_eq!(writable_save_path(Path::new("run1"), "json"), PathBuf::from("run1.json"));
}

#[test]
fn a_name_that_already_names_a_writer_is_left_alone() {
    for name in ["x.Chn", "x.spe", "x.Spc", "x.json", "x.txt", "dir.v2/x.CHN"] {
        assert_eq!(writable_save_path(Path::new(name), "spe"), PathBuf::from(name));
    }
}

#[test]
fn a_name_ending_in_a_format_with_no_writer_gets_one_that_has() {
    // .lis and .n42 open, but nothing writes them from a histogram.
    assert_eq!(writable_save_path(Path::new("x.lis"), "chn"), PathBuf::from("x.lis.Chn"));
    assert_eq!(writable_save_path(Path::new("x.n42"), "chn"), PathBuf::from("x.n42.Chn"));
}

#[test]
fn a_default_format_with_no_writer_falls_back_to_chn() {
    assert_eq!(writable_save_path(Path::new("run1"), "csv"), PathBuf::from("run1.Chn"));
}

#[test]
fn a_name_fixed_up_this_way_saves_and_reads_back() {
    let dir = scratch("fixed-name");
    for format in SAVE_FORMATS {
        let path = writable_save_path(&dir.join("Co60_10.5cm"), format);
        mantaray_formats::save_spectrum(&spectrum(), &path)
            .unwrap_or_else(|error| panic!("{} did not save: {error}", path.display()));
        let back = mantaray_formats::load_spectrum(&path)
            .unwrap_or_else(|error| panic!("{} did not load: {error}", path.display()));
        assert_eq!(back.channels, spectrum().channels, "{}", path.display());
    }
}

#[test]
fn an_added_extension_never_replaces_a_file_unasked() {
    // The dialog asked about "run1", not "run1.Chn".
    let dir = scratch("added-exists");
    std::fs::write(dir.join("run1.Chn"), b"the first run").expect("a file");
    let refusal = save_path_for(&dir.join("run1"), "chn").expect_err("an existing file");
    assert!(refusal.contains("run1.Chn"), "{refusal}");
    assert!(refusal.contains("already exists"), "{refusal}");
    // A name picked as it is was asked about by the dialog itself.
    assert_eq!(
        save_path_for(&dir.join("run1.Chn"), "chn"),
        Ok(dir.join("run1.Chn"))
    );
    assert_eq!(save_path_for(&dir.join("run2"), "chn"), Ok(dir.join("run2.Chn")));
}

#[test]
fn every_type_save_as_offers_has_a_writer() {
    let app = App::headless();
    let dir = scratch("filters");
    for (name, extensions) in app.save_filters() {
        for extension in extensions {
            let path = dir.join(format!("s.{extension}"));
            mantaray_formats::save_spectrum(&spectrum(), &path).unwrap_or_else(|error| {
                panic!("Save As offers {name} (.{extension}), which does not save: {error}")
            });
        }
    }
}

// ------------------------------------------------- GUI-1: the default-format setting

#[test]
fn every_default_format_the_settings_offer_has_a_writer() {
    let dir = scratch("defaults");
    for format in SAVE_FORMATS {
        let path = dir.join(format!("s.{format}"));
        assert!(
            SpectrumFormat::from_path(&path).is_some(),
            ".{format} is not a spectrum format"
        );
        mantaray_formats::save_spectrum(&spectrum(), &path)
            .unwrap_or_else(|error| panic!(".{format} is offered as a default and fails: {error}"));
    }
    assert!(!SAVE_FORMATS.contains(&"csv"), "no spectrum writer takes .csv");
}

#[test]
fn a_csv_default_left_in_old_settings_is_not_kept() {
    let mut persisted = App::headless().persisted();
    persisted.default_format = "csv".into();
    let mut app = App::headless();
    app.restore(persisted);
    assert!(
        SAVE_FORMATS.contains(&app.default_format.as_str()),
        "restored a default of .{}",
        app.default_format
    );
}

// ------------------------------------------------- GUI-2: a job's bare SAVE

fn run_job(app: &mut App, dir: &Path, text: &str) -> String {
    let path = dir.join("bare.job");
    std::fs::write(&path, text).expect("write the job");
    mantaray_gui::jobs::start(app, &path).expect("the job parses");
    for _ in 0..200 {
        mantaray_gui::jobs::step(app);
        if app.job.is_none() {
            break;
        }
        app.advance_by(0.5);
    }
    app.status.clone()
}

#[test]
fn a_bare_save_with_no_file_behind_the_window_is_an_error() {
    let dir = scratch("bare-save");
    let mut app = App::headless();
    app.apply_action(Action::OpenDetector(0));
    let status = run_job(&mut app, &dir, "SAVE\n");
    assert!(app.job.is_none(), "the job is still running: {status}");
    assert!(!status.contains("job finished"), "{status}");
    assert!(status.contains("SAVE"), "the error does not name SAVE: {status}");
}

#[test]
fn a_bare_save_to_a_file_with_no_writer_is_an_error() {
    let dir = scratch("bare-save-n42");
    let mut app = App::headless();
    app.open_buffer("x.n42".into(), spectrum(), Some(dir.join("x.n42")));
    let status = run_job(&mut app, &dir, "SAVE\n");
    assert!(!status.contains("job finished"), "{status}");
    assert!(!dir.join("x.n42").exists(), "something was written");
}

#[test]
fn a_bare_save_after_a_named_one_writes_the_same_file_again() {
    let dir = scratch("bare-save-again");
    let mut app = App::headless();
    app.apply_action(Action::OpenDetector(0));
    let status = run_job(&mut app, &dir, "SAVE \"first.Chn\"\nSAVE\n");
    assert!(status.contains("job finished"), "{status}");
    assert!(dir.join("first.Chn").exists());
}
