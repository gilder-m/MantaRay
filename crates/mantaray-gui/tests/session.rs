//! A whole working session, driven through the actions the interface issues.
//!
//! Acquire, clear, undo, mark, calibrate, save, report. Each test is written the
//! way the person using mantaray would describe the step, so a failure names the
//! thing that broke rather than the function that broke.

use std::path::PathBuf;

use mantaray_core::{Roi, Spectrum};
use mantaray_device::Mcb;
use mantaray_gui::app::{Action, App, Target};
use mantaray_gui::view::MarkMode;

fn scratch(name: &str) -> PathBuf {
    let dir = std::env::temp_dir().join("mantaray-session-tests");
    std::fs::create_dir_all(&dir).expect("scratch directory");
    dir.join(name)
}

/// An application with a spectrum open in a buffer window, ready to work on.
fn with_spectrum(channels: usize) -> App {
    let mut spectrum = Spectrum::new(channels);
    spectrum.live_time = 100.0;
    spectrum.real_time = 105.0;
    for channel in 0..channels {
        spectrum.channels[channel] = 30;
    }
    let centre = channels / 2;
    for offset in -10i64..=10 {
        let value = (5000.0 * (-0.5 * (offset as f64 / 3.5).powi(2)).exp()) as u64;
        spectrum.channels[(centre as i64 + offset) as usize] += value;
    }
    let mut app = App::headless();
    app.open_buffer("Test".into(), spectrum, None);
    app
}

#[test]
fn the_session_starts_with_the_simulator_ready_to_count() {
    let app = App::headless();
    assert!(
        !app.detectors.is_empty(),
        "a simulated detector should be configured"
    );
    let index = app.active.expect("its window should be open and active");
    assert_eq!(app.windows[index].target, Target::Detector(0));
    assert!(
        app.visible_windows().contains(&index),
        "the detector window should be drawn"
    );
    assert!(
        app.detectors[0].spectrum().total_counts() == 0,
        "nothing has been counted yet"
    );
}

#[test]
fn starting_a_count_fills_the_spectrum_and_stopping_holds_it() {
    let mut app = App::headless();
    app.apply_action(Action::Start);
    app.advance_by(5.0);
    let counted = app.detectors[0].spectrum().total_counts();
    assert!(counted > 0, "the count produced nothing");
    assert!(app.detectors[0].is_active());

    app.apply_action(Action::Stop);
    app.advance_by(5.0);
    assert!(!app.detectors[0].is_active());
    assert_eq!(
        app.detectors[0].spectrum().total_counts(),
        counted,
        "a stopped detector should not keep counting"
    );
}

#[test]
fn live_time_advances_and_stays_behind_real_time() {
    let mut app = App::headless();
    app.apply_action(Action::Start);
    app.advance_by(10.0);
    let spectrum = app.detectors[0].spectrum();
    assert!(spectrum.real_time >= 9.9, "real time did not advance");
    assert!(
        spectrum.live_time <= spectrum.real_time,
        "live time {} exceeded real time {}",
        spectrum.live_time,
        spectrum.real_time
    );
    assert!(
        spectrum.live_time > 0.0,
        "live time should advance while counting"
    );
}

#[test]
fn an_accidentally_cleared_detector_is_recovered_into_a_buffer_window() {
    let mut app = App::headless();
    app.apply_action(Action::Start);
    app.advance_by(5.0);
    app.apply_action(Action::Stop);
    let counted = app.detectors[0].spectrum().total_counts();
    let windows_before = app.windows.len();

    app.apply_action(Action::Clear);
    assert_eq!(
        app.detectors[0].spectrum().total_counts(),
        0,
        "Clear should empty the instrument"
    );

    app.apply_action(Action::Undo);

    assert_eq!(
        app.detectors[0].spectrum().total_counts(),
        0,
        "undo must not write back to instrument memory"
    );
    assert_eq!(
        app.windows.len(),
        windows_before + 1,
        "the data should come back in a new buffer window"
    );
    let index = app.active.expect("the recovered window should be active");
    assert_eq!(app.windows[index].target, Target::Buffer);
    assert_eq!(
        app.active_spectrum()
            .map(|spectrum| spectrum.total_counts()),
        Some(counted),
        "every count should have come back"
    );
    assert!(
        app.windows[index].title.starts_with("Recovered"),
        "the window should say what it is: {}",
        app.windows[index].title
    );
    assert!(
        app.visible_windows().contains(&index),
        "the recovered window should be on screen"
    );
}

#[test]
fn a_cleared_buffer_is_restored_in_place() {
    let mut app = with_spectrum(512);
    let index = app.active.expect("active");
    let before = app.active_spectrum().map(|s| s.channels.clone());

    app.apply_action(Action::Clear);
    assert_eq!(
        app.active_spectrum().map(|s| s.total_counts()),
        Some(0),
        "Clear should empty the buffer"
    );

    app.apply_action(Action::Undo);

    assert_eq!(app.active, Some(index), "the same window should come back");
    assert_eq!(app.windows.len(), 2, "no extra window for a buffer undo");
    assert_eq!(
        app.active_spectrum().map(|s| s.channels.clone()),
        before,
        "the buffer should hold exactly what it held before"
    );
}

#[test]
fn undo_and_redo_walk_the_history_both_ways() {
    let mut app = with_spectrum(256);
    let original = app.active_spectrum().map(|s| s.channels.clone());

    app.apply_action(Action::Smooth);
    let smoothed = app.active_spectrum().map(|s| s.channels.clone());
    assert_ne!(original, smoothed, "smoothing should change the data");

    app.apply_action(Action::Undo);
    assert_eq!(app.active_spectrum().map(|s| s.channels.clone()), original);

    app.apply_action(Action::Redo);
    assert_eq!(app.active_spectrum().map(|s| s.channels.clone()), smoothed);
}

#[test]
fn undo_is_offered_only_when_there_is_something_to_take_back() {
    let mut app = with_spectrum(128);
    assert!(!app.history.can_undo(), "a fresh session has no history");
    app.apply_action(Action::Undo);
    assert!(
        app.status.contains("nothing to undo"),
        "the user should be told: {}",
        app.status
    );

    app.apply_action(Action::Smooth);
    assert!(app.history.can_undo());
    assert_eq!(app.history.undo_label(), Some("Smooth"));
}

#[test]
fn marking_a_region_by_hand_records_it_and_undo_removes_it() {
    let mut app = with_spectrum(512);
    app.apply_action(Action::MarkMode(MarkMode::Mark));
    app.apply_action(Action::MarkRange(240, 270));

    let regions = app
        .active_spectrum()
        .map(|spectrum| spectrum.rois.len())
        .unwrap_or(0);
    assert_eq!(regions, 1, "the region was not marked: {}", app.status);

    app.apply_action(Action::Undo);
    assert_eq!(
        app.active_spectrum()
            .map(|spectrum| spectrum.rois.len())
            .unwrap_or(0),
        0,
        "undo should take the region back"
    );
}

#[test]
fn peak_search_marks_the_peak_and_peak_info_measures_it() {
    let mut app = with_spectrum(1024);
    app.apply_action(Action::PeakSearch);
    let spectrum = app.active_spectrum().expect("a spectrum");
    assert!(!spectrum.rois.is_empty(), "peak search found nothing");
    assert!(
        spectrum.rois.at(512).is_some(),
        "the peak at 512 should be inside a region"
    );

    app.apply_action(Action::Marker(512));
    app.apply_action(Action::PeakInfoAtMarker);
    let index = app.active.expect("active");
    let info = app.windows[index]
        .peak_info
        .as_ref()
        .expect("peak information");
    assert!((info.centroid - 512.0).abs() < 2.0);
    assert!(info.fwhm > 0.0, "a width should have been fitted");
    assert!(
        info.net_area > 10_000.0,
        "the peak should be measured, got {}",
        info.net_area
    );
    assert!(info.net_area_uncertainty > 0.0);
}

#[test]
fn peak_info_on_a_broad_uncalibrated_peak_takes_the_whole_peak() {
    // A scintillator-wide line - FWHM about forty channels - with no shape
    // calibration to say so. Peak Info used to assume eight channels and fit
    // a sliver of it; the width must instead be measured from the counts.
    let mut spectrum = Spectrum::new(1024);
    spectrum.live_time = 100.0;
    spectrum.real_time = 105.0;
    let sigma = 17.0;
    for channel in 0..1024 {
        let offset = channel as f64 - 600.0;
        spectrum.channels[channel] = 30 + (5000.0 * (-0.5 * (offset / sigma).powi(2)).exp()) as u64;
    }
    let mut app = App::headless();
    app.open_buffer("NaI".into(), spectrum, None);

    // Double-clicking the flank, not the apex: the peak the marker is on is
    // what should be fitted, centred where the peak is.
    app.apply_action(Action::Marker(590));
    app.apply_action(Action::PeakInfoAtMarker);
    let index = app.active.expect("active");
    let info = app.windows[index]
        .peak_info
        .as_ref()
        .expect("peak information");
    assert!(
        (info.centroid - 600.0).abs() < 2.0,
        "the fit should centre on the peak, got {}",
        info.centroid
    );
    let expected_fwhm = 2.3548 * sigma;
    assert!(
        (info.fwhm - expected_fwhm).abs() < 4.0,
        "the whole width should be measured, expected about {expected_fwhm:.1}, got {:.1}",
        info.fwhm
    );
    let expected_area = 5000.0 * sigma * (2.0 * std::f64::consts::PI).sqrt();
    assert!(
        info.net_area > 0.85 * expected_area,
        "the whole peak should be inside the fit, expected about {expected_area:.0}, got {:.0}",
        info.net_area
    );
}

