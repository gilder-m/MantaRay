//! Publication snapshots: the spectrum plot as an SVG file.
//!
//! The display draws for the screen; this draws the same picture for a report -
//! vector, self-contained, and styled by the same palette, so what lands in a
//! lab book matches what was on the instrument. Kept deliberately simpler than
//! the live view: no overview inset, no hover work, no selection chrome.

use mantaray_core::{NuclideLibrary, Spectrum};

use crate::theme::SpectrumColors;
use crate::viewmodel::{DisplayState, VerticalScale};

/// Left room for count labels, bottom room for energy labels and the title.
const LEFT: f32 = 64.0;
const BOTTOM: f32 = 46.0;
const TOP: f32 = 34.0;
const RIGHT: f32 = 16.0;

/// Renders what the display shows as a standalone SVG document.
pub fn plot_svg(
    spectrum: &Spectrum,
    display: &DisplayState,
    colors: &SpectrumColors,
    library: Option<&NuclideLibrary>,
    width: f32,
    height: f32,
) -> String {
    let mut svg = String::with_capacity(64 * 1024);
    let hex = |rgb: crate::theme::Rgb| format!("#{:02x}{:02x}{:02x}", rgb.0, rgb.1, rgb.2);
    let plot = (
        LEFT,
        TOP,
        (width - LEFT - RIGHT).max(50.0),
        (height - TOP - BOTTOM).max(50.0),
    );
    let (px, py, pw, ph) = plot;

    svg.push_str(&format!(
        r#"<svg xmlns="http://www.w3.org/2000/svg" width="{width}" height="{height}" viewBox="0 0 {width} {height}" font-family="monospace">"#
    ));
    svg.push('\n');
    // Background, with the same subtle wash the screen has.
    svg.push_str(&format!(
        r#"<defs><linearGradient id="wash" x1="0" y1="0" x2="0" y2="1">
<stop offset="0" stop-color="{}"/><stop offset="1" stop-color="{}"/>
</linearGradient></defs>
<rect width="{width}" height="{height}" fill="{}"/>
<rect x="{px}" y="{py}" width="{pw}" height="{ph}" fill="url(#wash)"/>
"#,
        hex(colors.background),
        hex(colors.background_wash()),
        hex(colors.background),
    ));

    let full_scale = display.full_scale(spectrum);
    let (start, end) = display.visible();
    let axis = hex(colors.axes);

    // Horizontal gridlines and count labels: decades in log, quarters otherwise.
    match display.vertical {
        VerticalScale::Logarithmic => {
            let mut decade = 1u64;
            while decade <= full_scale {
                let fraction = display.height_fraction(decade, full_scale);
                if fraction > 0.02 && fraction < 0.99 {
                    let y = py + ph - fraction * ph;
                    svg.push_str(&format!(
                        r#"<line x1="{px}" y1="{y:.1}" x2="{:.1}" y2="{y:.1}" stroke="{axis}" stroke-opacity="0.3"/>
<text x="{:.1}" y="{:.1}" fill="{axis}" font-size="11" text-anchor="end">{}</text>
"#,
                        px + pw,
                        px - 6.0,
                        y + 4.0,
                        crate::view::format_axis_count(decade),
                    ));
                }
                decade = decade.saturating_mul(10);
            }
        }
        _ => {
            for step in 1..=4u32 {
                let fraction = step as f32 / 4.0;
                let y = py + ph - fraction * ph;
                svg.push_str(&format!(
                    r#"<line x1="{px}" y1="{y:.1}" x2="{:.1}" y2="{y:.1}" stroke="{axis}" stroke-opacity="0.3"/>
<text x="{:.1}" y="{:.1}" fill="{axis}" font-size="11" text-anchor="end">{}</text>
"#,
                    px + pw,
                    px - 6.0,
                    y + 4.0,
                    crate::view::format_axis_count((full_scale as f64 * fraction as f64) as u64),
                ));
            }
        }
    }

    // Vertical ticks at round values.
    let calibrated = spectrum.energy_calibration.is_some() && display.energy_axis;
    let (low, high) = match (&spectrum.energy_calibration, calibrated) {
        (Some(cal), true) => (cal.energy(start as f64), cal.energy(end as f64)),
        _ => (start as f64, end as f64),
    };
    let (low, high) = (low.min(high), low.max(high));
    for value in crate::view::nice_ticks(low, high, (pw / 90.0).clamp(3.0, 10.0) as usize) {
        let channel = match (&spectrum.energy_calibration, calibrated) {
            (Some(cal), true) => match cal.channel(value) {
                Some(channel) => channel,
                None => continue,
            },
            _ => value,
        };
        let span = (end - start).max(1) as f64;
        let fraction = ((channel - start as f64) / span) as f32;
        if !(0.0..=1.0).contains(&fraction) {
            continue;
        }
        let x = px + pw * fraction;
        svg.push_str(&format!(
            r#"<line x1="{x:.1}" y1="{py}" x2="{x:.1}" y2="{:.1}" stroke="{axis}" stroke-opacity="0.08"/>
<line x1="{x:.1}" y1="{:.1}" x2="{x:.1}" y2="{:.1}" stroke="{axis}"/>
<text x="{x:.1}" y="{:.1}" fill="{axis}" font-size="11" text-anchor="middle">{value:.0}</text>
"#,
            py + ph,
            py + ph,
            py + ph + 4.0,
            py + ph + 17.0,
        ));
    }

    // The trace: one column per horizontal SVG unit, like the screen. Filled
    // areas become per-run polygons, the outline a polyline.
    let columns = pw.floor().max(1.0) as usize;
    let visible = end.saturating_sub(start) + 1;
    let mut outline = String::new();
    let mut runs: Vec<(bool, Vec<(f32, f32)>)> = Vec::new();
    for column in 0..columns {
        let first = start + column * visible / columns;
        let last = (start + ((column + 1) * visible) / columns)
            .max(first + 1)
            .min(spectrum.len());
        if first >= spectrum.len() {
            break;
        }
        let value = spectrum.channels[first..last]
            .iter()
            .copied()
            .max()
            .unwrap_or(0);
        let fraction = display.height_fraction(value, full_scale);
        let x = px + column as f32 + 0.5;
        let y = py + ph - fraction * ph;
        outline.push_str(&format!("{x:.1},{y:.1} "));
        let in_roi = (first..last).any(|channel| spectrum.rois.at(channel).is_some());
        match runs.last_mut() {
            Some((was_roi, points)) if *was_roi == in_roi => points.push((x, y)),
            _ => runs.push((in_roi, vec![(x, y)])),
        }
    }
    for (in_roi, points) in &runs {
        if points.len() < 2 {
            continue;
        }
        let colour = if *in_roi {
            hex(colors.roi)
        } else {
            hex(colors.foreground)
        };
        let mut polygon = String::new();
        for (x, y) in points {
            polygon.push_str(&format!("{x:.1},{y:.1} "));
        }
        let (last_x, first_x) = (points[points.len() - 1].0, points[0].0);
        polygon.push_str(&format!(
            "{last_x:.1},{:.1} {first_x:.1},{:.1}",
            py + ph,
            py + ph
        ));
        let opacity = if *in_roi { 0.55 } else { 0.35 };
        svg.push_str(&format!(
            r#"<polygon points="{polygon}" fill="{colour}" fill-opacity="{opacity}"/>
"#
        ));
    }
    svg.push_str(&format!(
        r#"<polyline points="{outline}" fill="none" stroke="{}" stroke-width="1.2"/>
"#,
        hex(colors.foreground),
    ));

    // Region labels, as on screen: left to right, skipping any that would land
    // on the one before, so a busy spectrum stays legible.
    if calibrated {
        let mut last_right = f32::MIN;
        for roi in spectrum.rois.iter() {
            if roi.end < start || roi.start > end {
                continue;
            }
            let centre = roi.center().round() as usize;
            let Some(fraction) = display.fraction_of(centre) else {
                continue;
            };
            let Some(label) = crate::view::peak_label(spectrum, library, centre) else {
                continue;
            };
            let x = px + pw * fraction;
            // Monospace at 10px runs about 6px a character.
            let half_width = label.len() as f32 * 3.0;
            if x - half_width <= last_right + 6.0 {
                continue;
            }
            last_right = x + half_width;
            svg.push_str(&format!(
                r#"<text x="{x:.1}" y="{:.1}" fill="{}" font-size="10" text-anchor="middle">{}</text>
"#,
                py - 4.0,
                hex(colors.roi),
                escape(&label),
            ));
        }
    }

    // Frame, titles and provenance. The name, not the whole path: the picture
    // is for a report, and the reader wants "eu152_spectra.Spe".
    let title = spectrum
        .source
        .as_deref()
        .map(|source| {
            std::path::Path::new(source)
                .file_name()
                .map(|name| name.to_string_lossy().into_owned())
                .unwrap_or_else(|| source.to_string())
        })
        .unwrap_or_else(|| "spectrum".into());
    let title = title.as_str();
    let axis_title = match (&spectrum.energy_calibration, calibrated) {
        (Some(cal), true) => format!("energy ({})", cal.units),
        _ => "channel".into(),
    };
    svg.push_str(&format!(
        r#"<rect x="{px}" y="{py}" width="{pw}" height="{ph}" fill="none" stroke="{axis}" stroke-opacity="0.8"/>
<text x="{px}" y="{:.1}" fill="{axis}" font-size="12">{}</text>
<text x="{:.1}" y="{:.1}" fill="{axis}" font-size="11" text-anchor="middle">{axis_title}</text>
<text x="14" y="{:.1}" fill="{axis}" font-size="11" transform="rotate(-90 14 {:.1})" text-anchor="middle">counts</text>
<text x="{:.1}" y="{:.1}" fill="{axis}" fill-opacity="0.65" font-size="10" text-anchor="end">LT {:.1} s · RT {:.1} s · {} counts</text>
</svg>
"#,
        py - 12.0,
        escape(title),
        px + pw / 2.0,
        height - 8.0,
        py + ph / 2.0,
        py + ph / 2.0,
        px + pw,
        py - 12.0,
        spectrum.live_time,
        spectrum.real_time,
        spectrum.total_counts(),
    ));
    svg
}

