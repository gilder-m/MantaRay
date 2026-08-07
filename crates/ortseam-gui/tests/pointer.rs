//! Clicking, dragging and the wheel, driven as real pointer input.
//!
//! The display is rendered headless and fed synthetic pointer events, so the
//! whole chain is covered: egui's response, the origin of a drag, the mapping
//! from a pixel to a channel, and the event the display reports back. Dragging
//! being broken once - the drag origin was computed from the wrong end - is the
//! reason this suite exists.

use egui::{Event, Modifiers, PointerButton, Pos2, RawInput, Rect, Vec2};
use ortseam_core::Spectrum;
use ortseam_gui::theme::Theme;
use ortseam_gui::view::{MarkMode, ViewEvent, ViewHeader, ViewInput, draw};
use ortseam_gui::viewmodel::DisplayState;

/// The area the display is drawn into.
const WIDTH: f32 = 1000.0;
const HEIGHT: f32 = 600.0;

/// A spectrum with a peak in the middle.
fn spectrum(channels: usize) -> Spectrum {
    let mut spectrum = Spectrum::new(channels);
    spectrum.live_time = 100.0;
    spectrum.real_time = 100.0;
    for channel in 0..channels {
        spectrum.channels[channel] = 100;
    }
    for offset in -6i64..=6 {
        let centre = channels as i64 / 2;
        spectrum.channels[(centre + offset) as usize] += 5_000;
    }
    spectrum
}

/// A test harness: draws the display over and over, feeding pointer events.
struct Display {
    ctx: egui::Context,
    spectrum: Spectrum,
    display: DisplayState,
    mark_mode: MarkMode,
    events: Vec<ViewEvent>,
}

impl Display {
    fn new(channels: usize) -> Self {
        let spectrum = spectrum(channels);
        Self {
            ctx: egui::Context::default(),
            display: DisplayState::for_length(spectrum.len()),
            spectrum,
            mark_mode: MarkMode::Off,
            events: Vec::new(),
        }
    }

    /// Runs one frame with the given pointer events, replacing what was reported.
    fn frame(&mut self, events: Vec<Event>) {
        self.events.clear();
        self.frame_keeping_events(events);
    }

    /// Runs one frame, adding to what was reported. A gesture spans frames, and
    /// the application applies what each frame reports, so the test keeps the lot.
    fn frame_keeping_events(&mut self, events: Vec<Event>) {
        let input = RawInput {
            screen_rect: Some(Rect::from_min_size(Pos2::ZERO, Vec2::new(WIDTH, HEIGHT))),
            events,
            ..Default::default()
        };
        let colors = Theme::DeepSpace.colors();
        let spectrum = &self.spectrum;
        let display = &mut self.display;
        let mark_mode = self.mark_mode;
        let mut reported = Vec::new();
        let _ = self.ctx.run_ui(input, |ui| {
            reported = draw(
                ui,
                ViewInput {
                    spectrum,
                    compare: None,
                    compare_offset: 0,
                    display,
                    colors: &colors,
                    mark_mode,
                    library: None,
                    isotope: None,
                    isotope_min_intensity: 1.0,
                    selection: None,
                    peak_info: None,
                    peak_font: 11.0,
                    header: ViewHeader {
                        source: "pointer test",
                        counting: false,
                        dead_time: None,
                        active: true,
                        maximized: true,
                    },
                },
            );
        });
        self.events.extend(reported);
    }

    /// Settles the layout, so positions mean what they look like.
    fn settle(&mut self) {
        self.frame(Vec::new());
        self.frame(Vec::new());
    }

    /// A position inside the plot, as fractions of the drawn area.
    ///
    /// The vertical fraction stays in the middle third, below the header bar and
    /// above the axis labels, and the horizontal one clears the left gutter.
    fn inside(&self, across: f32) -> Pos2 {
        Pos2::new(80.0 + across * (WIDTH - 100.0), HEIGHT * 0.6)
    }

    fn click(&mut self, at: Pos2) {
        self.frame(vec![
            Event::PointerMoved(at),
            Event::PointerButton {
                pos: at,
                button: PointerButton::Primary,
                pressed: true,
                modifiers: Modifiers::default(),
            },
            Event::PointerButton {
                pos: at,
                button: PointerButton::Primary,
                pressed: false,
                modifiers: Modifiers::default(),
            },
        ]);
    }

