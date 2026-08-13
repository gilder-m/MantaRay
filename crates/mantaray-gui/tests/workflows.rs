//! Whole jobs, done the way a person does them.
//!
//! The other suites check that a command works. These check that a sequence of
//! commands gets a job finished, because that is where things actually break:
//! a step that quietly undoes an earlier one, a calibration that does not reach
//! the file, an analysis that reads the spectrum as it was rather than as it is.
//!
//! Each test is written as the run of actions the interface issues, in the order
//! somebody would issue them, so a failure names the step rather than the
//! function.

use std::path::PathBuf;

use mantaray_core::Spectrum;
use mantaray_gui::app::{Action, App};
use mantaray_gui::view::MarkMode;

fn scratch(name: &str) -> PathBuf {
    let directory = std::env::temp_dir().join("mantaray-workflow-tests");
    std::fs::create_dir_all(&directory).expect("a scratch directory");
    directory.join(name)
}

/// A spectrum with peaks at known channels, so the arithmetic can be checked.
///
/// Three lines, at a quarter, a half and three quarters of the range, on a
/// falling continuum - the shape of a real measurement without the noise that
/// would make a test flaky.
fn source(channels: usize) -> Spectrum {
    let mut spectrum = Spectrum::new(channels);
    spectrum.live_time = 600.0;
    spectrum.real_time = 620.0;
    for channel in 0..channels {
        spectrum.channels[channel] = 800 - (600 * channel as u64 / channels as u64);
    }
    for centre in [channels / 4, channels / 2, 3 * channels / 4] {
        for offset in -12i64..=12 {
            let value = (60_000.0 * (-0.5 * (offset as f64 / 3.0).powi(2)).exp()) as u64;
            spectrum.channels[(centre as i64 + offset) as usize] += value;
        }
    }
    spectrum
}

/// An application holding that spectrum in a buffer window.
fn with_source(channels: usize) -> App {
    let mut app = App::headless();
    app.open_buffer("workflow.Spe".into(), source(channels), None);
    app
}

#[test]
fn calibrating_from_two_known_lines_and_saving_keeps_the_calibration() {
    // The commonest job there is: open a spectrum of a known source, find its
    // peaks, tell the program what two of them are, save, and have the file
    // remember. Every step below is one a person performs.
    let mut app = with_source(1024);

    app.apply_action(Action::PeakSearch);
    let found = app
        .active_spectrum()
        .map(|spectrum| spectrum.rois.len())
        .unwrap_or(0);
    assert!(found >= 3, "the three lines should be found, got {found}");

    // Two points, entered at the peaks rather than at guessed channels: this is
    // what the marker and the Calibrate box do between them.
    for (channel, energy) in [(256usize, 200.0), (768, 600.0)] {
        app.apply_action(Action::Marker(channel));
        app.apply_action(Action::AddCalibrationPoint(energy));
    }

    let calibration = app
        .active_spectrum()
        .and_then(|spectrum| spectrum.energy_calibration.clone())
        .expect("two points make a calibration");
    // The line at half range should now read half way between the two, which is
    // the arithmetic a person would check by eye.
    let middle = calibration.energy(512.0);
    assert!(
        (middle - 400.0).abs() < 2.0,
        "the middle line should read about 400 keV, got {middle}"
    );

    let path = scratch("calibrated.Spe");
    // Deleted before the recall, not after: a calibrated.Spe left by an
    // earlier run would open in a new window, make itself active, and be
    // saved back out - so the test would then compare that old file's
    // calibration against this run's, and fail forever after any change
    // that moves a centroid.
    let _ = std::fs::remove_file(&path);
    app.apply_action(Action::RecallFile(path.clone()));
    app.save_active_to(&path).expect("it should save");

    let reopened = mantaray_formats::load_spectrum(&path).expect("the file reads back");
    let saved = reopened
        .energy_calibration
        .expect("the calibration should have been written");
    assert!(
        (saved.energy(512.0) - middle).abs() < 1e-6,
        "the saved calibration should be the one that was in force"
    );
}

#[test]
fn a_region_marked_by_hand_survives_being_saved_and_reopened() {
    let mut app = with_source(512);
    app.apply_action(Action::MarkMode(MarkMode::Mark));
    app.apply_action(Action::MarkRange(120, 136));
    let marked = app
        .active_spectrum()
        .map(|spectrum| spectrum.rois.len())
        .unwrap_or(0);
    assert_eq!(marked, 1, "the region should be marked: {}", app.status);

    let path = scratch("regions.Spe");
    let _ = std::fs::remove_file(&path);
    app.save_active_to(&path).expect("it should save");

    let reopened = mantaray_formats::load_spectrum(&path).expect("it reads back");
    assert_eq!(
        reopened.rois.len(),
        1,
        "a region is part of the measurement and belongs in the file"
    );
    let region = reopened.rois.iter().next().expect("the region");
    assert!(
        region.contains(128),
        "the region should still cover what it covered"
    );
}

