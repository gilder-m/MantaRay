//! The spectrum display: the expanded view, the inset full view, the marker and
//! all the mouse work (MAESTRO §3.3 and §3.5).

use egui::{
    Align2, Color32, CornerRadius, FontId, Pos2, Rect, Sense, Shape, Stroke, StrokeKind, Ui, Vec2,
};
use ortseam_core::{NuclideLibrary, PeakInfo, Spectrum};

use crate::theme::SpectrumColors;
use crate::viewmodel::{DisplayState, FillMode, VerticalScale};

/// Which region-marking mode the ROI menu has selected.
#[derive(Clone, Copy, Debug, Default, PartialEq, Eq)]
pub enum MarkMode {
    /// Dragging selects a range without changing regions.
    #[default]
    Off,
    /// Dragging marks a region.
    Mark,
    /// Dragging unmarks a region.
    UnMark,
}

impl MarkMode {
    /// Label for the ROI status area.
    pub fn label(&self) -> &'static str {
        match self {
            Self::Off => "Off",
            Self::Mark => "Marking",
            Self::UnMark => "UnMarking",
        }
    }
}

/// Commands on the right-mouse-button menu (MAESTRO §4.8).
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub enum MenuCommand {
    /// Start data collection.
    Start,
    /// Stop data collection.
    Stop,
    /// Clear the spectrum.
    Clear,
    /// Copy the detector data to a buffer.
    CopyToBuffer,
    /// Zoom in.
    ZoomIn,
    /// Zoom out.
    ZoomOut,
    /// Show the whole spectrum again.
    UndoZoom,
    /// Mark a region over the selection.
    MarkRoi,
    /// Clear the region under the marker.
    ClearRoi,
    /// Peak information at the marker.
    PeakInfo,
    /// Show the input count rate.
    InputCountRate,
    /// Sum the selection.
    Sum,
    /// Open the instrument settings.
    McbProperties,
}

/// What the user did in the display.
#[derive(Clone, Copy, Debug, PartialEq)]
pub enum ViewEvent {
    /// A command was chosen from the right-mouse-button menu.
    Menu(MenuCommand),
    /// The marker moved to a channel.
    Marker(usize),
    /// A channel range was selected with the rubber rectangle.
    Selected(usize, usize),
    /// The selection was cleared.
    SelectionCleared,
    /// A region should be marked over this range.
    MarkRoi(usize, usize),
    /// A region should be unmarked over this range.
    UnmarkRoi(usize, usize),
    /// Peak information was asked for at a channel.
    PeakInfo(usize),
    /// The view should zoom to a channel range.
    ZoomTo(usize, usize),
    /// The wheel asked to zoom in or out.
    Zoom(i32),
    /// The wheel asked to zoom about the channel under the pointer.
    ZoomAt(usize, i32),
    /// The wheel asked to scroll by channels.
    Scroll(i64),
    /// The window should fill the spectrum area, or go back to its own size.
    ToggleMaximize,
    /// The window should be brought to the front and made active.
    Activate,
    /// The peak-information box was dismissed.
    ClosePeakInfo,
}

/// The one-line summary drawn above the spectrum.
pub struct ViewHeader<'a> {
    /// Where the data came from, for tooltips.
    pub source: &'a str,
    /// Whether a detector is collecting into it.
    pub counting: bool,
    /// Dead time as a percentage, when known.
    pub dead_time: Option<f64>,
    /// Whether this is the active window.
    pub active: bool,
    /// Whether the window currently fills the spectrum area.
    pub maximized: bool,
}

/// Everything the display needs to draw one spectrum window.
pub struct ViewInput<'a> {
    /// The spectrum shown.
    pub spectrum: &'a Spectrum,
    /// A comparison spectrum drawn behind it.
    pub compare: Option<&'a Spectrum>,
    /// Vertical offset of the comparison trace, in counts.
    pub compare_offset: i64,
    /// What is on screen; the view updates it in place.
    pub display: &'a mut DisplayState,
    /// Colours.
    pub colors: &'a SpectrumColors,
    /// Region-marking mode.
    pub mark_mode: MarkMode,
    /// Library whose lines are drawn when isotope markers are on.
    pub library: Option<&'a NuclideLibrary>,
    /// The current rubber-rectangle selection, if any.
    pub selection: Option<(usize, usize)>,
    /// Peak information to draw over the peak it belongs to.
    pub peak_info: Option<&'a PeakInfo>,
    /// Font size of the peak-information card (Preferences).
    pub peak_font: f32,
    /// The header line.
    pub header: ViewHeader<'a>,
}

/// Room left for the axis labels: numbers on the left, numbers and the axis name
/// below, and the count unit above so it never lands on a tick label.
const LEFT_GUTTER: f32 = 54.0;
const BOTTOM_GUTTER: f32 = 30.0;
const TOP_GUTTER: f32 = 14.0;