#[test]
fn a_two_point_calibration_turns_channels_into_energies() {
    // Exactly what the Calibrate dialog does: put the marker on a peak, type the
    // energy, press Enter, twice.
    let mut app = with_spectrum(1024);
    app.apply_action(Action::Marker(100));
    app.apply_action(Action::AddCalibrationPoint(200.0));
    app.apply_action(Action::Marker(500));
    app.apply_action(Action::AddCalibrationPoint(1000.0));

    let index = app.active.expect("active");
    assert_eq!(app.windows[index].calibration.len(), 2);
    let spectrum = app.active_spectrum().expect("a spectrum");
    let cal = spectrum
        .energy_calibration
        .as_ref()
        .expect("two points should calibrate the spectrum");
    assert!((cal.energy(100.0) - 200.0).abs() < 1e-6);
    assert!((cal.energy(500.0) - 1000.0).abs() < 1e-6);

    app.apply_action(Action::DestroyCalibration);
    assert!(
        app.active_spectrum()
            .is_some_and(|spectrum| spectrum.energy_calibration.is_none()),
        "destroying should leave the spectrum in channels"
    );
}

#[test]
fn a_third_calibration_point_fits_a_curve_through_all_three() {
    let mut app = with_spectrum(1024);
    for (channel, energy) in [(100usize, 200.0), (500, 1000.0), (900, 1810.0)] {
        app.apply_action(Action::Marker(channel));
        app.apply_action(Action::AddCalibrationPoint(energy));
    }
    let spectrum = app.active_spectrum().expect("a spectrum");
    let cal = spectrum.energy_calibration.as_ref().expect("a calibration");
    assert_eq!(cal.order(), 2, "three points should fit a quadratic");
    for (channel, energy) in [(100.0, 200.0), (500.0, 1000.0), (900.0, 1810.0)] {
        assert!(
            (cal.energy(channel) - energy).abs() < 1.0,
            "channel {channel} should be near {energy}, got {}",
            cal.energy(channel)
        );
    }
}

#[test]
fn a_spectrum_can_be_saved_and_the_window_stops_being_modified() {
    let mut app = with_spectrum(256);
    app.apply_action(Action::Smooth);
    let index = app.active.expect("active");
    assert!(app.windows[index].modified);

    let path = scratch("saved.chn");
    let spectrum = app.active_spectrum().cloned().expect("a spectrum");
    mantaray_formats::save_spectrum(&spectrum, &path).expect("save");
    app.windows[index].path = Some(path.clone());
    app.windows[index].modified = false;

    // What is on disk is what was on screen.
    let reread = mantaray_formats::load_spectrum(&path).expect("read it back");
    assert_eq!(reread.channels, spectrum.channels);
    assert!(!app.windows[index].modified);
}

#[test]
fn copying_a_detector_to_a_buffer_freezes_a_copy_of_the_data() {
    let mut app = App::headless();
    app.apply_action(Action::Start);
    app.advance_by(3.0);
    let counted = app.detectors[0].spectrum().total_counts();

    app.apply_action(Action::CopyToBuffer);

    let index = app.active.expect("the buffer should be active");
    assert_eq!(app.windows[index].target, Target::Buffer);
    assert_eq!(
        app.active_spectrum().map(|s| s.total_counts()),
        Some(counted)
    );

    // The buffer does not follow the instrument.
    app.advance_by(3.0);
    assert!(app.detectors[0].spectrum().total_counts() > counted);
    assert_eq!(
        app.active_spectrum().map(|s| s.total_counts()),
        Some(counted),
        "a buffer is a snapshot, not a live view"
    );
}

#[test]
fn the_view_can_be_zoomed_and_reset_without_leaving_the_spectrum() {
    let mut app = with_spectrum(1024);
    app.apply_action(Action::Marker(512));
    for _ in 0..12 {
        app.apply_action(Action::ZoomIn);
    }
    let index = app.active.expect("active");
    let (low, high) = app.windows[index].display.visible();
    assert!(low <= 512 && 512 <= high, "zoom should keep the marker");
    assert!(high - low < 1023, "zooming in should narrow the view");

    app.apply_action(Action::FullView);
    let (low, high) = app.windows[index].display.visible();
    assert_eq!((low, high), (0, 1023), "Full view should show everything");
}

#[test]
fn a_report_describes_the_regions_that_were_marked() {
    let mut app = with_spectrum(1024);
    app.apply_action(Action::MarkRange(495, 530));
    app.apply_action(Action::RoiReport);
    let report = app.report_text.as_deref().unwrap_or("");
    assert!(!report.is_empty(), "no report was produced: {}", app.status);
    assert!(
        report.contains("495") || report.contains("512"),
        "the report should mention the region: {report}"
    );
}

#[test]
fn clearing_all_regions_can_be_taken_back() {
    let mut app = with_spectrum(512);
    app.apply_action(Action::MarkRange(100, 130));
    app.apply_action(Action::MarkRange(240, 270));
    assert_eq!(app.active_spectrum().map(|s| s.rois.len()), Some(2));

    app.apply_action(Action::ClearAllRois);
    assert_eq!(app.active_spectrum().map(|s| s.rois.len()), Some(0));

    app.apply_action(Action::Undo);
    assert_eq!(
        app.active_spectrum().map(|s| s.rois.len()),
        Some(2),
        "undo should bring the regions back"
    );
}

#[test]
fn a_summed_region_reports_its_contents() {
    let mut app = with_spectrum(512);
    let expected: u64 = app
        .active_spectrum()
        .map(|spectrum| spectrum.channels[200..=220].iter().sum())
        .unwrap_or(0);
    app.apply_action(Action::Select(200, 220));
    app.apply_action(Action::Sum);
    assert!(
        app.status.contains(&expected.to_string()),
        "the sum {expected} should be reported: {}",
        app.status
    );
}

#[test]
fn stripping_a_background_leaves_less_than_was_there() {
    let mut app = with_spectrum(512);
    let before = app
        .active_spectrum()
        .map(|spectrum| spectrum.total_counts())
        .unwrap_or(0);
    // A background file with the same shape, at a tenth of the size.
    let mut background = app.active_spectrum().cloned().expect("a spectrum");
    for channel in background.channels.iter_mut() {
        *channel /= 10;
    }
    let path = scratch("background.chn");
    mantaray_formats::save_spectrum(&background, &path).expect("write the background");

    app.strip_from(path, Some(1.0));

    let after = app
        .active_spectrum()
        .map(|spectrum| spectrum.total_counts())
        .unwrap_or(0);
    assert!(
        after < before,
        "stripping should remove counts: {before} -> {after}"
    );
    app.apply_action(Action::Undo);
    assert_eq!(
        app.active_spectrum().map(|s| s.total_counts()),
        Some(before),
        "undo should put the background back"
    );
}

#[test]
fn the_marker_can_be_moved_and_stays_inside_the_spectrum() {
    let mut app = with_spectrum(256);
    app.apply_action(Action::Marker(1000));
    let index = app.active.expect("active");
    assert!(
        app.windows[index].display.marker <= 255,
        "the marker should stay in the spectrum, got {}",
        app.windows[index].display.marker
    );
    app.apply_action(Action::Marker(128));
    assert_eq!(app.windows[index].display.marker, 128);
}

#[test]
fn closing_a_modified_buffer_asks_before_losing_the_data() {
    let mut app = with_spectrum(256);
    app.apply_action(Action::Smooth);
    let index = app.active.expect("active");
    let id = app.windows[index].id;
    let windows_before = app.windows.len();

    // The close does not happen; the question opens instead.
    app.apply_action(Action::CloseWindow(id));
    assert_eq!(
        app.windows.len(),
        windows_before,
        "the window must survive until the question is answered"
    );
    assert!(
        app.pending_close.is_some(),
        "the question should be pending"
    );

    // Discard answers it.
    app.apply_action(Action::ForceCloseWindow(id));
    assert_eq!(app.windows.len(), windows_before - 1);
    assert!(
        app.windows.iter().all(|window| window.id != id),
        "the discarded window is gone"
    );
}

#[test]
fn closing_an_unmodified_buffer_just_closes() {
    let mut app = with_spectrum(256);
    let index = app.active.expect("active");
    let id = app.windows[index].id;
    let windows_before = app.windows.len();
    app.apply_action(Action::CloseWindow(id));
    assert_eq!(app.windows.len(), windows_before - 1);
    assert!(app.pending_close.is_none());
}

#[test]
fn auto_clear_makes_a_peak_search_start_fresh() {
    let mut app = with_spectrum(1024);
    app.apply_action(Action::MarkRange(10, 30));
    assert_eq!(app.active_spectrum().map(|s| s.rois.len()), Some(1));

    // Off: the search adds to what is there (the manual's behaviour).
    app.apply_action(Action::PeakSearch);
    let with_stale = app.active_spectrum().map(|s| s.rois.len()).unwrap_or(0);
    assert!(with_stale >= 2, "the hand-marked region should remain");
    assert!(
        app.active_spectrum()
            .is_some_and(|s| s.rois.at(20).is_some()),
        "the stale region survives without Auto Clear"
    );

    // On: the search clears first.
    app.auto_clear_roi = true;
    app.apply_action(Action::PeakSearch);
    assert!(
        app.active_spectrum()
            .is_some_and(|s| s.rois.at(20).is_none()),
        "Auto Clear should remove the stale region"
    );
    assert!(
        app.active_spectrum()
            .is_some_and(|s| s.rois.at(512).is_some()),
        "the real peak is still found"
    );
}

#[test]
fn typing_an_energy_jumps_the_marker_there() {
    let mut app = with_spectrum(1024);
    // A 2-point calibration: channel = energy (1 keV per channel from zero).
    app.apply_action(Action::Marker(100));
    app.apply_action(Action::AddCalibrationPoint(100.0));
    app.apply_action(Action::Marker(900));
    app.apply_action(Action::AddCalibrationPoint(900.0));

    app.apply_action(Action::GotoEnergy(512.0));
    let index = app.active.expect("active");
    assert_eq!(app.windows[index].display.marker, 512);
    let (start, end) = app.windows[index].display.visible();
    assert!(
        start <= 512 && 512 <= end,
        "the view should hold the marker: {start}-{end}"
    );

    // An energy beyond the spectrum is refused with an explanation.
    app.apply_action(Action::GotoEnergy(5000.0));
    assert_eq!(app.windows[index].display.marker, 512, "the marker stayed");
    assert!(
        app.status.contains("outside the spectrum"),
        "the refusal should be explained: {}",
        app.status
    );
}