    /// Presses at one point, moves, and releases at another, over three frames -
    /// which is how a drag really arrives.
    fn drag(&mut self, from: Pos2, to: Pos2) {
        self.frame(vec![
            Event::PointerMoved(from),
            Event::PointerButton {
                pos: from,
                button: PointerButton::Primary,
                pressed: true,
                modifiers: Modifiers::default(),
            },
        ]);
        self.frame_keeping_events(vec![Event::PointerMoved(to)]);
        self.frame_keeping_events(vec![
            Event::PointerMoved(to),
            Event::PointerButton {
                pos: to,
                button: PointerButton::Primary,
                pressed: false,
                modifiers: Modifiers::default(),
            },
        ]);
    }

    /// The last selection reported during a gesture.
    fn final_selection(&self) -> Option<(usize, usize)> {
        self.events.iter().rev().find_map(|event| match event {
            ViewEvent::Selected(low, high) => Some((*low, *high)),
            _ => None,
        })
    }

    fn marker(&self) -> Option<usize> {
        self.events.iter().find_map(|event| match event {
            ViewEvent::Marker(channel) => Some(*channel),
            _ => None,
        })
    }

    fn selection(&self) -> Option<(usize, usize)> {
        self.events.iter().find_map(|event| match event {
            ViewEvent::Selected(low, high) => Some((*low, *high)),
            _ => None,
        })
    }

    fn regions_marked(&self) -> Option<(usize, usize)> {
        self.events.iter().find_map(|event| match event {
            ViewEvent::MarkRoi(low, high) => Some((*low, *high)),
            _ => None,
        })
    }
}

#[test]
fn clicking_in_the_plot_moves_the_marker_there() {
    let mut display = Display::new(1024);
    display.settle();

    display.click(display.inside(0.5));

    let channel = display
        .marker()
        .expect("a click in the plot should move the marker");
    assert!(
        (400..=624).contains(&channel),
        "a click in the middle landed on channel {channel}"
    );
}

#[test]
fn clicking_further_right_gives_a_higher_channel() {
    let mut display = Display::new(1024);
    display.settle();

    display.click(display.inside(0.2));
    let left = display.marker().expect("a marker");
    display.click(display.inside(0.8));
    let right = display.marker().expect("a marker");

    assert!(
        left < right,
        "channel {left} at 20% should be below channel {right} at 80%"
    );
}

#[test]
fn a_drag_selects_everything_between_its_ends() {
    let mut display = Display::new(1024);
    display.settle();

    display.drag(display.inside(0.25), display.inside(0.75));

    let (low, high) = display
        .selection()
        .expect("a drag should report a selection");
    assert!(
        low < high,
        "the selection should span channels: {low}-{high}"
    );
    // A drag across half the plot covers roughly half the spectrum.
    let span = high - low;
    assert!(
        (300..=700).contains(&span),
        "a drag over half the plot selected {span} of 1024 channels"
    );
}

#[test]
fn the_finished_selection_is_reported_when_the_button_comes_up() {
    // egui forgets where the button went down on the very frame it comes up, so
    // the display has to remember it. Without that, the last frame of a drag - the
    // one that ends it - reports nothing, and marking a region loses the end of
    // the drag.
    let mut display = Display::new(1024);
    display.settle();
    let (from, to) = (display.inside(0.2), display.inside(0.7));

    display.frame(vec![
        Event::PointerMoved(from),
        Event::PointerButton {
            pos: from,
            button: PointerButton::Primary,
            pressed: true,
            modifiers: Modifiers::default(),
        },
    ]);
    display.frame(vec![Event::PointerMoved(to)]);
    let while_dragging = display
        .final_selection()
        .expect("a selection while dragging");

    // Only the events of the frame that releases the button.
    display.frame(vec![
        Event::PointerMoved(to),
        Event::PointerButton {
            pos: to,
            button: PointerButton::Primary,
            pressed: false,
            modifiers: Modifiers::default(),
        },
    ]);
    let on_release = display
        .final_selection()
        .expect("the release frame should report the finished selection");
    assert_eq!(
        while_dragging, on_release,
        "the selection should not change when the button comes up"
    );
}

