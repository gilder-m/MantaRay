//! Whole frames, drawn without a window.
//!
//! egui can run headless, so these render the real interface - menus, toolbar,
//! sidebar, spectrum windows, dialogs and the in-plot peak information - and fail
//! if any of it panics or draws nothing. Every earlier drawing fault in this
//! program was of that kind: a borrow conflict, a clashing widget id, a rectangle
//! with a negative size.

use ortseam_core::Spectrum;
use ortseam_gui::app::{Action, App};
use ortseam_gui::dialogs::Dialog;
use ortseam_gui::view::MarkMode;
use ortseam_gui::viewmodel::{FillMode, VerticalScale};

/// Draws one frame at a given size and returns what was painted.
fn frame(app: &mut App, ctx: &egui::Context, size: [f32; 2]) -> egui::FullOutput {
    let input = egui::RawInput {
        screen_rect: Some(egui::Rect::from_min_size(
            egui::Pos2::ZERO,
            egui::vec2(size[0], size[1]),
        )),
        ..Default::default()
    };
    // `run_ui` hands over the root Ui, which is exactly what eframe gives the
    // application, so the test draws the same thing the window would.
    ctx.run_ui(input, |ui| app.draw(ui))
}

/// How many shapes a frame painted; a blank frame paints almost nothing.
fn shapes(output: &egui::FullOutput) -> usize {
    output
        .shapes
        .iter()
        .map(|clipped| match &clipped.shape {
            egui::Shape::Vec(inner) => inner.len(),
            _ => 1,
        })
        .sum()
}

/// Every piece of text a frame painted, concatenated.
fn painted_text(output: &egui::FullOutput) -> String {
    fn collect(shape: &egui::Shape, into: &mut String) {
        match shape {
            egui::Shape::Text(text) => {
                into.push_str(text.galley.text());
                into.push('\n');
            }
            egui::Shape::Vec(inner) => {
                for shape in inner {
                    collect(shape, into);
                }
            }
            _ => {}
        }
    }
    let mut text = String::new();
    for clipped in &output.shapes {
        collect(&clipped.shape, &mut text);
    }
    text
}

/// A spectrum with a peak, a calibration and regions: the interesting case.
fn realistic(channels: usize) -> Spectrum {
    let mut spectrum = Spectrum::new(channels);
    spectrum.live_time = 300.0;
    spectrum.real_time = 315.0;
    spectrum.energy_calibration = Some(ortseam_core::EnergyCalibration::linear(0.5, 0.36));
    for channel in 0..channels {
        // A falling continuum, so the log scale has something to show.
        spectrum.channels[channel] = 500 - (400 * channel as u64 / channels as u64).min(400);
    }
    for centre in [channels / 4, channels / 2, 3 * channels / 4] {
        for offset in -12i64..=12 {
            let value = (30_000.0 * (-0.5 * (offset as f64 / 3.0).powi(2)).exp()) as u64;
            spectrum.channels[(centre as i64 + offset) as usize] += value;
        }
    }
    spectrum
}

fn with_data() -> App {
    let mut app = App::headless();
    app.open_buffer("frames.Spe".into(), realistic(4096), None);
    app
}

#[test]
fn an_empty_session_draws() {
    let ctx = egui::Context::default();
    let mut app = App::headless();
    let output = frame(&mut app, &ctx, [1280.0, 800.0]);
    assert!(shapes(&output) > 20, "the interface drew almost nothing");
}

#[test]
fn a_spectrum_window_draws_the_trace() {
    let ctx = egui::Context::default();
    let mut app = with_data();
    // Two frames: the first lays the window out, the second draws into it.
    frame(&mut app, &ctx, [1280.0, 800.0]);
    let output = frame(&mut app, &ctx, [1280.0, 800.0]);
    assert!(
        shapes(&output) > 200,
        "a 4096-channel spectrum should paint a lot of shapes, got {}",
        shapes(&output)
    );
}

#[test]
fn every_scale_and_fill_combination_draws() {
    let ctx = egui::Context::default();
    let mut app = with_data();
    let index = app.active.expect("active");
    for scale in [
        VerticalScale::Logarithmic,
        VerticalScale::Automatic,
        VerticalScale::Linear(50_000),
    ] {
        for fill in [FillMode::FillAll, FillMode::FillRoi, FillMode::Points] {
            app.windows[index].display.vertical = scale;
            app.windows[index].display.fill = fill;
            let output = frame(&mut app, &ctx, [1280.0, 800.0]);
            assert!(
                shapes(&output) > 20,
                "{scale:?} with {fill:?} drew almost nothing"
            );
        }
    }
}