#[test]
fn without_a_calibration_the_go_to_box_takes_a_channel() {
    let mut app = with_spectrum(512);
    app.apply_action(Action::GotoEnergy(300.0));
    let index = app.active.expect("active");
    assert_eq!(app.windows[index].display.marker, 300);
}

#[test]
fn control_tab_cycles_through_the_windows() {
    let mut app = with_spectrum(256);
    let mut second = Spectrum::new(128);
    second.channels[10] = 5;
    app.open_buffer("second".into(), second, None);
    let last = app.active.expect("active");

    app.apply_action(Action::CycleWindow(1));
    let next = app.active.expect("active");
    assert_ne!(last, next, "the active window should change");

    // Cycling all the way round comes back to where it began.
    let count = app.windows.len();
    for _ in 1..count {
        app.apply_action(Action::CycleWindow(1));
    }
    assert_eq!(app.active, Some(last), "a full cycle returns");

    // Backwards undoes forwards.
    app.apply_action(Action::CycleWindow(1));
    app.apply_action(Action::CycleWindow(-1));
    assert_eq!(app.active, Some(last));
}

#[test]
fn cycling_in_full_area_mode_pages_through_maximised_spectra() {
    let mut app = with_spectrum(256);
    let mut second = Spectrum::new(128);
    second.channels[10] = 5;
    app.open_buffer("second".into(), second, None);
    app.apply_action(Action::MaximizeActive);
    assert_eq!(app.visible_windows().len(), 1);
    let before = app.visible_windows();

    app.apply_action(Action::CycleWindow(1));

    let after = app.visible_windows();
    assert_eq!(after.len(), 1, "still one window filling the area");
    assert_ne!(before, after, "a different window now fills it");
}

#[test]
fn an_uncalibrated_europium_spectrum_calibrates_itself_from_its_peaks() {
    // The real Eu-152 fixture, stripped of the calibration it carries: the
    // instrument's own answer is the standard the automatic one must match.
    let fixture = std::path::Path::new(env!("CARGO_MANIFEST_DIR"))
        .join("../mantaray-formats/tests/fixtures/eu152_spectra.Spe");
    if !fixture.is_file() {
        eprintln!("fixture missing - skipping");
        return;
    }
    let mut spectrum = mantaray_formats::load_spectrum(&fixture).expect("load");
    let instrument = spectrum
        .energy_calibration
        .take()
        .expect("the fixture is calibrated");
    spectrum.rois = mantaray_core::RoiSet::default();

    let mut app = App::headless();
    // No library ships, so one is loaded before anything identifies against it.
    app.library = mantaray_core::NuclideLibrary::sample_for_tests();
    app.open_buffer("eu152".into(), spectrum, None);
    app.apply_action(Action::AutoCalibrate("Eu-152".into()));

    let recovered = app
        .active_spectrum()
        .and_then(|spectrum| spectrum.energy_calibration.clone())
        .unwrap_or_else(|| panic!("no calibration was fitted: {}", app.status));
    // Judge where it matters: the fitted energies of the main lines.
    for energy in [121.78, 344.28, 778.9, 1408.01] {
        let channel = instrument.channel(energy).expect("in range");
        let difference = (recovered.energy(channel) - instrument.energy(channel)).abs();
        assert!(
            difference < 1.0,
            "at {energy} keV the automatic calibration is {difference:.2} keV \
             from the instrument's: {}",
            app.status
        );
    }
    assert!(
        app.status.contains("calibrated from"),
        "the result should be announced: {}",
        app.status
    );
    // And it can be taken back.
    app.apply_action(Action::Undo);
    assert!(
        app.active_spectrum()
            .is_some_and(|spectrum| spectrum.energy_calibration.is_none()),
        "undo should remove the automatic calibration"
    );
}

#[test]
fn auto_calibration_against_the_wrong_source_is_refused() {
    let fixture = std::path::Path::new(env!("CARGO_MANIFEST_DIR"))
        .join("../mantaray-formats/tests/fixtures/eu152_spectra.Spe");
    if !fixture.is_file() {
        return;
    }
    let mut spectrum = mantaray_formats::load_spectrum(&fixture).expect("load");
    spectrum.energy_calibration = None;
    spectrum.rois = mantaray_core::RoiSet::default();
    let mut app = App::headless();
    // The library is loaded so the refusal below is for the right reason -
    // too few lines to fit - rather than for having nothing to look up.
    app.library = mantaray_core::NuclideLibrary::sample_for_tests();
    app.open_buffer("eu152".into(), spectrum, None);

    // Co-60 has two lines: never enough to calibrate from, so no answer is
    // invented for a source that is not in the spectrum.
    app.apply_action(Action::AutoCalibrate("Co-60".into()));
    assert!(
        app.active_spectrum()
            .is_some_and(|spectrum| spectrum.energy_calibration.is_none()),
        "no calibration should have been applied: {}",
        app.status
    );
    assert!(
        app.status.contains("could not calibrate"),
        "the refusal should be explained: {}",
        app.status
    );
}

#[test]
fn the_plot_can_be_written_as_a_picture() {
    let mut app = with_spectrum(1024);
    app.apply_action(Action::PeakSearch);
    let index = app.active.expect("active");
    let spectrum = app.active_spectrum().cloned().expect("a spectrum");
    let path = scratch("plot.svg");

    app.write_plot_svg(&spectrum, index, &path);

    let svg = std::fs::read_to_string(&path).expect("the picture should exist");
    assert!(svg.starts_with("<svg "));
    assert!(svg.trim_end().ends_with("</svg>"));
    assert!(
        svg.contains("<polyline"),
        "the trace should be in the picture"
    );
    assert!(
        app.status.contains("plot written"),
        "the export should be confirmed: {}",
        app.status
    );
}

#[test]
fn field_mode_stores_spectra_in_the_instrument_and_downloads_them() {
    let mut app = App::headless();

    // Two samples: count, store, count, store.
    for _ in 0..2 {
        app.apply_action(Action::Start);
        app.advance_by(3.0);
        app.apply_action(Action::Stop);
        app.apply_action(Action::StoreSpectrum);
        assert!(
            app.status.contains("stored in the instrument"),
            "{}",
            app.status
        );
    }
    // Storing cleared the live data each time.
    assert_eq!(app.detectors[0].spectrum().total_counts(), 0);
    assert_eq!(app.detectors[0].stored_spectra(), 2);

    let windows_before = app.windows.len();
    app.apply_action(Action::DownloadSpectra);
    assert_eq!(
        app.windows.len(),
        windows_before + 2,
        "each stored spectrum gets a buffer window: {}",
        app.status
    );
    assert!(
        app.windows
            .iter()
            .any(|window| window.title.contains("store 1")),
        "the windows say where they came from"
    );
    // The download copied; the instrument still holds its spectra.
    assert_eq!(app.detectors[0].stored_spectra(), 2);
    assert!(
        app.active_spectrum()
            .is_some_and(|spectrum| spectrum.total_counts() > 0),
        "the downloaded data is real"
    );
}

#[test]
fn a_job_can_walk_the_stored_spectra_with_loop_spectra_and_view() {
    let mut app = App::headless();
    // Put two spectra into instrument memory.
    for _ in 0..2 {
        app.apply_action(Action::Start);
        app.advance_by(2.0);
        app.apply_action(Action::Stop);
        app.apply_action(Action::StoreSpectrum);
    }
    let windows_before = app.windows.len();

    // A field-mode job: LOOP SPECTRA runs once per stored spectrum (the loop
    // counter proves the passes), and VIEW opens them.
    let job = scratch("walk-stores.job");
    std::fs::write(
        &job,
        "LOOP SPECTRA\nDESCRIBE_SAMPLE \"pass ???\"\nEND_LOOP\nVIEW 1\nVIEW 2\n",
    )
    .expect("write the job");
    mantaray_gui::jobs::start(&mut app, &job).expect("the job should parse");
    for _ in 0..100 {
        mantaray_gui::jobs::step(&mut app);
        if app.job.is_none() {
            break;
        }
    }
    assert!(app.job.is_none(), "the job should have finished");
    assert_eq!(
        app.sample.description, "pass 002",
        "LOOP SPECTRA should have run once per stored spectrum"
    );
    assert_eq!(
        app.windows.len(),
        windows_before + 2,
        "VIEW should have opened each stored spectrum: {}",
        app.status
    );
}

#[test]
fn printing_builds_a_document_the_browser_can_print() {
    // SAFETY: test-local env var, read by the code under test.
    unsafe { std::env::set_var("MANTARAY_NO_OPEN", "1") };
    let mut app = with_spectrum(1024);
    // No library ships, so one is loaded before anything identifies against it.
    app.library = mantaray_core::NuclideLibrary::sample_for_tests();
    app.apply_action(Action::PeakSearch);
    let document = app.print_document().expect("a document");
    assert!(document.starts_with("<!DOCTYPE html>"));
    assert!(document.contains("<svg "), "the plot rides along");
    assert!(
        document.contains("Net Area") || document.contains("net area") || document.contains("ROI"),
        "the region report rides along"
    );
    assert!(
        !document.contains("<script"),
        "nothing executable in a report"
    );
    assert!(
        !document.contains("Nuclide"),
        "no analysis has been run, so none is printed"
    );

    // Run the analysis; the same page now carries the activities too.
    app.apply_action(Action::Analyse);
    let document = app.print_document().expect("a document");
    assert!(
        document.to_lowercase().contains("nuclide"),
        "the analysis rides along once it exists"
    );

    app.apply_action(Action::Print);
    assert!(
        app.status.contains("browser"),
        "printing should hand over to the browser: {}",
        app.status
    );
}