/// The five XML-special characters, escaped.
fn escape(text: &str) -> String {
    text.replace('&', "&amp;")
        .replace('<', "&lt;")
        .replace('>', "&gt;")
        .replace('"', "&quot;")
        .replace('\'', "&apos;")
}

#[cfg(test)]
mod tests {
    use super::*;
    use mantaray_core::EnergyCalibration;

    fn spectrum() -> Spectrum {
        let mut spectrum = Spectrum::new(2048);
        spectrum.live_time = 300.0;
        spectrum.real_time = 320.0;
        spectrum.energy_calibration = Some(EnergyCalibration::linear(0.0, 0.5));
        for channel in 0..2048 {
            spectrum.channels[channel] = 50 + (channel as u64 % 7);
        }
        for offset in -8i64..=8 {
            spectrum.channels[(1000 + offset) as usize] += 20_000;
        }
        spectrum.rois.mark(mantaray_core::Roi::new(985, 1015));
        spectrum
    }

    #[test]
    fn the_snapshot_is_a_complete_svg_document() {
        let spectrum = spectrum();
        let display = DisplayState::for_length(spectrum.len());
        let colors = crate::theme::Theme::DeepSpace.colors();
        let svg = plot_svg(&spectrum, &display, &colors, None, 1200.0, 700.0);
        assert!(svg.starts_with("<svg "));
        assert!(svg.trim_end().ends_with("</svg>"));
        // Every element opened is closed or self-closing: crude but effective.
        assert_eq!(svg.matches("<svg").count(), svg.matches("</svg>").count());
        assert_eq!(
            svg.matches("<polyline").count(),
            1,
            "one trace outline expected"
        );
        assert!(
            svg.matches("<polygon").count() >= 2,
            "the fill should split at the region boundary"
        );
    }

