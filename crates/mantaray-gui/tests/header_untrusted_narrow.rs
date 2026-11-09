//! Review of c449cd6 (Lucius, review-fix-batches): while a reading is not
//! believed, the window header's "not updating" mark pushed its counts and
//! maximise button past the right edge of any window narrower than about
//! 850 px. Measured on the drawn header (egui_kittest), at the widths a tiled
//! or half-screen detector window gets.

use egui_kittest::Harness;
use egui_kittest::kittest::Queryable;
use mantaray_core::Spectrum;
use mantaray_gui::theme::Theme;
use mantaray_gui::view::{MarkMode, ViewHeader, ViewInput, draw};
use mantaray_gui::viewmodel::DisplayState;

const REASON: &str = "the status reply is not a status line: \"ERR 0\"";

fn header_harness(width: f32, untrusted: Option<&'static str>) -> Harness<'static> {
    let mut spectrum = Spectrum::new(1024);
    spectrum.live_time = 100.0;
    spectrum.real_time = 100.0;
    spectrum.channels.fill(100);
    let mut display = DisplayState::for_length(spectrum.len());
    let colors = Theme::DeepSpace.colors();
    let mut harness = Harness::builder()
        .with_size(egui::vec2(width, 300.0))
        .build_ui(move |ui| {
            let _ = draw(
                ui,
                ViewInput {
                    spectrum: &spectrum,
                    compare: None,
                    compare_offset: 0,
                    display: &mut display,
                    colors: &colors,
                    mark_mode: MarkMode::Off,
                    library: None,
                    isotope: None,
                    isotope_min_intensity: 1.0,
                    style: mantaray_gui::theme::SchemeStyle::default(),
                    selection: None,
                    peak_info: None,
                    peak_font: 11.0,
                    header: ViewHeader {
                        source: "narrow header test",
                        counting: true,
                        dead_time: Some(3.2),
                        active: true,
                        maximized: false,
                        untrusted,
                    },
                },
            );
        });
    for _ in 0..3 {
        harness.step();
    }
    harness
}

/// The header's own rect for `label`: the topmost node naming it (the
/// marker line further down has a "cnts" of its own).
fn header_rect(harness: &Harness<'static>, label: &str, case: &str) -> egui::Rect {
    harness
        .query_all_by_label_contains(label)
        .map(|node| node.rect())
        .min_by(|a, b| a.top().total_cmp(&b.top()))
        .unwrap_or_else(|| panic!("{case}: no {label:?} in the header"))
}

fn assert_header_fits(width: f32, untrusted: Option<&'static str>) {
    let case = format!(
        "{width} px, readings {}",
        if untrusted.is_some() {
            "not believed"
        } else {
            "believed"
        }
    );
    let harness = header_harness(width, untrusted);
    let button = header_rect(&harness, "maximise", &case);
    let counts = header_rect(&harness, "cnts", &case);
    let dead = header_rect(&harness, "dead", &case);
    assert!(
        button.left() >= 0.0 && button.right() <= width,
        "{case}: the maximise button is off the window: {button:?}"
    );
    assert!(
        counts.right() <= button.left(),
        "{case}: the counts run into the maximise button: counts {counts:?}, button {button:?}"
    );
    assert!(dead.right() <= counts.left(), "{case}: dead time {dead:?}");
    if untrusted.is_some() {
        let mark = header_rect(&harness, "not updating", &case);
        assert!(
            mark.right() <= counts.left(),
            "{case}: the mark {mark:?} runs into the counts {counts:?}"
        );
    }
}

#[test]
fn the_header_fits_with_readings_believed() {
    for width in [360.0, 480.0, 700.0, 900.0] {
        assert_header_fits(width, None);
    }
}

#[test]
fn the_header_fits_while_readings_are_not_believed() {
    for width in [360.0, 480.0, 700.0, 900.0] {
        assert_header_fits(width, Some(REASON));
    }
}

/// A roomy window still says it in full.
#[test]
fn a_roomy_window_says_the_last_good_reading_in_full() {
    let harness = header_harness(1000.0, Some(REASON));
    assert!(
        harness
            .query_by_label_contains("not updating - last good reading")
            .is_some()
    );
}
