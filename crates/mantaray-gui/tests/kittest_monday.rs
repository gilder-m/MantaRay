//! The bench-day flow faults that need a hand on the real widgets: a preset
//! typed without Apply (flow review S2), and a sample description typed into
//! its box (S3). Driven in an `egui_kittest` harness by accessibility name,
//! on a remote instrument over a recording line, so what is checked is what
//! went down the wire.

use egui::accesskit::Role;
use egui_kittest::Harness;
use egui_kittest::kittest::Queryable;
use mantaray_device::{CommandLog, RecordingTransport};
use mantaray_gui::app::{Action, App};
use mantaray_gui::dialogs::Dialog;

fn harness() -> (Harness<'static, App>, CommandLog) {
    let (remote, log) = RecordingTransport::connect(1, "BENCH").expect("the recording line");
    let mut app = App::headless();
    app.detectors = vec![remote.into()];
    app.apply_action(Action::OpenDetector(0));
    let mut harness = Harness::builder()
        .with_size(egui::vec2(1400.0, 900.0))
        .build_ui_state(|ui, app: &mut App| app.draw(ui), app);
    harness
        .ctx
        .all_styles_mut(|style| style.animation_time = 0.0);
    harness.run();
    (harness, log)
}

fn open_presets(harness: &mut Harness<'_, App>) {
    harness.get_by_label("Acquire").click();
    harness.run();
    harness.get_by_label("MCB Properties...").click();
    harness.run();
    harness.get_by_label("Presets").click();
    harness.run();
}

/// Ticks Real time and types `seconds` into it, one key a frame. The
/// recording line's instrument holds a live time of 300 and no real time.
fn type_real_time(harness: &mut Harness<'_, App>, seconds: &str) {
    harness
        .get_by_role_and_label(Role::CheckBox, "Real time (s)")
        .click();
    harness.run();
    harness
        .get_by_role_and_label(Role::SpinButton, "Real time (s)")
        .click();
    harness.run();
    harness.key_press_modifiers(egui::Modifiers::COMMAND, egui::Key::A);
    harness.run();
    for key in seconds.chars() {
        harness
            .get_by_role_and_label(Role::SpinButton, "Real time (s)")
            .type_text(&key.to_string());
        harness.run();
    }
    harness.key_press(egui::Key::Enter);
    harness.run();
}

fn press_start(harness: &mut Harness<'_, App>) {
    harness.state_mut().apply_action(Action::Start);
    harness.run();
}

fn started(log: &CommandLog) -> bool {
    RecordingTransport::mutating(log)
        .iter()
        .any(|command| command == "START")
}

fn not_applied_shown(harness: &Harness<'_, App>) -> bool {
    harness.query_by_label("not applied yet").is_some()
}

#[test]
fn a_typed_preset_says_it_is_not_applied_yet() {
    let (mut harness, log) = harness();
    open_presets(&mut harness);
    assert!(!not_applied_shown(&harness), "nothing typed yet");
    type_real_time(&mut harness, "60");
    assert!(not_applied_shown(&harness));
    assert_eq!(RecordingTransport::mutating(&log), Vec::<String>::new());
}

#[test]
fn start_refuses_while_a_typed_preset_is_not_applied() {
    let (mut harness, log) = harness();
    open_presets(&mut harness);
    type_real_time(&mut harness, "60");
    press_start(&mut harness);
    assert!(!started(&log), "the count went out without the typed preset");
    let status = harness.state().status.clone();
    assert!(status.contains("not applied"), "{status}");
    assert!(status.contains("Apply presets"), "{status}");
}

#[test]
fn closing_the_dialog_does_not_lose_the_typed_preset_or_let_start_through() {
    let (mut harness, log) = harness();
    open_presets(&mut harness);
    type_real_time(&mut harness, "60");
    harness
        .state_mut()
        .dialogs
        .set(Dialog::McbProperties, false);
    harness.run();
    press_start(&mut harness);
    assert!(!started(&log), "closing the dialog let the unlimited count start");

    // Reopened, the draft is there, marked.
    open_presets(&mut harness);
    assert!(not_applied_shown(&harness));
}

#[test]
fn applying_the_typed_preset_lets_start_go_with_it() {
    let (mut harness, log) = harness();
    open_presets(&mut harness);
    type_real_time(&mut harness, "60");
    harness.get_by_label("Apply presets").click();
    harness.run();
    press_start(&mut harness);
    let sent = RecordingTransport::mutating(&log);
    let start = sent.iter().position(|command| command == "START");
    let preset = sent
        .iter()
        .position(|command| command.contains("REAL") && command.contains("60"));
    assert!(
        matches!((preset, start), (Some(preset), Some(start)) if preset < start),
        "the preset must go before START: {sent:?}"
    );
}

#[test]
fn discarding_the_typed_preset_sends_nothing_and_lets_start_go() {
    let (mut harness, log) = harness();
    open_presets(&mut harness);
    type_real_time(&mut harness, "60");
    harness.get_by_label("Discard preset changes").click();
    harness.run();
    assert!(!not_applied_shown(&harness));
    assert_eq!(RecordingTransport::mutating(&log), Vec::<String>::new());
    press_start(&mut harness);
    assert!(started(&log), "{}", harness.state().status);
}

#[test]
fn typing_a_sample_description_makes_the_window_unsaved() {
    let (mut harness, _log) = harness();
    let index = harness.state().active.expect("the detector window");
    harness.state_mut().windows[index].modified = false;
    harness.state_mut().apply_action(Action::Show(Dialog::Sample));
    harness.run();
    harness
        .get_by_role_and_label(Role::MultilineTextInput, "Sample Description")
        .click();
    harness.run();
    harness
        .get_by_role_and_label(Role::MultilineTextInput, "Sample Description")
        .type_text("Soil core 4");
    harness.run();
    assert_eq!(harness.state().sample.description, "Soil core 4");
    assert!(
        harness.state().windows[index].modified,
        "the description goes into the next save, and Exit would not ask"
    );
}