#[test]
fn a_drag_leftwards_selects_the_same_range_as_a_drag_rightwards() {
    let mut display = Display::new(1024);
    display.settle();

    display.drag(display.inside(0.3), display.inside(0.7));
    let forwards = display.selection().expect("a selection");
    display.drag(display.inside(0.7), display.inside(0.3));
    let backwards = display.selection().expect("a selection");

    let close = |a: usize, b: usize| a.abs_diff(b) <= 2;
    assert!(
        close(forwards.0, backwards.0) && close(forwards.1, backwards.1),
        "dragging back should select the same range: {forwards:?} vs {backwards:?}"
    );
}

#[test]
fn a_drag_in_marking_mode_marks_a_region() {
    let mut display = Display::new(1024);
    display.mark_mode = MarkMode::Mark;
    display.settle();

    display.drag(display.inside(0.4), display.inside(0.6));

    let (low, high) = display
        .regions_marked()
        .expect("dragging in marking mode should mark a region");
    assert!(low < high);
}

#[test]
fn the_wheel_zooms_about_the_channel_under_the_pointer() {
    let mut display = Display::new(1024);
    display.settle();
    // Zoom in first, so there is somewhere to scroll to.
    display.display.zoom_in();
    display.display.zoom_in();

    display.frame(vec![
        Event::PointerMoved(display.inside(0.5)),
        Event::MouseWheel {
            unit: egui::MouseWheelUnit::Line,
            delta: Vec2::new(0.0, 3.0),
            phase: egui::TouchPhase::Move,
            modifiers: Modifiers::default(),
        },
    ]);
    let zoomed_at = display.events.iter().find_map(|event| match event {
        ViewEvent::ZoomAt(channel, direction) => Some((*channel, *direction)),
        _ => None,
    });
    let (channel, direction) =
        zoomed_at.unwrap_or_else(|| panic!("the wheel did nothing: {:?}", display.events));
    assert_eq!(direction, 1, "wheel up should zoom in");
    let (start, end) = display.display.visible();
    let middle = (start + end) / 2;
    let quarter = (end - start) / 4;
    assert!(
        channel.abs_diff(middle) <= quarter,
        "the zoom should centre near the pointer: channel {channel}, view {start}-{end}"
    );

    // With control held, egui reports the wheel as a zoom gesture instead of a
    // scroll; the plot must still zoom.
    display.frame(vec![
        Event::PointerMoved(display.inside(0.5)),
        Event::MouseWheel {
            unit: egui::MouseWheelUnit::Line,
            delta: Vec2::new(0.0, 3.0),
            phase: egui::TouchPhase::Move,
            modifiers: Modifiers {
                ctrl: true,
                command: true,
                ..Default::default()
            },
        },
    ]);
    assert!(
        display
            .events
            .iter()
            .any(|event| matches!(event, ViewEvent::Zoom(_) | ViewEvent::ZoomAt(_, _))),
        "control and the wheel should zoom: {:?}",
        display.events
    );

    // Shift and the wheel scrolls along the spectrum instead. egui smooths the
    // wheel over several frames, so the previous gesture is left to die away
    // before the next begins - as it does between two real gestures.
    for _ in 0..30 {
        display.frame(Vec::new());
    }
    display.frame(vec![
        Event::PointerMoved(display.inside(0.5)),
        Event::MouseWheel {
            unit: egui::MouseWheelUnit::Line,
            delta: Vec2::new(0.0, 3.0),
            phase: egui::TouchPhase::Move,
            modifiers: Modifiers {
                shift: true,
                ..Default::default()
            },
        },
    ]);
    assert!(
        display
            .events
            .iter()
            .any(|event| matches!(event, ViewEvent::Scroll(_))),
        "shift and the wheel should scroll: {:?}",
        display.events
    );
}

#[test]
fn a_middle_button_drag_pans_the_plot() {
    let mut display = Display::new(4096);
    display.display.zoom_to(1000, 1400);
    display.settle();

    let from = display.inside(0.6);
    let to = display.inside(0.3);
    let press = |at: Pos2, pressed: bool| Event::PointerButton {
        pos: at,
        button: PointerButton::Middle,
        pressed,
        modifiers: Modifiers::default(),
    };
    display.frame(vec![Event::PointerMoved(from), press(from, true)]);
    display.frame_keeping_events(vec![Event::PointerMoved(to)]);
    display.frame_keeping_events(vec![Event::PointerMoved(to), press(to, false)]);

    let scrolled: i64 = display
        .events
        .iter()
        .filter_map(|event| match event {
            ViewEvent::Scroll(delta) => Some(*delta),
            _ => None,
        })
        .sum();
    assert!(
        scrolled > 50,
        "dragging left should scroll towards higher channels, got {scrolled}: {:?}",
        display.events
    );
    assert!(
        display.selection().is_none(),
        "a middle-button pan is not a selection"
    );
}