#[test]
fn the_peak_information_card_draws_over_the_peak() {
    let ctx = egui::Context::default();
    let mut app = with_data();
    app.apply_action(Action::PeakSearch);
    app.apply_action(Action::Marker(2048));
    app.apply_action(Action::PeakInfoAtMarker);
    let index = app.active.expect("active");
    assert!(
        app.windows[index].peak_info.is_some(),
        "there should be something to draw: {}",
        app.status
    );
    frame(&mut app, &ctx, [1280.0, 800.0]);
    let with_card = shapes(&frame(&mut app, &ctx, [1280.0, 800.0]));

    app.windows[index].peak_info = None;
    let without_card = shapes(&frame(&mut app, &ctx, [1280.0, 800.0]));
    assert!(
        with_card > without_card,
        "the card should add to what is drawn: {with_card} vs {without_card}"
    );
}

#[test]
fn a_maximised_window_draws_at_any_size() {
    let ctx = egui::Context::default();
    let mut app = with_data();
    let index = app.active.expect("active");
    let id = app.windows[index].id;
    app.apply_action(Action::ToggleMaximize(id));
    // Deliberately awkward sizes, including one small enough to squeeze the plot.
    for size in [[1920.0, 1080.0], [1280.0, 800.0], [900.0, 600.0]] {
        let output = frame(&mut app, &ctx, size);
        assert!(shapes(&output) > 20, "nothing drawn at {size:?}");
    }
}

#[test]
fn a_window_too_small_for_the_plot_still_draws() {
    // The plot has gutters for its axes; at some point they no longer fit, and
    // that must not produce a rectangle with a negative size.
    let ctx = egui::Context::default();
    let mut app = with_data();
    for size in [[420.0, 320.0], [200.0, 200.0], [120.0, 100.0]] {
        frame(&mut app, &ctx, size);
    }
}

#[test]
fn an_empty_spectrum_draws_its_invitation_instead_of_a_trace() {
    let ctx = egui::Context::default();
    let mut app = App::headless();
    app.open_buffer("empty".into(), Spectrum::new(0), None);
    let output = frame(&mut app, &ctx, [1280.0, 800.0]);
    assert!(shapes(&output) > 10);
}

#[test]
fn several_windows_and_a_comparison_draw_together() {
    let ctx = egui::Context::default();
    let mut app = with_data();
    app.open_buffer("second".into(), realistic(1024), None);
    app.open_buffer("third".into(), realistic(512), None);
    let index = app.active.expect("active");
    app.windows[index].compare = Some(realistic(512));
    app.windows[index].compare_offset = 200;
    let output = frame(&mut app, &ctx, [1600.0, 900.0]);
    assert!(shapes(&output) > 200);
}

#[test]
fn marking_modes_and_a_selection_draw() {
    let ctx = egui::Context::default();
    let mut app = with_data();
    let index = app.active.expect("active");
    app.windows[index].selection = Some((1000, 1400));
    for mode in [MarkMode::Off, MarkMode::Mark, MarkMode::UnMark] {
        app.apply_action(Action::MarkMode(mode));
        let output = frame(&mut app, &ctx, [1280.0, 800.0]);
        assert!(shapes(&output) > 20, "{mode:?} drew almost nothing");
    }
}

#[test]
fn isotope_markers_draw_from_the_working_library() {
    let ctx = egui::Context::default();
    let mut app = with_data();
    app.apply_action(Action::ToggleIsotopeMarkers);
    let output = frame(&mut app, &ctx, [1280.0, 800.0]);
    assert!(shapes(&output) > 20);
}

#[test]
fn every_dialog_opens_and_draws() {
    // A dialog that panics when opened is a dialog nobody can use. This walks the
    // whole set, with a spectrum open so the ones that need data have some.
    let ctx = egui::Context::default();
    let mut app = with_data();
    app.apply_action(Action::PeakSearch);
    for dialog in Dialog::all() {
        app.dialogs.open(*dialog);
        let output = frame(&mut app, &ctx, [1400.0, 900.0]);
        assert!(shapes(&output) > 10, "{dialog:?} drew almost nothing");
        app.dialogs.set(*dialog, false);
    }
}