#[test]
fn a_job_can_run_a_real_program_and_wait_for_it() {
    let mut app = with_spectrum(64);
    let job = scratch("run.job");
    // A command every platform in CI has.
    let line = if cfg!(windows) {
        "WAIT \"cmd /C exit 0\"\n"
    } else {
        "WAIT \"true\"\n"
    };
    std::fs::write(&job, line).expect("write");
    mantaray_gui::jobs::start(&mut app, &job).expect("parse");
    // The job parks while the program runs and is looked in on once per
    // frame; frames come milliseconds apart, so the steps here must too, or
    // fifty of them pass before the child can so much as exit. The ceiling
    // is CI-sized: on a loaded two-core runner even `true` can wait a while
    // to be scheduled, and a healthy run leaves after a few iterations.
    for _ in 0..400 {
        mantaray_gui::jobs::step(&mut app);
        if app.job.is_none() {
            break;
        }
        std::thread::sleep(std::time::Duration::from_millis(25));
    }
    assert!(app.job.is_none(), "the job should finish: {}", app.status);
    assert!(
        !app.status.to_lowercase().contains("failed"),
        "{}",
        app.status
    );

    // A failing program fails the job, with the exit status in the message.
    let job = scratch("run-fails.job");
    let line = if cfg!(windows) {
        "WAIT \"cmd /C exit 3\"\n"
    } else {
        "WAIT \"false\"\n"
    };
    std::fs::write(&job, line).expect("write");
    mantaray_gui::jobs::start(&mut app, &job).expect("parse");
    for _ in 0..400 {
        mantaray_gui::jobs::step(&mut app);
        if app.job.is_none() {
            break;
        }
        std::thread::sleep(std::time::Duration::from_millis(25));
    }
    assert!(
        app.status.contains("exited with") || app.status.contains("failed"),
        "the failure should surface: {}",
        app.status
    );
}

#[test]
fn a_job_can_maximise_and_restore_the_window() {
    let mut app = with_spectrum(64);
    let job = scratch("zoom.job");
    std::fs::write(&job, "ZOOM 3\n").expect("write");
    mantaray_gui::jobs::start(&mut app, &job).expect("parse");
    for _ in 0..20 {
        mantaray_gui::jobs::step(&mut app);
    }
    assert_eq!(
        app.visible_windows().len(),
        1,
        "ZOOM 3 should leave one window filling the area"
    );

    let job = scratch("zoom-restore.job");
    std::fs::write(&job, "ZOOM 1\n").expect("write");
    mantaray_gui::jobs::start(&mut app, &job).expect("parse");
    for _ in 0..20 {
        mantaray_gui::jobs::step(&mut app);
    }
    assert!(
        app.visible_windows().len() > 1,
        "ZOOM 1 should restore the windows"
    );
}

#[test]
fn a_job_can_optimise_the_amplifier_and_wait_for_it() {
    let mut app = App::headless();
    // Detune the pole zero so the routine has work to do.
    let mut properties = app.detectors[0].properties().clone();
    properties.amplifier.pole_zero = 50;
    app.detectors[0].set_properties(properties).expect("set");
    assert!(app.detectors[0].pole_zero_error().abs() > 0.05);

    let job = scratch("tune.job");
    std::fs::write(&job, "START_OPTIMIZE\nWAIT_AUTO\nSTART_PZ\nWAIT_PZ\n").expect("write");
    mantaray_gui::jobs::start(&mut app, &job).expect("parse");
    for _ in 0..100 {
        mantaray_gui::jobs::step(&mut app);
        if app.job.is_none() {
            break;
        }
    }
    assert!(
        app.job.is_none(),
        "the job should have finished: {}",
        app.status
    );
    assert!(
        app.detectors[0].pole_zero_error().abs() < 1e-9,
        "the routines should have landed the pole zero"
    );
    assert!(!app.detectors[0].optimizing());
    assert!(!app.detectors[0].pole_zeroing());
}

#[test]
fn a_network_instrument_joins_the_pick_list_and_counts() {
    use std::sync::{Arc, Mutex};
    // A simulator served over TCP on a background thread - the stand-in for
    // bench hardware speaking the same dialect.
    let listener = std::net::TcpListener::bind("127.0.0.1:0").expect("bind");
    let address = listener.local_addr().expect("address").to_string();
    let served: Arc<Mutex<mantaray_device::SimulatedMcb>> = Arc::new(Mutex::new(
        mantaray_device::SimulatedMcb::new(42, "FAR-SIDE"),
    ));
    let behind = Arc::clone(&served);
    std::thread::spawn(move || {
        if let Ok((stream, _)) = listener.accept() {
            let _ = mantaray_device::server::serve_connection(stream, behind);
        }
    });

    let mut app = App::headless();
    let before = app.detectors.len();
    app.add_detector(mantaray_device::DetectorEntry {
        number: 42,
        name: "BENCH".into(),
        model: "remote".into(),
        kind: mantaray_device::DetectorKind::Network,
        channels: 0,
        description: address,
        serial: String::new(),
    });
    assert_eq!(
        app.detectors.len(),
        before + 1,
        "the connection should have succeeded: {}",
        app.status
    );
    assert!(app.status.contains("connected"), "{}", app.status);

    // Open its window, start it, and let the far clock run. The application
    // keeps a network instrument on a courier, so its counts arrive when the
    // round trip completes rather than inside `advance_by` - the loop gives
    // that real wall time, bounded so a genuine failure still fails.
    app.apply_action(Action::OpenDetector(before));
    app.apply_action(Action::Start);
    let deadline = std::time::Instant::now() + std::time::Duration::from_secs(5);
    loop {
        {
            let mut instrument = served.lock().unwrap();
            mantaray_device::advance(&mut *instrument, 1.0).expect("far clock");
        }
        app.advance_by(1.0);
        if app
            .active_spectrum()
            .is_some_and(|spectrum| spectrum.total_counts() > 0)
        {
            break;
        }
        assert!(
            std::time::Instant::now() < deadline,
            "counts from the far instrument should be on screen"
        );
        std::thread::sleep(std::time::Duration::from_millis(5));
    }
}

#[test]
fn a_network_instrument_that_drops_mid_count_is_reported_not_fatal() {
    use std::sync::{Arc, Mutex};
    let listener = std::net::TcpListener::bind("127.0.0.1:0").expect("bind");
    let address = listener.local_addr().expect("address").to_string();
    let served: Arc<Mutex<mantaray_device::SimulatedMcb>> = Arc::new(Mutex::new(
        mantaray_device::SimulatedMcb::new(42, "FAR-SIDE"),
    ));
    let behind = Arc::clone(&served);
    // The serving thread hands back a clone of the wire so the test can cut it.
    let (sender, receiver) = std::sync::mpsc::channel();
    std::thread::spawn(move || {
        if let Ok((stream, _)) = listener.accept() {
            let _ = sender.send(stream.try_clone().expect("clone"));
            let _ = mantaray_device::server::serve_connection(stream, behind);
        }
    });

    let mut app = App::headless();
    let before = app.detectors.len();
    app.add_detector(mantaray_device::DetectorEntry {
        number: 42,
        name: "BENCH".into(),
        model: "remote".into(),
        kind: mantaray_device::DetectorKind::Network,
        channels: 0,
        description: address,
        serial: String::new(),
    });
    app.apply_action(Action::OpenDetector(before));
    app.apply_action(Action::Start);
    app.advance_by(1.0);

    // Cut the wire mid-count, the way real bench hardware does.
    let wire = receiver.recv().expect("the served stream");
    wire.shutdown(std::net::Shutdown::Both).expect("shutdown");

    // The session must survive the loss and say which instrument it lost.
    // The courier meets the dead wire on its own thread, so the report
    // arrives on whichever poll collects the failure - bounded, because a
    // report that never comes must still fail loudly here.
    let deadline = std::time::Instant::now() + std::time::Duration::from_secs(5);
    while !app.status.contains("42") {
        app.advance_by(1.0);
        assert!(
            std::time::Instant::now() < deadline,
            "the trouble should name the detector: {:?}",
            app.status
        );
        std::thread::sleep(std::time::Duration::from_millis(5));
    }
}

#[test]
fn removing_a_detector_closes_its_window_and_keeps_the_rest_straight() {
    let mut app = App::headless();
    app.add_detector(mantaray_device::DetectorEntry {
        number: 7,
        name: "SPARE".into(),
        model: "simulated".into(),
        kind: mantaray_device::DetectorKind::Simulated,
        channels: 2048,
        description: String::new(),
        serial: String::new(),
    });
    assert_eq!(app.detectors.len(), 2);

    // A window on each: removing the first must close only its window and
    // re-point the second's at the detector's new position.
    app.apply_action(Action::OpenDetector(0));
    app.apply_action(Action::OpenDetector(1));
    let windows_before = app.windows.len();
    app.apply_action(Action::RemoveDetector(0));

    assert_eq!(app.detectors.len(), 1);
    assert_eq!(app.detectors[0].identity().number, 7);
    assert_eq!(app.windows.len(), windows_before - 1);
    assert!(
        app.windows.iter().any(
            |window| matches!(window.target, mantaray_gui::app::Target::Detector(index) if index == 0)
        ),
        "the surviving window should follow its detector to index 0"
    );
    assert!(
        !app.detector_list
            .entries()
            .iter()
            .any(|entry| entry.number != 7),
        "the pick list should only hold the survivor"
    );
    assert!(app.status.contains("removed"), "{}", app.status);
}