/// Draws a spectrum window and reports what the user did.
pub fn draw(ui: &mut Ui, input: ViewInput<'_>) -> Vec<ViewEvent> {
    let mut events = Vec::new();
    let ViewInput {
        spectrum,
        compare,
        compare_offset,
        display,
        colors,
        mark_mode,
        library,
        selection,
        peak_info,
        peak_font,
        header,
    } = input;

    display.set_length(spectrum.len());
    header_bar(ui, spectrum, &header, display, colors, &mut events);

    let available = ui.available_size();
    // The selection bar wraps to two rows in a narrow window.
    let footer = match selection {
        Some(_) if available.x < 560.0 => 80.0,
        Some(_) => 58.0,
        None => 34.0,
    };
    let plot_height = (available.y - footer).max(90.0);
    let (response, painter) =
        ui.allocate_painter(Vec2::new(available.x, plot_height), Sense::click_and_drag());
    let outer = response.rect;
    painter.rect_filled(outer, CornerRadius::same(3), colors.background.to_color());
    draw_background_wash(&painter, outer, colors);
    if header.active {
        painter.rect_stroke(
            outer,
            CornerRadius::same(3),
            Stroke::new(1.0, colors.view_box.to_color().gamma_multiply(0.8)),
            StrokeKind::Inside,
        );
    }

    if spectrum.is_empty() {
        painter.text(
            outer.center(),
            Align2::CENTER_CENTER,
            "no data - Acquire/Start, or File/Recall",
            FontId::proportional(13.0),
            colors.axes.to_color(),
        );
        return events;
    }

    // The plotting area, leaving room for the axis labels.
    let plot = Rect::from_min_max(
        Pos2::new(outer.left() + LEFT_GUTTER, outer.top() + TOP_GUTTER),
        Pos2::new(outer.right() - 8.0, outer.bottom() - BOTTOM_GUTTER),
    );
    let full_scale = display.full_scale(spectrum);

    draw_axes(&painter, outer, plot, spectrum, display, colors, full_scale);
    if let Some(compare) = compare {
        draw_trace(
            &painter,
            plot,
            compare,
            display,
            full_scale,
            colors.compare.to_color(),
            compare_offset,
            FillMode::Points,
            None,
        );
        draw_compare_legend(&painter, plot, compare, compare_offset, colors);
    }
    draw_trace(
        &painter,
        plot,
        spectrum,
        display,
        full_scale,
        colors.foreground.to_color(),
        0,
        display.fill,
        Some(colors.roi.to_color()),
    );
    draw_peak_markers(
        &painter, plot, spectrum, display, colors, library, full_scale,
    );
    if display.isotope_markers {
        draw_library_lines(&painter, plot, spectrum, display, colors, library);
    }
    if let Some((low, high)) = selection {
        draw_selection(&painter, plot, display, low, high, colors);
    }
    // A spectrum of zeroes that nothing is counting into gets a gentle nudge
    // in the middle of the plot; a blank grid on its own explains nothing.
    if spectrum.total_counts() == 0 && !header.counting {
        painter.text(
            plot.center(),
            Align2::CENTER_CENTER,
            "nothing counted yet - press \u{25b6} Start, recall a file, or drop a .Spe here",
            FontId::proportional(13.0),
            colors.axes.to_color(),
        );
    }
    draw_marker(&painter, plot, spectrum, display, colors);
    draw_full_view(&painter, plot, spectrum, display, colors);
    let peak_box = peak_info.and_then(|info| {
        draw_peak_info(
            &painter, plot, spectrum, display, colors, info, library, peak_font,
        )
    });

    // ------------------------------------------------------------------
    // Mouse work
    // ------------------------------------------------------------------
    // A copy of the view, so the mapping from a pixel to a channel can be used
    // freely while the display itself is being written to.
    let view = *display;
    let channel_at =
        |position: Pos2| -> usize { view.channel_at_x(position.x, plot.left(), plot.width()) };
    // The overview inset is hidden on an empty spectrum, so it must not soak
    // up clicks either; an empty rect keeps every `inset.contains` false.
    let inset = if spectrum.total_counts() == 0 {
        Rect::NOTHING
    } else {
        full_view_rect(plot)
    };

    if response.clicked() || response.dragged() || response.double_clicked() {
        events.push(ViewEvent::Activate);
    }

    // Clicking the peak-information box dismisses it, as in MAESTRO.
    if response.clicked()
        && let (Some(box_rect), Some(position)) = (peak_box, response.interact_pointer_pos())
        && box_rect.contains(position)
    {
        events.push(ViewEvent::ClosePeakInfo);
        return events;
    }

    // Only the plot itself answers to a click: the axis gutters carry labels, and
    // clicking a label should not move the marker.
    let in_plot_or_inset =
        |position: &Pos2| plot.contains(*position) || full_view_rect(plot).contains(*position);
    if response.double_clicked() {
        if let Some(position) = response.interact_pointer_pos().filter(in_plot_or_inset) {
            if inset.contains(position) {
                // Double clicking the overview shows the whole spectrum.
                events.push(ViewEvent::ZoomTo(0, spectrum.len().saturating_sub(1)));
            } else {
                events.push(ViewEvent::Marker(channel_at(position)));
                events.push(ViewEvent::PeakInfo(channel_at(position)));
            }
        }
    } else if response.clicked()
        && let Some(position) = response.interact_pointer_pos().filter(in_plot_or_inset)
    {
        if inset.contains(position) {
            // Clicking the overview jumps there, keeping the zoom.
            let fraction = ((position.x - inset.left()) / inset.width().max(1.0)).clamp(0.0, 1.0);
            let channel = (fraction * (spectrum.len().saturating_sub(1)) as f32) as usize;
            let half = display.view_width / 2;
            events.push(ViewEvent::ZoomTo(
                channel.saturating_sub(half),
                channel + half,
            ));
            events.push(ViewEvent::Marker(channel));
        } else {
            events.push(ViewEvent::Marker(channel_at(position)));
            events.push(ViewEvent::SelectionCleared);
        }
    }

    // Dragging: the rubber rectangle, anchored where the button went down.
    //
    // egui clears its own `press_origin` on the frame the button comes up - the
    // frame the finished selection is wanted on - so the origin is remembered in
    // the display state as the drag begins.
    if response.drag_started_by(egui::PointerButton::Primary)
        && let Some(origin) = ui
            .ctx()
            .input(|i| i.pointer.press_origin())
            .or_else(|| response.interact_pointer_pos())
    {
        display.begin_drag(origin.x, inset.contains(origin));
    }
    let mut finished_selection = selection;
    // Selection and overview panning belong to the primary button alone; the
    // middle button pans the plot and must not leave a selection behind.
    if response.dragged_by(egui::PointerButton::Primary)
        || response.drag_stopped_by(egui::PointerButton::Primary)
    {
        if display.drag_in_inset() {
            // A drag that began in the overview pans the expanded view: the
            // window follows the pointer through the whole spectrum.
            if let Some(position) = response.interact_pointer_pos() {
                let fraction =
                    ((position.x - inset.left()) / inset.width().max(1.0)).clamp(0.0, 1.0);
                let channel = (fraction * (spectrum.len().saturating_sub(1)) as f32) as usize;
                let half = view.view_width / 2;
                events.push(ViewEvent::ZoomTo(
                    channel.saturating_sub(half),
                    channel + half,
                ));
            }
        } else {
            let origin = display
                .drag_origin()
                .map(|x| Pos2::new(x, plot.center().y))
                .or_else(|| ui.ctx().input(|i| i.pointer.press_origin()));
            if let (Some(origin), Some(position)) = (origin, response.interact_pointer_pos()) {
                let (low, high) =
                    display.drag_range(origin.x, position.x, plot.left(), plot.width());
                if low != high {
                    finished_selection = Some((low, high));
                    events.push(ViewEvent::Selected(low, high));
                    // While dragging, the marker follows the pointer so the
                    // readout keeps up.
                    events.push(ViewEvent::Marker(channel_at(position)));
                }
            }
        }
    }
    if response.drag_stopped_by(egui::PointerButton::Primary) {
        display.end_drag();
        if let Some((low, high)) = finished_selection {
            match mark_mode {
                MarkMode::Mark => events.push(ViewEvent::MarkRoi(low, high)),
                MarkMode::UnMark => events.push(ViewEvent::UnmarkRoi(low, high)),
                MarkMode::Off => {}
            }
        }
    }

    // Hovering: a hairline and a readout at the pointer.
    if let Some(position) = response.hover_pos()
        && plot.contains(position)
        && !inset.contains(position)
    {
        let channel = channel_at(position);
        let x = plot.left() + plot.width() * display.fraction_of(channel).unwrap_or(0.0);
        painter.line_segment(
            [Pos2::new(x, plot.top()), Pos2::new(x, plot.bottom())],
            Stroke::new(1.0, colors.axes.to_color().gamma_multiply(0.8)),
        );
        // The library's name for this energy rides along, so hovering a
        // peak answers "what is that?" without any clicking.
        let name = nuclide_at(spectrum, library, channel)
            .map(|name| format!("   {name}"))
            .unwrap_or_default();
        let text = match &spectrum.energy_calibration {
            Some(cal) => format!(
                "ch {channel}   {:.2} {}   {} cnts{name}",
                cal.energy(channel as f64),
                cal.units,
                spectrum.counts(channel)
            ),
            None => format!("ch {channel}   {} cnts", spectrum.counts(channel)),
        };
        let anchor = if position.x > plot.center().x {
            Align2::RIGHT_TOP
        } else {
            Align2::LEFT_TOP
        };
        let at = Pos2::new(
            if position.x > plot.center().x {
                x - 6.0
            } else {
                x + 6.0
            },
            plot.top() + 4.0,
        );
        let galley =
            painter.layout_no_wrap(text, FontId::monospace(11.0), colors.marker.to_color());
        let box_rect = anchor
            .anchor_size(at, galley.size())
            .expand2(Vec2::new(5.0, 3.0));
        painter.rect_filled(
            box_rect,
            CornerRadius::same(3),
            colors.background.to_color().gamma_multiply(2.2),
        );
        painter.galley(
            anchor.anchor_size(at, galley.size()).min,
            galley,
            colors.marker.to_color(),
        );
    }

    response.context_menu(|ui| {
        let mut choose = |ui: &mut Ui, label: &str, command: MenuCommand| {
            if ui.button(label).clicked() {
                events.push(ViewEvent::Menu(command));
                ui.close();
            }
        };
        choose(ui, "Start", MenuCommand::Start);
        choose(ui, "Stop", MenuCommand::Stop);
        choose(ui, "Clear", MenuCommand::Clear);
        choose(ui, "Copy to Buffer", MenuCommand::CopyToBuffer);
        ui.separator();
        choose(ui, "Zoom In", MenuCommand::ZoomIn);
        choose(ui, "Zoom Out", MenuCommand::ZoomOut);
        choose(ui, "Undo Zoom", MenuCommand::UndoZoom);
        ui.separator();
        choose(ui, "Mark ROI", MenuCommand::MarkRoi);
        choose(ui, "Clear ROI", MenuCommand::ClearRoi);
        ui.separator();
        choose(ui, "Peak Info", MenuCommand::PeakInfo);
        choose(ui, "Input Count Rate", MenuCommand::InputCountRate);
        choose(ui, "Sum", MenuCommand::Sum);
        ui.separator();
        choose(ui, "MCB Properties", MenuCommand::McbProperties);
    });

    // Middle-button drag pans the view, as in every plotting tool: the
    // spectrum follows the hand, one channel under the pointer staying under
    // it as far as rounding allows.
    if response.dragged_by(egui::PointerButton::Middle) {
        let delta = response.drag_delta().x;
        if delta.abs() >= 1.0 {
            let channels_per_pixel = view.view_width as f32 / plot.width().max(1.0);
            let channels = (-delta * channels_per_pixel).round() as i64;
            if channels != 0 {
                events.push(ViewEvent::Scroll(channels));
            }
        }
    }

    // The wheel: zoom by default, scroll with shift, and zoom again for the
    // gestures egui reports as zoom rather than scroll - a pinch on a trackpad, or
    // the wheel with the zoom modifier held, which egui converts before the plot
    // ever sees it.
    if response.hovered() {
        let (scroll, sideways, zoom, shift) = ui.ctx().input(|i| {
            (
                i.smooth_scroll_delta.y,
                i.smooth_scroll_delta.x,
                i.zoom_delta(),
                i.modifiers.shift,
            )
        });
        let step = || (display.view_width as f32 * 0.15).max(1.0) as i64;
        // Zooming happens about the channel under the pointer, so the peak being
        // looked at stays put while the rest of the spectrum moves.
        let at_pointer = response
            .hover_pos()
            .filter(|position| plot.contains(*position) && !inset.contains(*position))
            .map(channel_at);
        let zoom_event = |direction: i32| match at_pointer {
            Some(channel) => ViewEvent::ZoomAt(channel, direction),
            None => ViewEvent::Zoom(direction),
        };
        if (zoom - 1.0).abs() > 0.01 {
            events.push(zoom_event(if zoom > 1.0 { 1 } else { -1 }));
        } else if scroll.abs() > 0.5 {
            if shift {
                events.push(ViewEvent::Scroll(if scroll > 0.0 {
                    -step()
                } else {
                    step()
                }));
            } else {
                events.push(zoom_event(if scroll > 0.0 { 1 } else { -1 }));
            }
        } else if sideways.abs() > 0.5 {
            // A sideways wheel or a two-finger swipe pans along the spectrum.
            events.push(ViewEvent::Scroll(if sideways > 0.0 {
                -step()
            } else {
                step()
            }));
        }
    }

    // ------------------------------------------------------------------
    // Readout and the actions that apply to a selection
    // ------------------------------------------------------------------
    marker_line(ui, spectrum, display, selection);
    if let Some((low, high)) = selection {
        selection_actions(ui, spectrum, low, high, mark_mode, &mut events);
    }

    events
}