#[test]
fn dragging_in_the_overview_pans_the_zoomed_view() {
    let mut display = Display::new(1024);
    display.display.zoom_to(400, 600);
    display.settle();

    // Two points inside the overview inset (top right of the plot).
    let from = Pos2::new(900.0, 100.0);
    let to = Pos2::new(750.0, 100.0);
    display.drag(from, to);

    let zooms: Vec<(usize, usize)> = display
        .events
        .iter()
        .filter_map(|event| match event {
            ViewEvent::ZoomTo(low, high) => Some((*low, *high)),
            _ => None,
        })
        .collect();
    assert!(
        !zooms.is_empty(),
        "dragging the overview should pan: {:?}",
        display.events
    );
    let (low, high) = *zooms.last().expect("checked");
    assert!(
        high - low <= 260,
        "panning keeps the zoom width, got {low}-{high}"
    );
    // Dragging left moved the window towards lower channels than the start.
    let first = zooms.first().expect("checked");
    assert!(
        low <= first.0,
        "the window should follow the pointer leftwards: {zooms:?}"
    );
    // And no selection was made along the way.
    assert!(
        display.selection().is_none(),
        "an overview drag is a pan, not a selection"
    );
}

#[test]
fn a_click_outside_the_plot_does_not_move_the_marker() {
    let mut display = Display::new(1024);
    display.settle();
    // In the left gutter, where the axis labels are.
    display.click(Pos2::new(10.0, HEIGHT * 0.6));
    assert!(
        display.marker().is_none(),
        "clicking the axis should not move the marker"
    );
}

#[test]
fn a_click_maps_to_the_same_channel_the_view_model_reports() {
    // The pointer path and the keyboard path must agree about where a channel is.
    let mut display = Display::new(2048);
    display.settle();
    let at = display.inside(0.35);
    display.click(at);
    let from_pointer = display.marker().expect("a marker");
    // The plot starts after the left gutter; work the width out from two clicks.
    display.click(display.inside(0.0));
    let at_zero = display.marker().expect("a marker");
    display.click(display.inside(1.0));
    let at_one = display.marker().expect("a marker");
    let expected = at_zero + ((at_one - at_zero) as f32 * 0.35) as usize;
    assert!(
        from_pointer.abs_diff(expected) <= 8,
        "a click at 35% gave channel {from_pointer}, expected about {expected} \
         (0% is {at_zero}, 100% is {at_one})"
    );
    let _ = at;
}

#[test]
fn dragging_while_zoomed_selects_within_the_visible_range() {
    let mut display = Display::new(4096);
    display.display.marker = 2048;
    for _ in 0..6 {
        display.display.zoom_in();
    }
    let (low, high) = display.display.visible();
    display.settle();

    display.drag(display.inside(0.3), display.inside(0.6));

    let (from, to) = display.selection().expect("a selection");
    assert!(
        from >= low && to <= high,
        "the selection {from}-{to} left the visible range {low}-{high}"
    );
}

#[test]
fn a_tiny_display_still_handles_a_click() {
    // The plot can be squeezed until its gutters barely fit; clicking must not
    // panic or produce a channel outside the spectrum.
    let mut display = Display::new(512);
    let input = RawInput {
        screen_rect: Some(Rect::from_min_size(Pos2::ZERO, Vec2::new(160.0, 140.0))),
        events: vec![
            Event::PointerMoved(Pos2::new(120.0, 100.0)),
            Event::PointerButton {
                pos: Pos2::new(120.0, 100.0),
                button: PointerButton::Primary,
                pressed: true,
                modifiers: Modifiers::default(),
            },
            Event::PointerButton {
                pos: Pos2::new(120.0, 100.0),
                button: PointerButton::Primary,
                pressed: false,
                modifiers: Modifiers::default(),
            },
        ],
        ..Default::default()
    };
    let colors = Theme::DeepSpace.colors();
    let spectrum = &display.spectrum;
    let state = &mut display.display;
    let mut reported = Vec::new();
    let _ = display.ctx.run_ui(input, |ui| {
        reported = draw(
            ui,
            ViewInput {
                spectrum,
                compare: None,
                compare_offset: 0,
                display: state,
                colors: &colors,
                mark_mode: MarkMode::Off,
                library: None,
                isotope: None,
                isotope_min_intensity: 1.0,
                selection: None,
                peak_info: None,
                peak_font: 11.0,
                header: ViewHeader {
                    source: "tiny",
                    counting: false,
                    dead_time: None,
                    active: true,
                    maximized: false,
                },
            },
        );
    });
    for event in reported {
        if let ViewEvent::Marker(channel) = event {
            assert!(channel < 512, "channel {channel} is outside the spectrum");
        }
    }
}

