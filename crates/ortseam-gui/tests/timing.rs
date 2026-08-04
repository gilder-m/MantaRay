//! Frame timing, so that "it feels slow" can be answered with a number.
//!
//! Ignored by default: these measure rather than assert, and a machine under
//! load would fail an assertion that a fast machine passes. Run them with
//!
//! ```text
//! cargo test -p ortseam-gui --test timing --release -- --ignored --nocapture
//! ```
//!
//! The case measured is the worst one a person actually meets: a full-size
//! spectrum with every peak marked, drawn at a full-screen size.

use std::time::Instant;

use ortseam_core::Spectrum;
use ortseam_gui::app::{Action, App};

/// A spectrum the size of the biggest instrument, with structure to find.
fn realistic(channels: usize) -> Spectrum {
    let mut spectrum = Spectrum::new(channels);
    spectrum.live_time = 3600.0;
    spectrum.real_time = 3660.0;
    spectrum.energy_calibration = Some(ortseam_core::EnergyCalibration::linear(0.5, 0.36));
    for channel in 0..channels {
        spectrum.channels[channel] = 2000 - (1800 * channel as u64 / channels as u64);
    }
    // Two hundred peaks, which is what a real background spectrum gives.
    for index in 0..200 {
        let centre = channels * (index + 1) / 201;
        for offset in -10i64..=10 {
            let value = (40_000.0 * (-0.5 * (offset as f64 / 2.5).powi(2)).exp()) as u64;
            let at = centre as i64 + offset;
            if at >= 0 && (at as usize) < channels {
                spectrum.channels[at as usize] += value;
            }
        }
    }
    spectrum
}

fn frame(app: &mut App, ctx: &egui::Context, size: [f32; 2]) -> usize {
    let input = egui::RawInput {
        screen_rect: Some(egui::Rect::from_min_size(
            egui::Pos2::ZERO,
            egui::vec2(size[0], size[1]),
        )),
        ..Default::default()
    };
    let output = ctx.run_ui(input, |ui| app.draw(ui));
    output
        .shapes
        .iter()
        .map(|clipped| match &clipped.shape {
            egui::Shape::Vec(inner) => inner.len(),
            _ => 1,
        })
        .sum()
}

/// Milliseconds per frame, averaged over a run, after letting the layout settle.
fn measure(app: &mut App, ctx: &egui::Context, label: &str) {
    let size = [1920.0, 1080.0];
    for _ in 0..5 {
        frame(app, ctx, size);
    }
    let rounds = 60;
    let started = Instant::now();
    let mut shapes = 0;
    for _ in 0..rounds {
        shapes = frame(app, ctx, size);
    }
    let each = started.elapsed().as_secs_f64() * 1000.0 / rounds as f64;
    println!("  {label:<34} {each:6.2} ms/frame   {shapes:>7} shapes");
}

#[test]
#[ignore = "measures rather than asserts; run with --ignored --nocapture"]
fn how_long_a_frame_takes() {
    println!("\nframe timing at 1920x1080:");
    for channels in [4096usize, 8192, 16384] {
        let ctx = egui::Context::default();
        let mut app = App::headless();
        app.open_buffer(format!("{channels}.Spe"), realistic(channels), None);
        app.apply_action(Action::MaximizeActive);
        measure(&mut app, &ctx, &format!("{channels} channels, no regions"));

        app.apply_action(Action::PeakSearch);
        let regions = app
            .active_spectrum()
            .map(|spectrum| spectrum.rois.len())
            .unwrap_or(0);
        measure(
            &mut app,
            &ctx,
            &format!("{channels} channels, {regions} regions"),
        );

        app.apply_action(Action::ToggleLog);
        measure(
            &mut app,
            &ctx,
            &format!("{channels} channels, {regions} regions, log"),
        );
    }
    println!();
}