#[test]
fn smoothing_can_be_taken_back_after_looking_at_the_result() {
    // Smoothing is destructive and people try it to see. Undo has to put the
    // counts back exactly, not approximately.
    let mut app = with_source(512);
    let before = app
        .active_spectrum()
        .map(|spectrum| spectrum.channels.clone())
        .expect("a spectrum");

    app.apply_action(Action::Smooth);
    let after = app
        .active_spectrum()
        .map(|spectrum| spectrum.channels.clone())
        .expect("a spectrum");
    assert_ne!(before, after, "smoothing should change the counts");

    app.apply_action(Action::Undo);
    assert_eq!(
        app.active_spectrum()
            .map(|spectrum| spectrum.channels.clone()),
        Some(before),
        "undo should put back exactly what was there"
    );
}

#[test]
fn a_whole_analysis_run_reaches_a_report() {
    // Search, calibrate, analyse, report - and the report has to describe the
    // spectrum as it is at the end, not as it was at the start.
    let mut app = with_source(1024);
    app.apply_action(Action::PeakSearch);
    for (channel, energy) in [(256usize, 200.0), (768, 600.0)] {
        app.apply_action(Action::Marker(channel));
        app.apply_action(Action::AddCalibrationPoint(energy));
    }
    app.apply_action(Action::Analyse);
    app.apply_action(Action::RoiReport);

    let report = app.report_text.clone().unwrap_or_default();
    assert!(!report.is_empty(), "a report should have been produced");
    assert!(
        report.contains("ROI"),
        "the report should describe the regions:\n{report}"
    );
    // The energies in the report must be the calibrated ones. Before the
    // calibration the middle line sat at channel 512; after it, at 400 keV.
    assert!(
        report.contains("400.") || report.contains("399.") || report.contains("401."),
        "the report should be in the energies the calibration gives:\n{report}"
    );
}

#[test]
fn the_order_of_a_job_does_not_change_what_it_finds() {
    // Calibrating before searching and searching before calibrating are both
    // things people do, and they should end in the same place.
    let energies = |app: App| -> Vec<f64> {
        let calibration = app
            .active_spectrum()
            .and_then(|spectrum| spectrum.energy_calibration.clone());
        let settings = app.settings;
        let spectrum = app.active_spectrum().expect("a spectrum").clone();
        let mut found: Vec<f64> = spectrum
            .rois
            .iter()
            .filter_map(|roi| mantaray_core::peak_info(&spectrum, *roi, &settings).ok())
            .map(|info| match &calibration {
                Some(calibration) => calibration.energy(info.centroid),
                None => info.centroid,
            })
            .collect();
        found.sort_by(|a, b| a.partial_cmp(b).unwrap_or(std::cmp::Ordering::Equal));
        found
    };

    let mut search_first = with_source(1024);
    search_first.apply_action(Action::PeakSearch);
    for (channel, energy) in [(256usize, 200.0), (768, 600.0)] {
        search_first.apply_action(Action::Marker(channel));
        search_first.apply_action(Action::AddCalibrationPoint(energy));
    }

    let mut calibrate_first = with_source(1024);
    for (channel, energy) in [(256usize, 200.0), (768, 600.0)] {
        calibrate_first.apply_action(Action::Marker(channel));
        calibrate_first.apply_action(Action::AddCalibrationPoint(energy));
    }
    calibrate_first.apply_action(Action::PeakSearch);

    let one = energies(search_first);
    let two = energies(calibrate_first);
    assert_eq!(one.len(), two.len(), "the same peaks should be found");
    for (a, b) in one.iter().zip(two.iter()) {
        assert!(
            (a - b).abs() < 0.5,
            "the same peak should land at the same energy either way: {a} against {b}"
        );
    }
}

