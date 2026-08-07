//! mantaray - a modern multichannel-analyzer emulator and spectrum workbench.
//!
//! The application itself is [`mantaray_gui`]; this only starts it.

#![forbid(unsafe_code)]
// A desktop application, not a console one, on Windows.
#![cfg_attr(not(debug_assertions), windows_subsystem = "windows")]

use mantaray_gui::app;

/// Files named on the command line: spectra are opened in buffer windows and a
/// `.JOB` file is run, as MAESTRO's command line does.
fn arguments() -> Vec<std::path::PathBuf> {
    std::env::args_os()
        .skip(1)
        .map(std::path::PathBuf::from)
        .filter(|path| path.exists())
        .collect()
}

/// An `x,y` pair from an environment variable, for placing the window on a
/// chosen display (`MANTARAY_WINDOW_POS=2560,0` puts it on a second monitor).
fn pair_from_env(name: &str) -> Option<[f32; 2]> {
    let value = std::env::var(name).ok()?;
    let (x, y) = value.split_once(',')?;
    Some([x.trim().parse().ok()?, y.trim().parse().ok()?])
}

/// The window icon, drawn rather than shipped: a spectrum in miniature, in the
/// default palette - a teal peak over its amber region on the dark plot.
fn icon() -> egui::IconData {
    const SIZE: usize = 32;
    let colors = mantaray_gui::theme::Theme::DeepSpace.colors();
    let mut rgba = vec![0u8; SIZE * SIZE * 4];
    // The trace: a Gaussian peak on a sloping continuum.
    let height_at = |x: usize| -> f64 {
        let t = x as f64 / (SIZE - 1) as f64;
        let continuum = 0.28 - 0.12 * t;
        let peak = 0.62 * (-((t - 0.42) / 0.10).powi(2)).exp();
        continuum + peak
    };
    for x in 0..SIZE {
        let level = (height_at(x) * SIZE as f64) as usize;
        let in_region = (10..=17).contains(&x);
        for y in 0..SIZE {
            let from_bottom = SIZE - 1 - y;
            let index = (y * SIZE + x) * 4;
            let colour = if from_bottom == level || from_bottom == level + 1 {
                colors.foreground
            } else if from_bottom < level && in_region {
                colors.roi
            } else if from_bottom < level {
                mantaray_gui::theme::Rgb(
                    colors.background.0 + 18,
                    colors.background.1 + 22,
                    colors.background.2 + 26,
                )
            } else {
                colors.background
            };
            rgba[index] = colour.0;
            rgba[index + 1] = colour.1;
            rgba[index + 2] = colour.2;
            rgba[index + 3] = 255;
        }
    }
    egui::IconData {
        rgba,
        width: SIZE as u32,
        height: SIZE as u32,
    }
}

fn main() -> eframe::Result {
    // A windows-subsystem binary has no console; a panic must leave a file.
    mantaray_gui::crash::install();
    // Before eframe opens its storage, not after: this program was called
    // something else until recently, and everything a previous run saved -
    // the theme, the tuned colours, the recent files, and any snapshot of
    // work a crash left behind - is filed under the old name. Reading the new
    // location first would find it empty and start as though this were a
    // fresh install, which for the snapshot means losing a count outright.
    let _ = mantaray_gui::session::directory();
    let files = arguments();
    let mut viewport = egui::ViewportBuilder::default()
        .with_inner_size(pair_from_env("MANTARAY_WINDOW_SIZE").unwrap_or([1360.0, 860.0]))
        .with_min_inner_size([900.0, 600.0])
        .with_icon(icon())
        // Created hidden, and shown once there is something to look at. A
        // window mapped before the first paint is an empty frame flashing on
        // screen while start-up work happens behind it.
        .with_visible(false)
        .with_title(mantaray_gui::DISPLAY_NAME);
    if let Some(position) = pair_from_env("MANTARAY_WINDOW_POS") {
        viewport = viewport.with_position(position);
    }
    let options = eframe::NativeOptions {
        viewport,
        ..Default::default()
    };
    eframe::run_native(
        mantaray_gui::APPLICATION_ID,
        options,
        Box::new(move |cc| {
            let mut app = app::App::new(cc);
            let had_arguments = !files.is_empty();
            app.open_arguments(&files);
            app.maybe_reopen_last(had_arguments);
            // Anything a previous run left behind, offered before the first
            // frame so the question is the first thing seen.
            app.look_for_unfinished_work();
            app.settle_layout();
            Ok(Box::new(app))
        }),
    )
}
