//! Writes a sample SVG from the real Eu-152 fixture, for eyeballing.
//!
//! `cargo test -p ortseam-gui --test svg_sample -- --nocapture` prints where.

use ortseam_gui::app::{Action, App};

#[test]
fn write_a_sample_svg_from_real_data() {
    let fixture = std::path::Path::new(env!("CARGO_MANIFEST_DIR"))
        .join("../ortseam-formats/tests/fixtures/eu152_spectra.Spe");
    if !fixture.is_file() {
        eprintln!("fixture missing - skipping");
        return;
    }
    let mut app = App::headless();
    app.recall_path(fixture);
    app.apply_action(Action::PeakSearch);
    let index = app.active.expect("active");
    let spectrum = app.active_spectrum().cloned().expect("data");
    let out = std::env::temp_dir().join("ortseam-sample.svg");
    app.write_plot_svg(&spectrum, index, &out);
    assert!(out.is_file());
    eprintln!("sample written to {}", out.display());
}