#[test]
fn a_spectrum_can_go_out_one_format_and_come_back_another() {
    // Converting is a job in itself, and the counts are what must not change.
    let mut app = with_source(512);
    let original = app
        .active_spectrum()
        .map(|spectrum| spectrum.channels.clone())
        .expect("a spectrum");

    let mut previous = original.clone();
    for name in ["hop.Spe", "hop.Chn", "hop.json"] {
        let path = scratch(name);
        let _ = std::fs::remove_file(&path);
        app.save_active_to(&path).expect("it should save");
        let reopened = mantaray_formats::load_spectrum(&path).expect("it reads back");
        assert_eq!(
            reopened.channels, previous,
            "{name} should carry the counts through unchanged"
        );
        previous = reopened.channels.clone();
        app.recall_path(path);
    }
    assert_eq!(
        previous, original,
        "three formats in a row should still be the spectrum that started"
    );
}

#[test]
fn clearing_a_detector_by_accident_is_recoverable_mid_job() {
    // The worst moment in a working day: a count is running, Clear is pressed
    // by mistake, and the data has to come back.
    let mut app = App::headless();
    app.apply_action(Action::Start);
    app.advance_by(20.0);
    app.apply_action(Action::Stop);
    let counted = app
        .active_spectrum()
        .map(|spectrum| spectrum.total_counts())
        .unwrap_or(0);
    assert!(counted > 0, "the count should have collected something");

    app.apply_action(Action::Clear);
    app.apply_action(Action::Undo);

    // It comes back in a buffer rather than in the instrument, which is the
    // point: instrument memory is never written to behind the operator's back.
    let recovered = app
        .active_spectrum()
        .map(|spectrum| spectrum.total_counts())
        .unwrap_or(0);
    assert_eq!(recovered, counted, "every count should have come back");

    // And it can be saved from there like anything else.
    let path = scratch("recovered.Spe");
    let _ = std::fs::remove_file(&path);
    app.save_active_to(&path).expect("it should save");
    let reopened = mantaray_formats::load_spectrum(&path).expect("it reads back");
    assert_eq!(reopened.total_counts(), counted);
}

/// Asking about a nuclide by name, the way somebody at a detector would.
#[test]
fn a_nuclide_asked_for_by_name_is_found_however_it_is_written() {
    let mut app = with_source(1024);
    app.library = mantaray_core::NuclideLibrary::sample_for_tests();

    for typed in ["cs137", "Cs-137", "137Cs", "cs 137"] {
        app.isotope.typed = typed.into();
        app.isotope.resolve(&app.library);
        assert_eq!(
            app.isotope.found.as_deref(),
            Some("Cs-137"),
            "{typed:?} should find Cs-137"
        );
        assert!(
            app.isotope.missing.is_none(),
            "{typed:?}: {:?}",
            app.isotope.missing
        );
        // And the nuclide itself comes back, with its lines.
        let nuclide = app.isotope.nuclide(&app.library).expect("the nuclide");
        assert!(
            nuclide
                .peaks
                .iter()
                .any(|peak| (peak.energy - 661.657).abs() < 0.01)
        );
    }
}

/// The two ways a lookup fails are different problems, and are told apart.
#[test]
fn a_nuclide_that_is_not_there_says_which_kind_of_not_there() {
    let mut app = with_source(1024);

    // No library at all: a thing the operator can fix in one action, and
    // reporting it as "not found" would send them hunting for a nuclide that
    // was never searched for.
    assert!(app.library.is_empty(), "a library is not shipped");
    app.isotope.typed = "Cs-137".into();
    app.isotope.resolve(&app.library);
    assert!(app.isotope.found.is_none());
    let reason = app.isotope.missing.clone().expect("a reason");
    assert!(
        reason.contains("no nuclide library"),
        "should say the library is missing, said {reason:?}"
    );

    // A library that simply does not hold it, named so the operator knows
    // which library was searched.
    app.library = mantaray_core::NuclideLibrary::sample_for_tests();
    app.isotope.typed = "Xe-133".into();
    app.isotope.resolve(&app.library);
    let reason = app.isotope.missing.clone().expect("a reason");
    assert!(
        reason.contains("Xe-133") && reason.contains(&app.library.name),
        "should name the nuclide and the library, said {reason:?}"
    );

    // And a name that is not a name at all is refused as such, rather than
    // being reported as absent from the library.
    app.isotope.typed = "not a nuclide".into();
    app.isotope.resolve(&app.library);
    let reason = app.isotope.missing.clone().expect("a reason");
    assert!(
        reason.contains("not a nuclide name"),
        "should say it is not a name, said {reason:?}"
    );

    // An empty box is not an error - it is the state before anybody asked.
    app.isotope.typed = String::new();
    app.isotope.resolve(&app.library);
    assert!(app.isotope.found.is_none() && app.isotope.missing.is_none());
}