#[test]
fn a_counting_detector_cannot_be_removed_out_from_under_its_count() {
    let mut app = App::headless();
    app.apply_action(Action::OpenDetector(0));
    app.apply_action(Action::Start);
    app.apply_action(Action::RemoveDetector(0));
    assert_eq!(app.detectors.len(), 1, "the count protects the detector");
    assert!(
        app.status.contains("stop"),
        "the refusal should say what to do: {}",
        app.status
    );
}

#[test]
fn the_shipped_build_has_no_simulator() {
    // The test suites need an instrument to drive, so they build with the
    // `simulator` feature on. What must never happen is that feature reaching a
    // default build, where invented counts could be mistaken for measurements.
    // This asserts the manifest itself, which is the thing that would regress.
    let manifest = std::fs::read_to_string(
        std::path::Path::new(env!("CARGO_MANIFEST_DIR")).join("Cargo.toml"),
    )
    .expect("the crate manifest");
    let default = manifest
        .lines()
        .find(|line| line.trim_start().starts_with("default = ["))
        .expect("a default feature list");
    assert!(
        !default.contains("simulator"),
        "the simulator must not be a default feature: {default}"
    );
}

#[test]
fn a_detector_kind_with_no_driver_behind_it_is_refused_not_simulated() {
    // A saved detector list can name a kind this build cannot serve. Handing
    // back a simulator would put fabricated counts on screen under the name of
    // real hardware, which is the one mistake a spectroscopy program must not
    // make. Local is served now - through the 32-bit bridge - but only when the
    // bridge is there, and a test binary has no bridge beside it, so the same
    // rule is what this checks.
    for (kind, expected) in [
        (mantaray_device::DetectorKind::Local, ""),
        (mantaray_device::DetectorKind::FileReplay, "replay"),
    ] {
        let mut app = App::headless();
        let before = app.detectors.len();
        app.add_detector(mantaray_device::DetectorEntry {
            number: 44,
            name: "BENCH".into(),
            model: "926".into(),
            kind,
            channels: 8192,
            // A detector number no configuration has, so this says the same
            // thing on a bench with instruments plugged in as on one without.
            description: "999".into(),
            serial: String::new(),
        });
        assert_eq!(
            app.detectors.len(),
            before,
            "{kind:?} should add nothing at all"
        );
        assert!(
            expected.is_empty() || app.status.to_lowercase().contains(expected),
            "{kind:?} should say what is missing, got: {}",
            app.status
        );
        assert!(
            !app.status.to_lowercase().contains("detector added"),
            "{kind:?} should not report success: {}",
            app.status
        );
    }
}

#[test]
fn renaming_a_detector_reaches_the_list_the_window_and_the_next_save() {
    // A rename that stops at the pick list is only half true: the window
    // title would still say the old name, and so would every spectrum saved
    // from it afterwards. The name is what somebody reads off a file next
    // year, so all three follow.
    let mut app = App::headless();
    let detector = app
        .detectors
        .iter()
        .position(|mcb| mantaray_device::Mcb::identity(mcb).number == 1)
        .expect("the headless session starts with a simulated detector");
    let number = mantaray_device::Mcb::identity(&app.detectors[detector]).number;
    app.apply_action(Action::OpenDetector(detector));

    app.apply_action(Action::RenameDetector(detector, "Bench HPGe".into()));

    assert_eq!(
        mantaray_device::Mcb::identity(&app.detectors[detector]).name,
        "Bench HPGe",
        "the open instrument follows the rename"
    );
    assert_eq!(
        app.detector_list
            .get(number)
            .map(|entry| entry.name.clone()),
        Some("Bench HPGe".to_string()),
        "and so does the saved pick list"
    );
    assert!(
        app.windows
            .iter()
            .any(|window| window.title == "Bench HPGe"),
        "and the window title: {:?}",
        app.windows.iter().map(|w| &w.title).collect::<Vec<_>>()
    );
    assert_eq!(
        mantaray_device::Mcb::spectrum(&app.detectors[detector]).detector_name,
        "Bench HPGe",
        "and what the next saved file will say it came from"
    );

    // An empty name is refused rather than leaving a nameless detector.
    app.apply_action(Action::RenameDetector(detector, "   ".into()));
    assert_eq!(
        mantaray_device::Mcb::identity(&app.detectors[detector]).name,
        "Bench HPGe",
        "an empty name is not a rename"
    );
}

#[test]
fn a_fresh_session_has_no_library_and_says_so_rather_than_showing_nothing() {
    // No nuclide library ships: line energies and emission probabilities
    // belong to whoever evaluated them. An empty analysis table would read as
    // "nothing found", which is a different claim from "nothing to look for".
    let mut app = with_spectrum(1024);
    assert!(
        app.library.is_empty(),
        "a fresh session must start with no library"
    );

    app.apply_action(Action::PeakSearch);
    app.apply_action(Action::Analyse);

    assert!(
        app.analysis.is_none(),
        "an analysis against nothing is not an analysis"
    );
    assert!(
        app.status.to_lowercase().contains("library"),
        "the state has to be named, and where to fix it: {}",
        app.status
    );

    // With one loaded, the same action works.
    app.library = mantaray_core::NuclideLibrary::sample_for_tests();
    app.apply_action(Action::Analyse);
    assert!(app.analysis.is_some(), "{}", app.status);
}

#[test]
fn a_running_count_builds_a_stability_trace_and_clear_starts_it_over() {
    // The readout says what the rate is now; this is what says whether it has
    // been that all along.
    let mut app = App::headless();
    let number = mantaray_device::Mcb::identity(&app.detectors[0]).number;
    app.apply_action(Action::Start);
    for _ in 0..40 {
        app.advance_by(1.0);
    }

    let log = app
        .stability
        .get(&number)
        .expect("the running detector is logged");
    assert!(
        log.len() > 5,
        "forty seconds of counting should leave a trace: {} readings",
        log.len()
    );
    assert!(log.peak_rate() > 0.0, "the simulator is producing counts");
    assert!(
        log.drift().is_some(),
        "enough readings to compare the start against the end"
    );

    // Clearing is a different acquisition; drawing a line across it would be
    // a lie about the data.
    app.apply_action(Action::Clear);
    app.advance_by(1.0);
    assert!(
        app.stability.get(&number).is_none_or(|log| log.len() <= 1),
        "the log should start again after a Clear"
    );
}

#[test]
fn work_in_progress_survives_a_snapshot_and_comes_back() {
    // What the crash on 2026-08-06 would have cost: a recalled file, a
    // smoothed working copy, calibration points half entered. None of it is on
    // disk until somebody saves, and nobody saves before a crash.
    let mut app = with_spectrum(512);
    app.apply_action(Action::MarkMode(MarkMode::Mark));
    app.apply_action(Action::MarkRange(240, 270));
    app.apply_action(Action::Smooth);
    app.apply_action(Action::Marker(256));
    let counts = app
        .active_spectrum()
        .map(|spectrum| spectrum.total_counts())
        .expect("a spectrum");

    let snapshot = app.snapshot_session();
    assert_eq!(
        snapshot.windows.len(),
        1,
        "the buffer window is what exists nowhere else"
    );
    assert!(!snapshot.saved_at.is_empty(), "the prompt quotes when");

    // A fresh session, as after a crash, told to reopen what was found.
    let mut after = App::headless();
    let buffers_before = after
        .windows
        .iter()
        .filter(|window| window.target == Target::Buffer)
        .count();
    after.restore_session(snapshot);

    let restored = after
        .windows
        .iter()
        .find(|window| window.target == Target::Buffer && window.title == "Test")
        .expect("the window comes back under its own name");
    assert_eq!(
        restored.buffer.as_ref().map(|s| s.total_counts()),
        Some(counts),
        "with the smoothed counts, not the file's"
    );
    assert_eq!(
        restored.buffer.as_ref().map(|s| s.rois.len()),
        Some(1),
        "and the regions that were marked"
    );
    assert_eq!(restored.display.marker, 256, "and where the marker was");
    assert!(
        restored.modified,
        "it is unsaved work: coming back as saved would invite closing it"
    );
    assert_eq!(
        after
            .windows
            .iter()
            .filter(|window| window.target == Target::Buffer)
            .count(),
        buffers_before + 1
    );
}

#[test]
fn a_detector_window_is_not_snapshotted() {
    // Instrument memory survives this process dying, and restoring a stale
    // copy of it would put counts on screen that the instrument no longer has.
    let app = App::headless();
    assert!(
        app.windows
            .iter()
            .any(|window| matches!(window.target, Target::Detector(_))),
        "the headless session opens its detector"
    );
    assert!(
        app.snapshot_session().is_empty(),
        "a detector window is a view, not the data"
    );
}

#[test]
fn peak_info_over_a_selection_fits_it_and_marks_nothing() {
    // Drag out a span, right-click, Peak Info: the fit is over what was
    // dragged, and no region is left behind. Measuring a peak should not edit
    // the spectrum's regions as a side effect.
    let mut app = with_spectrum(1024);
    let regions_before = app.active_spectrum().map(|s| s.rois.len()).unwrap_or(0);

    app.apply_action(Action::Select(500, 524));
    app.apply_action(Action::PeakInfoAtMarker);

    let index = app.active.expect("active");
    let info = app.windows[index]
        .peak_info
        .as_ref()
        .expect("peak information over the selection");
    assert!(
        (info.centroid - 512.0).abs() < 3.0,
        "the fit should sit on the peak inside the selection, got {}",
        info.centroid
    );
    assert_eq!(
        app.active_spectrum().map(|s| s.rois.len()),
        Some(regions_before),
        "measuring must not mark a region"
    );
}

