//! Opening spectra: File/Recall, the command line, and what the user sees after.
//!
//! These drive the real application state machine through [`App::apply_action`],
//! the same path the menus, the toolbar and JOB files take. The visibility
//! assertions matter as much as the loading ones: a spectrum that loads into a
//! window nothing draws is, to the person using the program, a spectrum that did
//! not open.

use std::path::{Path, PathBuf};

use mantaray_core::Spectrum;
use mantaray_device::Mcb;
use mantaray_gui::app::{Action, App, Target};

/// A scratch directory of this test's own.
fn scratch(name: &str) -> PathBuf {
    let dir = std::env::temp_dir().join("mantaray-recall-tests");
    std::fs::create_dir_all(&dir).expect("scratch directory");
    dir.join(name)
}

/// A synthetic spectrum with a peak in it, written to a file mantaray can read.
fn written_spectrum(name: &str, channels: usize) -> PathBuf {
    let mut spectrum = Spectrum::new(channels);
    spectrum.live_time = 300.0;
    spectrum.real_time = 310.0;
    for channel in 0..channels {
        spectrum.channels[channel] = 20;
    }
    // A Gaussian-ish peak, so peak search and peak info have something to find.
    let centre = channels / 2;
    for offset in -8i64..=8 {
        let channel = centre as i64 + offset;
        let value = (2000.0 * (-0.5 * (offset as f64 / 3.0).powi(2)).exp()) as u64;
        spectrum.channels[channel as usize] += value;
    }
    let path = scratch(name);
    mantaray_formats::save_spectrum(&spectrum, &path).expect("write the spectrum");
    path
}

/// The window the user is looking at, if any.
fn visible_titles(app: &App) -> Vec<String> {
    app.visible_windows()
        .into_iter()
        .map(|index| app.windows[index].title.clone())
        .collect()
}

#[test]
fn a_recalled_spectrum_opens_in_a_window_the_user_can_see() {
    let path = written_spectrum("recalled.chn", 1024);
    let mut app = App::headless();
    let before = app.windows.len();

    app.recall_path(path.clone());

    assert_eq!(app.windows.len(), before + 1, "no window was opened");
    let index = app.active.expect("the recalled window should be active");
    assert_eq!(app.windows[index].target, Target::Buffer);
    assert_eq!(app.windows[index].path.as_deref(), Some(path.as_path()));
    assert!(
        app.visible_windows().contains(&index),
        "the recalled window is not drawn: visible windows are {:?}",
        visible_titles(&app)
    );
    let spectrum = app.active_spectrum().expect("the window should hold data");
    assert_eq!(spectrum.len(), 1024);
    assert!(spectrum.total_counts() > 0);
}

#[test]
fn a_spectrum_recalled_while_one_window_fills_the_screen_is_the_one_shown() {
    // This is the bug this suite was written for: with a window maximised, a
    // recall used to load into a window that was never drawn, so recalling
    // appeared to do nothing at all.
    let path = written_spectrum("maximised.chn", 512);
    let mut app = App::headless();
    let first = app.windows.first().map(|window| window.id).expect(
        "the simulator detector window should be open at startup, so that the test starts \
         from the real initial state",
    );
    app.apply_action(Action::ToggleMaximize(first));
    assert_eq!(
        app.visible_windows().len(),
        1,
        "maximising should leave one window drawn"
    );

    app.recall_path(path);

    let index = app.active.expect("the recalled window should be active");
    assert_eq!(
        app.visible_windows(),
        vec![index],
        "the recalled spectrum should be the window on screen"
    );
    assert!(
        app.active_spectrum().is_some_and(|s| s.total_counts() > 0),
        "the visible window should be showing the recalled data"
    );
}

#[test]
fn the_command_line_leaves_the_named_spectrum_on_screen() {
    let path = written_spectrum("argument.spe", 2048);
    let mut app = App::headless();

    app.open_arguments(std::slice::from_ref(&path));
    app.settle_layout();

    let index = app.active.expect("the file should be active");
    assert!(app.visible_windows().contains(&index));
    assert_eq!(
        app.windows[index].title, "argument.spe",
        "the window should be named after the file"
    );
    assert_eq!(
        app.active_spectrum().map(|spectrum| spectrum.len()),
        Some(2048)
    );
}