/// The window's own header: source, times, counts, mode and a maximise button.
fn header_bar(
    ui: &mut Ui,
    spectrum: &Spectrum,
    header: &ViewHeader<'_>,
    display: &DisplayState,
    colors: &SpectrumColors,
    events: &mut Vec<ViewEvent>,
) {
    // The header adapts to the window's width rather than forcing it: a tiled
    // or narrow window keeps the essentials - counts, dead time, the maximise
    // button - and the rest returns as there is room.
    let width = ui.available_width();
    let (roomy, medium) = (width >= 640.0, width >= 430.0);
    ui.horizontal(|ui| {
        if header.counting {
            ui.label(egui::RichText::new("● REC").color(colors.alarm.to_color()));
        }
        // The window's own title bar already carries the name, so the header
        // shows what the title bar cannot: the shape of the data.
        if medium {
            ui.monospace(format!("{} ch", spectrum.len()));
        }
        if roomy {
            if let Some(cal) = &spectrum.energy_calibration {
                ui.monospace(format!("{:.4} {}/ch", cal.coefficients[1], cal.units));
            } else {
                ui.label(egui::RichText::new("uncalibrated").weak());
            }
            ui.separator();
        }
        if roomy {
            ui.monospace(format!(
                "LT {:.1} s   RT {:.1} s",
                spectrum.live_time, spectrum.real_time
            ));
        } else if medium {
            ui.monospace(format!("LT {:.1} s", spectrum.live_time));
        }
        if let Some(dead) = header.dead_time {
            let colour = if dead > 50.0 {
                colors.alarm.to_color()
            } else if dead > 20.0 {
                colors.roi.to_color()
            } else {
                colors.healthy.to_color()
            };
            ui.label(egui::RichText::new(format!("dead {dead:.1} %")).color(colour));
        }
        ui.monospace(format!(
            "{} cnts",
            crate::view::format_counts(spectrum.total_counts())
        ));
        if roomy {
            ui.separator();
            ui.label(
                egui::RichText::new(format!(" {} ", spectrum.mode.label()))
                    .monospace()
                    .color(colors.background.to_color())
                    .background_color(colors.foreground.with_alpha(0.85)),
            );
            ui.label(
                egui::RichText::new(match display.vertical {
                    VerticalScale::Logarithmic => "log".to_string(),
                    VerticalScale::Automatic => "auto".to_string(),
                    VerticalScale::Linear(max) => format!("lin {}", format_counts(max)),
                })
                .weak(),
            );
        }
        ui.with_layout(egui::Layout::right_to_left(egui::Align::Center), |ui| {
            // ⛶ is in egui's fonts; the fancier restore glyphs are not.
            let label = if header.maximized {
                "⛶ restore"
            } else {
                "⛶ maximise"
            };
            let _ = header.source;
            if ui
                .small_button(label)
                .on_hover_text("fill the spectrum area (Ctrl+M; Ctrl+Tab cycles windows)")
                .clicked()
            {
                events.push(ViewEvent::ToggleMaximize);
            }
            // The ROI count is the first thing to go when the header runs out
            // of room - otherwise it collides with the counts on the left and
            // reads as one glued-together number ("5.85M cnts12 ROI").
            if !spectrum.rois.is_empty() && ui.available_width() > 70.0 {
                ui.label(
                    egui::RichText::new(format!("{} ROI", spectrum.rois.len()))
                        .monospace()
                        .weak(),
                );
            }
        });
    });
}

/// The marker information line (§3.2 item 10).
fn marker_line(
    ui: &mut Ui,
    spectrum: &Spectrum,
    display: &DisplayState,
    selection: Option<(usize, usize)>,
) {
    ui.add_space(1.0);
    // As with the header, the line adapts to a narrow window rather than
    // forcing it wider: the marker's position always shows, the rest as fits.
    let width = ui.available_width();
    let (roomy, medium) = (width >= 560.0, width >= 380.0);
    ui.horizontal(|ui| {
        let channel = display.marker;
        ui.monospace(format!("Marker: {channel:>6}"));
        match &spectrum.energy_calibration {
            Some(cal) => ui.monospace(format!(
                "= {:>9.2} {}",
                cal.energy(channel as f64),
                cal.units
            )),
            None => ui.monospace("= uncalibrated"),
        };
        if medium {
            ui.monospace(format!("{:>9} cnts", spectrum.counts(channel)));
        }
        let (start, end) = display.visible();
        if roomy {
            ui.separator();
            ui.monospace(format!("view {start}-{end}"));
        }
        if let Some((low, high)) = selection {
            ui.separator();
            let sum = spectrum.sum_range(low, high);
            // The midpoint the guide line is drawn at, as a number: the line
            // shows where the centre of the band falls, this says what it is.
            let middle = (low + high) as f64 / 2.0;
            match &spectrum.energy_calibration {
                Some(cal) => ui.monospace(format!(
                    "selected {:.1}-{:.1} {} (mid {:.1}, {} ch, {} cnts)",
                    cal.energy(low as f64),
                    cal.energy(high as f64),
                    cal.units,
                    cal.energy(middle),
                    high - low + 1,
                    sum
                )),
                None => ui.monospace(format!(
                    "selected {low}-{high} (mid {middle:.1}, {} ch, {sum} cnts)",
                    high - low + 1
                )),
            };
        }
    });
}

/// Buttons that act on the rubber-rectangle selection.
fn selection_actions(
    ui: &mut Ui,
    spectrum: &Spectrum,
    low: usize,
    high: usize,
    mark_mode: MarkMode,
    events: &mut Vec<ViewEvent>,
) {
    // Wrapped, so a narrow window gets two short rows of buttons rather than
    // being forced wide.
    ui.horizontal_wrapped(|ui| {
        if ui
            .small_button("Zoom to selection")
            .on_hover_text("show only these channels")
            .clicked()
        {
            events.push(ViewEvent::ZoomTo(low, high));
            events.push(ViewEvent::SelectionCleared);
        }
        if ui.small_button("Mark ROI").clicked() {
            events.push(ViewEvent::MarkRoi(low, high));
        }
        if ui.small_button("UnMark").clicked() {
            events.push(ViewEvent::UnmarkRoi(low, high));
        }
        if ui.small_button("Peak Info").clicked() {
            events.push(ViewEvent::PeakInfo((low + high) / 2));
        }
        if ui.small_button("Sum").clicked() {
            events.push(ViewEvent::Menu(MenuCommand::Sum));
        }
        if ui.small_button("Clear selection").clicked() {
            events.push(ViewEvent::SelectionCleared);
        }
        let _ = (spectrum, mark_mode);
    });
}

/// A small legend naming the comparison trace, so two spectra on one plot never
/// leave the reader guessing which is which.
fn draw_compare_legend(
    painter: &egui::Painter,
    plot: Rect,
    compare: &Spectrum,
    offset: i64,
    colors: &SpectrumColors,
) {
    let name = compare
        .source
        .as_deref()
        .and_then(|source| {
            std::path::Path::new(source)
                .file_name()
                .map(|name| name.to_string_lossy().into_owned())
        })
        .unwrap_or_else(|| "comparison".into());
    let text = if offset != 0 {
        format!("· {name}  {offset:+}")
    } else {
        format!("· {name}")
    };
    let galley = painter.layout_no_wrap(text, FontId::monospace(10.0), colors.compare.to_color());
    let at = Pos2::new(plot.left() + 6.0, plot.top() + 4.0);
    let box_rect = Rect::from_min_size(at, galley.size()).expand2(Vec2::new(5.0, 2.0));
    painter.rect_filled(
        box_rect,
        CornerRadius::same(3),
        colors.background.to_color().gamma_multiply(2.0),
    );
    painter.galley(at, galley, colors.compare.to_color());
}

fn full_view_rect(plot: Rect) -> Rect {
    let width = (plot.width() * 0.32).clamp(110.0, 400.0);
    let height = (plot.height() * 0.26).clamp(44.0, 130.0);
    Rect::from_min_size(
        Pos2::new(plot.right() - width - 6.0, plot.top() + 4.0),
        Vec2::new(width, height),
    )
}