#[test]
fn every_dialog_opens_and_draws_with_nothing_loaded() {
    // The same walk with no spectrum at all: dialogs must cope with an empty
    // session rather than assuming there is data to describe.
    let ctx = egui::Context::default();
    let mut app = App::headless();
    app.apply_action(Action::CloseWindow(app.windows[0].id));
    for dialog in Dialog::all() {
        app.dialogs.open(*dialog);
        frame(&mut app, &ctx, [1400.0, 900.0]);
        app.dialogs.set(*dialog, false);
    }
}

#[test]
fn every_theme_draws() {
    let ctx = egui::Context::default();
    let mut app = with_data();
    for theme in ortseam_gui::theme::Theme::all() {
        app.theme = *theme;
        app.colors = theme.colors();
        let output = frame(&mut app, &ctx, [1280.0, 800.0]);
        assert!(shapes(&output) > 100, "{:?} drew too little", theme.label());
    }
}

#[test]
fn a_frame_while_counting_draws_and_asks_to_be_drawn_again() {
    let ctx = egui::Context::default();
    let mut app = App::headless();
    app.apply_action(Action::Start);
    app.advance_by(2.0);
    let output = frame(&mut app, &ctx, [1280.0, 800.0]);
    assert!(shapes(&output) > 20);
    assert!(
        output
            .viewport_output
            .values()
            .any(|viewport| viewport.repaint_delay.as_millis() < 1000),
        "a counting session should keep repainting"
    );
}

#[test]
fn a_hundred_frames_of_a_running_count_stay_healthy() {
    // Long enough to fill regions, trip presets and scroll the view: the sort of
    // sustained running that shook out the earlier layout faults.
    let ctx = egui::Context::default();
    let mut app = App::headless();
    app.apply_action(Action::Start);
    for step in 0..100 {
        app.advance_by(0.5);
        if step == 20 {
            app.apply_action(Action::PeakSearch);
        }
        if step == 40 {
            app.apply_action(Action::ZoomIn);
            app.apply_action(Action::PeakInfoAtMarker);
        }
        if step == 60 {
            app.apply_action(Action::CopyToBuffer);
        }
        if step == 80 {
            app.apply_action(Action::MaximizeActive);
        }
        frame(&mut app, &ctx, [1280.0, 800.0]);
    }
    assert!(!app.windows.is_empty());
    assert!(!app.visible_windows().is_empty());
}

/// Where a spectrum window ended up on screen, from egui's own memory.
fn window_rect(ctx: &egui::Context, app: &App, index: usize) -> Option<egui::Rect> {
    let id = egui::Id::new(("spectrum", app.windows[index].id));
    ctx.memory(|memory| memory.area_rect(id))
}

#[test]
fn tiled_windows_share_the_area_without_overlapping() {
    let ctx = egui::Context::default();
    let mut app = with_data();
    app.open_buffer("second".into(), realistic(1024), None);
    app.open_buffer("third".into(), realistic(512), None);
    frame(&mut app, &ctx, [1600.0, 900.0]);

    let expected = app.visible_windows().len();
    for action in [Action::TileHorizontally, Action::TileVertically] {
        app.apply_action(action.clone());
        frame(&mut app, &ctx, [1600.0, 900.0]);
        let rects: Vec<egui::Rect> = app
            .visible_windows()
            .into_iter()
            .filter_map(|index| window_rect(&ctx, &app, index))
            .collect();
        assert_eq!(rects.len(), expected, "every window should be placed");
        for (i, a) in rects.iter().enumerate() {
            for b in rects.iter().skip(i + 1) {
                assert!(
                    !a.shrink(1.0).intersects(b.shrink(1.0)),
                    "{action:?} left windows overlapping: {a:?} vs {b:?}"
                );
            }
        }
    }
}

#[test]
fn tiling_leaves_full_area_mode() {
    let ctx = egui::Context::default();
    let mut app = with_data();
    app.open_buffer("second".into(), realistic(256), None);
    app.apply_action(Action::MaximizeActive);
    assert_eq!(app.visible_windows().len(), 1);

    app.apply_action(Action::TileVertically);
    frame(&mut app, &ctx, [1400.0, 900.0]);
    assert!(
        app.visible_windows().len() > 1,
        "tiling should bring every window back"
    );
}