#[test]
fn clear_roi_over_a_selection_takes_every_region_in_it() {
    let mut app = with_spectrum(1024);
    app.apply_action(Action::MarkMode(MarkMode::Mark));
    app.apply_action(Action::MarkRange(100, 130));
    app.apply_action(Action::MarkRange(300, 330));
    app.apply_action(Action::MarkRange(700, 730));
    assert_eq!(app.active_spectrum().map(|s| s.rois.len()), Some(3));

    // A span covering the first two, and not the third.
    app.apply_action(Action::Select(90, 400));
    app.apply_action(Action::ClearRoi);

    assert_eq!(
        app.active_spectrum().map(|s| s.rois.len()),
        Some(1),
        "both regions in the selection should go: {}",
        app.status
    );
    assert!(
        app.active_spectrum()
            .is_some_and(|s| s.rois.at(715).is_some()),
        "the region outside the selection stays"
    );

    // And it is one undo step, not one per region.
    app.apply_action(Action::Undo);
    assert_eq!(
        app.active_spectrum().map(|s| s.rois.len()),
        Some(3),
        "a single undo should bring all of them back"
    );
}

#[test]
fn a_local_instrument_without_a_detector_number_says_so() {
    // The description carries the number ORTEC's configuration gave the
    // instrument. Without it there is nothing to open, and guessing one would
    // open somebody else's detector.
    let mut app = App::headless();
    let before = app.detectors.len();
    app.add_detector(mantaray_device::DetectorEntry {
        number: 45,
        name: "BENCH".into(),
        model: "926".into(),
        kind: mantaray_device::DetectorKind::Local,
        channels: 8192,
        description: String::new(),
        serial: String::new(),
    });
    assert_eq!(app.detectors.len(), before);
    assert!(
        app.status.contains("detector number"),
        "the refusal should name what is missing: {}",
        app.status
    );
}

#[test]
fn a_dead_address_is_reported_not_hung() {
    let mut app = App::headless();
    let before = app.detectors.len();
    app.add_detector(mantaray_device::DetectorEntry {
        number: 43,
        name: "NOWHERE".into(),
        model: "remote".into(),
        kind: mantaray_device::DetectorKind::Network,
        channels: 0,
        // A reserved port on localhost: refused immediately.
        description: "127.0.0.1:1".into(),
        serial: String::new(),
    });
    assert_eq!(app.detectors.len(), before, "nothing should be added");
    assert!(
        app.status.contains("could not reach"),
        "the failure should be explained: {}",
        app.status
    );
}

#[test]
fn the_look_and_the_recent_files_survive_a_restart() {
    let real = scratch("survives.chn");
    mantaray_formats::save_spectrum(&Spectrum::new(64), &real).expect("write");
    let mut first = App::headless();
    first.theme = mantaray_gui::theme::Theme::AmberCrt;
    first.colors = first.theme.colors();
    first.recall_path(real.clone());
    first.recall_path(scratch("gone.chn")); // fails to open; not remembered
    let saved = serde_json::to_string(&first.persisted()).expect("serialise");

    // "Restart": a new application restoring the saved settings.
    let mut second = App::headless();
    let persisted: mantaray_gui::app::Persisted =
        serde_json::from_str(&saved).expect("deserialise");
    second.restore(persisted);

    assert_eq!(second.theme, mantaray_gui::theme::Theme::AmberCrt);
    assert_eq!(second.colors, second.theme.colors());
    assert_eq!(second.recent_files(), std::slice::from_ref(&real));

    // A remembered file that has since been deleted is not offered.
    std::fs::remove_file(&real).expect("delete");
    let mut third = App::headless();
    third.restore(serde_json::from_str(&saved).expect("deserialise"));
    assert!(
        third.recent_files().is_empty(),
        "a deleted file should drop off the list"
    );
}

#[test]
fn the_reference_date_parses_both_ways_and_refuses_nonsense() {
    let mut app = App::headless();
    app.set_reference_date("2026-01-15").expect("a date alone");
    assert_eq!(
        app.sample
            .reference_date
            .map(|d| d.format("%Y-%m-%d %H:%M").to_string()),
        Some("2026-01-15 00:00".into())
    );
    app.set_reference_date("2026-01-15 14:30")
        .expect("date and time");
    assert_eq!(
        app.sample
            .reference_date
            .map(|d| d.format("%H:%M").to_string()),
        Some("14:30".into())
    );
    app.set_reference_date("  ").expect("empty clears");
    assert!(app.sample.reference_date.is_none());
    let error = app
        .set_reference_date("yesterday-ish")
        .expect_err("nonsense");
    assert!(error.contains("YYYY-MM-DD"), "{error}");
}

#[test]
fn decay_corrections_change_the_reported_activity_when_asked() {
    // A short-lived nuclide, counted for a while: correcting for decay during
    // the count must raise the start-of-count activity above the average.
    let mut spectrum = Spectrum::new(512);
    spectrum.live_time = 3600.0;
    spectrum.real_time = 3600.0;
    spectrum.energy_calibration = Some(mantaray_core::EnergyCalibration::linear(0.0, 1.0));
    for channel in 0..512 {
        spectrum.channels[channel] = 20;
    }
    // A Gaussian line, not the flat-topped block an earlier version drew:
    // the matched-filter search rejects an eleven-channel rectangle for the
    // same reason it rejects a Compton edge - nothing physical has that
    // shape - and this test is about decay arithmetic, not shape tolerance.
    let sigma = 3.0f64;
    for channel in 0..512usize {
        let z = (channel as f64 - 140.0) / sigma;
        let gaussian =
            22_000.0 / (sigma * (2.0 * std::f64::consts::PI).sqrt()) * (-0.5 * z * z).exp();
        spectrum.channels[channel] += gaussian.round() as u64;
    }
    let mut app = App::headless();
    // A private library with a two-hour half life at 140 keV.
    app.library = mantaray_core::NuclideLibrary::new("test");
    app.library.nuclides.push(mantaray_core::Nuclide::new(
        "Xx-1",
        7200.0,
        vec![mantaray_core::LibraryPeak::new(140.0, 90.0)],
    ));
    app.efficiency = Some(mantaray_core::EfficiencyCalibration::constant(0.1));
    app.open_buffer("decay".into(), spectrum, None);
    app.apply_action(Action::PeakSearch);

    app.apply_action(Action::Analyse);
    let plain = app
        .analysis
        .as_ref()
        .and_then(|report| report.nuclides.first().map(|nuclide| nuclide.activity));
    app.dialogs.correct_during_count = true;
    app.apply_action(Action::Analyse);
    let corrected = app
        .analysis
        .as_ref()
        .and_then(|report| report.nuclides.first().map(|nuclide| nuclide.activity));
    let (plain, corrected) = (plain.expect("plain"), corrected.expect("corrected"));
    assert!(
        corrected > plain * 1.1,
        "an hour of a two-hour half life should raise the corrected activity \
         well above the average: {plain:.1} -> {corrected:.1}"
    );
}

#[test]
fn efficiency_measures_itself_from_a_certified_source_spectrum() {
    // The real Eu-152 fixture standing in for a certified source: every line
    // the library knows that peak search finds becomes an efficiency point,
    // and the fitted curve has the shape germanium really has.
    let fixture = std::path::Path::new(env!("CARGO_MANIFEST_DIR"))
        .join("../mantaray-formats/tests/fixtures/eu152_spectra.Spe");
    if !fixture.is_file() {
        eprintln!("fixture missing - skipping");
        return;
    }
    let mut app = App::headless();
    // No library ships, so one is loaded before anything identifies against it.
    app.library = mantaray_core::NuclideLibrary::sample_for_tests();
    app.recall_path(fixture);

    let points = app
        .efficiency_points_from_source("Eu-152", 10_000.0, 3.0, 0.0)
        .expect("the source's lines should be measurable");
    assert!(
        points.len() >= 4,
        "Eu-152 has many usable lines, got {}",
        points.len()
    );
    assert!(points.iter().all(|point| point.efficiency > 0.0));
    assert!(
        points
            .windows(2)
            .all(|pair| pair[0].energy < pair[1].energy),
        "points come sorted by energy"
    );
    // Above the knee, germanium efficiency falls with energy: the 344 keV
    // point must sit well above the 1408 keV point.
    let at = |energy: f64| {
        points
            .iter()
            .find(|point| (point.energy - energy).abs() < 1.0)
            .map(|point| point.efficiency)
    };
    if let (Some(low), Some(high)) = (at(344.28), at(1408.01)) {
        assert!(
            low > high * 1.5,
            "efficiency should fall with energy: {low:.5} vs {high:.5}"
        );
    }
    // The certificate's uncertainty rides on every point.
    assert!(points.iter().all(|point| point.uncertainty > 0.0));

    // The wrong nuclide is refused with a reason, not a junk curve.
    let error = app
        .efficiency_points_from_source("Co-60", 10_000.0, 3.0, 0.0)
        .expect_err("Co-60 is not in this spectrum");
    assert!(error.contains("lines"), "{error}");
}

#[test]
fn the_peak_card_and_the_report_can_be_copied() {
    let mut app = with_spectrum(1024);
    app.apply_action(Action::PeakSearch);
    app.apply_action(Action::Marker(512));
    app.apply_action(Action::PeakInfoAtMarker);

    app.apply_action(Action::CopyPeakInfo);
    let card = app
        .pending_clipboard
        .take()
        .expect("the card's text should be bound for the clipboard");
    assert!(card.contains("Peak:"), "the card starts with the peak line");
    assert!(card.contains("Net Area:"), "the numbers ride along");
    assert!(
        !card.contains("dismiss"),
        "the on-screen hint stays on the screen"
    );

    app.apply_action(Action::CopyReport);
    let report = app.pending_clipboard.take().expect("the report");
    assert!(report.contains("ROI"), "the region report copied: {report}");

    app.apply_action(Action::CopyCsv);
    let csv = app.pending_clipboard.take().expect("the CSV");
    assert!(
        csv.lines().count() > 1000,
        "every channel goes to the spreadsheet"
    );

    // Without a card open, the copy explains itself instead of copying nothing.
    let index = app.active.expect("active");
    app.windows[index].peak_info = None;
    app.apply_action(Action::CopyPeakInfo);
    assert!(app.pending_clipboard.is_none());
    assert!(app.status.contains("Peak Info"), "{}", app.status);
}