    #[test]
    fn the_snapshot_shows_what_the_display_shows() {
        let spectrum = spectrum();
        let mut display = DisplayState::for_length(spectrum.len());
        let colors = crate::theme::Theme::DeepSpace.colors();

        // Calibrated: energy ticks at round values across 0-1023 keV.
        let svg = plot_svg(&spectrum, &display, &colors, None, 1200.0, 700.0);
        assert!(svg.contains(">400<"), "a round energy tick is missing");
        assert!(svg.contains("energy (keV)"));
        assert!(svg.contains("LT 300.0 s"));

        // Zoomed to the peak, the ticks follow the view.
        display.zoom_to(950, 1050);
        let svg = plot_svg(&spectrum, &display, &colors, None, 1200.0, 700.0);
        assert!(
            svg.contains(">490<") || svg.contains(">500<"),
            "ticks should cover the zoomed energies"
        );
        // 200 keV is far outside the zoom, and unlike 100 it cannot be
        // mistaken for a decade label on the count axis.
        assert!(!svg.contains(">200<"), "energies far outside the view");
    }

    #[test]
    fn special_characters_in_the_source_name_do_not_break_the_file() {
        let mut spectrum = spectrum();
        spectrum.source = Some("sample <3> & \"friends\".Spe".into());
        let display = DisplayState::for_length(spectrum.len());
        let colors = crate::theme::Theme::Paper.colors();
        let svg = plot_svg(&spectrum, &display, &colors, None, 900.0, 500.0);
        assert!(svg.contains("&lt;3&gt; &amp; &quot;friends&quot;"));
        assert!(!svg.contains("sample <3>"));
    }
}