#[test]
fn cascaded_windows_stagger_and_stay_inside_the_area() {
    let ctx = egui::Context::default();
    let mut app = with_data();
    app.open_buffer("second".into(), realistic(1024), None);
    frame(&mut app, &ctx, [1400.0, 900.0]);
    let expected = app.visible_windows().len();
    app.apply_action(Action::Cascade);
    frame(&mut app, &ctx, [1400.0, 900.0]);
    let rects: Vec<egui::Rect> = app
        .visible_windows()
        .into_iter()
        .filter_map(|index| window_rect(&ctx, &app, index))
        .collect();
    assert_eq!(rects.len(), expected);
    assert_ne!(rects[0].min, rects[1].min, "cascade should stagger");
    let screen = egui::Rect::from_min_size(egui::Pos2::ZERO, egui::vec2(1400.0, 900.0));
    for rect in &rects {
        assert!(
            screen.contains_rect(*rect),
            "a cascaded window left the screen: {rect:?}"
        );
    }
}

#[test]
fn a_tiled_window_can_still_be_moved_afterwards() {
    // The tile is applied for one frame only; afterwards the window must be an
    // ordinary, draggable window, not one pinned in place.
    let ctx = egui::Context::default();
    let mut app = with_data();
    app.open_buffer("second".into(), realistic(256), None);
    frame(&mut app, &ctx, [1400.0, 900.0]);
    app.apply_action(Action::TileVertically);
    frame(&mut app, &ctx, [1400.0, 900.0]);
    // One settled frame after the tile, as there always is in use.
    frame(&mut app, &ctx, [1400.0, 900.0]);
    let before = window_rect(&ctx, &app, 1).expect("placed");
    // Drag the second window's title bar 60 px right.
    let grip = egui::Pos2::new(before.center().x, before.top() + 8.0);
    let to = grip + egui::vec2(60.0, 0.0);
    let press = |pos, pressed| egui::Event::PointerButton {
        pos,
        button: egui::PointerButton::Primary,
        pressed,
        modifiers: egui::Modifiers::default(),
    };
    for events in [
        vec![egui::Event::PointerMoved(grip), press(grip, true)],
        vec![egui::Event::PointerMoved(to)],
        vec![egui::Event::PointerMoved(to), press(to, false)],
    ] {
        let input = egui::RawInput {
            screen_rect: Some(egui::Rect::from_min_size(
                egui::Pos2::ZERO,
                egui::vec2(1400.0, 900.0),
            )),
            events,
            ..Default::default()
        };
        let _ = ctx.run_ui(input, |ui| app.draw(ui));
    }
    let after = window_rect(&ctx, &app, 1).expect("still there");
    assert!(
        (after.left() - before.left()) > 30.0,
        "the window should have moved: {} -> {}",
        before.left(),
        after.left()
    );
}

#[test]
fn the_list_mode_replay_plays_the_acquisition_back() {
    // A list-mode file whose events arrive over 100 seconds, one per second.
    let mut list = ortseam_formats::ListModeFile::new(64);
    for second in 0..100 {
        list.push(second as f64, (second % 64) as u32);
    }
    list.real_time = 100.0;
    list.live_time = 100.0;
    let path = std::env::temp_dir().join("ortseam-replay-test.lis");
    std::fs::write(
        &path,
        ortseam_formats::list_mode::write(&list).expect("bytes"),
    )
    .expect("write");

    let ctx = egui::Context::default();
    let mut app = App::headless();
    app.recall_path(path);
    let index = app.active.expect("the file opened");
    assert!(
        app.windows[index].list.is_some(),
        "the window should carry the events"
    );
    app.dialogs.open(Dialog::ListRange);
    app.dialogs.list_start = 0.0;
    app.dialogs.list_duration = 10.0;
    app.dialogs.list_speed = 50.0;
    app.dialogs.list_playing = true;

    // Let the replay run a few frames and watch the window move through time.
    frame(&mut app, &ctx, [1400.0, 900.0]);
    let early = app.dialogs.list_start;
    for _ in 0..20 {
        frame(&mut app, &ctx, [1400.0, 900.0]);
    }
    assert!(
        app.dialogs.list_start > early,
        "playing should move the window start forward"
    );
    let counts = app.windows[index]
        .buffer
        .as_ref()
        .map(|spectrum| spectrum.total_counts())
        .unwrap_or(0);
    assert!(
        counts > 0 && counts <= 11,
        "the window should hold one slice of about ten events, got {counts}"
    );
    // The replay parks at the end rather than running off it.
    for _ in 0..300 {
        frame(&mut app, &ctx, [1400.0, 900.0]);
    }
    assert!(
        !app.dialogs.list_playing,
        "the replay should stop at the end"
    );
    assert!(app.dialogs.list_start <= 90.0 + 1e-6);
}

