//! The MCB Properties dialog, driven the way a hand drives it.
//!
//! These run the whole app in an `egui_kittest` harness and find widgets by
//! the name a screen reader would read, then press, drag and type on them.
//! The instrument is a remote one on a recording line, so what a test checks
//! is the exact list of commands that went down the wire, not a field in the
//! app's own state. A test here fails if the widget itself starts sending,
//! whatever the code behind it looks like.

use egui::accesskit::Role;
use egui_kittest::Harness;
use egui_kittest::kittest::Queryable;
use mantaray_device::{CommandLog, RecordingTransport};
use mantaray_gui::app::{Action, App};

/// The app with one remote instrument (number 1) on a recording line, its
/// window open, in a harness the size of a laptop screen. The log starts
/// empty: the handshake is not part of what a test causes.
fn harness() -> (Harness<'static, App>, CommandLog) {
    let (remote, log) = RecordingTransport::connect(1, "BENCH").expect("the recording line");
    let mut app = App::headless();
    app.detectors = vec![remote.into()];
    app.apply_action(Action::OpenDetector(0));
    let mut harness = Harness::builder()
        .with_size(egui::vec2(1400.0, 900.0))
        .build_ui_state(|ui, app: &mut App| app.draw(ui), app);
    // A headless frame does not advance the clock, so an animation (a menu
    // opening) would stop part-way. Tests want the end state.
    harness
        .ctx
        .all_styles_mut(|style| style.animation_time = 0.0);
    harness.run();
    (harness, log)
}

/// What the app sent that changes the instrument, oldest first.
fn mutating(log: &CommandLog) -> Vec<String> {
    RecordingTransport::mutating(log)
}

/// Opens MCB Properties from the Acquire menu and shows `tab`.
fn open_properties(harness: &mut Harness<'_, App>, tab: &str) {
    harness.get_by_label("Acquire").click();
    harness.run();
    harness.get_by_label("MCB Properties...").click();
    harness.run();
    harness.get_by_label(tab).click();
    harness.run();
}

/// The number field named `label`.
fn field<'h>(harness: &'h Harness<'_, App>, label: &'h str) -> egui_kittest::Node<'h> {
    harness.get_by_role_and_label(Role::SpinButton, label)
}

/// What the field named `label` shows, as a number.
fn shown(harness: &Harness<'_, App>, label: &str) -> f64 {
    let value = field(harness, label)
        .value()
        .unwrap_or_else(|| panic!("{label:?} shows no value"));
    value
        .trim()
        .parse()
        .unwrap_or_else(|_| panic!("{label:?} shows {value:?}, not a number"))
}

/// Presses on the field named `label`, moves the pointer `dx` points to the
/// right over `steps` frames, and lets go: a drag, frame by frame.
fn drag(harness: &mut Harness<'_, App>, label: &str, dx: f32, steps: u16) {
    let start = field(harness, label).rect().center();
    harness.hover_at(start);
    harness.drag_at(start);
    harness.run();
    for step in 1..=steps {
        let at = start + egui::vec2(dx * f32::from(step) / f32::from(steps), 0.0);
        harness.hover_at(at);
        harness.run();
    }
    harness.drop_at(start + egui::vec2(dx, 0.0));
    harness.run();
}

/// Clicks the field named `label`, types `text` over what it held one key a
/// frame, as fingers do, and presses Enter.
fn type_into(harness: &mut Harness<'_, App>, label: &str, text: &str) {
    field(harness, label).click();
    harness.run();
    harness.key_press_modifiers(egui::Modifiers::COMMAND, egui::Key::A);
    harness.run();
    for key in text.chars() {
        field(harness, label).type_text(&key.to_string());
        harness.run();
    }
    harness.key_press(egui::Key::Enter);
    harness.run();
}

fn not_applied_shown(harness: &Harness<'_, App>) -> bool {
    harness.query_by_label("not applied yet").is_some()
}

/// The shared case: `label` on `tab` is dragged, sends nothing, and Apply
/// sends exactly the value the field then shows, once.
fn a_drag_sends_nothing_until_applied(tab: &str, label: &str, command: &str) {
    let (mut harness, log) = harness();
    open_properties(&mut harness, tab);
    assert_eq!(mutating(&log), Vec::<String>::new(), "opening the tab sent");
    let before = shown(&harness, label);

    drag(&mut harness, label, 40.0, 8);
    let after = shown(&harness, label);
    assert!(
        after > before,
        "the drag did not raise {label:?}: {before} -> {after}"
    );
    assert_eq!(
        mutating(&log),
        Vec::<String>::new(),
        "the drag of {label:?} reached the instrument"
    );
    assert!(
        not_applied_shown(&harness),
        "the tab says the value is pending"
    );

    harness.get_by_label("Apply").click();
    harness.run();
    assert_eq!(mutating(&log), vec![format!("{command} {after:.0}")]);
    assert!(
        !not_applied_shown(&harness),
        "an applied value is not pending"
    );
    assert_eq!(shown(&harness, label), after, "Apply kept the shown value");
}

#[test]
fn a_coarse_gain_drag_sends_nothing_until_applied() {
    // The coarse gain went to the instrument as the field changed: a drag
    // from 20 to 40 sent every gain it crossed (ledger C-1).
    a_drag_sends_nothing_until_applied("Amplifier", "Coarse gain", "SET_GAIN_COARSE");
}

#[test]
fn a_bias_drag_sends_nothing_until_applied() {
    // The bias drag sent every voltage it crossed to a live detector.
    a_drag_sends_nothing_until_applied("High Voltage", "Target (V)", "SET_HV");
}

/// The shared case for typing: every keystroke stays in the field, and
/// Apply sends the whole number typed.
fn typing_sends_nothing_until_applied(tab: &str, label: &str, typed: &str, command: &str) {
    let (mut harness, log) = harness();
    open_properties(&mut harness, tab);
    type_into(&mut harness, label, typed);
    assert_eq!(
        mutating(&log),
        Vec::<String>::new(),
        "typing {typed} into {label:?} sent its first digits"
    );
    assert_eq!(shown(&harness, label).to_string(), typed);
    assert!(
        not_applied_shown(&harness),
        "the tab says the value is pending"
    );

    harness.get_by_label("Apply").click();
    harness.run();
    assert_eq!(mutating(&log), vec![format!("{command} {typed}")]);
    assert!(!not_applied_shown(&harness));
}

#[test]
fn a_coarse_gain_typed_into_the_field_does_not_reach_the_instrument_until_applied() {
    // Typing 40 sent 4 and then 40.
    typing_sends_nothing_until_applied("Amplifier", "Coarse gain", "40", "SET_GAIN_COARSE");
}

#[test]
fn a_bias_being_typed_does_not_reach_the_instrument_until_applied() {
    // Typing 1500 sent 1, 15 and 150 first.
    typing_sends_nothing_until_applied("High Voltage", "Target (V)", "1500", "SET_HV");
}

#[test]
fn walking_the_property_tabs_sends_nothing() {
    // Merely opening the ADC tab once sent SET_HV 3000 (remote.rs, the
    // comment over set_properties).
    let (mut harness, log) = harness();
    open_properties(&mut harness, "Amplifier");
    for tab in [
        "Amplifier",
        "ADC",
        "Presets",
        "High Voltage",
        "Stabilizer",
        "About",
        "Amplifier",
    ] {
        harness.get_by_label(tab).click();
        harness.run();
        assert_eq!(
            mutating(&log),
            Vec::<String>::new(),
            "showing the {tab} tab sent"
        );
    }
}
