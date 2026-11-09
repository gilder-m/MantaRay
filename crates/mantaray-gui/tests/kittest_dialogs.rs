//! The MCB Properties dialog, driven the way a hand drives it.
//!
//! These run the whole app in an `egui_kittest` harness and find widgets by
//! the name a screen reader would read, then press, drag and type on them.
//! The instrument is a remote one on a recording line, so what a test checks
//! is the exact list of commands that went down the wire, not a field in the
//! app's own state. A test here fails if the widget itself starts sending,
//! whatever the code behind it looks like.
//!
//! Finding a widget by name only works if it has one, so the last tests here
//! walk every screen and dialog for controls a screen reader could not name.

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

// ---------------------------------------------------------------------------
// Every control has a name.

use egui_kittest::kittest::{AccessKitNode, NodeT, by};
use mantaray_gui::dialogs::Dialog;

/// Whether a screen reader, or a test, can act on this node: it takes the
/// keyboard or answers a click.
fn interactive(node: &AccessKitNode<'_>) -> bool {
    let data = node.data();
    data.supports_action(egui::accesskit::Action::Focus)
        || data.supports_action(egui::accesskit::Action::Click)
}

/// Whether the node has a name to be read out and found by. A label's name is
/// its text, which accesskit keeps as its value.
fn named(node: &AccessKitNode<'_>) -> bool {
    let name = if node.role() == Role::Label {
        node.value()
    } else {
        node.label()
    };
    name.is_some_and(|name| !name.trim().is_empty())
}

/// The title of the window the node is in, if it is in one.
fn window_of(node: &AccessKitNode<'_>) -> Option<String> {
    let mut up = node.parent();
    while let Some(parent) = up {
        if parent.role() == Role::Window {
            return parent.label();
        }
        up = parent.parent();
    }
    None
}

/// Whether egui made this node itself, as part of a container, rather than
/// the app as a control: a scroll bar, the edge of the status panel that
/// drags to resize it, the floor of a menu (it takes a click only to come to
/// the front, and holds the menu's items), or the strip of a window's title
/// bar that takes the drag. None is ours to name; the panel, menu or window
/// carries the name. The strip and the edge are matched by egui's own ids, so
/// if egui ever changes them they show up here as unnamed rather than
/// slipping by.
fn made_by_egui(node: &AccessKitNode<'_>) -> bool {
    if node.role() == Role::ScrollBar {
        return true;
    }
    if node.locate().0 == egui::Id::new("status").with("__resize").accesskit_id() {
        return true;
    }
    if node.role() == Role::Unknown && node.children().next().is_some() {
        return true;
    }
    // egui keys a window by its title as `Window::new` holds it, an
    // `Option` of the text.
    window_of(node).is_some_and(|title| {
        node.locate().0
            == egui::Id::new(Some(title.as_str()))
                .with("__title_click")
                .accesskit_id()
    })
}

/// One line saying where an unnamed control is: its role, what it shows,
/// where it sits, and the window it is in.
fn describe(screen: &str, node: &AccessKitNode<'_>) -> String {
    let at = node
        .bounding_box()
        .map(|rect| {
            format!(
                "({:.0}, {:.0}) {:.0}x{:.0}",
                rect.x0,
                rect.y0,
                rect.width(),
                rect.height()
            )
        })
        .unwrap_or_default();
    format!(
        "{screen}: {:?} showing {:?} at {at} in window {:?}",
        node.role(),
        node.value().unwrap_or_default(),
        window_of(node).unwrap_or_default(),
    )
}

/// Every control on screen that has no name, by node, described where it
/// was first seen.
type Unnamed = std::collections::BTreeMap<egui::accesskit::NodeId, String>;

/// Adds every control on screen that has no name to `into`.
fn unnamed(harness: &Harness<'_, App>, screen: &str, into: &mut Unnamed) {
    for node in harness
        .query_all(by().predicate(|node| interactive(node) && !named(node) && !made_by_egui(node)))
    {
        let node = node.accesskit_node();
        into.entry(node.locate().0)
            .or_insert_with(|| describe(screen, &node));
    }
}

/// Opens every collapsed section on screen, so what it hides is drawn too.
fn expand_everything(harness: &mut Harness<'_, App>) {
    for _ in 0..20 {
        let closed = harness
            .query_all(by().predicate(|node| {
                node.role() != Role::ComboBox && node.data().is_expanded() == Some(false)
            }))
            .next();
        match closed {
            Some(node) => node.click(),
            None => return,
        }
        harness.run_steps(2);
    }
}