#[test]
fn every_writable_format_can_be_recalled_after_being_saved() {
    // The formats mantaray can write are the ones a user will hand back to it.
    for extension in ["chn", "spe", "json", "txt"] {
        let path = written_spectrum(&format!("round-trip.{extension}"), 512);
        let mut app = App::headless();
        app.recall_path(path.clone());
        let spectrum = app
            .active_spectrum()
            .unwrap_or_else(|| panic!(".{extension} did not open: {}", app.status));
        assert_eq!(spectrum.len(), 512, ".{extension} lost its channel count");
        assert!(
            spectrum.total_counts() > 0,
            ".{extension} lost its counts: {}",
            app.status
        );
        assert!(
            app.status.contains("round-trip"),
            ".{extension} did not report what was opened: {}",
            app.status
        );
    }
}

#[test]
fn a_spectrum_whose_name_says_nothing_still_opens() {
    let source = written_spectrum("nameless-source.chn", 256);
    let target = scratch("NAMELESS");
    std::fs::copy(&source, &target).expect("copy without an extension");
    let mut app = App::headless();

    app.recall_path(target);

    assert!(
        app.active_spectrum().is_some_and(|s| s.len() == 256),
        "a file with no extension should still open: {}",
        app.status
    );
}

#[test]
fn a_file_that_is_not_a_spectrum_is_reported_and_changes_nothing() {
    let path = scratch("notes.txt");
    std::fs::write(&path, "Just some notes about the experiment.\n").expect("write the file");
    let mut app = App::headless();
    let before = app.windows.len();

    app.recall_path(path);

    assert_eq!(
        app.windows.len(),
        before,
        "a bad file should open no window"
    );
    assert!(
        app.status.to_lowercase().contains("could not read"),
        "the failure should be explained: {}",
        app.status
    );
}

#[test]
fn a_missing_file_is_reported_and_changes_nothing() {
    let mut app = App::headless();
    let before = app.windows.len();

    app.recall_path(PathBuf::from("no-such-directory/no-such-file.Spe"));

    assert_eq!(app.windows.len(), before);
    assert!(
        app.status.to_lowercase().contains("could not read"),
        "the failure should be explained: {}",
        app.status
    );
}

#[test]
fn several_spectra_can_be_open_at_once_and_each_can_be_brought_forward() {
    let first = written_spectrum("first.chn", 256);
    let second = written_spectrum("second.chn", 512);
    let mut app = App::headless();

    app.recall_path(first);
    app.recall_path(second);

    assert!(app.windows.len() >= 3, "detector plus two buffers");
    // Nothing is maximised, so every window with data is drawn.
    let visible = app.visible_windows();
    assert!(
        visible.len() >= 2,
        "both recalled windows should be drawn: {:?}",
        visible_titles(&app)
    );
    // Activating a window makes it the one commands apply to. Activate takes
    // the window's id, so a shifted list cannot focus the wrong window.
    let first_buffer = app
        .windows
        .iter()
        .position(|window| window.title == "first.chn")
        .expect("the first file should have a window");
    app.apply_action(Action::Activate(app.windows[first_buffer].id));
    assert_eq!(app.active, Some(first_buffer));
    assert_eq!(
        app.active_spectrum().map(|spectrum| spectrum.len()),
        Some(256)
    );
}

#[test]
fn closing_the_window_that_fills_the_screen_does_not_leave_the_area_blank() {
    let path = written_spectrum("closing.chn", 256);
    let mut app = App::headless();
    app.recall_path(path);
    let index = app.active.expect("active");
    let id = app.windows[index].id;
    app.apply_action(Action::ToggleMaximize(id));
    assert_eq!(app.visible_windows(), vec![index]);

    app.apply_action(Action::CloseWindow(id));

    assert!(
        !app.windows.is_empty(),
        "the detector window should still be there"
    );
    assert!(
        !app.visible_windows().is_empty(),
        "something must still be drawn after closing a maximised window"
    );
}

