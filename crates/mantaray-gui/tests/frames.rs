//! Whole frames, drawn without a window.
//!
//! egui can run headless, so these render the real interface - menus, toolbar,
//! sidebar, spectrum windows, dialogs and the in-plot peak information - and fail
//! if any of it panics or draws nothing. Every earlier drawing fault in this
//! program was of that kind: a borrow conflict, a clashing widget id, a rectangle
//! with a negative size.

use mantaray_core::Spectrum;
use mantaray_gui::app::{Action, App};
use mantaray_gui::dialogs::Dialog;
use mantaray_gui::view::MarkMode;
use mantaray_gui::viewmodel::{FillMode, VerticalScale};

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

/// Draws one frame with key events injected, as a keyboard would.
fn frame_with_events(app: &mut App, ctx: &egui::Context, events: Vec<egui::Event>) {
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

/// Text painted inside the spectrum plot, ignoring the sidebar and menus.
///
/// The region list names the same nuclides the plot does, so a test that
/// searched the whole frame would find "Cs-137" whether the plot labels were
/// drawn or not.
fn plot_text(output: &egui::FullOutput) -> String {
    fn collect(shape: &egui::Shape, into: &mut String) {
        match shape {
            egui::Shape::Text(text) if text.pos.x < 900.0 && text.pos.y > 60.0 => {
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

/// The rectangle a piece of text was painted in, if it was painted at all.
fn text_rect(output: &egui::FullOutput, wanted: &str) -> Option<egui::Rect> {
    fn walk(shape: &egui::Shape, wanted: &str, found: &mut Option<egui::Rect>) {
        match shape {
            egui::Shape::Text(text) if found.is_none() && text.galley.text() == wanted => {
                *found = Some(egui::Rect::from_min_size(text.pos, text.galley.size()));
            }
            egui::Shape::Vec(inner) => {
                for shape in inner {
                    walk(shape, wanted, found);
                }
            }
            _ => {}
        }
    }
    let mut found = None;
    for clipped in &output.shapes {
        walk(&clipped.shape, wanted, &mut found);
    }
    found
}

/// Opens a menu and clicks one of its items, the way a hand would.
///
/// Driving the action directly proves the drawing honours it and nothing about
/// whether the menu item is connected to anything. A button wired to the wrong
/// action, or to none, is invisible to every test that skips the menu - so this
/// finds the item by the text actually painted on screen and clicks where it is.
fn click_menu_item(app: &mut App, ctx: &egui::Context, menu: &str, item: &str) {
    fn click_at(app: &mut App, ctx: &egui::Context, at: egui::Pos2) -> egui::FullOutput {
        let input = egui::RawInput {
            screen_rect: Some(egui::Rect::from_min_size(
                egui::Pos2::ZERO,
                egui::vec2(1400.0, 900.0),
            )),
            events: vec![
                egui::Event::PointerMoved(at),
                egui::Event::PointerButton {
                    pos: at,
                    button: egui::PointerButton::Primary,
                    pressed: true,
                    modifiers: egui::Modifiers::default(),
                },
                egui::Event::PointerButton {
                    pos: at,
                    button: egui::PointerButton::Primary,
                    pressed: false,
                    modifiers: egui::Modifiers::default(),
                },
            ],
            ..Default::default()
        };
        ctx.run_ui(input, |ui| app.draw(ui))
    }

    let output = frame(app, ctx, [1400.0, 900.0]);
    let menu_rect =
        text_rect(&output, menu).unwrap_or_else(|| panic!("no {menu:?} menu on screen"));
    // Opening takes a click; the popup is painted on the frame after it.
    let opened = click_at(app, ctx, menu_rect.center());
    let item_rect = text_rect(&opened, item)
        .or_else(|| text_rect(&frame(app, ctx, [1400.0, 900.0]), item))
        .unwrap_or_else(|| panic!("no {item:?} item under the {menu:?} menu"));
    click_at(app, ctx, item_rect.center());
    // One more frame, because an action queued by a click is applied on it.
    frame(app, ctx, [1400.0, 900.0]);
}

/// Scrolls a dialog from the top, gathering everything it paints on the way.
///
/// A dialog is bounded to the screen and scrolls, so what is below the fold is
/// not painted until it is scrolled to - which is the point. Asking "does this
/// dialog offer X" therefore means looking at all of it, not at one frameful.
fn painted_while_scrolling(
    app: &mut App,
    ctx: &egui::Context,
    size: [f32; 2],
    over: egui::Pos2,
) -> String {
    let mut seen = String::new();
    for step in 0..14 {
        let events = if step == 0 {
            vec![egui::Event::PointerMoved(over)]
        } else {
            vec![
                egui::Event::PointerMoved(over),
                egui::Event::MouseWheel {
                    unit: egui::MouseWheelUnit::Point,
                    delta: egui::vec2(0.0, -90.0),
                    modifiers: egui::Modifiers::default(),
                    phase: egui::TouchPhase::Move,
                },
            ]
        };
        let input = egui::RawInput {
            screen_rect: Some(egui::Rect::from_min_size(
                egui::Pos2::ZERO,
                egui::vec2(size[0], size[1]),
            )),
            events,
            ..Default::default()
        };
        let output = ctx.run_ui(input, |ui| app.draw(ui));
        seen.push_str(&painted_text(&output));
    }
    seen
}

/// Scrolls a dialog until a piece of text is on screen, and says where it is.
///
/// The same idea as [`painted_while_scrolling`], but stopping at the thing
/// wanted so that its position is the one it currently occupies - a rectangle
/// from an earlier scroll position would be somewhere else by the time it was
/// clicked.
fn scroll_until(
    app: &mut App,
    ctx: &egui::Context,
    size: [f32; 2],
    over: egui::Pos2,
    wanted: &str,
) -> Option<egui::Rect> {
    for step in 0..14 {
        let mut events = vec![egui::Event::PointerMoved(over)];
        if step > 0 {
            events.push(egui::Event::MouseWheel {
                unit: egui::MouseWheelUnit::Point,
                delta: egui::vec2(0.0, -90.0),
                modifiers: egui::Modifiers::default(),
                phase: egui::TouchPhase::Move,
            });
        }
        let input = egui::RawInput {
            screen_rect: Some(egui::Rect::from_min_size(
                egui::Pos2::ZERO,
                egui::vec2(size[0], size[1]),
            )),
            events,
            ..Default::default()
        };
        let output = ctx.run_ui(input, |ui| app.draw(ui));
        if text_rect(&output, wanted).is_some() {
            // Found - but the wheel's smooth scrolling is still in flight,
            // so the row is not yet where it will come to rest, and a click
            // at the rect just seen would land where the row used to be.
            // Let the animation die away and take the rect where it settles.
            // Growing the dialog by one row is what showed this: the field
            // was found, typed at, and unchanged.
            let mut settled = None;
            for _ in 0..12 {
                let quiet = egui::RawInput {
                    screen_rect: Some(egui::Rect::from_min_size(
                        egui::Pos2::ZERO,
                        egui::vec2(size[0], size[1]),
                    )),
                    ..Default::default()
                };
                let output = ctx.run_ui(quiet, |ui| app.draw(ui));
                settled = text_rect(&output, wanted);
            }
            return settled;
        }
    }
    None
}

/// A context whose animations finish at once.
///
/// A headless frame does not advance the clock, so anything egui animates -
/// a collapsing section opening, most of all - freezes part-way through and
/// stays there however many frames are drawn. Tests that open a section and
/// look inside it need the section open, not opening.
fn still_context() -> egui::Context {
    let ctx = egui::Context::default();
    ctx.all_styles_mut(|style| style.animation_time = 0.0);
    ctx
}

/// A spectrum with a peak, a calibration and regions: the interesting case.
fn realistic(channels: usize) -> Spectrum {
    let mut spectrum = Spectrum::new(channels);
    spectrum.live_time = 300.0;
    spectrum.real_time = 315.0;
    spectrum.energy_calibration = Some(mantaray_core::EnergyCalibration::linear(0.5, 0.36));
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
fn plain_shortcuts_stand_down_while_something_has_the_keyboard() {
    // The bug this pins: pressing Delete to fix a typo in the Calibrate
    // dialog's energy box silently cleared the region under the marker, so
    // Enter then recorded the bare marker channel instead of the region's
    // centroid. Every unmodified key binding must stand down while a widget
    // holds the keyboard.
    let ctx = egui::Context::default();
    let mut app = App::headless();
    let mut spectrum = Spectrum::from_counts(vec![50; 128]);
    spectrum.rois.mark(mantaray_core::Roi::new(40, 60));
    app.open_buffer("ROI".into(), spectrum, None);
    app.apply_action(Action::Marker(50));

    let delete = egui::Event::Key {
        key: egui::Key::Delete,
        physical_key: None,
        pressed: true,
        repeat: false,
        modifiers: egui::Modifiers::NONE,
    };

    // A focused widget - a text field mid-edit - owns the keyboard.
    let field = egui::Id::new("an energy box");
    ctx.memory_mut(|memory| memory.request_focus(field));
    frame_with_events(&mut app, &ctx, vec![delete.clone()]);
    assert_eq!(
        app.active_spectrum().map(|s| s.rois.len()),
        Some(1),
        "Delete while typing must not clear the region under the marker"
    );

    // With the keyboard free again, the same key does its job.
    ctx.memory_mut(|memory| memory.surrender_focus(field));
    frame_with_events(&mut app, &ctx, vec![delete]);
    assert_eq!(
        app.active_spectrum().map(|s| s.rois.len()),
        Some(0),
        "Delete with nothing focused clears the region"
    );
}

#[test]
fn a_preset_being_typed_survives_the_next_frame() {
    // A preset is entered over several frames - tick the box, then type the
    // number - so the edit must outlive the frame it started in. Reading the
    // instrument's own presets back each frame instead un-ticks the box the
    // moment it is ticked, which is what this pins.
    let ctx = egui::Context::default();
    let mut app = App::headless();
    app.apply_action(Action::OpenDetector(0));
    app.dialogs.open(Dialog::McbProperties);
    app.dialogs.properties_tab = 2;

    // The state a tick leaves behind: a preset enabled but not yet applied.
    let typed = mantaray_device::Presets {
        live_time: Some(0.0),
        ..Default::default()
    };
    // Kept by the detector's number, 1 for the headless instrument.
    app.dialogs.presets_edit.insert(1, typed);

    for _ in 0..3 {
        frame(&mut app, &ctx, [1400.0, 900.0]);
        assert_eq!(
            app.dialogs.presets_edit.get(&1),
            Some(&typed),
            "the half-finished preset was erased by a redraw"
        );
    }

    // Applying it is what sends it to the instrument, and ends the edit.
    let applied = mantaray_device::Mcb::set_presets(&mut app.detectors[0], typed);
    assert!(applied.is_ok(), "{applied:?}");
    app.dialogs.presets_edit.clear();
    frame(&mut app, &ctx, [1400.0, 900.0]);
    assert!(
        app.dialogs.presets_edit.is_empty(),
        "what the instrument already holds is not an unsaved edit"
    );
}

#[test]
fn a_bias_edit_belongs_to_its_detector_and_its_dialog() {
    let ctx = egui::Context::default();
    let mut app = App::headless();
    app.apply_action(Action::OpenDetector(0));
    app.dialogs.open(Dialog::McbProperties);
    app.dialogs.properties_tab = 3;

    // Typed for a detector that does not exist: not shown here, not kept.
    app.dialogs.hv_edit.insert(99, 1200.0);
    let output = frame(&mut app, &ctx, [1400.0, 900.0]);
    assert!(!painted_text(&output).contains("not applied yet"));
    assert!(app.dialogs.hv_edit.is_empty());

    // Closing the dialog drops a pending edit.
    app.dialogs.hv_edit.insert(1, 1200.0);
    app.dialogs.set(Dialog::McbProperties, false);
    assert!(app.dialogs.hv_edit.is_empty());
}

#[test]
fn a_constant_efficiency_of_zero_is_refused() {
    // Zero passed for a curve and reported every nuclide found at 0 Bq.
    let ctx = egui::Context::default();
    let mut app = App::headless();
    app.dialogs.open(Dialog::Efficiency);
    app.dialogs.efficiency_constant = 0.0;
    click_text(&mut app, &ctx, "Use");
    assert_eq!(app.efficiency, None);
    assert!(app.status.contains("efficiency of 0"), "{}", app.status);

    app.dialogs.efficiency_constant = 0.05;
    click_text(&mut app, &ctx, "Use");
    assert_eq!(
        app.efficiency,
        Some(mantaray_core::EfficiencyCalibration::constant(0.05))
    );
}

#[test]
fn the_batch_dialog_says_whether_there_is_a_sample_changer() {
    let ctx = egui::Context::default();
    let mut app = App::headless();
    app.dialogs.open(Dialog::Batch);
    let with = painted_text(&frame(&mut app, &ctx, [1400.0, 900.0]));
    assert!(with.contains("changer at position"), "{with}");
    assert!(with.contains("Change sample"));

    app.changer = None;
    let without = painted_text(&frame(&mut app, &ctx, [1400.0, 900.0]));
    assert!(without.contains("no sample changer"), "{without}");
    assert!(
        !without.contains("Change sample"),
        "no button for a changer that is not there"
    );
}

/// Clicks the first piece of painted text that reads exactly `text`, then
/// draws one more frame so whatever the click queued is applied.
fn click_text(app: &mut App, ctx: &egui::Context, text: &str) {
    let output = frame(app, ctx, [1400.0, 900.0]);
    let at = text_rect(&output, text)
        .unwrap_or_else(|| panic!("no {text:?} on screen"))
        .center();
    frame_with_events(
        app,
        ctx,
        vec![
            egui::Event::PointerMoved(at),
            egui::Event::PointerButton {
                pos: at,
                button: egui::PointerButton::Primary,
                pressed: true,
                modifiers: egui::Modifiers::default(),
            },
            egui::Event::PointerButton {
                pos: at,
                button: egui::PointerButton::Primary,
                pressed: false,
                modifiers: egui::Modifiers::default(),
            },
        ],
    );
    frame(app, ctx, [1400.0, 900.0]);
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
    for theme in mantaray_gui::theme::Theme::all() {
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
    let mut list = mantaray_formats::ListModeFile::new(64);
    for second in 0..100 {
        list.push(second as f64, (second % 64) as u32);
    }
    list.real_time = 100.0;
    list.live_time = 100.0;
    let path = std::env::temp_dir().join("mantaray-replay-test.lis");
    std::fs::write(
        &path,
        mantaray_formats::list_mode::write(&list).expect("bytes"),
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
    use mantaray_gui::dialogs::Dialog;
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
            2 => Action::ZoomAbout(channel, if value & 1 == 0 { 0.8 } else { 1.25 }),
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
        std::path::Path::new(env!("CARGO_MANIFEST_DIR")).join("../mantaray-formats/tests/fixtures");
    // The fixtures are real measured spectra, deliberately kept out of git, so
    // a clean checkout has none of them. Skipping out loud is the only honest
    // thing: a green run that drew nothing must not look like one that did.
    let Ok(entries) = std::fs::read_dir(&fixtures) else {
        eprintln!(
            "skipping: no fixture library at {}. The measured spectra are not \
             in git; see crates/mantaray-formats/tests/fixtures/README.md.",
            fixtures.display()
        );
        return;
    };
    let mut app = App::headless();
    let mut opened = 0;
    for entry in entries {
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
    if opened == 0 {
        eprintln!("skipping: the fixture library holds no .Spe spectra");
        return;
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
        text.contains("Exit mantaray") && text.contains("unsaved"),
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
    // No library ships, so an analysis has nothing to identify against until
    // one is loaded - which is the real workflow, and what this table needs.
    app.library = mantaray_core::NuclideLibrary::sample_for_tests();
    app.apply_action(Action::PeakSearch);
    app.apply_action(Action::Analyse);
    assert!(app.analysis.is_some(), "{}", app.status);
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

/// The isotope box: what it says with a nuclide found, and with one that is not.
#[test]
fn the_isotope_check_draws_what_it_found() {
    let ctx = egui::Context::default();
    let mut app = with_data();
    app.library = mantaray_core::NuclideLibrary::sample_for_tests();

    // Before anybody asks, the section is there and says nothing else.
    let text = painted_text(&frame(&mut app, &ctx, [1400.0, 900.0]));
    assert!(
        text.contains("Isotope"),
        "the section should be in the sidebar"
    );

    // Found: the name the library uses, and its half life, whatever was typed.
    app.isotope.typed = "137cs".into();
    app.isotope.resolve(&app.library);
    let output = frame(&mut app, &ctx, [1400.0, 900.0]);
    let text = painted_text(&output);
    assert!(
        text.contains("Cs-137"),
        "the found nuclide should be named:\n{text}"
    );
    assert!(
        text.contains("661.66 keV"),
        "its lines should be listed:\n{text}"
    );
    // And drawn over the spectrum, named there and labelled with how strong
    // each line is - a line that lands on nothing means one thing at 85% and
    // quite another at 0.1%.
    assert!(
        text.contains("85%"),
        "each drawn line should carry its emission probability:\n{text}"
    );
    assert!(
        text.contains("Cs-137 (1 line)"),
        "the plot should name what is drawn on it, and count it:\n{text}"
    );

    // Not found: said plainly, naming the library that was searched.
    app.isotope.typed = "Xe-133".into();
    app.isotope.resolve(&app.library);
    let text = painted_text(&frame(&mut app, &ctx, [1400.0, 900.0]));
    assert!(
        text.contains("Xe-133") && text.contains("is not in"),
        "should say which nuclide is missing from which library:\n{text}"
    );

    // Nonsense: refused as a name, not reported as absent.
    app.isotope.typed = "qq".into();
    app.isotope.resolve(&app.library);
    let text = painted_text(&frame(&mut app, &ctx, [1400.0, 900.0]));
    assert!(
        text.contains("not a nuclide name"),
        "should refuse the name itself:\n{text}"
    );
}

/// A cutoff above every line of the nuclide says so, rather than drawing blank.
#[test]
fn an_isotope_with_no_line_above_the_cutoff_says_so() {
    let ctx = egui::Context::default();
    let mut app = with_data();
    app.library = mantaray_core::NuclideLibrary::sample_for_tests();
    app.isotope.typed = "cs137".into();
    app.isotope.min_intensity = 99.9;
    app.isotope.resolve(&app.library);
    let text = painted_text(&frame(&mut app, &ctx, [1400.0, 900.0]));
    assert!(
        text.contains("no line above"),
        "an empty list should explain itself:\n{text}"
    );
}

/// Without a calibration there is no energy axis to place a line on.
#[test]
fn an_isotope_over_an_uncalibrated_spectrum_says_what_is_missing() {
    let ctx = egui::Context::default();
    let mut app = App::headless();
    let mut spectrum = realistic(4096);
    spectrum.energy_calibration = None;
    app.open_buffer("uncalibrated.Spe".into(), spectrum, None);
    app.library = mantaray_core::NuclideLibrary::sample_for_tests();
    app.isotope.typed = "cs137".into();
    app.isotope.resolve(&app.library);
    let text = painted_text(&frame(&mut app, &ctx, [1400.0, 900.0]));
    assert!(
        text.contains("needs an energy calibration"),
        "should say why no lines are drawn:\n{text}"
    );
}

/// Peak labels can be quieted without losing the markers that place them.
#[test]
fn peak_labels_can_be_turned_off() {
    let ctx = egui::Context::default();
    let mut app = with_data();
    app.library = mantaray_core::NuclideLibrary::sample_for_tests();
    app.apply_action(Action::PeakSearch);

    // On by default: the peaks are named, or at least given their energies.
    let on = painted_text(&frame(&mut app, &ctx, [1400.0, 900.0]));
    let labelled = on.lines().filter(|line| line.contains('.')).count();
    assert!(labelled > 0, "peaks should be named by default:\n{on}");

    // Named specifically: the library recognises these lines, so the labels
    // are nuclide names rather than bare energies.
    assert!(on.contains("Cs-137") || on.contains("Co-60"), "{on}");

    app.apply_action(Action::TogglePeakLabels);
    // Every one of them, not merely fewer. The sidebar's region list names the
    // same nuclides, so the plot is checked on its own.
    let plot_names_off = plot_text(&frame(&mut app, &ctx, [1400.0, 900.0]));
    assert!(
        !plot_names_off.contains("Cs-137") && !plot_names_off.contains("Co-60"),
        "no peak should still be named on the plot:\n{plot_names_off}"
    );
    let plot_names_on = {
        app.apply_action(Action::TogglePeakLabels);
        let text = plot_text(&frame(&mut app, &ctx, [1400.0, 900.0]));
        app.apply_action(Action::TogglePeakLabels);
        text
    };
    assert!(
        plot_names_on.len() > plot_names_off.len(),
        "the plot should carry labels when they are on"
    );

    // The carets are shapes rather than text, so the spectrum stays marked:
    // where a peak was found is a different claim from what it is, and only
    // the second one is being withheld.
    let with_labels_off = shapes(&frame(&mut app, &ctx, [1400.0, 900.0]));
    assert!(with_labels_off > 20, "the regions should still be marked");

    app.apply_action(Action::TogglePeakLabels);
    let back = painted_text(&frame(&mut app, &ctx, [1400.0, 900.0]));
    assert_eq!(back.len(), on.len(), "toggling back should restore them");
}

/// The Display menu's own entries reach the actions they claim to.
///
/// The toggles above are driven by calling the action directly, which proves
/// the drawing honours it and nothing about whether the menu item is wired to
/// anything. A button that pushes no action, or the wrong one, looks identical
/// in every test that skips the menu.
#[test]
fn the_display_menu_toggles_what_it_says_it_does() {
    let ctx = egui::Context::default();
    let mut app = with_data();
    app.apply_action(Action::PeakSearch);

    let labels_before = app.active_display().map(|display| display.peak_labels);
    let markers_before = app.active_display().map(|display| display.isotope_markers);
    assert_eq!(labels_before, Some(true), "peaks are named by default");
    assert_eq!(markers_before, Some(false), "the whole library is not");

    click_menu_item(&mut app, &ctx, "Display", "Peak Labels");
    assert_eq!(
        app.active_display().map(|display| display.peak_labels),
        Some(false),
        "Display / Peak Labels should turn the names off"
    );
    // And it must not have turned anything else off on the way past.
    assert_eq!(
        app.active_display().map(|display| display.isotope_markers),
        Some(false)
    );

    click_menu_item(&mut app, &ctx, "Display", "Isotope Markers");
    assert_eq!(
        app.active_display().map(|display| display.isotope_markers),
        Some(true),
        "Display / Isotope Markers should turn the library lines on"
    );
    assert_eq!(
        app.active_display().map(|display| display.peak_labels),
        Some(false),
        "and must not have put the peak names back"
    );
}

/// Editing a colour has to reach the chrome, not only the plot.
///
/// This is the defect the palette work started from. `apply` took the *preset*
/// and asked it for its colours, so a hand-edited palette reached the trace and
/// stopped there: every selection, hover, link and warning around it stayed the
/// shade the preset had chosen. Worse, the chrome was only re-dressed when the
/// preset changed, so editing a colour re-dressed nothing at all.
#[test]
fn editing_a_colour_redresses_the_interface_around_the_plot() {
    let ctx = egui::Context::default();
    let mut app = with_data();
    frame(&mut app, &ctx, [1400.0, 900.0]);

    // A panel colour nothing would arrive at by accident.
    app.colors.panel = mantaray_gui::theme::Rgb(90, 20, 40);
    frame(&mut app, &ctx, [1400.0, 900.0]);
    assert_eq!(
        ctx.style_of(ctx.theme()).visuals.panel_fill,
        egui::Color32::from_rgb(90, 20, 40),
        "the panels should follow the edited colour"
    );

    // And the accent, which is taken from the spectrum colour, follows it too.
    app.colors.foreground = mantaray_gui::theme::Rgb(255, 0, 255);
    frame(&mut app, &ctx, [1400.0, 900.0]);
    assert_eq!(
        ctx.style_of(ctx.theme()).visuals.hyperlink_color,
        egui::Color32::from_rgb(255, 0, 255),
        "the accent should follow the edited spectrum colour"
    );
}

/// A scheme with light chrome around a dark plot dresses itself correctly.
#[test]
fn light_chrome_around_a_dark_plot_gets_dark_text() {
    let ctx = egui::Context::default();
    let mut app = with_data();
    app.theme = mantaray_gui::theme::Theme::Conductor;
    app.colors = mantaray_gui::theme::SpectrumColors::conductor();
    frame(&mut app, &ctx, [1400.0, 900.0]);

    assert!(
        !ctx.style_of(ctx.theme()).visuals.dark_mode,
        "grey chrome wants dark text, whatever colour the plot behind it is"
    );
    // The plot stays dark while the chrome around it is light: the two are
    // separate decisions, which is the whole point of this scheme.
    assert!(
        app.colors.background.luminance() < 0.05,
        "the plot should still be dark"
    );
    // And a text field belongs to the chrome, not to the plot. Filled from the
    // plot colour - which is what it used to be - every field in the sidebar
    // became a dark box with dark text written on it.
    let field = ctx.style_of(ctx.theme()).visuals.extreme_bg_color;
    let field = mantaray_gui::theme::Rgb(field.r(), field.g(), field.b());
    assert!(
        field.luminance() > 0.5,
        "a text field in light chrome should be light, not {field}"
    );
    assert!(
        field.contrast(app.colors.background) > 4.5,
        "and clearly apart from the plot behind it"
    );
}

/// Every scheme is offered in the dialog, by name.
#[test]
fn the_theme_dialog_offers_every_scheme() {
    let ctx = still_context();
    let mut app = with_data();
    app.apply_action(Action::Show(Dialog::Preferences));
    // Tall enough for the dialog with its colours opened out. Whether it fits
    // a short screen is the business of the test below; this one asks what it
    // offers.
    // The dialog is bounded to the screen and scrolls, so everything it offers
    // is gathered by scrolling through it rather than from one frame.
    let screen = [1400.0, 900.0];
    let text = painted_while_scrolling(&mut app, &ctx, screen, egui::pos2(400.0, 400.0));

    for theme in mantaray_gui::theme::Theme::all() {
        assert!(
            text.contains(theme.label()),
            "{} is missing from the dialog:\n{text}",
            theme.label()
        );
    }
    // Every colour in the palette is editable, including the four that used to
    // be left out - they are the ones a scheme for a particular room needs.
    for role in [
        "Background",
        "Spectrum",
        "Regions",
        "Comparison",
        "Composite",
        "Axes",
        "Marker",
        "Library lines",
        "Overview box",
    ] {
        assert!(text.contains(role), "{role} is not editable:\n{text}");
    }
    // And the colours are shown as hex, which is how a scheme gets shared.
    assert!(
        text.contains(&app.colors.background.to_string()),
        "the hex value should be shown and editable:\n{text}"
    );
}

/// Two named spectra, on top of whatever a bare application already holds.
fn two_spectra() -> App {
    let mut app = App::headless();
    app.open_buffer("first.Spe".into(), realistic(1024), None);
    app.open_buffer("second.Spe".into(), realistic(1024), None);
    app
}

/// The window with a given title. Found by name rather than by position,
/// because a bare application already has one and the indices are not what
/// the order they were opened in suggests.
fn window_id(app: &App, title: &str) -> usize {
    app.windows
        .iter()
        .find(|window| window.title == title)
        .unwrap_or_else(|| panic!("no window called {title:?}"))
        .id
}

/// How many spectra were actually drawn: each one puts a marker readout under
/// its plot, so counting those counts the spectra given room.
fn spectra_drawn(text: &str) -> usize {
    text.lines()
        .filter(|line| line.starts_with("Marker:"))
        .count()
}

/// No dialog can grow tall enough to push its own close button off the screen.
///
/// This happened: the theme dialog gained a style section and a fuller colour
/// list, egui pushed the window up to keep the bottom on screen, and the title
/// bar went off the top with the close button on it. Escape still worked, and
/// nothing else did.
#[test]
fn every_dialog_keeps_its_title_bar_on_screen() {
    let ctx = egui::Context::default();
    let mut app = with_data();
    app.library = mantaray_core::NuclideLibrary::sample_for_tests();
    app.apply_action(Action::PeakSearch);

    // A short screen, which is where a tall dialog goes wrong.
    let screen = [1100.0, 620.0];
    for dialog in Dialog::all() {
        app.dialogs.close_all();
        app.apply_action(Action::Show(*dialog));
        // Two frames: egui measures a window it has not placed before on one
        // and paints it on the next.
        frame(&mut app, &ctx, screen);
        let output = frame(&mut app, &ctx, screen);
        if !app.dialogs.is_open(*dialog) {
            continue;
        }
        let painted: Vec<egui::Rect> = {
            fn walk(shape: &egui::Shape, into: &mut Vec<egui::Rect>) {
                match shape {
                    egui::Shape::Text(text) => {
                        into.push(egui::Rect::from_min_size(text.pos, text.galley.size()))
                    }
                    egui::Shape::Vec(inner) => {
                        for shape in inner {
                            walk(shape, into);
                        }
                    }
                    _ => {}
                }
            }
            let mut found = Vec::new();
            for clipped in &output.shapes {
                walk(&clipped.shape, &mut found);
            }
            found
        };
        // Nothing may be painted off either end of the screen. A dialog taller
        // than the screen runs off the bottom, and egui then slides it up to
        // compensate, taking the title bar and the close button with it.
        for rect in painted {
            assert!(
                rect.top() >= -1.0 && rect.bottom() <= screen[1] + 1.0,
                "{dialog:?} painted at {rect:?}, off a {screen:?} screen"
            );
        }
    }
}

/// A workspace decides which sidebar sections are on screen.
#[test]
fn a_workspace_shows_the_sections_its_job_needs() {
    use mantaray_gui::workspace::Workspace;
    let ctx = egui::Context::default();
    let mut app = with_data();
    app.library = mantaray_core::NuclideLibrary::sample_for_tests();
    app.apply_action(Action::PeakSearch);

    // Nothing is hidden until somebody asks for it to be.
    assert_eq!(app.workspace.name, "Everything");
    let text = painted_text(&frame(&mut app, &ctx, [1400.0, 900.0]));
    for section in ["Pulse Ht. Analysis", "Preset Limits", "Isotope", "Regions"] {
        assert!(text.contains(section), "{section} should be shown:\n{text}");
    }

    // Counting: the clock and what will stop it, without the interpretation.
    app.workspace = Workspace::acquisition();
    let text = painted_text(&frame(&mut app, &ctx, [1400.0, 900.0]));
    assert!(text.contains("Preset Limits"), "{text}");
    assert!(
        !text.contains("Isotope"),
        "the nuclide lookup is not wanted mid-count:\n{text}"
    );
    assert!(!text.contains("Regions"), "nor the region list:\n{text}");

    // Interpreting: the regions and the library, without the preset limits,
    // which describe a run that has already finished.
    app.workspace = Workspace::analysis();
    let text = painted_text(&frame(&mut app, &ctx, [1400.0, 900.0]));
    assert!(text.contains("Regions"), "{text}");
    assert!(text.contains("Isotope"), "{text}");
    assert!(
        !text.contains("Preset Limits"),
        "the presets are done with:\n{text}"
    );
    // The counts stay in both: they are what a spectrum is.
    assert!(text.contains("Pulse Ht. Analysis"), "{text}");
}

/// The workspace is chosen from the corner, and kept across a restart.
#[test]
fn the_workspace_is_chosen_from_the_corner_and_remembered() {
    let ctx = egui::Context::default();
    let mut app = with_data();

    // It is named in the status bar, which is where it is chosen from.
    let text = painted_text(&frame(&mut app, &ctx, [1400.0, 900.0]));
    assert!(text.contains("Everything"), "{text}");

    app.workspace = mantaray_gui::workspace::Workspace::analysis();
    app.workspaces.push(mantaray_gui::workspace::Workspace::new(
        "Bench",
        app.workspace.sections,
    ));

    // Both survive the round trip through the settings file.
    let written = serde_json::to_string(&app.persisted()).expect("write");
    let persisted: mantaray_gui::app::Persisted = serde_json::from_str(&written).expect("read");
    let mut restored = App::headless();
    restored.restore(persisted);
    assert_eq!(restored.workspace.name, "Analysis");
    assert_eq!(restored.workspaces.len(), 1);
    assert_eq!(restored.workspaces[0].name, "Bench");

    // And settings written before workspaces existed still load, with the
    // arrangement that hides nothing.
    let older = r##"{"theme":"Paper","colors":{"background":"#fcfcfa","foreground":"#0b4e73",
        "roi":"#a0480a","compare":"#603cb4","composite":"#b23c3c","axes":"#5a606e",
        "marker":"#1e1e22","library":"#a02882","view_box":"#8c96af","panel":"#eeeeec",
        "alarm":"#be1e1e","healthy":"#147850"},"recent":[],"time_scale":1.0}"##;
    let persisted: mantaray_gui::app::Persisted =
        serde_json::from_str(older).expect("settings from before workspaces existed");
    assert_eq!(persisted.workspace.name, "Everything");
    assert!(persisted.workspaces.is_empty());
}

/// Tabs by default: one spectrum fills the area, the rest are names in a strip.
#[test]
fn spectra_are_arranged_in_tabs_by_default() {
    let ctx = egui::Context::default();
    let mut app = two_spectra();

    assert_eq!(
        app.style.layout,
        mantaray_gui::theme::Layout::Tabs,
        "tabs are the default arrangement"
    );

    // Both are named in the strip, whichever one is being shown.
    let text = painted_text(&frame(&mut app, &ctx, [1400.0, 900.0]));
    assert!(text.contains("first.Spe"), "{text}");
    assert!(text.contains("second.Spe"), "{text}");
    assert_eq!(
        spectra_drawn(&text),
        1,
        "exactly one spectrum should have the area:\n{text}"
    );

    // And clicking a tab shows that one.
    let first = window_id(&app, "first.Spe");
    assert_ne!(app.active.map(|index| app.windows[index].id), Some(first));
    let output = frame(&mut app, &ctx, [1400.0, 900.0]);
    let tab = text_rect(&output, "first.Spe").expect("the first tab");
    click_at(&mut app, &ctx, [1400.0, 900.0], tab.center());
    assert_eq!(
        app.active.map(|index| app.windows[index].id),
        Some(first),
        "clicking a tab should show that spectrum"
    );
}

/// A spectrum can be pulled out of the strip, and put back.
#[test]
fn a_tab_can_be_opened_in_a_window_of_its_own() {
    let ctx = egui::Context::default();
    let mut app = two_spectra();
    let second = window_id(&app, "second.Spe");
    let floating = |app: &App| {
        app.windows
            .iter()
            .find(|window| window.id == second)
            .expect("still open")
            .floating
    };

    assert!(!floating(&app), "everything starts docked");
    let before = spectra_drawn(&painted_text(&frame(&mut app, &ctx, [1400.0, 900.0])));

    app.apply_action(Action::ToggleFloating(second));
    assert!(floating(&app));

    // Two frames: egui measures a window it has not placed before on one frame
    // and paints it on the next. It settles itself, because a repaint is
    // requested, but a test that draws once would see the measuring frame.
    frame(&mut app, &ctx, [1400.0, 900.0]);
    let text = painted_text(&frame(&mut app, &ctx, [1400.0, 900.0]));
    // Its tab stays in the strip, so the strip is still a complete list of
    // what is open - a window behind another is otherwise hard to find.
    assert!(text.contains("second.Spe"), "{text}");
    // And one more spectrum is drawn than before: the tab that had the area,
    // plus the one now in a window beside it.
    assert_eq!(
        spectra_drawn(&text),
        before + 1,
        "the pulled-out one should be drawn as well:\n{text}"
    );

    app.apply_action(Action::ToggleFloating(second));
    assert!(!floating(&app), "and it can be put back");
    frame(&mut app, &ctx, [1400.0, 900.0]);
    assert_eq!(
        spectra_drawn(&painted_text(&frame(&mut app, &ctx, [1400.0, 900.0]))),
        before,
        "and the window goes with it"
    );
}

/// The other arrangement puts every spectrum in a window, as before.
#[test]
fn the_window_arrangement_draws_them_all() {
    let ctx = egui::Context::default();
    let mut app = two_spectra();
    let open = app.visible_windows().len();
    assert!(open >= 3, "the two opened here, and the one already there");

    app.style.layout = mantaray_gui::theme::Layout::Windows;
    let text = painted_text(&frame(&mut app, &ctx, [1400.0, 900.0]));
    assert_eq!(
        spectra_drawn(&text),
        open,
        "every window should be drawn:\n{text}"
    );
}

/// Clicks once at a point on a screen of a given size, and lets the queued
/// action apply on the frame after.
fn click_at(app: &mut App, ctx: &egui::Context, size: [f32; 2], at: egui::Pos2) {
    let input = egui::RawInput {
        screen_rect: Some(egui::Rect::from_min_size(
            egui::Pos2::ZERO,
            egui::vec2(size[0], size[1]),
        )),
        events: vec![
            egui::Event::PointerMoved(at),
            egui::Event::PointerButton {
                pos: at,
                button: egui::PointerButton::Primary,
                pressed: true,
                modifiers: egui::Modifiers::default(),
            },
            egui::Event::PointerButton {
                pos: at,
                button: egui::PointerButton::Primary,
                pressed: false,
                modifiers: egui::Modifiers::default(),
            },
        ],
        ..Default::default()
    };
    let _ = ctx.run_ui(input, |ui| app.draw(ui));
    frame(app, ctx, size);
}

/// The hex field can be typed into a character at a time.
///
/// Rebuilt from the colour every frame, it could not be: `#0b1` is not a
/// colour, so the colour did not change, so the next frame overwrote what had
/// been typed and every keystroke vanished as it was made. Nothing looked
/// wrong in a screenshot and nothing failed in a test that only checked the
/// hex was displayed - it had to be typed at to show.
#[test]
fn a_colour_can_be_typed_in_as_hex() {
    let ctx = still_context();
    let mut app = with_data();
    app.apply_action(Action::Show(Dialog::Preferences));

    // Scrolled to, the way a person reaches a row further down a dialog than
    // the screen is tall.
    let screen = [1400.0, 900.0];
    let before = app.colors.foreground;
    let field = scroll_until(
        &mut app,
        &ctx,
        screen,
        egui::pos2(400.0, 400.0),
        &before.to_string(),
    )
    .expect("the spectrum colour's hex should be reachable");
    type_into(&mut app, &ctx, screen, field.center(), "#ff00ff");

    assert_eq!(
        app.colors.foreground,
        mantaray_gui::theme::Rgb(255, 0, 255),
        "typing a colour into the field should set it"
    );
}

/// Clicks at a point to focus whatever is there, then types, one character per
/// frame - which is the case that matters, because the partial text has to
/// survive between them.
fn type_into(app: &mut App, ctx: &egui::Context, size: [f32; 2], at: egui::Pos2, text: &str) {
    let screen = egui::Rect::from_min_size(egui::Pos2::ZERO, egui::vec2(size[0], size[1]));
    let run = |app: &mut App, events: Vec<egui::Event>| {
        let input = egui::RawInput {
            screen_rect: Some(screen),
            events,
            ..Default::default()
        };
        let _ = ctx.run_ui(input, |ui| app.draw(ui));
    };
    run(
        app,
        vec![
            egui::Event::PointerMoved(at),
            egui::Event::PointerButton {
                pos: at,
                button: egui::PointerButton::Primary,
                pressed: true,
                modifiers: egui::Modifiers::default(),
            },
            egui::Event::PointerButton {
                pos: at,
                button: egui::PointerButton::Primary,
                pressed: false,
                modifiers: egui::Modifiers::default(),
            },
        ],
    );
    // Select what is there so the typing replaces it rather than appending.
    run(
        app,
        vec![egui::Event::Key {
            key: egui::Key::A,
            physical_key: None,
            pressed: true,
            repeat: false,
            modifiers: egui::Modifiers::COMMAND,
        }],
    );
    for character in text.chars() {
        run(app, vec![egui::Event::Text(character.to_string())]);
    }
}

/// A saved scheme is kept, replaces itself under the same name, and persists.
#[test]
fn a_saved_scheme_is_kept_and_comes_back() {
    let mut app = App::headless();
    app.colors = mantaray_gui::theme::SpectrumColors::conductor();
    app.schemes
        .push(mantaray_gui::theme::Scheme::new("Bench", app.colors));

    // It survives the round trip through the settings file.
    let text = serde_json::to_string(&app.persisted()).expect("write");
    let persisted: mantaray_gui::app::Persisted = serde_json::from_str(&text).expect("read");
    let mut restored = App::headless();
    restored.restore(persisted);
    assert_eq!(restored.schemes.len(), 1);
    assert_eq!(restored.schemes[0].name, "Bench");
    assert_eq!(
        restored.schemes[0].colors,
        mantaray_gui::theme::SpectrumColors::conductor()
    );

    // Settings written before schemes existed still load, with none.
    // A longer delimiter, because every colour in it contains `"#`, which would
    // close an `r#"` string in the middle of the first one.
    let older = r##"{"theme":"Paper","colors":{"background":"#fcfcfa","foreground":"#0b4e73",
        "roi":"#a0480a","compare":"#603cb4","composite":"#b23c3c","axes":"#5a606e",
        "marker":"#1e1e22","library":"#a02882","view_box":"#8c96af","panel":"#eeeeec",
        "alarm":"#be1e1e","healthy":"#147850"},"recent":[],"time_scale":1.0}"##;
    let persisted: mantaray_gui::app::Persisted =
        serde_json::from_str(older).expect("settings from before schemes existed");
    assert!(persisted.schemes.is_empty());
}

/// Types `typed` into the field that now reads `shown`, and presses Enter.
fn type_into_field(app: &mut App, ctx: &egui::Context, shown: &str, typed: &str) {
    let output = frame(app, ctx, [1400.0, 900.0]);
    let at = text_rect(&output, shown)
        .unwrap_or_else(|| panic!("no field reading {shown:?} on screen"))
        .center();
    type_into(app, ctx, [1400.0, 900.0], at, typed);
    frame_with_events(
        app,
        ctx,
        vec![egui::Event::Key {
            key: egui::Key::Enter,
            physical_key: None,
            pressed: true,
            repeat: false,
            modifiers: egui::Modifiers::NONE,
        }],
    );
    frame(app, ctx, [1400.0, 900.0]);
}

#[test]
fn a_pending_edit_survives_a_switch_to_another_detector_and_back() {
    let ctx = still_context();
    let mut app = App::headless();
    app.detectors
        .push(mantaray_device::SimulatedMcb::new(2, "SIM-02").into());
    app.apply_action(Action::OpenDetector(0));
    app.apply_action(Action::OpenDetector(1));
    app.dialogs.open(Dialog::McbProperties);
    app.dialogs.properties_tab = 0;
    let windows_for = |app: &App, detector: usize| {
        app.windows
            .iter()
            .position(
                |w| matches!(w.target, mantaray_gui::app::Target::Detector(d) if d == detector),
            )
            .expect("a window for the detector")
    };
    app.active = Some(windows_for(&app, 0));
    app.dialogs.coarse_edit.insert(1, 64.0);
    app.dialogs.hv_edit.insert(1, 1200.0);
    let output = frame(&mut app, &ctx, [1400.0, 900.0]);
    assert!(painted_text(&output).contains("not applied yet"));

    // The other detector has no edit of its own: nothing pending there, and
    // looking at it does not throw detector 0's away.
    app.active = Some(windows_for(&app, 1));
    let output = frame(&mut app, &ctx, [1400.0, 900.0]);
    assert!(!painted_text(&output).contains("not applied yet"));
    assert_eq!(app.dialogs.coarse_edit.get(&1), Some(&64.0));
    assert_eq!(app.dialogs.hv_edit.get(&1), Some(&1200.0));

    app.active = Some(windows_for(&app, 0));
    let output = frame(&mut app, &ctx, [1400.0, 900.0]);
    assert!(painted_text(&output).contains("not applied yet"));
}

/// The index of the detector window that looks at `detector`.
fn window_of(app: &App, detector: usize) -> usize {
    app.windows
        .iter()
        .position(|w| matches!(w.target, mantaray_gui::app::Target::Detector(d) if d == detector))
        .expect("a window for the detector")
}

fn simulated_entry(number: u16) -> mantaray_device::DetectorEntry {
    mantaray_device::DetectorEntry {
        number,
        name: format!("SIM-{number:02}"),
        model: "simulated".into(),
        kind: mantaray_device::DetectorKind::Simulated,
        channels: 2048,
        description: String::new(),
        serial: String::new(),
    }
}

#[test]
fn removing_a_detector_never_moves_a_pending_edit_onto_another_instrument() {
    // Three instruments, numbers 1, 2 and 3. A coarse gain is typed for the
    // second and not applied. The first is removed, so the third moves up to
    // the second's old place in the list. An edit kept by place would now be
    // waiting on the third instrument.
    let ctx = still_context();
    let mut app = App::headless();
    app.add_detector(simulated_entry(2));
    app.add_detector(simulated_entry(3));
    for detector in 0..3 {
        app.apply_action(Action::OpenDetector(detector));
    }
    app.dialogs.open(Dialog::McbProperties);
    app.dialogs.properties_tab = 0;
    app.active = Some(window_of(&app, 1));
    type_into_field(&mut app, &ctx, "20", "64");
    let output = frame(&mut app, &ctx, [1400.0, 900.0]);
    assert!(painted_text(&output).contains("not applied yet"));

    app.apply_action(Action::RemoveDetector(0));
    assert_eq!(app.detectors.len(), 2);
    assert_eq!(mantaray_device::Mcb::identity(&app.detectors[1]).number, 3);
    app.dialogs.open(Dialog::McbProperties);
    app.active = Some(window_of(&app, 1));
    let output = frame(&mut app, &ctx, [1400.0, 900.0]);
    assert!(
        !painted_text(&output).contains("not applied yet"),
        "instrument 3 shows an edit that was typed for instrument 2"
    );
    // Applying what it shows sends its own number, never the stranger's 64.
    click_text(&mut app, &ctx, "Apply");
    let gain = |app: &App, index: usize| {
        mantaray_device::Mcb::properties(&app.detectors[index])
            .amplifier
            .coarse_gain
    };
    assert_eq!(gain(&app, 1), 20.0);
    assert_eq!(gain(&app, 0), 20.0, "instrument 2 was never applied");
    // The edit is still instrument 2's.
    assert_eq!(app.dialogs.coarse_edit.get(&2), Some(&64.0));
    app.active = Some(window_of(&app, 0));
    let output = frame(&mut app, &ctx, [1400.0, 900.0]);
    assert!(painted_text(&output).contains("not applied yet"));
}

#[test]
fn removing_a_detector_drops_its_pending_edits() {
    let mut app = App::headless();
    app.add_detector(simulated_entry(2));
    app.dialogs.coarse_edit.insert(1, 64.0);
    app.dialogs.hv_edit.insert(1, 1200.0);
    app.dialogs
        .presets_edit
        .insert(1, mantaray_device::Presets::default());
    app.dialogs.coarse_edit.insert(2, 65.0);
    app.apply_action(Action::RemoveDetector(0));
    assert!(app.dialogs.hv_edit.is_empty());
    assert!(app.dialogs.presets_edit.is_empty());
    assert_eq!(app.dialogs.coarse_edit.get(&2), Some(&65.0));
    assert_eq!(app.dialogs.coarse_edit.len(), 1);
}

#[test]
fn the_not_applied_note_is_under_the_table_not_a_cell_of_the_coarse_gain_row() {
    let ctx = still_context();
    let mut app = App::headless();
    app.apply_action(Action::OpenDetector(0));
    app.dialogs.open(Dialog::McbProperties);
    app.dialogs.properties_tab = 0;
    type_into_field(&mut app, &ctx, "20", "64");
    let mut output = frame(&mut app, &ctx, [1400.0, 900.0]);
    for _ in 0..2 {
        output = frame(&mut app, &ctx, [1400.0, 900.0]);
    }
    let row = text_rect(&output, "Coarse gain").expect("the row's label");
    let note = text_rect(&output, "not applied yet").expect("the note");
    assert!(
        note.min.y >= row.max.y,
        "the note ({note:?}) sits in the row of the field ({row:?}) instead of under the table"
    );
}

/// Draws a pending-apply field for a mirror that holds `mirror`, as the
/// dialog does, `frames` times, and returns what it reported and painted.
fn draw_pending_field(
    mirror: f64,
    pending: &mut mantaray_gui::dialogs::PendingEdits,
    frames: usize,
) -> (mantaray_gui::dialogs::PendingField, String) {
    let ctx = still_context();
    let mut last = None;
    let mut text = String::new();
    for _ in 0..frames {
        let input = egui::RawInput {
            screen_rect: Some(egui::Rect::from_min_size(
                egui::Pos2::ZERO,
                egui::vec2(600.0, 200.0),
            )),
            ..Default::default()
        };
        let output = ctx.run_ui(input, |ui| {
            let label = ui.label("Coarse gain").id;
            let field = mantaray_gui::dialogs::pending_field(
                ui,
                label,
                pending,
                1,
                mirror,
                mantaray_device::AmplifierSettings::COARSE_GAIN_MIN
                    ..=mantaray_device::AmplifierSettings::COARSE_GAIN_MAX,
                mantaray_device::AmplifierSettings::check_coarse_gain,
            );
            mantaray_gui::dialogs::pending_notes(ui, &field);
            last = Some(field);
        });
        text = painted_text(&output);
    }
    (last.expect("a frame was drawn"), text)
}

#[test]
fn a_mirror_that_is_not_a_whole_number_in_range_is_shown_as_it_is_with_a_warning() {
    let mut pending = mantaray_gui::dialogs::PendingEdits::new();
    // A fraction keeps its decimals: 20 would be a number the settings do not hold.
    let (field, text) = draw_pending_field(20.4, &mut pending, 3);
    assert!(text.contains("20.4"), "{text}");
    assert!(field.warning.is_some() && !field.differs);
    assert!(pending.is_empty());
    // Out of the range, not clamped into it.
    let (field, text) = draw_pending_field(5000.0, &mut pending, 3);
    assert!(text.contains("5000"), "{text}");
    assert!(field.warning.is_some() && !field.differs);
    // A whole, in-range mirror has neither.
    let (field, _) = draw_pending_field(40.0, &mut pending, 3);
    assert!(field.warning.is_none() && !field.differs);
}

#[test]
fn an_edit_is_compared_with_the_mirror_exactly() {
    // The mirror holds 20.4 and the field has 20 in it: that is a pending
    // edit, though both round to 20.
    let mut pending = mantaray_gui::dialogs::PendingEdits::new();
    pending.insert(1, 20.0);
    let (field, text) = draw_pending_field(20.4, &mut pending, 3);
    assert!(field.differs, "20 is not 20.4");
    assert!(text.contains("not applied yet"), "{text}");
    assert_eq!(pending.get(&1), Some(&20.0));
}

#[test]
fn a_mirror_that_is_nan_warns_and_is_never_a_pending_edit() {
    let mut pending = mantaray_gui::dialogs::PendingEdits::new();
    let (field, text) = draw_pending_field(f64::NAN, &mut pending, 3);
    assert!(!field.differs, "NaN against NaN is no edit");
    assert!(field.warning.is_some());
    assert!(!text.contains("not applied yet"), "{text}");
    assert!(pending.is_empty(), "a NaN mirror became a pending edit");
}

#[test]
fn a_fraction_typed_into_the_field_is_rounded_while_the_mirror_can_be_sent() {
    // A 20.4 left in the field could never be applied (the command carries a
    // whole number): it would show "not applied yet" forever. When the
    // settings hold a sendable value the field holds only whole numbers.
    let mut pending = mantaray_gui::dialogs::PendingEdits::new();
    pending.insert(1, 20.4);
    let (field, text) = draw_pending_field(40.0, &mut pending, 3);
    assert_eq!(pending.get(&1), Some(&20.0), "the edit was not rounded");
    assert!(field.differs && field.warning.is_none());
    assert!(!text.contains("20.4"), "{text}");
    // Rounding to the mirror itself is no edit at all.
    pending.insert(1, 40.3);
    let (field, _) = draw_pending_field(40.0, &mut pending, 3);
    assert!(!field.differs, "40.3 rounds to the 40 the settings hold");
    assert!(pending.is_empty());
}

#[test]
fn negative_zero_is_zero_in_the_field() {
    let mut pending = mantaray_gui::dialogs::PendingEdits::new();
    pending.insert(1, -0.0);
    let (field, text) = draw_pending_field(0.0, &mut pending, 3);
    assert!(!field.differs, "-0 is 0");
    assert!(pending.is_empty());
    assert!(!text.contains("-0"), "{text}");
}

/// A spectrum with one Cs-137 peak, counted 1000 s, and an app that has
/// analysed it with a curve that is zero at 661.7 keV: Cs-137 is withheld.
fn app_with_a_withheld_nuclide() -> App {
    let mut counts = vec![20u64; 2048];
    for off in -12i64..=12 {
        counts[(662 + off) as usize] +=
            (50_000.0 / 7.5 * (-0.5 * (off as f64 / 2.0).powi(2)).exp()) as u64;
    }
    let mut spectrum = Spectrum::from_counts(counts);
    spectrum.live_time = 1000.0;
    spectrum.real_time = 1010.0;
    spectrum.energy_calibration = Some(mantaray_core::EnergyCalibration::linear(0.0, 1.0));
    spectrum.rois.mark(mantaray_core::Roi::new(650, 674));
    let mut app = App::headless();
    app.library = mantaray_core::NuclideLibrary::sample_for_tests();
    app.efficiency = Some(mantaray_core::EfficiencyCalibration::interpolated(vec![
        mantaray_core::EfficiencyPoint::new(100.0, 0.1),
        mantaray_core::EfficiencyPoint::new(500.0, 0.0),
    ]));
    app.open_buffer("withheld.Spe".into(), spectrum, None);
    app.apply_action(Action::Analyse);
    app
}

/// The dialog's text with every run of whitespace, wrapping included, as one
/// space.
fn analysis_dialog_text(app: &mut App) -> String {
    app.apply_action(Action::Show(Dialog::Analysis));
    let ctx = still_context();
    let mut text = String::new();
    for _ in 0..3 {
        text = painted_text(&frame(app, &ctx, [1400.0, 900.0]));
    }
    text.split_whitespace().collect::<Vec<_>>().join(" ")
}

#[test]
fn the_analysis_dialog_shows_why_a_nuclide_was_withheld() {
    // The curve is zero at Cs-137's 661.7 keV line, so the analysis withholds
    // it. The dialog listed no nuclide and nothing else, as if the peak had
    // never been identified; now it is a row, with the reason in it.
    let mut app = app_with_a_withheld_nuclide();
    let report = app.analysis.clone().expect("the analysis ran");
    assert!(report.nuclides.is_empty());
    assert_eq!(report.withheld.len(), 1, "{:?}", report.withheld);
    let flat = analysis_dialog_text(&mut app);
    let withheld = &report.withheld[0];
    assert!(flat.contains(&withheld.nuclide), "{flat}");
    assert!(flat.contains("not reported"), "{flat}");
    let head: String = withheld.reason.chars().take(40).collect();
    assert!(
        flat.contains(&head),
        "{head:?} is not in the dialog:\n{flat}"
    );
}

#[test]
fn a_failed_csv_export_is_reported_in_the_analysis_dialog() {
    let mut app = app_with_a_withheld_nuclide();
    let missing = std::env::temp_dir()
        .join("mantaray-no-such-folder-for-the-csv-test")
        .join("results.csv");
    assert!(!missing.exists());
    mantaray_gui::dialogs::export_analysis_csv(&mut app, &missing);
    let flat = analysis_dialog_text(&mut app);
    assert!(flat.contains("could not write"), "{flat}");
    assert!(app.status.contains("could not write"), "{}", app.status);
    // A good write says so instead, and the error goes away.
    let good = std::env::temp_dir().join(format!("mantaray-csv-{}.csv", std::process::id()));
    mantaray_gui::dialogs::export_analysis_csv(&mut app, &good);
    let flat = analysis_dialog_text(&mut app);
    assert!(!flat.contains("could not write"), "{flat}");
    assert!(flat.contains("wrote"), "{flat}");
    let written = std::fs::read_to_string(&good).expect("the CSV was written");
    std::fs::remove_file(&good).ok();
    assert!(written.starts_with("nuclide,"), "{written}");
}

#[test]
fn an_analysis_saved_before_nuclides_were_withheld_by_name_still_loads() {
    let app = app_with_a_withheld_nuclide();
    let report = app.analysis.clone().expect("the analysis ran");
    let mut json = serde_json::to_value(&report).expect("write");
    json.as_object_mut().unwrap().remove("withheld");
    let loaded: mantaray_core::QuantitativeReport = serde_json::from_value(json).expect("read");
    assert!(loaded.withheld.is_empty());
    assert_eq!(loaded.notes, report.notes);
}

/// The number of lines each painted text takes, for the texts that contain
/// `needle`: one for a label cut to its column, several for one that wraps.
fn lines_of_text_containing(output: &egui::FullOutput, needle: &str) -> Vec<usize> {
    fn collect(shape: &egui::Shape, needle: &str, into: &mut Vec<usize>) {
        match shape {
            egui::Shape::Text(text) if text.galley.text().contains(needle) => {
                into.push(text.galley.rows.len());
            }
            egui::Shape::Vec(inner) => {
                for shape in inner {
                    collect(shape, needle, into);
                }
            }
            _ => {}
        }
    }
    let mut lines = Vec::new();
    for clipped in &output.shapes {
        collect(&clipped.shape, needle, &mut lines);
    }
    lines
}

#[test]
fn a_long_withheld_reason_is_cut_to_one_line_in_the_dialog() {
    let mut app = app_with_a_withheld_nuclide();
    let long = format!("no activity: {}", "the curve is zero here ".repeat(60));
    app.analysis.as_mut().unwrap().withheld[0] = mantaray_core::Withheld::new("Cs-137", &long);
    app.apply_action(Action::Show(Dialog::Analysis));
    let ctx = still_context();
    let mut output = frame(&mut app, &ctx, [1400.0, 900.0]);
    for _ in 0..2 {
        output = frame(&mut app, &ctx, [1400.0, 900.0]);
    }
    let lines = lines_of_text_containing(&output, "the curve is zero here");
    assert_eq!(lines, [1], "the reason must not wrap inside the table");
}

#[test]
fn nuclides_withheld_for_one_reason_are_one_row_in_the_dialog() {
    let mut app = app_with_a_withheld_nuclide();
    // Two found nuclides withheld for the same reason: one row.
    let report = app.analysis.as_mut().unwrap();
    report.withheld = vec![
        mantaray_core::Withheld::new("Co-60", "no activity: the spectrum has no live time"),
        mantaray_core::Withheld::new("Cs-137", "no activity: the spectrum has no live time"),
    ];
    let flat = analysis_dialog_text(&mut app);
    assert_eq!(flat.matches("Co-60, Cs-137").count(), 1, "{flat}");
    assert_eq!(
        flat.matches("no activity: the spectrum has no live time")
            .count(),
        1,
        "{flat}"
    );
}

#[test]
fn the_status_line_after_an_analysis_counts_the_nuclides_it_did_not_report() {
    let app = app_with_a_withheld_nuclide();
    assert!(
        app.status.contains("0 nuclide(s) reported")
            && app.status.contains("1 not reported")
            && app.status.contains("Nuclide analysis"),
        "{}",
        app.status
    );
}

#[test]
fn the_status_line_says_nothing_of_withheld_nuclides_when_there_are_none() {
    let mut app = app_with_a_withheld_nuclide();
    app.efficiency = Some(mantaray_core::EfficiencyCalibration::constant(0.05));
    app.apply_action(Action::Analyse);
    assert!(app.status.contains("reported"), "{}", app.status);
    assert!(!app.status.contains("not reported"), "{}", app.status);
}

#[test]
fn a_new_analysis_clears_the_last_export_message() {
    let mut app = app_with_a_withheld_nuclide();
    let missing = std::env::temp_dir()
        .join("mantaray-no-such-folder-for-the-csv-test")
        .join("results.csv");
    mantaray_gui::dialogs::export_analysis_csv(&mut app, &missing);
    assert!(matches!(app.dialogs.csv_export, Some(Err(_))));
    app.apply_action(Action::Analyse);
    assert!(app.dialogs.csv_export.is_none());
    let flat = analysis_dialog_text(&mut app);
    assert!(!flat.contains("could not write"), "{flat}");
}

/// An instrument line that keeps a log of every command it is sent and
/// reports a 300 s live preset already in its registers.
struct Logged {
    log: std::sync::Arc<std::sync::Mutex<Vec<String>>>,
    counting: bool,
}

impl mantaray_device::Transport for Logged {
    fn exchange(&mut self, command: &str) -> Result<String, mantaray_device::DeviceError> {
        self.log.lock().expect("log").push(command.to_string());
        // The instrument follows START and STOP, as a real one does.
        match command {
            "STOP" => self.counting = false,
            "START" => self.counting = true,
            _ => {}
        }
        Ok(match command {
            "SHOW_CONFIGURATION" => "MODEL=M SERIAL=S FIRMWARE=F CHANNELS=64".into(),
            "SHOW_STATUS" => format!(
                "RT=0.00 LT=0.00 DT=0.0% ICR=0 ACTIVE={} TOTAL=0",
                u8::from(self.counting)
            ),
            "SHOW_DATA" => format!("DATA 64{}", " 0".repeat(64)),
            "SHOW_PRESETS" => "PRESETS REAL=0.00 LIVE=300.00 PEAK=0 INTEG=0".into(),
            _ => "OK".into(),
        })
    }

    fn peer(&self) -> String {
        "logged".into()
    }
}

#[test]
fn apply_presets_with_only_a_new_mda_target_sends_nothing_to_the_instrument() {
    // Through the button the operator presses, with the detector idle: the
    // Presets tab's "Apply presets" used to send SET_PRESET_CLEAR and every
    // preset again for an MDA target the instrument never held.
    let log = std::sync::Arc::new(std::sync::Mutex::new(Vec::new()));
    let remote = mantaray_device::RemoteMcb::connect(
        Box::new(Logged {
            log: log.clone(),
            counting: false,
        }),
        1,
        "BENCH",
    )
    .expect("connect");
    let ctx = egui::Context::default();
    let mut app = App::headless();
    app.detectors[0] = remote.into();
    app.apply_action(Action::OpenDetector(0));
    app.dialogs.open(Dialog::McbProperties);
    app.dialogs.properties_tab = 2;

    let mut typed = *mantaray_device::Mcb::presets(&app.detectors[0]);
    assert_eq!(typed.live_time, Some(300.0), "the instrument's own preset");
    typed.mda = Some(mantaray_device::MdaPreset {
        target: 5.0,
        region: mantaray_core::Roi::new(1, 10),
        efficiency: 0.05,
        yield_fraction: 0.85,
    });
    app.dialogs.presets_edit.insert(1, typed);

    let output = frame(&mut app, &ctx, [1400.0, 900.0]);
    let button = text_rect(&output, "Apply presets").expect("the Apply presets button is painted");
    log.lock().expect("log").clear();
    let at = button.center();
    let events = vec![
        egui::Event::PointerMoved(at),
        egui::Event::PointerButton {
            pos: at,
            button: egui::PointerButton::Primary,
            pressed: true,
            modifiers: egui::Modifiers::default(),
        },
        egui::Event::PointerButton {
            pos: at,
            button: egui::PointerButton::Primary,
            pressed: false,
            modifiers: egui::Modifiers::default(),
        },
    ];
    frame_with_events(&mut app, &ctx, events);
    frame(&mut app, &ctx, [1400.0, 900.0]);

    let sent: Vec<String> = log
        .lock()
        .expect("log")
        .iter()
        .filter(|line| line.starts_with("SET_"))
        .cloned()
        .collect();
    assert_eq!(sent, Vec::<String>::new(), "nothing for a host-side change");
    assert_eq!(
        mantaray_device::Mcb::presets(&app.detectors[0])
            .mda
            .map(|m| m.target),
        Some(5.0),
        "the Apply button was really pressed"
    );
}

#[test]
fn a_batch_sample_with_presets_equal_to_the_mirror_still_re_arms_the_instrument() {
    // The instrument already holds 300 s live and so does the sample: the
    // queue is an explicit action and sends a clear and every held preset
    // anyway, or a second sample would never be armed. The detector is idle:
    // a counting one is refused ("stop it first").
    let log = std::sync::Arc::new(std::sync::Mutex::new(Vec::new()));
    let remote = mantaray_device::RemoteMcb::connect(
        Box::new(Logged {
            log: log.clone(),
            counting: false,
        }),
        1,
        "BENCH",
    )
    .expect("connect");
    let mut app = App::headless();
    app.changer = None;
    app.detectors[0] = remote.into();
    app.apply_action(Action::OpenDetector(0));
    app.queue
        .push(mantaray_device::SampleJob::new("S001", 300.0));
    log.lock().expect("log").clear();
    let actions = mantaray_gui::jobs::run_next_sample(&mut app);
    let sent: Vec<String> = log
        .lock()
        .expect("log")
        .iter()
        .filter(|line| line.starts_with("SET_"))
        .cloned()
        .collect();
    assert_eq!(
        sent,
        vec![
            "SET_PRESET_CLEAR".to_string(),
            "SET_PRESET_LIVE 300".to_string()
        ],
        "{actions:?}"
    );
}

#[test]
fn the_presets_tab_is_disabled_while_the_detector_is_counting() {
    // MAESTRO: "The presets can only be set on an MCB that is not acquiring
    // data (during acquisition the preset field backgrounds are gray
    // indicating that they are inactive)." Apply and Clear presets are
    // disabled too: pressing them does nothing, not even raise the refusal.
    let log = std::sync::Arc::new(std::sync::Mutex::new(Vec::new()));
    let remote = mantaray_device::RemoteMcb::connect(
        Box::new(Logged {
            log: log.clone(),
            counting: true,
        }),
        1,
        "BENCH",
    )
    .expect("connect");
    let ctx = egui::Context::default();
    let mut app = App::headless();
    app.detectors[0] = remote.into();
    app.apply_action(Action::OpenDetector(0));
    app.dialogs.open(Dialog::McbProperties);
    app.dialogs.properties_tab = 2;

    let mut typed = *mantaray_device::Mcb::presets(&app.detectors[0]);
    typed.live_time = Some(60.0);
    app.dialogs.presets_edit.insert(1, typed);

    let output = frame(&mut app, &ctx, [1400.0, 900.0]);
    assert!(
        painted_text(&output)
            .contains("Presets can only be set while the detector is not acquiring."),
        "the one-line note is missing"
    );
    let click = |app: &mut App, ctx: &egui::Context, at: egui::Pos2| {
        let press = |pressed| egui::Event::PointerButton {
            pos: at,
            button: egui::PointerButton::Primary,
            pressed,
            modifiers: egui::Modifiers::default(),
        };
        frame_with_events(
            app,
            ctx,
            vec![egui::Event::PointerMoved(at), press(true), press(false)],
        );
        frame(app, ctx, [1400.0, 900.0]);
    };
    let apply = text_rect(&output, "Apply presets").expect("Apply presets is painted");
    let clear = text_rect(&output, "Clear presets").expect("Clear presets is painted");
    log.lock().expect("log").clear();
    app.status.clear();
    click(&mut app, &ctx, apply.center());
    click(&mut app, &ctx, clear.center());
    assert_eq!(
        app.status, "",
        "a disabled button does nothing: {}",
        app.status
    );
    let sent: Vec<String> = log
        .lock()
        .expect("log")
        .iter()
        .filter(|line| line.starts_with("SET_"))
        .cloned()
        .collect();
    assert_eq!(sent, Vec::<String>::new());
    assert_eq!(
        mantaray_device::Mcb::presets(&app.detectors[0]).live_time,
        Some(300.0),
        "the mirror is untouched"
    );

    // Stopped, the same button works (on an edit typed now: the one typed
    // before was dropped when counting was seen).
    mantaray_device::Mcb::stop(&mut app.detectors[0]).expect("stop");
    let output = frame(&mut app, &ctx, [1400.0, 900.0]);
    assert!(
        !painted_text(&output).contains("Presets can only be set while"),
        "the note goes away when the detector stops"
    );
    app.dialogs.presets_edit.insert(1, typed);
    let output = frame(&mut app, &ctx, [1400.0, 900.0]);
    let apply = text_rect(&output, "Apply presets").expect("Apply presets is painted");
    click(&mut app, &ctx, apply.center());
    assert_eq!(
        mantaray_device::Mcb::presets(&app.detectors[0]).live_time,
        Some(60.0)
    );
}

#[test]
fn t32_a_counting_detectors_gray_presets_form_shows_the_instrument_and_drops_the_draft() {
    // T32: while the detector counts the form is gray and says what the
    // instrument holds (300 s live), never an unapplied edit (77 s) that is
    // not in force; fields, Apply and Clear are off and the note is shown.
    // The half-typed draft is gone when counting is seen (device-spec 2.8):
    // kept and brought back after the count it could be older than presets
    // another client set meanwhile (review of 21df1c3, #9).
    let log = std::sync::Arc::new(std::sync::Mutex::new(Vec::new()));
    let remote = mantaray_device::RemoteMcb::connect(
        Box::new(Logged {
            log: log.clone(),
            counting: true,
        }),
        1,
        "BENCH",
    )
    .expect("connect");
    let ctx = egui::Context::default();
    let mut app = App::headless();
    app.detectors[0] = remote.into();
    app.apply_action(Action::OpenDetector(0));
    app.dialogs.open(Dialog::McbProperties);
    app.dialogs.properties_tab = 2;

    let mut typed = *mantaray_device::Mcb::presets(&app.detectors[0]);
    typed.live_time = Some(77.0);
    app.dialogs.presets_edit.insert(1, typed);

    let output = frame(&mut app, &ctx, [1400.0, 900.0]);
    let shown = painted_text(&output);
    assert!(shown.contains("300"), "the instrument's preset: {shown}");
    assert!(!shown.contains("77"), "an unapplied edit is shown: {shown}");
    assert!(
        shown.contains("Presets can only be set while the detector is not acquiring."),
        "the note"
    );
    assert_eq!(
        app.dialogs.presets_edit.get(&1),
        None,
        "the draft is dropped when the count is seen"
    );
    // Disabled: a click on Apply sends nothing and says nothing. (Time is
    // frozen so no poll lands in the log either.)
    app.time_scale = 0.0;
    let apply = text_rect(&output, "Apply presets").expect("Apply presets is painted");
    log.lock().expect("log").clear();
    app.status.clear();
    press_at(&mut app, &ctx, apply.center());
    assert_eq!(app.status, "");
    assert_eq!(*log.lock().expect("log"), Vec::<String>::new());

    mantaray_device::Mcb::stop(&mut app.detectors[0]).expect("stop");
    let shown = painted_text(&frame(&mut app, &ctx, [1400.0, 900.0]));
    assert!(!shown.contains("77"), "the draft came back: {shown}");
    assert_eq!(app.dialogs.presets_edit.get(&1), None);
    assert_eq!(
        mantaray_device::Mcb::presets(&app.detectors[0]).live_time,
        Some(300.0),
        "and is still not applied"
    );
}

/// Presses and releases the primary button at `at`, then draws a frame.
fn press_at(app: &mut App, ctx: &egui::Context, at: egui::Pos2) {
    let press = |pressed| egui::Event::PointerButton {
        pos: at,
        button: egui::PointerButton::Primary,
        pressed,
        modifiers: egui::Modifiers::default(),
    };
    frame_with_events(
        app,
        ctx,
        vec![egui::Event::PointerMoved(at), press(true), press(false)],
    );
    frame(app, ctx, [1400.0, 900.0]);
}

/// A network instrument on a [`Logged`] line, its log cleared.
fn logged_remote(
    counting: bool,
) -> (
    mantaray_device::RemoteMcb,
    std::sync::Arc<std::sync::Mutex<Vec<String>>>,
) {
    let log = std::sync::Arc::new(std::sync::Mutex::new(Vec::new()));
    let remote = mantaray_device::RemoteMcb::connect(
        Box::new(Logged {
            log: log.clone(),
            counting,
        }),
        1,
        "BENCH",
    )
    .expect("connect");
    log.lock().expect("log").clear();
    (remote, log)
}

/// An app showing the Presets tab of detector 0, with time frozen so frames
/// poll nothing on their own.
fn presets_tab(remote: mantaray_device::RemoteMcb) -> App {
    let mut app = App::headless();
    app.time_scale = 0.0;
    app.detectors[0] = remote.into();
    app.apply_action(Action::OpenDetector(0));
    app.dialogs.open(Dialog::McbProperties);
    app.dialogs.properties_tab = 2;
    app
}

/// T17: Stop on a locked detector shows the lock, never "acquisition stopped".
#[test]
fn t17_stop_on_a_locked_detector_says_so() {
    let mut app = App::headless();
    app.apply_action(Action::OpenDetector(0));
    app.apply_action(Action::Start);
    mantaray_device::Mcb::lock(&mut app.detectors[0], "pw", "Ann").expect("lock");
    app.apply_action(Action::Stop);
    assert_eq!(app.status, "the detector is locked by Ann");
    assert!(
        mantaray_device::Mcb::is_active(&app.detectors[0]),
        "still counting"
    );
    mantaray_device::Mcb::unlock(&mut app.detectors[0], "pw").expect("unlock");
    app.apply_action(Action::Stop);
    assert_eq!(app.status, "acquisition stopped");
}

/// T18: Start all and Stop all name every detector that refused; the others
/// go ahead.
#[test]
fn t18_start_all_and_stop_all_name_each_refusal() {
    use mantaray_device::Mcb;
    let ctx = egui::Context::default();
    let mut app = App::headless();
    app.time_scale = 0.0;
    app.detectors
        .push(mantaray_device::SimulatedMcb::new(2, "SIM-02").into());
    let locked = app.detectors[0].identity().number;
    let free = app.detectors[1].identity().number;
    assert_ne!(locked, free);
    app.detectors[0].start().expect("start");
    app.detectors[0].lock("pw", "Ann").expect("lock");
    app.dialogs.open(Dialog::Dashboard);
    let output = frame(&mut app, &ctx, [1400.0, 900.0]);
    let start_all = text_rect(&output, "Start all").expect("Start all is painted");
    let stop_all = text_rect(&output, "Stop all").expect("Stop all is painted");

    press_at(&mut app, &ctx, start_all.center());
    assert_eq!(
        app.status,
        format!("Detector {locked}: the detector is locked by Ann")
    );
    assert!(app.detectors[1].is_active(), "the free detector started");

    press_at(&mut app, &ctx, stop_all.center());
    assert_eq!(
        app.status,
        format!("Detector {locked}: the detector is locked by Ann")
    );
    assert!(!app.detectors[1].is_active(), "the free detector stopped");
    assert!(app.detectors[0].is_active(), "the locked one counts on");
}

/// T26: an uncertainty limit of 0 and an MDA target of 0 are "off": applied,
/// they are stored as no preset at all, and the form's checkboxes follow.
#[test]
fn t26_a_zero_limit_from_the_form_is_no_preset() {
    let (remote, log) = logged_remote(false);
    let ctx = egui::Context::default();
    let mut app = presets_tab(remote);
    let mut typed = *mantaray_device::Mcb::presets(&app.detectors[0]);
    typed.uncertainty = Some(mantaray_device::UncertaintyPreset {
        limit_percent: 0.0,
        low_channel: 1,
        high_channel: 10,
    });
    typed.mda = Some(mantaray_device::MdaPreset {
        target: 0.0,
        region: mantaray_core::Roi::new(1, 10),
        efficiency: 0.05,
        yield_fraction: 0.85,
    });
    app.dialogs.presets_edit.insert(1, typed);
    let output = frame(&mut app, &ctx, [1400.0, 900.0]);
    assert!(painted_text(&output).contains("limit %"), "the typed draft");
    let apply = text_rect(&output, "Apply presets").expect("Apply presets is painted");
    press_at(&mut app, &ctx, apply.center());
    let held = *mantaray_device::Mcb::presets(&app.detectors[0]);
    assert_eq!(held.uncertainty, None, "{held:?}");
    assert_eq!(held.mda, None, "{held:?}");
    assert!(
        log.lock()
            .expect("log")
            .contains(&"SHOW_STATUS".to_string())
    );
    let shown = painted_text(&frame(&mut app, &ctx, [1400.0, 900.0]));
    assert!(
        !shown.contains("limit %") && !shown.contains("target Bq"),
        "both checkboxes are off: {shown}"
    );
}

/// T27 (GUI half): the form's MDA efficiency and yield controls offer
/// exactly what the device accepts.
#[test]
fn t27_the_form_offers_only_fractions_the_device_accepts() {
    let range = mantaray_gui::dialogs::MDA_FRACTION_RANGE;
    assert!(*range.start() > 0.0);
    assert_eq!(*range.end(), 1.0);
    let with = |fraction: f64| mantaray_device::Presets {
        mda: Some(mantaray_device::MdaPreset {
            target: 5.0,
            region: mantaray_core::Roi::new(1, 10),
            efficiency: fraction,
            yield_fraction: fraction,
        }),
        ..mantaray_device::Presets::default()
    };
    assert!(with(*range.start()).validate().is_ok());
    assert!(with(*range.end()).validate().is_ok());
    assert!(with(range.end() + 0.01).validate().is_err());
    assert!(with(0.0).validate().is_err());
}

/// T31: a counting detector with its Presets tab open costs nothing on the
/// wire, frame after frame; an amplifier edit asks nothing; one host-only
/// Apply asks the status exactly once.
#[test]
fn t31_frames_ask_the_instrument_nothing_and_a_click_asks_once() {
    let (remote, log) = logged_remote(true);
    let ctx = egui::Context::default();
    let mut app = presets_tab(remote);
    for _ in 0..100 {
        frame(&mut app, &ctx, [1400.0, 900.0]);
    }
    assert_eq!(*log.lock().expect("log"), Vec::<String>::new());
    let mut properties = mantaray_device::Mcb::properties(&app.detectors[0]).clone();
    properties.amplifier.rise_time_us = 4.0;
    mantaray_device::Mcb::set_properties(&mut app.detectors[0], properties).expect("amplifier");
    assert_eq!(*log.lock().expect("log"), Vec::<String>::new());

    // Idle, one host-only Apply through the button: one status read.
    mantaray_device::Mcb::stop(&mut app.detectors[0]).expect("stop");
    frame(&mut app, &ctx, [1400.0, 900.0]);
    let mut typed = *mantaray_device::Mcb::presets(&app.detectors[0]);
    typed.mda = Some(mantaray_device::MdaPreset {
        target: 5.0,
        region: mantaray_core::Roi::new(1, 10),
        efficiency: 0.05,
        yield_fraction: 0.85,
    });
    app.dialogs.presets_edit.insert(1, typed);
    let output = frame(&mut app, &ctx, [1400.0, 900.0]);
    let apply = text_rect(&output, "Apply presets").expect("Apply presets is painted");
    log.lock().expect("log").clear();
    press_at(&mut app, &ctx, apply.center());
    for _ in 0..20 {
        frame(&mut app, &ctx, [1400.0, 900.0]);
    }
    assert_eq!(*log.lock().expect("log"), vec!["SHOW_STATUS".to_string()]);
    assert!(
        mantaray_device::Mcb::presets(&app.detectors[0])
            .mda
            .is_some()
    );
}

/// A detector that counts and never stops, whatever it is sent.
struct NeverStops;

impl mantaray_device::Transport for NeverStops {
    fn exchange(&mut self, command: &str) -> Result<String, mantaray_device::DeviceError> {
        Ok(match command {
            "SHOW_CONFIGURATION" => "MODEL=M SERIAL=S FIRMWARE=F CHANNELS=64".into(),
            "SHOW_STATUS" => "RT=1.00 LT=1.00 DT=0.0% ICR=0 ACTIVE=1 TOTAL=0".into(),
            "SHOW_DATA" => format!("DATA 64{}", " 0".repeat(64)),
            "SHOW_PRESETS" => "PRESETS REAL=0.00 LIVE=0.00 PEAK=0 INTEG=0".into(),
            _ => "OK".into(),
        })
    }

    fn peer(&self) -> String {
        "never-stops".into()
    }
}

/// Review 3: Stop all does not stack the detectors' STOP confirms: three
/// detectors that never stop cost about one bound (ten reads 100 ms apart,
/// about a second), not three, and each is named.
#[test]
fn review3_stop_all_does_not_stack_the_confirm_waits() {
    let ctx = egui::Context::default();
    let mut app = App::headless();
    app.time_scale = 0.0;
    app.detectors.clear();
    for number in 1..=3 {
        let remote = mantaray_device::RemoteMcb::connect(Box::new(NeverStops), number, "STUCK")
            .expect("connect");
        app.detectors.push(remote.into());
    }
    app.dialogs.open(Dialog::Dashboard);
    let output = frame(&mut app, &ctx, [1400.0, 900.0]);
    let stop_all = text_rect(&output, "Stop all").expect("Stop all is painted");
    let began = std::time::Instant::now();
    press_at(&mut app, &ctx, stop_all.center());
    let took = began.elapsed();
    for number in 1..=3 {
        assert!(
            app.status.contains(&format!("Detector {number}: ")),
            "{}",
            app.status
        );
    }
    assert!(app.status.contains("did not stop"), "{}", app.status);
    // One bound is about 0.9 s of pauses; three stacked would be 2.7 s.
    assert!(took < std::time::Duration::from_millis(1800), "{took:?}");
}

/// Every shape a frame painted, with nested lists flattened.
fn flat_shapes(output: &egui::FullOutput) -> Vec<egui::Shape> {
    fn collect(shape: &egui::Shape, into: &mut Vec<egui::Shape>) {
        match shape {
            egui::Shape::Vec(inner) => inner.iter().for_each(|s| collect(s, into)),
            other => into.push(other.clone()),
        }
    }
    let mut shapes = Vec::new();
    for clipped in &output.shapes {
        collect(&clipped.shape, &mut shapes);
    }
    shapes
}

/// An analysis with no nuclides and `count` general notes, "caveat 1"...
fn analysis_with_notes(count: usize) -> mantaray_core::QuantitativeReport {
    mantaray_core::QuantitativeReport {
        peaks: Vec::new(),
        nuclides: Vec::new(),
        unidentified: Vec::new(),
        has_efficiency: false,
        sample_unit: String::new(),
        live_time: 100.0,
        real_time: 100.0,
        notes: (1..=count).map(|n| format!("caveat {n}")).collect(),
        withheld: Vec::new(),
    }
}

#[test]
fn the_analysis_dialog_shows_five_notes_and_counts_the_rest() {
    // Five notes are shown as they are; past five the rest are counted.
    let ctx = egui::Context::default();
    for (count, more) in [(5, None), (6, Some(1)), (7, Some(2))] {
        let mut app = App::headless();
        app.analysis = Some(analysis_with_notes(count));
        app.dialogs.open(Dialog::Analysis);
        let _ = frame(&mut app, &ctx, [1400.0, 1600.0]);
        let text = painted_text(&frame(&mut app, &ctx, [1400.0, 1600.0]));
        for n in 1..=5 {
            assert!(text.contains(&format!("caveat {n}")), "{count}: {text}");
        }
        assert!(!text.contains("caveat 6"), "{count}: {text}");
        match more {
            Some(rest) => assert!(
                text.contains(&format!("and {rest} more in the nuclide report")),
                "{count}: {text}"
            ),
            None => assert!(!text.contains("more in the nuclide report"), "{text}"),
        }
    }
}

#[test]
fn the_efficiency_plot_samples_the_curve_evenly_in_log_energy_from_20_to_3000_kev() {
    // The plot's x axis is ln(E / 20 keV) / ln(3000 / 20) of its width, and
    // the curve is sampled at 121 energies 20 * 150^(k / 120) keV, so the
    // samples sit at equal steps across it. The entered points are drawn with
    // their own x, independent of the sampling: points at 20 keV, at
    // 20 * 150^(1/2) = 244.949 keV and at 3000 keV fall on samples 0, 60 and
    // 120 of a flat curve exactly.
    let ctx = egui::Context::default();
    let mut app = App::headless();
    app.efficiency = Some(mantaray_core::EfficiencyCalibration::constant(0.05));
    let middle = 20.0 * 150f64.sqrt();
    app.dialogs.efficiency_points = vec![
        ("20".into(), "0.05".into()),
        (format!("{middle}"), "0.05".into()),
        ("3000".into(), "0.05".into()),
    ];
    app.dialogs.open(Dialog::Efficiency);
    let _ = frame(&mut app, &ctx, [1400.0, 2000.0]);
    let shapes = flat_shapes(&frame(&mut app, &ctx, [1400.0, 2000.0]));
    let curves: Vec<&Vec<egui::Pos2>> = shapes
        .iter()
        .filter_map(|shape| match shape {
            egui::Shape::Path(path) if path.points.len() == 121 => Some(&path.points),
            _ => None,
        })
        .collect();
    assert_eq!(curves.len(), 1, "one 121-sample curve");
    let curve = curves[0];
    let step = (curve[120].x - curve[0].x) / 120.0;
    assert!(step > 0.0, "the curve runs left to right: {curve:?}");
    for pair in curve.windows(2) {
        let dx = pair[1].x - pair[0].x;
        assert!((dx - step).abs() < 1e-3, "uneven step {dx} vs {step}");
    }
    let circles: Vec<egui::Pos2> = shapes
        .iter()
        .filter_map(|shape| match shape {
            egui::Shape::Circle(circle) if circle.radius == 3.0 => Some(circle.center),
            _ => None,
        })
        .collect();
    for k in [0, 60, 120] {
        assert!(
            circles
                .iter()
                .any(|centre| (centre.x - curve[k].x).abs() < 1e-2
                    && (centre.y - curve[k].y).abs() < 1e-2),
            "sample {k} at {:?} has no entered point on it: {circles:?}",
            curve[k]
        );
    }
}