/// The inset full spectrum view, always logarithmic, with the expanded region
/// outlined (MAESTRO Fig. 6).
fn draw_full_view(
    painter: &egui::Painter,
    plot: Rect,
    spectrum: &Spectrum,
    display: &DisplayState,
    colors: &SpectrumColors,
) {
    // An overview of nothing is just clutter; it appears with the first count.
    if spectrum.total_counts() == 0 {
        return;
    }
    let inset = full_view_rect(plot);
    painter.rect_filled(
        inset,
        CornerRadius::same(2),
        colors.background.to_color().gamma_multiply(2.0),
    );
    painter.rect_stroke(
        inset,
        CornerRadius::same(2),
        Stroke::new(1.0, colors.axes.to_color().gamma_multiply(0.7)),
        StrokeKind::Inside,
    );

    let length = spectrum.len();
    if length == 0 {
        return;
    }
    let peak = spectrum.max_count().max(1) as f64;
    let columns = (inset.width() - 2.0).floor().max(1.0) as usize;
    let inner_height = inset.height() - 4.0;
    let mut fill = Vec::with_capacity(columns);
    for column in 0..columns {
        let start = column * length / columns;
        let end = (((column + 1) * length) / columns)
            .max(start + 1)
            .min(length);
        let value = spectrum.channels[start..end]
            .iter()
            .copied()
            .max()
            .unwrap_or(0);
        if value == 0 {
            continue;
        }
        let fraction = ((value as f64).ln_1p() / peak.ln_1p()) as f32;
        let x = inset.left() + 1.0 + column as f32 + 0.5;
        let y = inset.bottom() - 2.0 - fraction * inner_height;
        // Columns inside a marked region take the region colour, so the overview
        // also answers "where are my regions?" when the expanded view is zoomed
        // somewhere else entirely.
        let in_roi = spectrum.rois.intersects(start, end.saturating_sub(1));
        let color = if in_roi {
            colors.roi.to_color().gamma_multiply(0.9)
        } else {
            colors.foreground.to_color().gamma_multiply(0.55)
        };
        fill.push(Shape::line_segment(
            [Pos2::new(x, inset.bottom() - 2.0), Pos2::new(x, y)],
            Stroke::new(1.0, color),
        ));
    }
    painter.extend(fill);

    // The marker, so the overview also answers "where am I?".
    if length > 1 {
        let marker_x = inset.left()
            + 1.0
            + (inset.width() - 2.0) * display.marker.min(length - 1) as f32 / (length - 1) as f32;
        painter.line_segment(
            [
                Pos2::new(marker_x, inset.top() + 1.0),
                Pos2::new(marker_x, inset.bottom() - 1.0),
            ],
            Stroke::new(1.0, colors.marker.to_color().gamma_multiply(0.65)),
        );
    }

    // The rectangle showing what the expanded view covers.
    let (start, end) = display.visible();
    let left = inset.left() + 1.0 + (inset.width() - 2.0) * start as f32 / length as f32;
    let right = inset.left() + 1.0 + (inset.width() - 2.0) * (end + 1) as f32 / length as f32;
    let box_rect = Rect::from_min_max(
        Pos2::new(left, inset.top() + 1.0),
        Pos2::new(right.max(left + 2.0), inset.bottom() - 1.0),
    );
    painter.rect_filled(
        box_rect,
        CornerRadius::ZERO,
        colors.view_box.to_color().gamma_multiply(0.25),
    );
    painter.rect_stroke(
        box_rect,
        CornerRadius::ZERO,
        Stroke::new(1.0, colors.view_box.to_color()),
        StrokeKind::Inside,
    );
    painter.text(
        Pos2::new(inset.left() + 4.0, inset.bottom() - 2.0),
        Align2::LEFT_BOTTOM,
        "full",
        FontId::monospace(9.0),
        colors.axes.to_color().gamma_multiply(0.9),
    );
}

/// A vertical wash over the plot background, dark at the top and carrying a
/// trace of the data hue at the bottom, where the spectrum's own body sits.
///
/// It gives the plot depth; the contrast tests in [`crate::theme`] keep it from
/// ever competing with the trace drawn over it.
fn draw_background_wash(painter: &egui::Painter, outer: Rect, colors: &SpectrumColors) {
    let top = colors.background.to_color();
    let bottom = colors.background_wash().to_color();
    let mut mesh = egui::Mesh::default();
    let inner = outer.shrink(1.0);
    mesh.colored_vertex(inner.left_top(), top);
    mesh.colored_vertex(inner.right_top(), top);
    mesh.colored_vertex(inner.left_bottom(), bottom);
    mesh.colored_vertex(inner.right_bottom(), bottom);
    mesh.add_triangle(0, 1, 2);
    mesh.add_triangle(2, 1, 3);
    painter.add(Shape::mesh(mesh));
}

/// Axis frame, gridlines and labels.
fn draw_axes(
    painter: &egui::Painter,
    outer: Rect,
    plot: Rect,
    spectrum: &Spectrum,
    display: &DisplayState,
    colors: &SpectrumColors,
    full_scale: u64,
) {
    let axis = colors.axes.to_color();
    let grid = axis.gamma_multiply(0.35);
    let label_font = FontId::monospace(10.0);

    // Vertical: decades in logarithmic mode, quarters otherwise.
    match display.vertical {
        VerticalScale::Logarithmic => {
            let minor = axis.gamma_multiply(0.14);
            let mut decade = 1u64;
            while decade <= full_scale {
                let fraction = display.height_fraction(decade, full_scale);
                let y = plot.bottom() - fraction * plot.height();
                if fraction > 0.02 && fraction < 0.99 {
                    painter.line_segment(
                        [Pos2::new(plot.left(), y), Pos2::new(plot.right(), y)],
                        Stroke::new(1.0, grid),
                    );
                    painter.text(
                        Pos2::new(plot.left() - 6.0, y),
                        Align2::RIGHT_CENTER,
                        format_axis_count(decade),
                        label_font.clone(),
                        axis,
                    );
                }
                // The 2 and 5 lines inside the decade, when there is room for
                // them to mean something - the mark of a log plot made with care.
                let next = display.height_fraction(decade.saturating_mul(10), full_scale);
                if (next - fraction) * plot.height() > 40.0 {
                    for step in [2u64, 5] {
                        let value = decade.saturating_mul(step);
                        let fraction = display.height_fraction(value, full_scale);
                        if fraction > 0.02 && fraction < 0.99 {
                            let y = plot.bottom() - fraction * plot.height();
                            painter.line_segment(
                                [Pos2::new(plot.left(), y), Pos2::new(plot.right(), y)],
                                Stroke::new(1.0, minor),
                            );
                        }
                    }
                }
                decade = decade.saturating_mul(10);
            }
        }
        _ => {
            for step in 1..=4 {
                let fraction = step as f32 / 4.0;
                let y = plot.bottom() - fraction * plot.height();
                if step < 4 {
                    painter.line_segment(
                        [Pos2::new(plot.left(), y), Pos2::new(plot.right(), y)],
                        Stroke::new(1.0, grid),
                    );
                }
                painter.text(
                    Pos2::new(plot.left() - 6.0, y),
                    Align2::RIGHT_CENTER,
                    format_axis_count((full_scale as f64 * fraction as f64) as u64),
                    label_font.clone(),
                    axis,
                );
            }
        }
    }

    // The axis lines themselves.
    painter.line_segment(
        [plot.left_bottom(), plot.right_bottom()],
        Stroke::new(1.0, axis.gamma_multiply(0.8)),
    );
    painter.line_segment(
        [plot.left_top(), plot.left_bottom()],
        Stroke::new(1.0, axis.gamma_multiply(0.8)),
    );

    // Horizontal: ticks at round values of whatever the axis shows - energies
    // when calibrated, channels otherwise. "608, 1199, 1791" is what a naive
    // even split gives; "500, 1000, 1500" is what a plot made with care shows.
    let (start, end) = display.visible();
    let calibrated = spectrum.energy_calibration.is_some() && display.energy_axis;
    let (low, high) = match (&spectrum.energy_calibration, calibrated) {
        (Some(cal), true) => (cal.energy(start as f64), cal.energy(end as f64)),
        _ => (start as f64, end as f64),
    };
    let (low, high) = (low.min(high), low.max(high));
    let target = (plot.width() / 90.0).clamp(3.0, 10.0) as usize;
    for value in nice_ticks(low, high, target) {
        // Back from an axis value to a horizontal position.
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
        let x = plot.left() + plot.width() * fraction;
        painter.line_segment(
            [
                Pos2::new(x, plot.bottom()),
                Pos2::new(x, plot.bottom() + 3.0),
            ],
            Stroke::new(1.0, axis.gamma_multiply(0.8)),
        );
        // A faint vertical gridline, matching the horizontal ones.
        painter.line_segment(
            [Pos2::new(x, plot.top()), Pos2::new(x, plot.bottom())],
            Stroke::new(1.0, axis.gamma_multiply(0.10)),
        );
        let text = if value.fract().abs() < 1e-9 {
            format!("{value:.0}")
        } else {
            format!("{value:.1}")
        };
        let anchor = if fraction < 0.03 {
            Align2::LEFT_TOP
        } else if fraction > 0.97 {
            Align2::RIGHT_TOP
        } else {
            Align2::CENTER_TOP
        };
        painter.text(
            Pos2::new(x, plot.bottom() + 4.0),
            anchor,
            text,
            label_font.clone(),
            axis,
        );
    }
    // Axis names go on their own line, under the tick numbers and above the plot,
    // so they never collide with a value.
    let axis_title = match (&spectrum.energy_calibration, calibrated) {
        (Some(cal), true) => format!("energy ({})", cal.units),
        _ => "channel".to_string(),
    };
    painter.text(
        Pos2::new(plot.center().x, outer.bottom() - 2.0),
        Align2::CENTER_BOTTOM,
        axis_title,
        FontId::monospace(9.0),
        axis.gamma_multiply(0.85),
    );
    painter.text(
        Pos2::new(outer.left() + 6.0, outer.top() + 2.0),
        Align2::LEFT_TOP,
        "counts",
        FontId::monospace(9.0),
        axis.gamma_multiply(0.85),
    );
}

