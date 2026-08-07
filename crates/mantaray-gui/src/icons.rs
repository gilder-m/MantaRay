//! A drawn icon set for the toolbar.
//!
//! Drawn rather than written. The obvious way to put a symbol on a button is a
//! character - and the fonts bundled with a graphics library are not the fonts
//! on the machine, so a character that looks right while writing the code
//! arrives at somebody else's screen as an empty box. That happened in this
//! program with a cross on the tab strip, which is how these came to be
//! strokes instead.
//!
//! Each is drawn inside a unit square and scaled to the rectangle it is given,
//! so one definition serves any size. They are deliberately plain: a toolbar
//! icon is read at eleven pixels out of the corner of the eye, and detail at
//! that size is noise.

use egui::{Color32, Painter, Pos2, Rect, Shape, Stroke, Vec2};

/// The toolbar's symbols.
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub enum Icon {
    /// Open a spectrum from a file.
    Open,
    /// Write it back out.
    Save,
    /// Begin counting.
    Start,
    /// Stop counting.
    Stop,
    /// Empty the spectrum.
    Clear,
    /// Copy what the instrument holds into a buffer.
    Buffer,
    /// Step back through the history.
    Undo,
    /// Step forward again.
    Redo,
    /// Find the peaks.
    Peaks,
    /// Report on the peak under the marker.
    PeakInfo,
    /// The written report.
    Report,
    /// The nuclide library.
    Nuclides,
    /// Centre the view on the marker.
    Centre,
    /// Show the whole spectrum.
    Full,
    /// Fill the working area with this spectrum.
    Maximise,
}

/// Paints an icon inside a rectangle, in one colour.
///
/// The rectangle is squared off first, so an icon given a wide button is drawn
/// round rather than stretched.
pub fn paint(painter: &Painter, rect: Rect, icon: Icon, color: Color32) {
    painter.extend(shapes(rect, icon, color));
}