/// Every vertical line the frame painted, as x positions with their extent.
fn vertical_lines(output: &egui::FullOutput) -> Vec<(f32, f32, f32)> {
    fn collect(shape: &egui::Shape, into: &mut Vec<(f32, f32, f32)>) {
        match shape {
            egui::Shape::LineSegment { points, .. } => {
                if (points[0].x - points[1].x).abs() < 0.5 {
                    let (top, bottom) = if points[0].y < points[1].y {
                        (points[0].y, points[1].y)
                    } else {
                        (points[1].y, points[0].y)
                    };
                    into.push((points[0].x, top, bottom));
                }
            }
            egui::Shape::Vec(inner) => {
                for shape in inner {
                    collect(shape, into);
                }
            }
            _ => {}
        }
    }
    let mut lines = Vec::new();
    for clipped in &output.shapes {
        collect(&clipped.shape, &mut lines);
    }
    lines
}

/// Draws one frame with the given selection and returns what was painted.
fn frame_with_selection(selection: Option<(usize, usize)>) -> egui::FullOutput {
    let ctx = egui::Context::default();
    let spectrum = spectrum(1024);
    let mut state = DisplayState::for_length(spectrum.len());
    let colors = Theme::DeepSpace.colors();
    let input = || RawInput {
        screen_rect: Some(Rect::from_min_size(Pos2::ZERO, Vec2::new(WIDTH, HEIGHT))),
        ..Default::default()
    };
    let render = |state: &mut DisplayState| {
        ctx.run_ui(input(), |ui| {
            draw(
                ui,
                ViewInput {
                    spectrum: &spectrum,
                    compare: None,
                    compare_offset: 0,
                    display: state,
                    colors: &colors,
                    mark_mode: MarkMode::Off,
                    library: None,
                    isotope: None,
                    isotope_min_intensity: 1.0,
                    selection,
                    peak_info: None,
                    peak_font: 11.0,
                    header: ViewHeader {
                        source: "selection",
                        counting: false,
                        dead_time: None,
                        active: true,
                        maximized: true,
                    },
                },
            );
        })
    };
    // Once to settle the layout, once for the frame under test.
    let _ = render(&mut state);
    render(&mut state)
}

/// How many tall vertical lines the frame painted at each x, to the pixel.
///
/// Counted rather than collected into a set: a selection edge can land on the
/// same pixel as a grid line, and then only the tally shows it was drawn.
/// "Tall" keeps tick marks out while keeping anything drawn across the plot.
fn tall_lines(output: &egui::FullOutput) -> std::collections::BTreeMap<i32, usize> {
    let mut tally = std::collections::BTreeMap::new();
    for (x, top, bottom) in vertical_lines(output) {
        if bottom - top > 100.0 {
            *tally.entry(x.round() as i32).or_insert(0) += 1;
        }
    }
    tally
}

#[test]
fn a_selection_is_marked_at_its_midpoint() {
    // MAESTRO draws the centre of the marked band, full height, so the eye can
    // put a peak's middle on a channel while dragging. Tallying against the
    // same frame with nothing selected isolates the lines the selection itself
    // adds, leaving the grid and the marker out of it.
    let plain = tall_lines(&frame_with_selection(None));
    let marked = tall_lines(&frame_with_selection(Some((200, 400))));
    let mut added = Vec::new();
    for (x, count) in &marked {
        let before = plain.get(x).copied().unwrap_or(0);
        for _ in before..*count {
            added.push(*x);
        }
    }
    assert_eq!(
        added.len(),
        3,
        "a selection should add its two edges and a midpoint, got {added:?}"
    );
    let midpoint = (added[0] + added[2]) / 2;
    assert!(
        (added[1] - midpoint).abs() <= 1,
        "the third line should sit halfway between the edges: {added:?}"
    );
}