/// The harness with a spectrum to work on as well: peaks found, a library
/// loaded, so the dialogs that list them have rows to show.
fn harness_with_data() -> (Harness<'static, App>, CommandLog) {
    let (mut harness, log) = harness();
    let app = harness.state_mut();
    let mut spectrum = mantaray_core::Spectrum::new(4096);
    spectrum.live_time = 300.0;
    spectrum.real_time = 315.0;
    spectrum.energy_calibration = Some(mantaray_core::EnergyCalibration::linear(0.5, 0.36));
    for (channel, counts) in spectrum.channels.iter_mut().enumerate() {
        *counts = 500 - (400 * channel as u64 / 4096).min(400);
    }
    for centre in [1024_i64, 2048, 3072] {
        for offset in -12_i64..=12 {
            let value = (30_000.0 * (-0.5 * (offset as f64 / 3.0).powi(2)).exp()) as u64;
            spectrum.channels[(centre + offset) as usize] += value;
        }
    }
    app.library = mantaray_core::NuclideLibrary::sample_for_tests();
    app.open_buffer("labels.Spe".into(), spectrum, None);
    app.apply_action(Action::PeakSearch);
    // A calibration point, an analysis and a report, so their tables have
    // rows.
    app.apply_action(Action::Marker(1024));
    app.apply_action(Action::AddCalibrationPoint(661.7));
    app.apply_action(Action::Analyse);
    app.apply_action(Action::RoiReport);
    harness.run();
    (harness, log)
}

/// Shows `dialog` alone, opens its folded sections, and adds what on it has
/// no name to `into`. False if the dialog did not open, so nothing was seen.
fn walk_dialog(harness: &mut Harness<'_, App>, dialog: Dialog, into: &mut Unnamed) -> bool {
    harness.state_mut().dialogs.close_all();
    harness.state_mut().apply_action(Action::Show(dialog));
    walk_open_dialog(harness, dialog, into)
}

/// The same, for a dialog something else has opened.
fn walk_open_dialog(harness: &mut Harness<'_, App>, dialog: Dialog, into: &mut Unnamed) -> bool {
    // A fixed number of frames: InSight's trace never stops moving, so
    // waiting for the screen to settle would wait forever.
    harness.run_steps(3);
    if !harness.state().dialogs.is_open(dialog) {
        return false;
    }
    expand_everything(harness);
    unnamed(harness, &format!("{dialog:?}"), into);
    true
}

#[test]
fn every_control_on_every_screen_has_a_name() {
    // A field beside a label is not named by it: the label is only text
    // painted nearby. Without a name a screen reader says "spin button" and
    // a test cannot find the field without counting pixels.
    let mut missing = Unnamed::new();
    let mut unopened = Vec::new();

    // The main window, its toolbar drawn each of the three ways.
    let (mut harness, _log) = harness_with_data();
    for style in [
        mantaray_gui::theme::IconStyle::Text,
        mantaray_gui::theme::IconStyle::Icons,
        mantaray_gui::theme::IconStyle::Both,
    ] {
        harness.state_mut().style.icons = style;
        harness.run();
        unnamed(
            &harness,
            &format!("main window, {style:?} toolbar"),
            &mut missing,
        );
    }
    harness.state_mut().style.icons = mantaray_gui::theme::IconStyle::Text;
    harness.run();

    // Every menu.
    for menu in [
        "File",
        "Edit",
        "Acquire",
        "Calculate",
        "Services",
        "ROI",
        "Display",
        "Window",
        "Help",
    ] {
        harness.get_by_label(menu).click();
        harness.run();
        unnamed(&harness, &format!("{menu} menu"), &mut missing);
        harness.get_by_label(menu).click();
        harness.run();
    }

    // Every dialog, over the spectrum and then over the instrument's own
    // window, since several show different controls for each.
    let mut dialogs: Vec<Dialog> = Dialog::all()
        .iter()
        .copied()
        .filter(|dialog| *dialog != Dialog::ConfirmClose)
        .collect();
    dialogs.push(Dialog::Workspace);
    let (mut detector, _log) = self::harness();
    for dialog in &dialogs {
        for app in [&mut harness, &mut detector] {
            if !walk_dialog(app, *dialog, &mut missing) {
                unopened.push(format!("{dialog:?}"));
            }
        }
    }

    // Every tab of the MCB Properties.
    detector.state_mut().dialogs.close_all();
    detector.run();
    open_properties(&mut detector, "Amplifier");
    for tab in [
        "Amplifier",
        "ADC",
        "Presets",
        "High Voltage",
        "Stabilizer",
        "About",
    ] {
        detector.get_by_label(tab).click();
        detector.run();
        expand_everything(&mut detector);
        unnamed(&detector, &format!("MCB Properties, {tab}"), &mut missing);
    }

    // Closing a spectrum that has unsaved changes, which asks first.
    harness.state_mut().dialogs.close_all();
    let id = {
        let app = harness.state_mut();
        let window = app.windows.last_mut().expect("the spectrum window");
        window.modified = true;
        window.id
    };
    harness.state_mut().apply_action(Action::CloseWindow(id));
    if !walk_open_dialog(&mut harness, Dialog::ConfirmClose, &mut missing) {
        unopened.push("ConfirmClose".into());
    }

    // The offer of unfinished work, which only a crash brings up.
    harness.state_mut().dialogs.close_all();
    harness.state_mut().recovered = Some(mantaray_gui::session::Session {
        saved_at: "earlier".into(),
        windows: vec![mantaray_gui::session::SavedWindow {
            title: "left behind".into(),
            path: None,
            spectrum: mantaray_core::Spectrum::from_counts(vec![1; 64]),
            display: Default::default(),
            calibration: Default::default(),
            modified: true,
            floating: false,
        }],
    });
    if !walk_dialog(&mut harness, Dialog::Recover, &mut missing) {
        unopened.push("Recover".into());
    }

    assert_eq!(unopened, Vec::<String>::new(), "these dialogs did not open");
    let missing: Vec<&String> = missing.values().collect();
    assert!(
        missing.is_empty(),
        "{} controls have no name:\n{}",
        missing.len(),
        missing
            .iter()
            .map(|line| line.as_str())
            .collect::<Vec<_>>()
            .join("\n")
    );
}