/// Draws one spectrum trace: a filled body with a bright top edge, and the
/// channels inside regions in the region colour.
#[allow(clippy::too_many_arguments)]
fn draw_trace(
    painter: &egui::Painter,
    plot: Rect,
    spectrum: &Spectrum,
    display: &DisplayState,
    full_scale: u64,
    color: Color32,
    vertical_offset: i64,
    fill: FillMode,
    roi_color: Option<Color32>,
) {
    let (start, end) = display.visible();
    let visible = end.saturating_sub(start) + 1;
    let columns = plot.width().floor().max(1.0) as usize;
    let baseline_y = plot.bottom();
    let mut body: Vec<Shape> = Vec::with_capacity(columns);
    let mut outline: Vec<Pos2> = Vec::with_capacity(columns);
    // Filled areas are collected as runs of columns and painted as gradient
    // ribbons: bright where they meet the trace, fading towards the baseline, so
    // a wide region reads as shading rather than as a solid slab.
    // Each run remembers whether it is inside a region, because the two fade
    // differently: an ordinary fill is a soft body under the trace, a region is
    // a bright crown on top of each channel that fades out immediately below.
    let mut runs: Vec<(Vec<(Pos2, Color32)>, bool)> = Vec::new();
    let mut run: Vec<(Pos2, Color32)> = Vec::new();
    let mut run_in_roi = false;
    let fill_color = color.gamma_multiply(0.55);
    // Regions tint the fill rather than replacing it. Replacing it means that on
    // a logarithmic scale, where the filled area reaches most of the way up the
    // plot, a marked region repaints the whole window in the region colour; a
    // tint says the same thing without taking the background over.
    // The trace itself does take the region colour, drawn over the ordinary
    // trace afterwards, so the eye still follows a marked peak.
    let mut roi_outline: Vec<Vec<Pos2>> = Vec::new();
    let mut roi_run: Vec<Pos2> = Vec::new();

    for column in 0..columns {
        let first = start + column * visible / columns;
        let last = (start + ((column + 1) * visible) / columns)
            .max(first + 1)
            .min(spectrum.len());
        if first >= spectrum.len() {
            break;
        }
        let slice = &spectrum.channels[first..last];
        let value = slice.iter().copied().max().unwrap_or(0);
        let offset_value = (value as i64 + vertical_offset).max(0) as u64;
        let fraction = display.height_fraction(offset_value, full_scale);
        let x = plot.left() + column as f32 + 0.5;
        let y = baseline_y - fraction * plot.height();
        let in_roi = roi_color.is_some() && spectrum.rois.intersects(first, last.saturating_sub(1));
        let stroke_color = if in_roi {
            roi_color.unwrap_or(fill_color)
        } else {
            fill_color
        };

        match fill {
            FillMode::Points => {
                if fraction > 0.0 {
                    body.push(Shape::circle_filled(Pos2::new(x, y), 1.1, stroke_color));
                }
            }
            FillMode::FillRoi => {
                if in_roi && fraction > 0.0 {
                    run_in_roi = true;
                    run.push((Pos2::new(x, y), stroke_color));
                } else if !run.is_empty() {
                    runs.push((std::mem::take(&mut run), run_in_roi));
                }
            }
            FillMode::FillAll => {
                if fraction > 0.0 {
                    if !run.is_empty() && in_roi != run_in_roi {
                        runs.push((std::mem::take(&mut run), run_in_roi));
                    }
                    run_in_roi = in_roi;
                    run.push((Pos2::new(x, y), stroke_color));
                } else if !run.is_empty() {
                    runs.push((std::mem::take(&mut run), run_in_roi));
                }
            }
        }
        outline.push(Pos2::new(x, y));
        if in_roi {
            roi_run.push(Pos2::new(x, y));
        } else if !roi_run.is_empty() {
            roi_outline.push(std::mem::take(&mut roi_run));
        }
    }
    if !run.is_empty() {
        runs.push((run, run_in_roi));
    }
    for (run, is_roi) in runs {
        let (top, bottom) = if is_roi {
            (ROI_TOP_ALPHA, ROI_BOTTOM_ALPHA)
        } else {
            (RIBBON_TOP_ALPHA, RIBBON_BOTTOM_ALPHA)
        };
        painter.add(gradient_ribbon(&run, baseline_y, top, bottom));
    }
    painter.extend(body);
    if outline.len() > 1 && !matches!(fill, FillMode::Points) {
        // A wide, faint stroke under a thin bright one reads as a glow, which
        // makes the trace the brightest thing on the plot without washing it out.
        painter.add(Shape::line(
            outline.clone(),
            Stroke::new(3.0, color.gamma_multiply(0.20)),
        ));
        painter.add(Shape::line(outline, Stroke::new(1.2, color)));
        if let Some(roi) = roi_color {
            if !roi_run.is_empty() {
                roi_outline.push(roi_run);
            }
            for run in roi_outline {
                if run.len() > 1 {
                    painter.add(Shape::line(run, Stroke::new(1.4, roi)));
                }
            }
        }
    }
}

/// How strongly a filled area is painted where it meets the trace, and where it
/// meets the baseline.
const RIBBON_TOP_ALPHA: f32 = 0.80;
const RIBBON_BOTTOM_ALPHA: f32 = 0.10;

/// The same, for a channel inside a region: full colour where it meets the
/// trace, gone almost at once below it. A region marks the tops of its
/// channels rather than colouring the column under them, which on a
/// logarithmic scale would be most of the window.
const ROI_TOP_ALPHA: f32 = 1.0;
const ROI_BOTTOM_ALPHA: f32 = 0.0;

/// A run of trace tops turned into one filled shape that fades downwards.
fn gradient_ribbon(
    run: &[(Pos2, Color32)],
    baseline_y: f32,
    top_alpha: f32,
    bottom_alpha: f32,
) -> Shape {
    // A lone column has no width to interpolate across, so it is drawn as a line.
    if let [(top, color)] = run {
        return Shape::line_segment(
            [Pos2::new(top.x, baseline_y), *top],
            Stroke::new(1.0, color.gamma_multiply(top_alpha)),
        );
    }
    let mut mesh = egui::Mesh::default();
    for (index, (top, color)) in run.iter().enumerate() {
        mesh.colored_vertex(*top, color.gamma_multiply(top_alpha));
        mesh.colored_vertex(
            Pos2::new(top.x, baseline_y),
            color.gamma_multiply(bottom_alpha),
        );
        if index > 0 {
            let (previous, current) = ((index as u32 - 1) * 2, index as u32 * 2);
            mesh.add_triangle(previous, previous + 1, current);
            mesh.add_triangle(current, previous + 1, current + 1);
        }
    }
    Shape::mesh(mesh)
}

fn draw_marker(
    painter: &egui::Painter,
    plot: Rect,
    spectrum: &Spectrum,
    display: &DisplayState,
    colors: &SpectrumColors,
) {
    if let Some(fraction) = display.fraction_of(display.marker) {
        let x = plot.left() + plot.width() * fraction;
        painter.line_segment(
            [Pos2::new(x, plot.top()), Pos2::new(x, plot.bottom())],
            Stroke::new(1.0, colors.marker.to_color()),
        );
        // A tick at the bottom, so the marker is findable when the trace is busy.
        painter.add(Shape::convex_polygon(
            vec![
                Pos2::new(x - 4.0, plot.bottom()),
                Pos2::new(x + 4.0, plot.bottom()),
                Pos2::new(x, plot.bottom() - 5.0),
            ],
            colors.marker.to_color(),
            Stroke::NONE,
        ));
    }
    let _ = spectrum;
}

fn draw_selection(
    painter: &egui::Painter,
    plot: Rect,
    display: &DisplayState,
    low: usize,
    high: usize,
    colors: &SpectrumColors,
) {
    let (start, end) = display.visible();
    if high < start || low > end {
        return;
    }
    let left = display.fraction_of(low.max(start)).unwrap_or(0.0);
    let right = display.fraction_of(high.min(end)).unwrap_or(1.0);
    let rect = Rect::from_min_max(
        Pos2::new(plot.left() + plot.width() * left, plot.top()),
        Pos2::new(plot.left() + plot.width() * right, plot.bottom()),
    );
    painter.rect_filled(
        rect,
        CornerRadius::ZERO,
        colors.marker.to_color().gamma_multiply(0.18),
    );
    for x in [rect.left(), rect.right()] {
        painter.line_segment(
            [Pos2::new(x, plot.top()), Pos2::new(x, plot.bottom())],
            Stroke::new(1.0, colors.marker.to_color().gamma_multiply(0.9)),
        );
    }
    // The centre of the band, full height, as MAESTRO draws it: while dragging
    // out a region it is what you line up with the middle of a peak. Fainter
    // than the edges, so it reads as a guide rather than a third boundary.
    let centre = (rect.left() + rect.right()) / 2.0;
    painter.line_segment(
        [
            Pos2::new(centre, plot.top()),
            Pos2::new(centre, plot.bottom()),
        ],
        Stroke::new(1.0, colors.marker.to_color().gamma_multiply(0.45)),
    );
}