#[test]
fn a_recalled_spectrum_is_ready_for_the_commands_that_follow() {
    // Recall is only useful if what comes next works on the data: peak search,
    // peak info, regions and reports all read the active window.
    let path = written_spectrum("analysed.chn", 1024);
    let mut app = App::headless();
    app.recall_path(path);

    app.apply_action(Action::PeakSearch);
    let found = app
        .active_spectrum()
        .map(|spectrum| spectrum.rois.len())
        .unwrap_or(0);
    assert!(found > 0, "peak search found nothing: {}", app.status);

    app.apply_action(Action::Marker(512));
    app.apply_action(Action::PeakInfoAtMarker);
    let index = app.active.expect("active");
    let info = app.windows[index]
        .peak_info
        .as_ref()
        .unwrap_or_else(|| panic!("no peak information was produced: {}", app.status));
    assert!(
        (info.centroid - 512.0).abs() < 3.0,
        "the peak was found at {} rather than 512",
        info.centroid
    );
    assert!(info.net_area > 0.0);
}

#[test]
fn recalling_regions_and_a_calibration_applies_them_to_the_open_spectrum() {
    let path = written_spectrum("with-regions.chn", 512);
    let mut app = App::headless();
    app.recall_path(path);

    // Regions come from a `.Roi` file, which is written from a spectrum.
    let mut regions = mantaray_core::RoiSet::default();
    regions.mark(mantaray_core::Roi::new(240, 270));
    let roi_path = scratch("regions.roi");
    mantaray_formats::save_rois(&regions, &roi_path).expect("write the region file");

    // The dialog-free path, which is what a job and this test drive.
    app.apply_roi_file(&roi_path);
    assert_eq!(
        app.active_spectrum().map(|spectrum| spectrum.rois.len()),
        Some(1),
        "the region file was not applied: {}",
        app.status
    );
}

#[test]
fn the_file_a_window_came_from_is_remembered_for_saving() {
    let path = written_spectrum("remembered.chn", 256);
    let mut app = App::headless();
    app.recall_path(path.clone());
    let index = app.active.expect("active");
    assert_eq!(app.windows[index].path.as_deref(), Some(path.as_path()));
    assert!(
        !app.windows[index].modified,
        "a freshly opened file is not modified"
    );

    // A command that changes the data marks the window, so Save has something to
    // offer and the title can show it.
    app.apply_action(Action::Smooth);
    assert!(
        app.windows[index].modified,
        "smoothing should mark the window as modified"
    );
}

#[test]
fn recall_does_not_disturb_a_detector_that_is_counting() {
    let mut app = App::headless();
    app.apply_action(Action::Start);
    app.advance_by(2.0);
    let counting_before = app
        .detectors
        .first()
        .map(|detector| detector.spectrum().total_counts())
        .unwrap_or(0);
    assert!(counting_before > 0, "the simulator should have counted");

    let path = written_spectrum("during-count.chn", 256);
    app.recall_path(path);
    app.advance_by(2.0);

    let counting_after = app
        .detectors
        .first()
        .map(|detector| detector.spectrum().total_counts())
        .unwrap_or(0);
    assert!(
        counting_after > counting_before,
        "the detector should have kept counting through the recall"
    );
}

#[test]
fn recall_works_from_a_relative_path() {
    // A JOB file, or a shell, hands over a relative name.
    let path = written_spectrum("relative.chn", 128);
    let directory = path.parent().expect("has a parent").to_path_buf();
    let previous = std::env::current_dir().expect("a working directory");
    // Serialised with the other tests by being the only one that does this.
    std::env::set_current_dir(&directory).expect("change directory");
    let mut app = App::headless();
    app.recall_path(PathBuf::from("relative.chn"));
    let opened = app.active_spectrum().map(|spectrum| spectrum.len());
    std::env::set_current_dir(previous).expect("restore the directory");
    assert_eq!(opened, Some(128), "a relative path should work: {}", {
        app.status
    });
}