#[test]
fn the_last_spectrum_reopens_at_start_when_asked_to() {
    let path = scratch("reopen-me.chn");
    mantaray_formats::save_spectrum(&Spectrum::new(128), &path).expect("write");

    // A session that opened the file and turned the preference on.
    let mut first = App::headless();
    first.reopen_last = true;
    first.recall_path(path.clone());
    let saved = serde_json::to_string(&first.persisted()).expect("serialise");

    // The next start, with nothing on the command line: the file comes back.
    let mut second = App::headless();
    second.restore(serde_json::from_str(&saved).expect("deserialise"));
    second.maybe_reopen_last(false);
    assert!(
        second
            .windows
            .iter()
            .any(|window| window.title == "reopen-me.chn"),
        "the preference should reopen the file"
    );

    // With a file on the command line, the preference stands aside.
    let mut third = App::headless();
    third.restore(serde_json::from_str(&saved).expect("deserialise"));
    third.maybe_reopen_last(true);
    assert!(
        third.windows.iter().all(|window| !window.is_buffer()),
        "arguments win over the preference"
    );

    // Off by default: nothing reopens.
    let mut fourth = App::headless();
    fourth.recall_path(path);
    let saved_off = serde_json::to_string(&fourth.persisted()).expect("serialise");
    let mut fifth = App::headless();
    fifth.restore(serde_json::from_str(&saved_off).expect("deserialise"));
    fifth.maybe_reopen_last(false);
    assert!(fifth.windows.iter().all(|window| !window.is_buffer()));
}

#[test]
fn regions_that_touch_are_merged_rather_than_stacked() {
    let mut app = with_spectrum(512);
    app.apply_action(Action::MarkRange(100, 150));
    app.apply_action(Action::MarkRange(140, 200));
    let regions = app
        .active_spectrum()
        .map(|spectrum| spectrum.rois.iter().copied().collect::<Vec<Roi>>())
        .unwrap_or_default();
    assert_eq!(regions.len(), 1, "overlapping regions should merge");
    assert_eq!(regions[0], Roi::new(100, 200));
}

#[test]
fn unmarking_removes_only_the_channels_asked_for() {
    let mut app = with_spectrum(512);
    app.apply_action(Action::MarkRange(100, 200));
    app.apply_action(Action::UnmarkRange(150, 160));
    let spectrum = app.active_spectrum().expect("a spectrum");
    assert!(spectrum.rois.at(120).is_some(), "the start should remain");
    assert!(spectrum.rois.at(155).is_none(), "the middle was unmarked");
    assert!(spectrum.rois.at(180).is_some(), "the end should remain");
}

/// The bridge's probe output, as it really comes back from two 926s where one
/// entry in the list points at an instrument that is not plugged in.
const PROBE_OUTPUT: &str = "\
Mcbcio32.dll loaded
UMCBI revision 2305
MIOStartup succeeded

instruments answering now:
  MCB 337: 0926-001

3 detector(s) in the configured list

  1: Rayleigh
      model    0926-001
      channels 8192
      state    idle
      version  \"$F0926-001\"
  2: Low_Background
      <MIOOpenDetector(2) failed: a communication failure (macro -3, micro 0)>
  3: XI MCSp-001 SN 0000
      model    MCSp-001
      channels 65535
      state    idle
";

#[test]
fn a_scan_separates_what_answered_from_what_only_the_list_claims() {
    use mantaray_gui::app::InstrumentScan;
    let scan = InstrumentScan::parse(PROBE_OUTPUT);
    assert_eq!(
        scan.reachable,
        vec![
            (1, "Rayleigh".to_string()),
            (3, "XI MCSp-001 SN 0000".to_string())
        ],
        "the instruments that answered"
    );
    assert_eq!(
        scan.missing,
        vec![(2, "Low_Background".to_string())],
        "the one that is listed but silent"
    );
    assert!(scan.scanned);
    assert!(scan.problem.is_none());
    assert!(
        scan.summary().contains('2') && scan.summary().contains("not answering"),
        "the summary should say something is wrong: {}",
        scan.summary()
    );
}

/// The probe output away from Windows, where the bridge walks the USB bus
/// itself: the same block shape, with the adapter serial standing in for a
/// configured name because there is no configuration to hold one.
const DIRECT_PROBE_OUTPUT: &str = "\
2 adapter(s) on the bus

  1: 0926-001 08134079
      model    0926-001
      channels 8192
      state    counting
  2: 11217584
      <claiming interface 0 of adapter 11217584: permission denied (errno 13)>
";

#[test]
fn a_scan_reads_the_direct_probe_the_same_as_the_windows_one() {
    use mantaray_gui::app::InstrumentScan;
    let scan = InstrumentScan::parse(DIRECT_PROBE_OUTPUT);
    assert_eq!(
        scan.reachable,
        vec![(1, "0926-001 08134079".to_string())],
        "the adapter that answered"
    );
    assert_eq!(
        scan.missing,
        vec![(2, "11217584".to_string())],
        "the adapter that could not be opened"
    );
    assert!(scan.scanned);
    assert!(scan.problem.is_none());
}

#[test]
fn a_scan_that_could_not_run_says_so_rather_than_reporting_nothing() {
    use mantaray_gui::app::InstrumentScan;
    // Nothing found and nothing missing reads as "no instruments here", which
    // is a different thing from "the search itself failed" and would send
    // somebody to check cables that are perfectly fine.
    let scan = InstrumentScan::failed("mantaray-mcb.exe is not beside mantaray");
    assert!(scan.reachable.is_empty() && scan.missing.is_empty());
    assert_eq!(scan.summary(), "mantaray-mcb.exe is not beside mantaray");
    assert!(scan.problem.is_some());
}

#[test]
fn a_machine_with_nothing_plugged_in_is_not_an_error() {
    use mantaray_gui::app::InstrumentScan;
    let scan = InstrumentScan::parse("no instruments answered the local transports\n");
    assert!(scan.scanned, "the search did run");
    assert!(scan.problem.is_none(), "and it did not fail");
    assert_eq!(scan.summary(), "no instruments found");
}

#[test]
fn a_mistyped_calibration_energy_can_be_corrected_in_the_table() {
    // The workflow this is for: three points entered, one energy typed wrong,
    // noticed because its residual is large, corrected in place. Starting the
    // calibration again over one digit is what the table exists to avoid.
    let mut app = with_spectrum(1024);
    for (channel, energy) in [(100usize, 50.0), (500, 2500.0), (900, 450.0)] {
        app.apply_action(Action::Marker(channel));
        app.apply_action(Action::AddCalibrationPoint(energy));
    }
    let index = app.active.expect("active");
    assert_eq!(app.windows[index].calibration.len(), 3);
    let wrong = app
        .active_spectrum()
        .and_then(|spectrum| spectrum.energy_calibration.as_ref())
        .expect("a fit, even a bad one")
        .energy(500.0);
    assert!(
        (wrong - 250.0).abs() > 100.0,
        "the mistyped point should pull the fit off, got {wrong}"
    );

    app.apply_action(Action::EditCalibrationPoint(1, 500.0, 250.0));

    assert_eq!(
        app.windows[index].calibration.len(),
        3,
        "correcting must not lose the point"
    );
    let calibration = app
        .active_spectrum()
        .and_then(|spectrum| spectrum.energy_calibration.as_ref())
        .expect("the corrected fit reached the spectrum");
    for (channel, energy) in [(100.0, 50.0), (500.0, 250.0), (900.0, 450.0)] {
        assert!(
            (calibration.energy(channel) - energy).abs() < 1e-6,
            "channel {channel} should read {energy}, got {}",
            calibration.energy(channel)
        );
    }
}

#[test]
fn a_calibration_point_can_be_dropped_and_the_rest_still_fit() {
    let mut app = with_spectrum(1024);
    for (channel, energy) in [(100usize, 50.0), (500, 2500.0), (900, 450.0)] {
        app.apply_action(Action::Marker(channel));
        app.apply_action(Action::AddCalibrationPoint(energy));
    }
    app.apply_action(Action::RemoveCalibrationPoint(1));
    let index = app.active.expect("active");
    assert_eq!(app.windows[index].calibration.len(), 2);
    let calibration = app
        .active_spectrum()
        .and_then(|spectrum| spectrum.energy_calibration.as_ref())
        .expect("two points still fit");
    assert!((calibration.energy(100.0) - 50.0).abs() < 1e-6);
    assert!((calibration.energy(900.0) - 450.0).abs() < 1e-6);
    assert!(
        app.status.contains("removed"),
        "the status should say so: {}",
        app.status
    );
}

#[test]
fn a_calibration_point_will_not_take_a_number_that_is_not_one() {
    let mut app = with_spectrum(1024);
    app.apply_action(Action::Marker(100));
    app.apply_action(Action::AddCalibrationPoint(50.0));
    app.apply_action(Action::Marker(900));
    app.apply_action(Action::AddCalibrationPoint(450.0));
    let before = app
        .active
        .map(|index| app.windows[index].calibration.points().to_vec());

    app.apply_action(Action::EditCalibrationPoint(0, f64::NAN, 50.0));

    let after = app
        .active
        .map(|index| app.windows[index].calibration.points().to_vec());
    assert_eq!(
        before, after,
        "a point that is not a number changes nothing"
    );
    assert!(
        app.status.contains("two numbers"),
        "and it says why: {}",
        app.status
    );
}