#[test]
fn an_empty_buffer_does_not_claim_a_view_width() {
    let ctx = egui::Context::default();
    let mut app = App::headless();
    app.apply_action(Action::OpenBuffer);
    let output = frame(&mut app, &ctx, [1280.0, 800.0]);
    let text = painted_text(&output);
    // The zoom width clamps to eight channels, but a spectrum with no channels
    // has no width to report - saying "8 ch" next to a header reading "0 ch"
    // is the toolbar contradicting the window.
    assert!(
        !text.contains("8 ch"),
        "the toolbar should not invent a view width for an empty buffer:\n{text}"
    );
    assert!(text.contains("- ch"), "it should say so plainly:\n{text}");
}

#[test]
fn a_monkey_hammering_actions_cannot_crash_the_session() {
    for seed in [0x5EED_CAFE_u64, 0xDEAD_BEEF, 0x0BAD_F00D] {
        monkey_round(seed);
    }
}

/// One deterministic monkey round; a failure names its seed, and the seed
/// replays the same run exactly.
fn monkey_round(seed: u64) {
    use ortseam_gui::dialogs::Dialog;
    let ctx = egui::Context::default();
    let mut app = with_data();
    let mut state = seed;
    let mut random = move || {
        // xorshift64: deterministic, so a failure replays exactly.
        state ^= state << 13;
        state ^= state >> 7;
        state ^= state << 17;
        state
    };
    for step in 0..1000_u64 {
        let value = random();
        let index = (value >> 32) as usize;
        let channel = index % 10_000; // sometimes past the spectrum's end
        let action = match value % 43 {
            0 => Action::Marker(channel),
            1 => Action::ZoomTo(channel, (value % 12_000) as usize),
            2 => Action::ZoomAbout(channel, if value & 1 == 0 { 1 } else { -1 }),
            3 => Action::ZoomIn,
            4 => Action::ZoomOut,
            5 => Action::GotoEnergy(match value % 5 {
                0 => f64::NAN,
                1 => f64::INFINITY,
                2 => -1.0e9,
                3 => 0.0,
                _ => (value % 3000) as f64,
            }),
            6 => Action::MarkRange(channel, (value % 12_000) as usize),
            7 => Action::UnmarkRange((value % 12_000) as usize, channel),
            8 => Action::MarkMode(match value % 3 {
                0 => MarkMode::Off,
                1 => MarkMode::Mark,
                _ => MarkMode::UnMark,
            }),
            9 => Action::MarkPeak,
            10 => Action::ClearRoi,
            11 => Action::ClearAllRois,
            12 => Action::PeakSearch,
            13 => Action::PeakInfoAtMarker,
            14 => Action::InputCountRate,
            15 => Action::Sum,
            16 => Action::Smooth,
            17 => Action::Analyse,
            18 => Action::AutoCalibrate(if value & 1 == 0 {
                "Eu-152".into()
            } else {
                "Nonesuch-999".into()
            }),
            19 => Action::DestroyCalibration,
            20 => Action::Undo,
            21 => Action::Redo,
            22 => Action::Start,
            23 => Action::Stop,
            24 => Action::Clear,
            25 => Action::CopyToBuffer,
            26 => Action::StoreSpectrum,
            27 => Action::DownloadSpectra,
            28 => Action::StartOptimize,
            29 => Action::StartPoleZero,
            30 => Action::OpenDetector(index % 3),
            31 => Action::OpenBuffer,
            32 => Action::CloseWindow(index),
            33 => Action::ForceCloseWindow(index % 8),
            34 => Action::Activate(index % 8),
            35 => Action::TileHorizontally,
            36 => Action::TileVertically,
            37 => Action::Cascade,
            38 => Action::ToggleMaximize(index % 8),
            39 => Action::CycleWindow(if value & 1 == 0 { 1 } else { -1 }),
            40 => Action::Show(match value % 6 {
                0 => Dialog::Analysis,
                1 => Dialog::Calibration,
                2 => Dialog::Strip,
                3 => Dialog::Efficiency,
                4 => Dialog::Qa,
                _ => Dialog::DetectorList,
            }),
            41 => Action::CopyReport,
            _ => Action::ToggleLog,
        };
        app.apply_action(action);
        if step % 25 == 0 {
            app.advance_by(0.5);
            frame(&mut app, &ctx, [1280.0, 800.0]);
        }
    }
    // Still alive, and still able to draw a full frame.
    let output = frame(&mut app, &ctx, [1280.0, 800.0]);
    assert!(
        shapes(&output) > 20,
        "the survivor of seed {seed:#x} should still draw"
    );
}