/// Draws the peak-information overlay over the peak it describes: the region,
/// the straight-line background, the fitted Gaussian, the width at half maximum
/// and the readout box (MAESTRO Fig. 60).
///
/// Returns the box's rectangle so a click on it can dismiss it.
#[allow(clippy::too_many_arguments)]
fn draw_peak_info(
    painter: &egui::Painter,
    plot: Rect,
    spectrum: &Spectrum,
    display: &DisplayState,
    colors: &SpectrumColors,
    info: &PeakInfo,
    library: Option<&NuclideLibrary>,
    peak_font: f32,
) -> Option<Rect> {
    let full_scale = display.full_scale(spectrum);
    let x_of = |channel: f64| -> f32 {
        let (start, end) = display.visible();
        let span = (end - start).max(1) as f64;
        plot.left() + plot.width() * ((channel - start as f64) / span) as f32
    };
    let y_of = |counts: f64| -> f32 {
        plot.bottom() - display.height_fraction(counts.max(0.0) as u64, full_scale) * plot.height()
    };

    let region = info.roi;
    let points = info.background_points.max(1);
    // The two background anchor points, as the peak-info calculation uses them.
    let mean = |from: usize, count: usize| -> f64 {
        let sum: u64 = (from..from + count).map(|c| spectrum.counts(c)).sum();
        sum as f64 / count as f64
    };
    let low_anchor = (
        region.start as f64 + (points as f64 - 1.0) / 2.0,
        mean(region.start, points),
    );
    let high_anchor = (
        region.end as f64 - (points as f64 - 1.0) / 2.0,
        mean(region.end + 1 - points, points),
    );

    // The region, shaded.
    let region_rect = Rect::from_min_max(
        Pos2::new(x_of(region.start as f64), plot.top()),
        Pos2::new(x_of(region.end as f64), plot.bottom()),
    );
    if region_rect.width() > 1.0 {
        painter.rect_filled(
            region_rect,
            CornerRadius::ZERO,
            colors.roi.to_color().gamma_multiply(0.10),
        );
    }

    // The straight-line background between the anchors, dashed.
    let background_line = [
        Pos2::new(x_of(low_anchor.0), y_of(low_anchor.1)),
        Pos2::new(x_of(high_anchor.0), y_of(high_anchor.1)),
    ];
    painter.add(Shape::dashed_line(
        &background_line,
        Stroke::new(1.4, colors.composite.to_color()),
        6.0,
        4.0,
    ));
    for point in background_line {
        painter.circle_filled(point, 2.5, colors.composite.to_color());
    }

    // The fitted Gaussian, on top of the background.
    if let Some(fit) = info.fit {
        let slope = if (high_anchor.0 - low_anchor.0).abs() > f64::EPSILON {
            (high_anchor.1 - low_anchor.1) / (high_anchor.0 - low_anchor.0)
        } else {
            0.0
        };
        let background_at = |channel: f64| low_anchor.1 + slope * (channel - low_anchor.0);
        let steps = 64;
        let curve: Vec<Pos2> = (0..=steps)
            .map(|step| {
                let channel =
                    region.start as f64 + (region.len() - 1) as f64 * step as f64 / steps as f64;
                let x = (channel - fit.centroid) / fit.sigma;
                let value = fit.amplitude * (-0.5 * x * x).exp() + background_at(channel);
                Pos2::new(x_of(channel), y_of(value))
            })
            .collect();
        painter.add(Shape::line(
            curve,
            Stroke::new(1.6, colors.marker.to_color().gamma_multiply(0.95)),
        ));

        // The width at half maximum, as a bar with end ticks.
        if info.fwhm > 0.0 {
            let half = background_at(fit.centroid) + fit.amplitude / 2.0;
            let y = y_of(half);
            let (left, right) = (
                x_of(fit.centroid - info.fwhm / 2.0),
                x_of(fit.centroid + info.fwhm / 2.0),
            );
            let stroke = Stroke::new(1.4, colors.library.to_color());
            painter.line_segment([Pos2::new(left, y), Pos2::new(right, y)], stroke);
            for x in [left, right] {
                painter.line_segment([Pos2::new(x, y - 4.0), Pos2::new(x, y + 4.0)], stroke);
            }
            painter.text(
                Pos2::new((left + right) / 2.0, y - 5.0),
                Align2::CENTER_BOTTOM,
                "FWHM",
                FontId::monospace(9.0),
                colors.library.to_color(),
            );
        }
    }

    // The centroid.
    let centroid_x = x_of(info.centroid);
    painter.line_segment(
        [
            Pos2::new(centroid_x, plot.top()),
            Pos2::new(centroid_x, plot.bottom()),
        ],
        Stroke::new(1.0, colors.marker.to_color().gamma_multiply(0.6)),
    );

    // The readout box.
    let mut lines = peak_info_lines(spectrum, info, library);
    lines.push("(click this box to dismiss)".to_string());

    let font = FontId::monospace(peak_font.clamp(9.0, 18.0));
    // PLACEHOLDER, so the per-line colour given at paint time really applies -
    // a concrete colour here would win, and on a light card the text vanished.
    let galleys: Vec<_> = lines
        .iter()
        .map(|line| painter.layout_no_wrap(line.clone(), font.clone(), Color32::PLACEHOLDER))
        .collect();
    let width = galleys
        .iter()
        .map(|galley| galley.size().x)
        .fold(0.0f32, f32::max)
        + 14.0;
    let line_height = galleys
        .first()
        .map(|galley| galley.size().y)
        .unwrap_or(12.0)
        + 1.0;
    let height = line_height * galleys.len() as f32 + 10.0;

    // Above the peak when there is room, otherwise beside it, always inside the
    // plot and away from the overview inset.
    let apex_y = y_of(
        info.fit
            .map(|fit| fit.amplitude + info.background / region.len() as f64)
            .unwrap_or_else(|| spectrum.counts(info.centroid as usize) as f64),
    );
    let box_rect = place_info_box(
        plot,
        full_view_rect(plot),
        centroid_x,
        apex_y,
        Vec2::new(width, height),
    );

    // A leader line from the box to the peak.
    painter.line_segment(
        [
            box_rect.center(),
            Pos2::new(centroid_x, apex_y.max(plot.top())),
        ],
        Stroke::new(1.0, colors.roi.with_alpha(0.55)),
    );
    // A dark card, so the peak stays visible behind it, with a region-coloured
    // spine tying it to the peak it describes.
    painter.rect_filled(
        box_rect,
        CornerRadius::same(4),
        colors.panel.to_color().gamma_multiply(1.45),
    );
    painter.rect_stroke(
        box_rect,
        CornerRadius::same(4),
        Stroke::new(1.0, colors.roi.with_alpha(0.75)),
        StrokeKind::Inside,
    );
    painter.rect_filled(
        Rect::from_min_size(box_rect.min, Vec2::new(3.0, box_rect.height())),
        CornerRadius::same(2),
        colors.roi.to_color(),
    );
    for (index, galley) in galleys.into_iter().enumerate() {
        // Headline in the marker colour, the hint quiet, the numbers in the data
        // colour so the box belongs to the spectrum it describes.
        let colour = if index == 0 {
            colors.marker.to_color()
        } else if index + 1 == lines.len() {
            colors.axes.with_alpha(0.85)
        } else {
            colors.foreground.to_color()
        };
        painter.galley(
            Pos2::new(
                box_rect.left() + 9.0,
                box_rect.top() + 5.0 + line_height * index as f32,
            ),
            galley,
            colour,
        );
    }
    Some(box_rect)
}

/// The peak-information card's text, one line per row.
///
/// Shared between the painter and Edit/Copy Peak Info, so what lands on the
/// clipboard is exactly what is on the screen.
pub(crate) fn peak_info_lines(
    spectrum: &Spectrum,
    info: &PeakInfo,
    library: Option<&NuclideLibrary>,
) -> Vec<String> {
    let calibration = spectrum.energy_calibration.as_ref();
    let live_time = spectrum.live_time;
    let region = info.roi;
    let points = info.background_points.max(1);
    let mut lines: Vec<String> = Vec::new();
    match calibration {
        Some(cal) => lines.push(format!(
            "Peak: {:.2} = {:.2} {}",
            info.centroid,
            info.centroid_energy(cal),
            cal.units
        )),
        None => lines.push(format!("Peak: {:.2} ch", info.centroid)),
    }
    match calibration {
        Some(cal) => lines.push(format!(
            "FWHM: {:.2} {}   FW(1/{})M: {:.2} {}",
            info.fwhm_energy(cal),
            cal.units,
            info.fw_x,
            info.fw_x_m_energy(cal),
            cal.units
        )),
        None => lines.push(format!(
            "FWHM: {:.2} ch   FW(1/{})M: {:.2} ch",
            info.fwhm, info.fw_x, info.fw_x_m
        )),
    }
    if let (Some(cal), Some(library)) = (calibration, library)
        && let Some(matched) = library.best_match(info.centroid_energy(cal), 2.5)
    {
        lines.push(format!(
            "Library: {} at {:.2} ; {:.1} cA",
            matched.nuclide.name,
            matched.peak.energy,
            info.activity(matched.peak.yield_percent, live_time)
        ));
    }
    lines.push(format!("Gross Area: {:.0}", info.gross_area));
    lines.push(format!(
        "Net Area: {:.0} \u{b1}{:.0}  ({:.2} %)",
        info.net_area,
        info.net_area_uncertainty,
        info.net_uncertainty_percent()
    ));
    lines.push(format!(
        "Background: {:.0} over {} ch, n = {}",
        info.background,
        region.len(),
        points
    ));
    if live_time > 0.0 {
        lines.push(format!(
            "Gross/Net Count Rate: {:.2} / {:.2} cps",
            info.gross_count_rate(live_time),
            info.net_count_rate(live_time)
        ));
    }
    if let Some(fit) = info.fit {
        lines.push(format!(
            "Fit: sigma {:.2} ch, height {:.0}, area {:.0}",
            fit.sigma,
            fit.amplitude,
            fit.area()
        ));
    } else {
        lines.push("Could Not Properly Fit Peak".to_string());
    }
    lines
}