/// The strokes an icon is made of.
///
/// Separated from the painting so the geometry can be looked at: an icon that
/// draws nothing, or draws outside its own button, is invisible to a test that
/// only checks the call did not panic.
pub fn shapes(rect: Rect, icon: Icon, color: Color32) -> Vec<Shape> {
    let mut out: Vec<Shape> = Vec::new();
    let painter = &mut out;
    let side = rect.width().min(rect.height());
    let box_rect = Rect::from_center_size(rect.center(), Vec2::splat(side));
    // A point inside the icon, given as a fraction of its box.
    let at = |x: f32, y: f32| Pos2::new(box_rect.left() + x * side, box_rect.top() + y * side);
    let stroke = Stroke::new((side * 0.09).clamp(1.0, 2.0), color);
    let line = |points: Vec<Pos2>| Shape::line(points, stroke);

    match icon {
        // A sheet with a corner turned down, and an arrow going into it.
        Icon::Open => {
            painter.push(line(vec![
                at(0.22, 0.86),
                at(0.22, 0.14),
                at(0.62, 0.14),
                at(0.78, 0.32),
                at(0.78, 0.86),
                at(0.22, 0.86),
            ]));
            painter.push(line(vec![at(0.62, 0.14), at(0.62, 0.32), at(0.78, 0.32)]));
        }
        // The same sheet with an arrow leaving it.
        Icon::Save => {
            painter.push(line(vec![
                at(0.2, 0.2),
                at(0.2, 0.8),
                at(0.8, 0.8),
                at(0.8, 0.2),
            ]));
            painter.push(line(vec![at(0.5, 0.3), at(0.5, 0.64)]));
            painter.push(line(vec![at(0.36, 0.5), at(0.5, 0.64), at(0.64, 0.5)]));
        }
        // A filled triangle: the one symbol nobody has ever had to learn.
        Icon::Start => {
            painter.push(Shape::convex_polygon(
                vec![at(0.3, 0.18), at(0.3, 0.82), at(0.82, 0.5)],
                color,
                Stroke::NONE,
            ));
        }
        Icon::Stop => {
            painter.push(Shape::rect_filled(
                Rect::from_min_max(at(0.26, 0.26), at(0.74, 0.74)),
                egui::CornerRadius::ZERO,
                color,
            ));
        }
        // A bin: a lid and a body.
        Icon::Clear => {
            painter.push(line(vec![at(0.2, 0.28), at(0.8, 0.28)]));
            painter.push(line(vec![
                at(0.4, 0.28),
                at(0.4, 0.18),
                at(0.6, 0.18),
                at(0.6, 0.28),
            ]));
            painter.push(line(vec![
                at(0.28, 0.28),
                at(0.33, 0.84),
                at(0.67, 0.84),
                at(0.72, 0.28),
            ]));
        }
        // Two sheets, one behind the other.
        Icon::Buffer => {
            painter.push(line(vec![
                at(0.16, 0.7),
                at(0.16, 0.16),
                at(0.62, 0.16),
                at(0.62, 0.28),
            ]));
            painter.push(line(vec![
                at(0.34, 0.32),
                at(0.34, 0.84),
                at(0.84, 0.84),
                at(0.84, 0.32),
                at(0.34, 0.32),
            ]));
        }
        // An arrow curving back on itself, and its mirror.
        Icon::Undo | Icon::Redo => {
            let flip = |x: f32| if icon == Icon::Undo { x } else { 1.0 - x };
            painter.push(line(vec![
                at(flip(0.2), 0.42),
                at(flip(0.56), 0.42),
                at(flip(0.74), 0.58),
                at(flip(0.6), 0.78),
            ]));
            painter.push(line(vec![
                at(flip(0.34), 0.28),
                at(flip(0.2), 0.42),
                at(flip(0.34), 0.56),
            ]));
        }
        // A trace with a caret over its tallest point: what a peak search
        // leaves behind.
        Icon::Peaks => {
            painter.push(line(vec![
                at(0.14, 0.78),
                at(0.34, 0.74),
                at(0.5, 0.34),
                at(0.66, 0.74),
                at(0.86, 0.78),
            ]));
            painter.push(Shape::convex_polygon(
                vec![at(0.44, 0.24), at(0.56, 0.24), at(0.5, 0.34)],
                color,
                Stroke::NONE,
            ));
        }
        // The same trace with a rule under one peak: the region it measured.
        Icon::PeakInfo => {
            painter.push(line(vec![
                at(0.14, 0.72),
                at(0.34, 0.68),
                at(0.5, 0.26),
                at(0.66, 0.68),
                at(0.86, 0.72),
            ]));
            painter.push(line(vec![at(0.34, 0.84), at(0.66, 0.84)]));
        }
        // A page of writing.
        Icon::Report => {
            painter.push(line(vec![
                at(0.24, 0.14),
                at(0.24, 0.86),
                at(0.76, 0.86),
                at(0.76, 0.14),
                at(0.24, 0.14),
            ]));
            for row in 0..3 {
                let y = 0.34 + row as f32 * 0.16;
                painter.push(line(vec![at(0.36, y), at(0.64, y)]));
            }
        }
        // A nucleus with an orbit round it.
        Icon::Nuclides => {
            painter.push(Shape::circle_filled(at(0.5, 0.5), side * 0.09, color));
            painter.push(Shape::circle_stroke(at(0.5, 0.5), side * 0.3, stroke));
        }
        // Brackets closing in on a line: the marker brought to the middle.
        Icon::Centre => {
            painter.push(line(vec![at(0.5, 0.16), at(0.5, 0.84)]));
            painter.push(line(vec![at(0.24, 0.34), at(0.36, 0.5), at(0.24, 0.66)]));
            painter.push(line(vec![at(0.76, 0.34), at(0.64, 0.5), at(0.76, 0.66)]));
        }
        // Arrows pushing out to the edges: the whole spectrum.
        Icon::Full => {
            painter.push(line(vec![at(0.16, 0.16), at(0.16, 0.84)]));
            painter.push(line(vec![at(0.84, 0.16), at(0.84, 0.84)]));
            painter.push(line(vec![at(0.3, 0.5), at(0.7, 0.5)]));
            painter.push(line(vec![at(0.3, 0.5), at(0.42, 0.38)]));
            painter.push(line(vec![at(0.7, 0.5), at(0.58, 0.62)]));
        }
        // A rectangle with its corners marked.
        Icon::Maximise => {
            painter.push(line(vec![
                at(0.18, 0.2),
                at(0.18, 0.8),
                at(0.82, 0.8),
                at(0.82, 0.2),
                at(0.18, 0.2),
            ]));
            painter.push(line(vec![at(0.18, 0.34), at(0.82, 0.34)]));
        }
    }
    out
}

/// A button carrying an icon, a label, or both.
///
/// The tooltip always names the action in words, whatever the button shows.
/// An icon nobody recognises is a guessing game, and the one thing worse than
/// a toolbar of unlabelled pictures is one that will not say what they are.
pub fn button(
    ui: &mut egui::Ui,
    icon: Icon,
    label: &str,
    style: crate::theme::IconStyle,
    enabled: bool,
) -> egui::Response {
    tinted(ui, icon, label, style, enabled, None)
}