#[test]
fn opening_the_same_file_twice_gives_two_independent_windows() {
    let path = written_spectrum("twice.chn", 256);
    let mut app = App::headless();
    app.recall_path(path.clone());
    let first = app.active.expect("active");
    app.recall_path(path);
    let second = app.active.expect("active");
    assert_ne!(first, second);
    assert_ne!(app.windows[first].id, app.windows[second].id);

    // Changing one leaves the other alone.
    app.apply_action(Action::Smooth);
    let smoothed = app.windows[second]
        .buffer
        .as_ref()
        .map(|spectrum| spectrum.channels.clone());
    let untouched = app.windows[first]
        .buffer
        .as_ref()
        .map(|spectrum| spectrum.channels.clone());
    assert_ne!(
        smoothed, untouched,
        "the two windows should hold separate copies"
    );
}

#[test]
fn a_directory_is_not_mistaken_for_a_spectrum() {
    let directory = scratch("a-directory");
    std::fs::create_dir_all(&directory).expect("make the directory");
    let mut app = App::headless();
    let before = app.windows.len();
    app.recall_path(directory);
    assert_eq!(app.windows.len(), before);
    assert!(!app.status.is_empty());
}

#[test]
fn the_files_opened_lately_are_offered_again() {
    let first = written_spectrum("recent-one.chn", 128);
    let second = written_spectrum("recent-two.chn", 128);
    let mut app = App::headless();
    app.recall_path(first.clone());
    app.recall_path(second.clone());

    // Most recent first, so the File menu lists them in the useful order.
    assert_eq!(app.recent_files(), [second.clone(), first.clone()]);

    // Opening one again moves it to the front rather than repeating it.
    app.recall_path(first.clone());
    assert_eq!(app.recent_files(), [first, second]);

    // A file that would not open is not offered again.
    let bad = scratch("recent-bad.txt");
    std::fs::write(&bad, "not a spectrum").expect("write");
    app.recall_path(bad.clone());
    assert!(!app.recent_files().contains(&bad));
}

#[test]
fn a_spectrum_dropped_on_the_window_opens() {
    // Dragging a file in is the fallback when a system file dialog will not
    // appear, so it goes through the same frame the interface runs in.
    let path = written_spectrum("dropped.chn", 256);
    let ctx = egui::Context::default();
    let mut app = App::headless();
    let input = egui::RawInput {
        screen_rect: Some(egui::Rect::from_min_size(
            egui::Pos2::ZERO,
            egui::vec2(1000.0, 700.0),
        )),
        dropped_files: vec![egui::DroppedFile {
            path: Some(path.clone()),
            ..Default::default()
        }],
        ..Default::default()
    };
    let _ = ctx.run_ui(input, |ui| app.draw(ui));

    assert!(
        app.active_spectrum().is_some_and(|s| s.len() == 256),
        "a dropped spectrum should open: {}",
        app.status
    );
    assert_eq!(app.recent_files().first(), Some(&path));
}

#[test]
fn the_fixture_spectra_all_open_through_the_application() {
    // The real instrument files in the formats crate, driven through the
    // application rather than the reader, so that the whole path is covered.
    let dir = Path::new(env!("CARGO_MANIFEST_DIR")).join("../mantaray-formats/tests/fixtures");
    let Ok(entries) = std::fs::read_dir(&dir) else {
        eprintln!("no fixtures at {} - skipping", dir.display());
        return;
    };
    let files: Vec<PathBuf> = entries
        .flatten()
        .map(|entry| entry.path())
        .filter(|path| {
            path.is_file() && mantaray_formats::SpectrumFormat::from_path(path).is_some()
        })
        .collect();
    if files.is_empty() {
        eprintln!("no fixtures at {} - skipping", dir.display());
        return;
    }
    for path in files {
        let mut app = App::headless();
        app.recall_path(path.clone());
        let name = path.file_name().expect("a name").to_string_lossy();
        let index = app
            .active
            .unwrap_or_else(|| panic!("{name} opened no window: {}", app.status));
        assert!(
            app.visible_windows().contains(&index),
            "{name} opened a window that is not drawn"
        );
        let spectrum = app
            .active_spectrum()
            .unwrap_or_else(|| panic!("{name} opened an empty window: {}", app.status));
        assert!(!spectrum.is_empty(), "{name} loaded zero channels");
    }
}