/// Where to put the peak-information card: beside the peak it describes,
/// inside the plot, and clear of the overview inset when that is possible at
/// all. Candidates are tried in order of preference, so a peak near an edge
/// still gets a legible box.
fn place_info_box(plot: Rect, inset: Rect, centroid_x: f32, apex_y: f32, size: Vec2) -> Rect {
    let margin = 4.0;
    let keep_out = inset.expand(6.0);
    let above = apex_y - size.y - 10.0;
    let below = apex_y + 14.0;
    let right = centroid_x + 12.0;
    let left = centroid_x - size.x - 12.0;
    let under_inset = keep_out.bottom() + 4.0;
    let candidates = [
        Pos2::new(right, above),
        Pos2::new(left, above),
        Pos2::new(right, below),
        Pos2::new(left, below),
        Pos2::new(right, under_inset),
        Pos2::new(left, under_inset),
        Pos2::new(plot.left() + margin, under_inset),
    ];

    let inside = |origin: Pos2| {
        origin.x >= plot.left() + margin
            && origin.y >= plot.top() + margin
            && origin.x + size.x <= plot.right() - margin
            && origin.y + size.y <= plot.bottom() - margin
    };
    let rect_of = |origin: Pos2| Rect::from_min_size(origin, size);

    if let Some(origin) = candidates
        .iter()
        .copied()
        .find(|&origin| inside(origin) && !rect_of(origin).intersects(keep_out))
        .or_else(|| candidates.iter().copied().find(|&origin| inside(origin)))
    {
        return rect_of(origin);
    }

    // Nothing fits outright: clamp the preferred position into the plot.
    let clamp = |value: f32, low: f32, high: f32| value.clamp(low, high.max(low));
    rect_of(Pos2::new(
        clamp(right, plot.left() + margin, plot.right() - size.x - margin),
        clamp(above, plot.top() + margin, plot.bottom() - size.y - margin),
    ))
}

/// The channel holding the most counts in a region, and how many.
fn region_peak(spectrum: &Spectrum, region: ortseam_core::roi::Roi) -> Option<(usize, u64)> {
    (region.start..=region.end.min(spectrum.len().saturating_sub(1)))
        .map(|channel| (channel, spectrum.counts(channel)))
        .max_by_key(|(_, counts)| *counts)
}

/// A caret over the top of every marked region, named where there is room.
///
/// This is the at-a-glance answer to "what did the peak search find, and what is
/// it?" - the region fills say where, the carets say which, and the label falls
/// back from a nuclide name to an energy when the library has nothing to offer.
#[allow(clippy::too_many_arguments)]
fn draw_peak_markers(
    painter: &egui::Painter,
    plot: Rect,
    spectrum: &Spectrum,
    display: &DisplayState,
    colors: &SpectrumColors,
    library: Option<&NuclideLibrary>,
    full_scale: u64,
) {
    let (start, end) = display.visible();
    let color = colors.roi.to_color();
    let font = FontId::monospace(9.0);
    // Labels are laid out left to right, and one is skipped when it would
    // overlap the last one drawn.
    let mut labelled: Vec<Rect> = Vec::new();
    for region in spectrum.rois.iter() {
        if region.end < start || region.start > end {
            continue;
        }
        let Some((channel, counts)) = region_peak(spectrum, *region) else {
            continue;
        };
        let Some(fraction) = display.fraction_of(channel) else {
            continue;
        };
        let x = plot.left() + plot.width() * fraction;
        let height = display.height_fraction(counts, full_scale);
        let apex = (plot.bottom() - height * plot.height()).max(plot.top() + 1.0);
        let tip = (apex - 4.0).max(plot.top() + 1.0);

        // A caret pointing down at the peak.
        painter.add(Shape::convex_polygon(
            vec![
                Pos2::new(x, tip + 5.0),
                Pos2::new(x - 3.5, tip),
                Pos2::new(x + 3.5, tip),
            ],
            color.gamma_multiply(0.85),
            Stroke::NONE,
        ));

        let label = peak_label(spectrum, library, channel);
        let Some(label) = label else { continue };
        let galley = painter.layout_no_wrap(label, font.clone(), color);
        let box_rect = Rect::from_min_size(
            Pos2::new(x - galley.size().x / 2.0, tip - galley.size().y - 1.0),
            galley.size(),
        );
        let fits = box_rect.left() >= plot.left()
            && box_rect.right() <= plot.right()
            && box_rect.top() >= plot.top()
            && !labelled
                .iter()
                .any(|other| other.expand2(Vec2::new(3.0, 0.0)).intersects(box_rect));
        if fits {
            painter.galley(box_rect.min, galley, color);
            labelled.push(box_rect);
        }
    }
}

/// What to write over a found peak: the nuclide when the library recognises the
/// energy, otherwise the energy itself, and nothing at all when the spectrum has
/// no calibration to name it by.
pub(crate) fn peak_label(
    spectrum: &Spectrum,
    library: Option<&NuclideLibrary>,
    channel: usize,
) -> Option<String> {
    let cal = spectrum.energy_calibration.as_ref()?;
    nuclide_at(spectrum, library, channel)
        .or_else(|| Some(format!("{:.0}", cal.energy(channel as f64))))
}

/// The nuclide the library puts at a channel's energy, if it names one.
pub(crate) fn nuclide_at(
    spectrum: &Spectrum,
    library: Option<&NuclideLibrary>,
    channel: usize,
) -> Option<String> {
    let cal = spectrum.energy_calibration.as_ref()?;
    let library = library?;
    let energy = cal.energy(channel as f64);
    // A window of a couple of channels either side, which is about a peak
    // width for the detectors this is used with.
    let span = (cal.energy((channel + 2) as f64) - energy).abs().max(0.5);
    library
        .peaks_in(energy - span, energy + span)
        .into_iter()
        .min_by(|a, b| {
            (a.peak.energy - energy)
                .abs()
                .partial_cmp(&(b.peak.energy - energy).abs())
                .unwrap_or(std::cmp::Ordering::Equal)
        })
        .map(|hit| hit.nuclide.name.clone())
}

/// Ticks and names at the library line energies (Display/Isotope Markers).
fn draw_library_lines(
    painter: &egui::Painter,
    plot: Rect,
    spectrum: &Spectrum,
    display: &DisplayState,
    colors: &SpectrumColors,
    library: Option<&NuclideLibrary>,
) {
    let (Some(library), Some(cal)) = (library, spectrum.energy_calibration.as_ref()) else {
        return;
    };
    let (start, end) = display.visible();
    let low = cal.energy(start as f64);
    let high = cal.energy(end as f64);
    let color = colors.library.to_color();
    let mut last_label_x = f32::MIN;
    for hit in library.peaks_in(low.min(high), high.max(low)) {
        let Some(channel) = cal.channel(hit.peak.energy) else {
            continue;
        };
        let Some(fraction) = display.fraction_of(channel.round().max(0.0) as usize) else {
            continue;
        };
        let x = plot.left() + plot.width() * fraction;
        painter.line_segment(
            [
                Pos2::new(x, plot.bottom()),
                Pos2::new(x, plot.bottom() - 10.0),
            ],
            Stroke::new(1.0, color.gamma_multiply(0.9)),
        );
        // Only label a line when there is room, so names do not pile up.
        if x - last_label_x > 44.0 {
            painter.text(
                Pos2::new(x + 2.0, plot.bottom() - 11.0),
                Align2::LEFT_BOTTOM,
                &hit.nuclide.name,
                FontId::monospace(9.0),
                color,
            );
            last_label_x = x;
        }
    }
}

/// Short count labels: 1234, 12.3k, 1.23M.
pub fn format_counts(value: u64) -> String {
    match value {
        0..=9_999 => value.to_string(),
        10_000..=999_999 => format!("{:.1}k", value as f64 / 1e3),
        1_000_000..=999_999_999 => format!("{:.2}M", value as f64 / 1e6),
        _ => format!("{:.2}G", value as f64 / 1e9),
    }
}