/// The same, in a colour of its own when it is enabled.
///
/// Start and Stop use it: green for the one that begins a measurement, red for
/// the one that ends it, whether they are drawn as words or as symbols. The
/// colour is as much of what the button says as the shape is.
pub fn tinted(
    ui: &mut egui::Ui,
    icon: Icon,
    label: &str,
    style: crate::theme::IconStyle,
    enabled: bool,
    tint: Option<Color32>,
) -> egui::Response {
    use crate::theme::IconStyle;
    if style == IconStyle::Text {
        let text = match tint.filter(|_| enabled) {
            Some(colour) => egui::RichText::new(label).color(colour),
            None => egui::RichText::new(label),
        };
        return ui.add_enabled(enabled, egui::Button::new(text));
    }

    let text = (style == IconStyle::Both).then_some(label);
    let font = egui::TextStyle::Button.resolve(ui.style());
    let text_width = text
        .map(|label| {
            ui.painter()
                .layout_no_wrap(label.to_string(), font.clone(), Color32::WHITE)
                .size()
                .x
                + 5.0
        })
        .unwrap_or(0.0);
    let padding = ui.spacing().button_padding;
    let glyph = ui.spacing().interact_size.y * 0.62;
    let size = Vec2::new(
        glyph + text_width + padding.x * 2.0,
        glyph + padding.y * 2.0,
    );
    let sense = if enabled {
        egui::Sense::click()
    } else {
        egui::Sense::hover()
    };
    let (rect, response) = ui.allocate_exact_size(size, sense);
    if !ui.is_rect_visible(rect) {
        return response;
    }

    let visuals = ui.style().interact(&response);
    let visuals = if enabled {
        visuals
    } else {
        &ui.style().visuals.widgets.noninteractive
    };
    ui.painter().rect(
        rect,
        visuals.corner_radius,
        visuals.weak_bg_fill,
        visuals.bg_stroke,
        egui::StrokeKind::Inside,
    );
    let colour = match tint.filter(|_| enabled) {
        Some(colour) => colour,
        None if enabled => visuals.fg_stroke.color,
        None => ui.style().visuals.weak_text_color(),
    };
    let glyph_rect = Rect::from_min_size(
        Pos2::new(rect.left() + padding.x, rect.center().y - glyph / 2.0),
        Vec2::splat(glyph),
    );
    paint(ui.painter(), glyph_rect, icon, colour);
    if let Some(label) = text {
        ui.painter().text(
            Pos2::new(glyph_rect.right() + 5.0, rect.center().y),
            egui::Align2::LEFT_CENTER,
            label,
            font,
            colour,
        );
    }
    response
}

#[cfg(test)]
mod tests {
    use super::*;

    /// Every icon in the set.
    const EVERY: [Icon; 15] = [
        Icon::Open,
        Icon::Save,
        Icon::Start,
        Icon::Stop,
        Icon::Clear,
        Icon::Buffer,
        Icon::Undo,
        Icon::Redo,
        Icon::Peaks,
        Icon::PeakInfo,
        Icon::Report,
        Icon::Nuclides,
        Icon::Centre,
        Icon::Full,
        Icon::Maximise,
    ];

    /// Every icon draws something, at every size, inside its own button.
    ///
    /// An icon that draws nothing looks exactly like a working button with a
    /// rendering fault, and one that draws outside its rectangle scribbles on
    /// its neighbours. The sizes include the degenerate ones a narrow toolbar
    /// can produce.
    #[test]
    fn every_icon_draws_inside_its_own_box() {
        for icon in EVERY {
            for side in [1.0_f32, 8.0, 14.0, 64.0] {
                let rect = Rect::from_min_size(Pos2::new(7.0, 3.0), Vec2::splat(side));
                let drawn = shapes(rect, icon, Color32::WHITE);
                assert!(!drawn.is_empty(), "{icon:?} at {side} drew nothing");
                for shape in &drawn {
                    let bounds = shape.visual_bounding_rect();
                    assert!(
                        rect.expand(2.0).contains_rect(bounds),
                        "{icon:?} at {side} drew outside its box: {bounds:?} not in {rect:?}"
                    );
                }
            }
        }
    }

    /// No two icons are the same drawing, or the toolbar says nothing.
    #[test]
    fn every_icon_is_distinct() {
        let rect = Rect::from_min_size(Pos2::ZERO, Vec2::splat(32.0));
        let drawn: Vec<String> = EVERY
            .iter()
            .map(|icon| format!("{:?}", shapes(rect, *icon, Color32::WHITE)))
            .collect();
        for (index, icon) in EVERY.iter().enumerate() {
            for (other_index, other) in EVERY.iter().enumerate().skip(index + 1) {
                assert_ne!(
                    drawn[index], drawn[other_index],
                    "{icon:?} and {other:?} are the same drawing"
                );
            }
        }
    }

    /// Undo and Redo are mirror images of one another.
    #[test]
    fn undo_and_redo_point_opposite_ways() {
        let rect = Rect::from_min_size(Pos2::ZERO, Vec2::splat(32.0));
        let undo = shapes(rect, Icon::Undo, Color32::WHITE);
        let redo = shapes(rect, Icon::Redo, Color32::WHITE);
        assert_eq!(undo.len(), redo.len(), "the same strokes either way");
        // Mirrored about the middle: each one's bounding box is the other's,
        // reflected. Drawn identically they would be a pair of buttons that
        // look the same and do opposite things.
        let span = |drawn: &[Shape]| {
            drawn
                .iter()
                .map(|shape| shape.visual_bounding_rect())
                .fold(Rect::NOTHING, |all, one| all.union(one))
        };
        let (left, right) = (span(&undo), span(&redo));
        assert!(
            (left.left() - (rect.width() - right.right())).abs() < 1.0,
            "{left:?} is not the mirror of {right:?}"
        );
    }
}