#[test]
fn every_real_fixture_recalls_and_draws() {
    let ctx = egui::Context::default();
    let fixtures =
        std::path::Path::new(env!("CARGO_MANIFEST_DIR")).join("../ortseam-formats/tests/fixtures");
    let mut app = App::headless();
    let mut opened = 0;
    for entry in std::fs::read_dir(&fixtures).expect("fixtures directory") {
        let path = entry.expect("entry").path();
        let is_spe = path
            .extension()
            .and_then(|extension| extension.to_str())
            .map(|extension| extension.eq_ignore_ascii_case("spe"))
            .unwrap_or(false);
        if !is_spe {
            continue;
        }
        app.recall_path(path.clone());
        opened += 1;
        frame(&mut app, &ctx, [1600.0, 900.0]);
        let output = frame(&mut app, &ctx, [1600.0, 900.0]);
        assert!(
            shapes(&output) > 50,
            "{} drew almost nothing",
            path.display()
        );
    }
    assert!(
        opened >= 25,
        "expected the whole fixture library, found {opened}"
    );
}

/// A frame whose viewport has been asked to close (the title-bar ✕).
fn frame_with_close_request(app: &mut App, ctx: &egui::Context) -> egui::FullOutput {
    let mut viewports = egui::ViewportIdMap::default();
    viewports.insert(
        egui::ViewportId::ROOT,
        egui::ViewportInfo {
            events: vec![egui::ViewportEvent::Close],
            ..Default::default()
        },
    );
    let input = egui::RawInput {
        screen_rect: Some(egui::Rect::from_min_size(
            egui::Pos2::ZERO,
            egui::vec2(1280.0, 800.0),
        )),
        viewports,
        ..Default::default()
    };
    ctx.run_ui(input, |ui| app.draw(ui))
}

fn cancels_close(output: &egui::FullOutput) -> bool {
    output
        .viewport_output
        .get(&egui::ViewportId::ROOT)
        .map(|viewport| {
            viewport
                .commands
                .contains(&egui::ViewportCommand::CancelClose)
        })
        .unwrap_or(false)
}

#[test]
fn the_title_bar_close_asks_about_unsaved_spectra() {
    let ctx = egui::Context::default();
    let mut app = with_data();
    let index = app.active.expect("active");
    app.windows[index].modified = true;

    let output = frame_with_close_request(&mut app, &ctx);
    assert!(
        cancels_close(&output),
        "an unsaved spectrum should hold the door"
    );
    let text = painted_text(&frame(&mut app, &ctx, [1280.0, 800.0]));
    assert!(
        text.contains("Exit ortseam") && text.contains("unsaved"),
        "the exit question should be on screen"
    );

    // Answering Exit lets the next close request straight through.
    app.quit_confirmed = true;
    let output = frame_with_close_request(&mut app, &ctx);
    assert!(!cancels_close(&output), "the answer must be respected");
}

#[test]
fn the_title_bar_close_needs_no_blessing_when_nothing_is_at_risk() {
    let ctx = egui::Context::default();
    let mut app = with_data();
    let output = frame_with_close_request(&mut app, &ctx);
    assert!(
        !cancels_close(&output),
        "nothing unsaved, nothing counting - just close"
    );
}