#[test]
fn controls_answer_to_the_words_beside_them() {
    // A name is only as good as the label it points at: these are found by
    // what the user reads next to them, so a field pointed at the wrong
    // label fails here even though it has a name.
    let (mut harness, _log) = harness_with_data();
    harness.state_mut().style.icons = mantaray_gui::theme::IconStyle::Icons;
    harness.run();
    // A drawn icon, by the word the Both arrangement puts beside it.
    harness.get_by_role_and_label(Role::Button, "Start");
    harness.get_by_label("Spectrum");

    // A slider is two widgets, the bar and its number box: both are named.
    harness
        .state_mut()
        .apply_action(Action::Show(Dialog::Settings));
    harness.run();
    assert_eq!(
        harness.query_all_by_label("Sensitivity factor").count(),
        2,
        "the slider and its number box"
    );

    // A row of a table, by the column header over it.
    harness.state_mut().dialogs.close_all();
    harness
        .state_mut()
        .apply_action(Action::Show(Dialog::Calibration));
    harness.run();
    harness.get_by_role_and_label(Role::TextInput, "channel");

    // A bare checkbox, by its row label rather than its empty text.
    let (mut detector, _log) = self::harness();
    open_properties(&mut detector, "Presets");
    detector.get_by_role_and_label(Role::CheckBox, "Real time (s)");
    detector.get_by_role_and_label(Role::SpinButton, "Real time (s)");
}

/// The app with no instrument at all, in a harness, the given dialog open.
fn bare_harness(dialog: mantaray_gui::dialogs::Dialog) -> Harness<'static, App> {
    let mut app = App::headless();
    while !app.detectors.is_empty() {
        app.apply_action(Action::RemoveDetector(0));
    }
    app.dialogs.open(dialog);
    let mut harness = Harness::builder()
        .with_size(egui::vec2(1400.0, 900.0))
        .build_ui_state(|ui, app: &mut App| app.draw(ui), app);
    harness
        .ctx
        .all_styles_mut(|style| style.animation_time = 0.0);
    harness.run();
    harness
}

#[test]
fn a_constant_efficiency_typed_as_a_percent_is_refused() {
    // Typed 5, meaning 5 %: the field clamped it to 1 and Use put 100 % in
    // force, so every activity came out 20 times too small for a 5 %
    // detector. The command line refuses the same value with the same words.
    let mut harness = bare_harness(mantaray_gui::dialogs::Dialog::Efficiency);
    let label = "Constant efficiency";
    harness
        .get_by_role_and_label(Role::SpinButton, label)
        .click();
    harness.run();
    harness.key_press_modifiers(egui::Modifiers::COMMAND, egui::Key::A);
    harness.run();
    harness
        .get_by_role_and_label(Role::SpinButton, label)
        .type_text("5");
    harness.run();
    harness.key_press(egui::Key::Enter);
    harness.run();
    harness.get_by_label("Use").click();
    harness.run();
    let app = harness.state();
    assert_eq!(app.efficiency, None, "typing 5 put a curve in force");
    assert!(!app.status.contains("in force"), "{}", app.status);
    assert!(
        app.status.contains(
            "5 is not an efficiency: give a fraction above 0 and at most 1 (5 % is 0.05)"
        ),
        "{}",
        app.status
    );

    // The fraction itself is taken.
    harness
        .get_by_role_and_label(Role::SpinButton, label)
        .click();
    harness.run();
    harness.key_press_modifiers(egui::Modifiers::COMMAND, egui::Key::A);
    harness.run();
    harness
        .get_by_role_and_label(Role::SpinButton, label)
        .type_text("0.05");
    harness.run();
    harness.key_press(egui::Key::Enter);
    harness.run();
    harness.get_by_label("Use").click();
    harness.run();
    let app = harness.state();
    assert_eq!(
        app.efficiency,
        Some(mantaray_core::EfficiencyCalibration::constant(0.05))
    );
    assert!(app.status.contains("in force"), "{}", app.status);
}