/// Round values to put axis ticks at: multiples of 1, 2 or 5 times a power of
/// ten, chosen so about `target` of them span `low..high`.
pub(crate) fn nice_ticks(low: f64, high: f64, target: usize) -> Vec<f64> {
    let span = high - low;
    if !(span.is_finite()) || span <= 0.0 || target == 0 {
        return Vec::new();
    }
    let raw_step = span / target as f64;
    let magnitude = 10f64.powf(raw_step.log10().floor());
    let step = [1.0, 2.0, 5.0, 10.0]
        .iter()
        .map(|multiple| multiple * magnitude)
        .find(|step| span / step <= target as f64)
        .unwrap_or(10.0 * magnitude);
    let first = (low / step).ceil() as i64;
    let last = (high / step).floor() as i64;
    (first..=last).map(|index| index as f64 * step).collect()
}

/// Axis labels: tighter than [`format_counts`], and consistent across the
/// scale so a logarithmic axis reads 1, 10, 100, 1k, 10k, 100k, 1M rather than
/// mixing plain numbers with abbreviations.
pub(crate) fn format_axis_count(value: u64) -> String {
    const UNITS: [(u64, char); 3] = [(1_000_000_000, 'G'), (1_000_000, 'M'), (1_000, 'k')];
    for (scale, suffix) in UNITS {
        if value >= scale {
            let scaled = value as f64 / scale as f64;
            return if scaled < 10.0 && (scaled.fract() * 10.0).round() != 0.0 {
                format!("{scaled:.1}{suffix}")
            } else {
                format!("{}{suffix}", scaled.round() as u64)
            };
        }
    }
    value.to_string()
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn a_filled_area_fades_from_the_trace_to_the_baseline() {
        let run = [
            (Pos2::new(0.0, 10.0), Color32::from_rgb(0, 200, 200)),
            (Pos2::new(1.0, 20.0), Color32::from_rgb(0, 200, 200)),
            (Pos2::new(2.0, 15.0), Color32::from_rgb(0, 200, 200)),
        ];
        let Shape::Mesh(mesh) = gradient_ribbon(&run, 100.0, RIBBON_TOP_ALPHA, RIBBON_BOTTOM_ALPHA)
        else {
            panic!("expected a mesh");
        };
        assert_eq!(mesh.vertices.len(), 6);
        // Two triangles per gap between columns.
        assert_eq!(mesh.indices.len(), 2 * 3 * (run.len() - 1));
        for pair in mesh.vertices.chunks(2) {
            let (top, bottom) = (pair[0], pair[1]);
            assert_eq!(bottom.pos.y, 100.0, "the fill should reach the baseline");
            assert!(
                top.color.a() > bottom.color.a(),
                "the fill should be strongest at the trace"
            );
        }
    }

    #[test]
    fn a_single_column_area_is_still_drawn() {
        let run = [(Pos2::new(4.0, 10.0), Color32::WHITE)];
        assert!(matches!(
            gradient_ribbon(&run, 50.0, RIBBON_TOP_ALPHA, RIBBON_BOTTOM_ALPHA),
            Shape::LineSegment { .. }
        ));
    }

    #[test]
    fn a_region_is_a_crown_on_its_channels_not_a_column_under_them() {
        // The complaint this fixes: on a logarithmic scale the filled area
        // reaches most of the way up, so a region drawn as a solid column
        // repainted the whole window in the region colour.
        let run = [
            (Pos2::new(0.0, 10.0), Color32::from_rgb(240, 170, 40)),
            (Pos2::new(1.0, 20.0), Color32::from_rgb(240, 170, 40)),
        ];
        let Shape::Mesh(mesh) = gradient_ribbon(&run, 100.0, ROI_TOP_ALPHA, ROI_BOTTOM_ALPHA)
        else {
            panic!("expected a mesh");
        };
        for pair in mesh.vertices.chunks(2) {
            let (top, bottom) = (pair[0], pair[1]);
            assert_eq!(
                top.color.a(),
                255,
                "a channel's own height should be the full region colour"
            );
            assert_eq!(
                bottom.color.a(),
                0,
                "and it should be gone by the baseline, leaving the background alone"
            );
        }
    }

    #[test]
    fn axis_ticks_land_on_round_numbers() {
        // The Eu-152 case that looked wrong: 16 to 2974 keV across the plot.
        let ticks = nice_ticks(16.0, 2974.0, 6);
        assert_eq!(
            ticks,
            vec![500.0, 1000.0, 1500.0, 2000.0, 2500.0],
            "a span of about 3000 gets ticks every 500"
        );
        // A tight zoom gets proportionally tighter round steps.
        let ticks = nice_ticks(117.0, 127.0, 5);
        assert_eq!(ticks, vec![118.0, 120.0, 122.0, 124.0, 126.0]);
        // Never more than asked for.
        for (low, high) in [(0.0, 8191.0), (3.0, 17.0), (0.0, 1.0)] {
            let ticks = nice_ticks(low, high, 6);
            assert!(
                (1..=6).contains(&ticks.len()),
                "{low}-{high} produced {} ticks",
                ticks.len()
            );
        }
        // Nonsense spans produce nothing rather than panicking.
        assert!(nice_ticks(5.0, 5.0, 6).is_empty());
        assert!(nice_ticks(10.0, 2.0, 6).is_empty());
        assert!(nice_ticks(0.0, f64::INFINITY, 6).is_empty());
    }

    #[test]
    fn axis_labels_are_consistent_across_decades() {
        assert_eq!(format_axis_count(1), "1");
        assert_eq!(format_axis_count(999), "999");
        assert_eq!(format_axis_count(1_000), "1k");
        assert_eq!(format_axis_count(2_500), "2.5k");
        assert_eq!(format_axis_count(10_000), "10k");
        assert_eq!(format_axis_count(100_000), "100k");
        assert_eq!(format_axis_count(1_000_000), "1M");
        assert_eq!(format_axis_count(1_500_000), "1.5M");
        assert_eq!(format_axis_count(4_000_000_000), "4G");
    }

    #[test]
    fn counts_are_abbreviated() {
        assert_eq!(format_counts(0), "0");
        assert_eq!(format_counts(9_999), "9999");
        assert_eq!(format_counts(12_345), "12.3k");
        assert_eq!(format_counts(999_999), "1000.0k");
        assert_eq!(format_counts(1_234_567), "1.23M");
        assert_eq!(format_counts(12_345_678_901), "12.35G");
    }

    #[test]
    fn mark_modes_have_labels() {
        assert_eq!(MarkMode::default(), MarkMode::Off);
        assert_eq!(MarkMode::Mark.label(), "Marking");
    }

    #[test]
    fn the_overview_sits_inside_the_plot() {
        let plot = Rect::from_min_size(Pos2::new(10.0, 10.0), Vec2::new(800.0, 400.0));
        let inset = full_view_rect(plot);
        assert!(plot.contains_rect(inset));
        assert!(inset.width() >= 110.0);
    }

    #[test]
    fn the_peak_info_box_stays_inside_the_plot() {
        let plot = Rect::from_min_size(Pos2::new(10.0, 10.0), Vec2::new(800.0, 400.0));
        let inset = full_view_rect(plot);
        let size = Vec2::new(320.0, 130.0);
        // Every peak position, including the corners, gets a box on screen.
        for x in [11.0, 100.0, 400.0, 780.0, 809.0] {
            for apex in [11.0, 30.0, 200.0, 405.0] {
                let card = place_info_box(plot, inset, x, apex, size);
                assert!(
                    plot.contains_rect(card),
                    "box for peak at {x},{apex} left the plot: {card:?}"
                );
            }
        }
    }

    #[test]
    fn the_peak_info_box_avoids_the_overview() {
        let plot = Rect::from_min_size(Pos2::new(10.0, 10.0), Vec2::new(800.0, 400.0));
        let inset = full_view_rect(plot);
        let size = Vec2::new(320.0, 130.0);
        // A tall peak right under the overview would otherwise be covered by it.
        let card = place_info_box(plot, inset, inset.center().x - 30.0, 200.0, size);
        assert!(!card.intersects(inset), "the card covers the overview");
        assert!(plot.contains_rect(card));
    }

    #[test]
    fn the_peak_info_box_prefers_to_sit_above_and_right_of_the_peak() {
        let plot = Rect::from_min_size(Pos2::new(10.0, 10.0), Vec2::new(800.0, 400.0));
        let inset = full_view_rect(plot);
        let size = Vec2::new(200.0, 100.0);
        let card = place_info_box(plot, inset, 200.0, 300.0, size);
        assert!(card.left() > 200.0, "expected the card right of the peak");
        assert!(card.bottom() < 300.0, "expected the card above the peak");
    }

    #[test]
    fn an_oversized_peak_info_box_is_still_anchored_in_the_plot() {
        let plot = Rect::from_min_size(Pos2::new(10.0, 10.0), Vec2::new(200.0, 120.0));
        let inset = full_view_rect(plot);
        // Wider and taller than the whole plot: it must not run off the top left.
        let card = place_info_box(plot, inset, 100.0, 60.0, Vec2::new(400.0, 300.0));
        assert_eq!(card.min, Pos2::new(plot.left() + 4.0, plot.top() + 4.0));
    }
}