#[test]
fn an_unanswered_offer_of_unfinished_work_is_not_thrown_away() {
    // The data-loss path this closes. A run that crashed leaves a snapshot;
    // the next start offers it back. But a session that has just started has
    // no buffer windows, so the twenty-second autosave would take its
    // "nothing open" branch and delete the snapshot - while the question is
    // still on screen - and Escape would dismiss the question entirely. Either
    // way the work existed nowhere else.
    let mut app = App::headless();
    app.recovered = Some(mantaray_gui::session::Session {
        saved_at: "2026-08-06 21:14".into(),
        windows: vec![],
    });
    app.dialogs.open(mantaray_gui::dialogs::Dialog::Recover);

    assert!(
        !app.may_replace_snapshot(),
        "the snapshot on disk is the offer; it must not be written over"
    );

    // Escape peels dialogs off, but not this one: it is a question with two
    // deliberate answers.
    app.dialogs.close_all();
    assert!(
        app.dialogs.is_open(mantaray_gui::dialogs::Dialog::Recover),
        "Escape must not dismiss the offer of unfinished work"
    );
    assert!(app.recovered.is_some());

    // Answering it releases the snapshot again.
    app.recovered = None;
    assert!(app.may_replace_snapshot());
}

#[test]
fn escape_still_closes_the_dialogs_that_are_only_panels() {
    let mut app = App::headless();
    app.dialogs.open(mantaray_gui::dialogs::Dialog::Calibration);
    app.dialogs.open(mantaray_gui::dialogs::Dialog::Settings);
    app.dialogs.close_all();
    assert!(
        !app.dialogs
            .is_open(mantaray_gui::dialogs::Dialog::Calibration)
    );
    assert!(!app.dialogs.is_open(mantaray_gui::dialogs::Dialog::Settings));
}

/// Settings written under the former name are still found after the rename.
///
/// Moving the file across is only half of it. What eframe stores is a map, and
/// the key inside that map carries the old name too: reading only the new key
/// finds nothing and starts with the defaults - the same loss the move exists
/// to prevent, one step further in. On the machine this was written on the
/// stranded entry held a tuned Midnight palette and four recent files.
///
/// This is the payload shape as it was actually written, colours as arrays and
/// all, because that is what a file from before the rename contains.
#[test]
fn settings_saved_under_the_former_name_are_restored() {
    const SAVED: &str = r#"{"theme":"Midnight","colors":{"background":[9,10,13],
        "foreground":[96,205,255],"roi":[255,158,60],"compare":[87,201,138],
        "composite":[244,164,96],"axes":[130,136,148],"marker":[255,245,200],
        "library":[216,148,255],"view_box":[88,96,112],"panel":[15,17,21],
        "alarm":[248,81,73],"healthy":[63,185,130]},"recent":[],
        "time_scale":1.0,"auto_clear_roi":false,"log_decade_top":false,
        "peak_font":12.0,"default_format":"chn","reopen_last":false}"#;

    let persisted: mantaray_gui::app::Persisted =
        serde_json::from_str(SAVED).expect("a payload written before the rename");
    let mut app = mantaray_gui::app::App::headless();
    app.restore(persisted);

    assert_eq!(
        app.theme,
        mantaray_gui::theme::Theme::Midnight,
        "the chosen theme should survive the rename"
    );
    // Every colour that was saved comes back as it was. The overview is the
    // exception and correctly so: it did not exist when this was written, so it
    // takes a neutral default rather than a value invented on its behalf.
    let midnight = mantaray_gui::theme::SpectrumColors::midnight();
    assert_eq!(
        mantaray_gui::theme::SpectrumColors {
            overview: midnight.overview,
            ..app.colors
        },
        midnight,
        "and the palette with it"
    );
}

/// A spectrum pulled out of the tab strip stays out - across a change of
/// workspace, and across a restart.
///
/// Both were asked in review, and they are different questions. Switching
/// workspace changes what the sidebar shows and nothing about the windows, so
/// nothing there could reset it. A restart is the one that was wrong: the
/// snapshot did not carry the arrangement, so a spectrum deliberately pulled
/// out to sit beside another came back docked, with no way to tell why.
#[test]
fn a_pulled_out_spectrum_stays_out() {
    use mantaray_gui::workspace::Workspace;

    let mut app = mantaray_gui::app::App::headless();
    app.open_buffer("first.Spe".into(), spectrum_with_counts(), None);
    app.open_buffer("second.Spe".into(), spectrum_with_counts(), None);
    let second = app
        .windows
        .iter()
        .find(|window| window.title == "second.Spe")
        .expect("the second spectrum")
        .id;
    app.apply_action(mantaray_gui::app::Action::ToggleFloating(second));

    let floating = |app: &mantaray_gui::app::App, title: &str| {
        app.windows
            .iter()
            .find(|window| window.title == title)
            .map(|window| window.floating)
    };
    assert_eq!(floating(&app, "second.Spe"), Some(true));

    // A change of workspace decides what the sidebar shows, and leaves the
    // arrangement of the spectra alone.
    app.workspace = Workspace::acquisition();
    assert_eq!(floating(&app, "second.Spe"), Some(true));
    app.workspace = Workspace::analysis();
    assert_eq!(
        floating(&app, "second.Spe"),
        Some(true),
        "a workspace should not put a pulled-out spectrum back"
    );

    // And it survives the snapshot a crash leaves behind.
    let session = app.snapshot_session();
    let mut restored = mantaray_gui::app::App::headless();
    restored.restore_session(session);
    assert_eq!(
        floating(&restored, "second.Spe"),
        Some(true),
        "and neither should a restart"
    );
    assert_eq!(
        floating(&restored, "first.Spe"),
        Some(false),
        "while one that was never pulled out stays where it was"
    );
}

/// A spectrum with something in it, so it is worth snapshotting.
fn spectrum_with_counts() -> mantaray_core::Spectrum {
    let mut spectrum = mantaray_core::Spectrum::new(256);
    spectrum.live_time = 10.0;
    spectrum.channels[100] = 500;
    spectrum
}

/// A detector's calibration survives the session, filed under its serial.
///
/// The mirror a detector window shows is rebuilt at every connect, so a
/// calibration made on one used to die with the session - reported from the
/// bench as "calibration is not being saved". It is now remembered against
/// the serial the instrument reports and put back at the next connect,
/// outranking whatever the connect road's configuration carries: the
/// remembered one is what the operator did, on this screen, since.
#[test]
fn a_detector_calibration_comes_back_at_the_next_connect() {
    use std::sync::{Arc, Mutex};
    let listener = std::net::TcpListener::bind("127.0.0.1:0").expect("bind");
    let address = listener.local_addr().expect("address").to_string();
    let served: Arc<Mutex<mantaray_device::SimulatedMcb>> =
        Arc::new(Mutex::new(mantaray_device::SimulatedMcb::new(7, "KEPT")));
    let behind = Arc::clone(&served);
    // Two sessions connect in turn, so the server keeps accepting.
    std::thread::spawn(move || {
        for stream in listener.incoming().flatten() {
            let shared: Arc<Mutex<dyn mantaray_device::Mcb>> = Arc::clone(&behind) as _;
            let _ = mantaray_device::server::serve_connection(stream, shared);
        }
    });
    let entry = || mantaray_device::DetectorEntry {
        number: 7,
        name: "KEPT".into(),
        model: "remote".into(),
        kind: mantaray_device::DetectorKind::Network,
        channels: 0,
        description: address.clone(),
        serial: String::new(),
    };

    // First session: connect and calibrate through the ordinary road -
    // marker on a channel, energy typed in, twice.
    let mut app = App::headless();
    let joined = app.detectors.len();
    app.add_detector(entry());
    assert!(app.status.contains("connected"), "{}", app.status);
    app.apply_action(Action::OpenDetector(joined));
    app.apply_action(Action::Marker(100));
    app.apply_action(Action::AddCalibrationPoint(100.0));
    app.apply_action(Action::Marker(200));
    app.apply_action(Action::AddCalibrationPoint(200.0));
    assert!(
        app.active_spectrum()
            .and_then(|spectrum| spectrum.energy_calibration.clone())
            .is_some(),
        "two points make a line: {}",
        app.status
    );
    assert_eq!(
        app.remembered_calibrations.len(),
        1,
        "the calibration is filed under the instrument's serial"
    );
    assert_eq!(app.remembered_calibrations[0].serial, "SIM-0007");

    // Second session: built from what the first would persist, connecting to
    // the same instrument. The first session ends first - the server accepts
    // one client at a time, as the real one does.
    let persisted = app.persisted();
    drop(app);
    let mut next = App::headless();
    next.restore(persisted);
    let joined = next.detectors.len();
    next.add_detector(entry());
    assert!(next.status.contains("connected"), "{}", next.status);
    next.apply_action(Action::OpenDetector(joined));
    let calibration = next
        .active_spectrum()
        .and_then(|spectrum| spectrum.energy_calibration.clone())
        .expect("the remembered calibration is back on the mirror");
    assert!(
        (calibration.coefficients[1] - 1.0).abs() < 1e-9,
        "the gain the operator fitted: {:?}",
        calibration.coefficients
    );
    assert!((calibration.coefficients[0]).abs() < 1e-9);
}

/// A calibration held in anything but keV is refused a `.Clb`, by name.
///
/// The format stores no units and reading assumes keV, so writing a MeV
/// calibration would come back a thousand times wrong with nothing to say
/// so. The units are a free-text field, so this cannot be converted away -
/// only refused, before the save dialog is ever reached, which is also what
/// lets this test run headless.
#[test]
fn a_calibration_not_in_kev_is_refused_a_clb_file() {
    let mut app = App::headless();
    let mut spectrum = spectrum_with_counts();
    spectrum.energy_calibration = Some(mantaray_core::EnergyCalibration::new(
        [0.0, 0.001, 0.0],
        "MeV",
    ));
    app.open_buffer("mev.Spe".into(), spectrum, None);
    app.apply_action(Action::SaveCalibration);
    assert!(
        app.status.contains("keV") && app.status.contains("MeV"),
        "the refusal names both units: {}",
        app.status
    );
}