#[test]
fn a_file_dragged_over_the_window_shows_the_drop_veil() {
    let ctx = egui::Context::default();
    let mut app = with_data();
    frame(&mut app, &ctx, [1280.0, 800.0]);

    let input = egui::RawInput {
        screen_rect: Some(egui::Rect::from_min_size(
            egui::Pos2::ZERO,
            egui::vec2(1280.0, 800.0),
        )),
        hovered_files: vec![egui::HoveredFile {
            path: Some("Eu-152.Spe".into()),
            mime: String::new(),
        }],
        ..Default::default()
    };
    let output = ctx.run_ui(input, |ui| app.draw(ui));
    let text = painted_text(&output);
    assert!(
        text.contains("drop to open Eu-152.Spe"),
        "the veil should name the file"
    );

    // And it disappears once the drag leaves.
    let output = frame(&mut app, &ctx, [1280.0, 800.0]);
    assert!(!painted_text(&output).contains("drop to open"));
}

#[test]
fn an_idle_empty_detector_says_what_to_do_next() {
    let ctx = egui::Context::default();
    let mut app = App::headless();
    frame(&mut app, &ctx, [1280.0, 800.0]);
    let output = frame(&mut app, &ctx, [1280.0, 800.0]);
    assert!(
        painted_text(&output).contains("nothing counted yet"),
        "the blank detector plot should carry the hint"
    );

    // Once a count is running the hint has served its purpose.
    app.apply_action(Action::Start);
    let output = frame(&mut app, &ctx, [1280.0, 800.0]);
    assert!(
        !painted_text(&output).contains("nothing counted yet"),
        "a counting detector must not claim to be idle"
    );
}

#[test]
fn a_recalled_spectrum_never_shows_the_empty_hint() {
    let ctx = egui::Context::default();
    let mut app = with_data();
    // Fill the screen with the recalled spectrum; the idle detector window
    // behind it is allowed to carry the hint, this window is not.
    app.apply_action(Action::MaximizeActive);
    frame(&mut app, &ctx, [1280.0, 800.0]);
    let output = frame(&mut app, &ctx, [1280.0, 800.0]);
    assert!(!painted_text(&output).contains("nothing counted yet"));
}

#[test]
fn the_analysis_table_draws_with_its_nuclide_links() {
    let ctx = egui::Context::default();
    let mut app = with_data();
    app.apply_action(Action::PeakSearch);
    app.apply_action(Action::Analyse);
    assert!(app.analysis.is_some());
    let output = frame(&mut app, &ctx, [1400.0, 900.0]);
    assert!(shapes(&output) > 50, "the filled table should draw");
}

#[test]
fn escape_closes_dialogs_before_touching_the_plot() {
    let ctx = egui::Context::default();
    let mut app = with_data();
    app.apply_action(Action::PeakSearch);
    app.apply_action(Action::Marker(2048));
    app.apply_action(Action::PeakInfoAtMarker);
    app.dialogs.open(Dialog::Settings);
    let index = app.active.expect("active");
    assert!(app.windows[index].peak_info.is_some());

    let escape = |app: &mut App, ctx: &egui::Context| {
        let input = egui::RawInput {
            screen_rect: Some(egui::Rect::from_min_size(
                egui::Pos2::ZERO,
                egui::vec2(1280.0, 800.0),
            )),
            events: vec![egui::Event::Key {
                key: egui::Key::Escape,
                physical_key: None,
                pressed: true,
                repeat: false,
                modifiers: egui::Modifiers::default(),
            }],
            ..Default::default()
        };
        let _ = ctx.run_ui(input, |ui| app.draw(ui));
    };

    // First Escape: the dialog goes, the peak card stays.
    escape(&mut app, &ctx);
    assert!(!app.dialogs.any_open(), "the dialog should have closed");
    assert!(
        app.windows[index].peak_info.is_some(),
        "the plot's overlays wait their turn"
    );

    // Second Escape: now the card goes.
    escape(&mut app, &ctx);
    assert!(app.windows[index].peak_info.is_none());
}

#[test]
fn the_report_window_draws_its_text() {
    let ctx = egui::Context::default();
    let mut app = with_data();
    app.apply_action(Action::PeakSearch);
    app.apply_action(Action::RoiReport);
    assert!(app.report_text.is_some(), "no report: {}", app.status);
    let output = frame(&mut app, &ctx, [1280.0, 800.0]);
    assert!(shapes(&output) > 50);
}
