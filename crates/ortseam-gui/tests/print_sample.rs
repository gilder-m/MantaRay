//! Writes a sample printable page from the real Eu-152 fixture, for eyeballing.

use ortseam_gui::app::{Action, App};

#[test]
fn write_a_sample_print_page() {
    let fixture = std::path::Path::new(env!("CARGO_MANIFEST_DIR"))
        .join("../ortseam-formats/tests/fixtures/eu152_spectra.Spe");
    if !fixture.is_file() {
        eprintln!("fixture missing - skipping");
        return;
    }
    let mut app = App::headless();
    app.recall_path(fixture);
    app.apply_action(Action::PeakSearch);
    let document = app.print_document().expect("a document");
    let out = std::env::temp_dir().join("ortseam-print-sample.html");
    std::fs::write(&out, document).expect("write");
    eprintln!("sample written to {}", out.display());
}
