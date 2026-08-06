//! Display state: what part of the spectrum is on screen, where the marker is
//! and how the vertical axis is scaled.
//!
//! This is the arithmetic behind the Display menu and the two spectrum views, in
//! one place and without any drawing, so it can be tested.

use ortseam_core::Spectrum;
use serde::{Deserialize, Serialize};

/// How the vertical axis is scaled.
#[derive(Clone, Copy, Debug, PartialEq, Serialize, Deserialize)]
pub enum VerticalScale {
    /// Logarithmic, as MAESTRO's `LOG` button selects.
    Logarithmic,
    /// Linear with a fixed full scale.
    Linear(u64),
    /// Linear, rescaled so the tallest visible channel fills the window.
    Automatic,
}

impl Default for VerticalScale {
    /// Logarithmic, because a gamma spectrum spans several decades and a linear
    /// axis scaled to the tallest peak hides everything else.
    fn default() -> Self {
        Self::Logarithmic
    }
}

/// How the spectrum is drawn (Display/Preferences).
#[derive(Clone, Copy, Debug, Default, PartialEq, Eq, Serialize, Deserialize)]
pub enum FillMode {
    /// One point per channel.
    Points,
    /// Fill the regions of interest only.
    FillRoi,
    /// Fill everything under the spectrum, which is how a spectrum is usually
    /// read and what MAESTRO's own figures show.
    #[default]
    FillAll,
}

/// The smallest window the expanded view can zoom to.
pub const MIN_VIEW_WIDTH: usize = 8;

/// How much one zoom step changes the width.
///
/// Twenty percent, rather than the halving and doubling of MAESTRO's keypad, so
/// the wheel and the buttons land where you meant.
pub const ZOOM_STEP: f64 = 1.2;

/// What the expanded view shows and where the marker sits.
#[derive(Clone, Copy, Debug, PartialEq, Serialize, Deserialize)]
pub struct DisplayState {
    /// Marker channel.
    pub marker: usize,
    /// First channel of the expanded view.
    pub view_start: usize,
    /// Channels across the expanded view.
    pub view_width: usize,
    /// Vertical scaling.
    pub vertical: VerticalScale,
    /// Counts at the bottom of the window (baseline zoom).
    pub baseline: u64,
    /// How the data is drawn.
    pub fill: FillMode,
    /// Whether the energy axis is used when a calibration exists.
    pub energy_axis: bool,
    /// Whether library lines are drawn (Display/Isotope Markers).
    pub isotope_markers: bool,
    /// Length of the spectrum this state belongs to.
    length: usize,
    /// Where the pointer went down, while a drag is in progress.
    ///
    /// egui forgets the press position on the frame the button comes up, which is
    /// exactly the frame the finished selection is wanted on, so the display
    /// remembers it here instead.
    #[serde(skip)]
    drag_origin: Option<f32>,
    /// Whether the drag in progress began inside the overview inset - those
    /// drags pan the expanded view instead of selecting.
    #[serde(skip)]
    drag_in_inset: bool,
}

impl Default for DisplayState {
    fn default() -> Self {
        Self {
            marker: 0,
            view_start: 0,
            view_width: MIN_VIEW_WIDTH,
            vertical: VerticalScale::default(),
            baseline: 0,
            fill: FillMode::default(),
            energy_axis: true,
            isotope_markers: false,
            length: 0,
            drag_origin: None,
            drag_in_inset: false,
        }
    }
}

impl DisplayState {
    /// State showing the whole of a spectrum of `length` channels.
    pub fn for_length(length: usize) -> Self {
        Self {
            length,
            view_width: length.max(MIN_VIEW_WIDTH),
            ..Self::default()
        }
    }

    /// Adjusts to a new spectrum length, keeping the view inside it.
    pub fn set_length(&mut self, length: usize) {
        if length == 0 {
            *self = Self::default();
            return;
        }
        let grew_from_nothing = self.length == 0;
        // A view that covered everything keeps covering everything: raising
        // the conversion gain 1024 -> 8192 must not leave the window showing
        // the first eighth of the new spectrum.
        let covered_everything = self.view_start == 0 && self.view_width >= self.length;
        self.length = length;
        if grew_from_nothing || covered_everything || self.view_width > length {
            self.view_width = length;
            self.view_start = 0;
        }
        self.clamp();
    }

    /// First and last visible channel, inclusive.
    pub fn visible(&self) -> (usize, usize) {
        if self.length == 0 {
            return (0, 0);
        }
        let start = self.view_start.min(self.length - 1);
        let end = (start + self.view_width - 1).min(self.length - 1);
        (start, end)
    }

    /// Shows the whole spectrum (Display/Full View).
    pub fn full_view(&mut self) {
        self.view_start = 0;
        self.view_width = self.length.max(MIN_VIEW_WIDTH);
        self.baseline = 0;
    }

    /// Zooms in one step about the marker (Display/Zoom In).
    pub fn zoom_in(&mut self) {
        self.zoom_by(1.0 / ZOOM_STEP);
    }

    /// Zooms out one step about the marker (Display/Zoom Out).
    pub fn zoom_out(&mut self) {
        self.zoom_by(ZOOM_STEP);
    }

    /// Scales the window by a factor about the marker.
    ///
    /// The width always changes by at least one channel, so a step is never lost
    /// to rounding on a narrow window.
    pub fn zoom_by(&mut self, factor: f64) {
        if self.length == 0 || factor <= 0.0 {
            return;
        }
        let scaled = (self.view_width as f64 * factor).round() as usize;
        let target = if factor < 1.0 {
            scaled.min(self.view_width.saturating_sub(1))
        } else {
            scaled.max(self.view_width + 1)
        };
        self.set_width_about_marker(target);
    }

    /// Scales the window by a factor, keeping `channel` at the same place on
    /// screen.
    ///
    /// This is what the wheel and a pinch use: the spectrum grows and shrinks
    /// around the point under the pointer, which is where the eye already is.
    /// The buttons and the keyboard still zoom about the marker.
    pub fn zoom_about(&mut self, channel: usize, factor: f64) {
        if self.length == 0 || factor <= 0.0 {
            return;
        }
        let channel = channel.min(self.length - 1);
        let (start, end) = self.visible();
        let fraction = if end > start {
            (channel.clamp(start, end) - start) as f64 / (end - start) as f64
        } else {
            0.5
        };
        let scaled = (self.view_width as f64 * factor).round() as usize;
        let target = if factor < 1.0 {
            scaled.min(self.view_width.saturating_sub(1))
        } else {
            scaled.max(self.view_width + 1)
        };
        let width = target.clamp(MIN_VIEW_WIDTH, self.length.max(MIN_VIEW_WIDTH));
        self.view_width = width;
        let offset = (fraction * (width.saturating_sub(1)) as f64).round() as usize;
        self.view_start = channel.saturating_sub(offset);
        self.clamp();
    }

    /// Zooms to a channel range, as the rubber rectangle does.
    pub fn zoom_to(&mut self, low: usize, high: usize) {
        let (low, high) = (low.min(high), low.max(high));
        let width = (high - low + 1).max(MIN_VIEW_WIDTH);
        self.view_width = width.min(self.length.max(MIN_VIEW_WIDTH));
        self.view_start = low;
        self.clamp();
    }

    /// Puts the marker in the middle of the window without moving it in the
    /// spectrum (Display/Center).
    pub fn center_on_marker(&mut self) {
        self.view_start = self.marker.saturating_sub(self.view_width / 2);
        self.clamp();
    }

    /// Moves the marker, scrolling the view when it would leave the window.
    pub fn set_marker(&mut self, channel: usize) {
        if self.length == 0 {
            self.marker = 0;
            return;
        }
        self.marker = channel.min(self.length - 1);
        let (start, end) = self.visible();
        if self.marker < start {
            self.view_start = self.marker;
        } else if self.marker > end {
            self.view_start = self.marker.saturating_sub(self.view_width - 1);
        }
        self.clamp();
    }

    /// Moves the marker by a signed number of channels.
    pub fn move_marker(&mut self, delta: i64) {
        let target = self.marker as i64 + delta;
        self.set_marker(target.max(0) as usize);
    }

    /// Scrolls the view without moving the marker in the spectrum.
    pub fn scroll(&mut self, delta: i64) {
        let target = self.view_start as i64 + delta;
        self.view_start = target.max(0) as usize;
        self.clamp();
    }

    /// Full scale the vertical axis should use for the visible channels.
    pub fn full_scale(&self, spectrum: &Spectrum) -> u64 {
        match self.vertical {
            VerticalScale::Linear(max) => max.max(1),
            VerticalScale::Logarithmic | VerticalScale::Automatic => {
                // An empty spectrum has no channels to slice - even 0..=0
                // would be out of range.
                if spectrum.is_empty() {
                    return self.baseline + 1;
                }
                let (start, end) = self.visible();
                let peak = spectrum.channels
                    [start.min(spectrum.len() - 1)..=end.min(spectrum.len() - 1)]
                    .iter()
                    .copied()
                    .max()
                    .unwrap_or(1);
                let peak = peak.max(self.baseline + 1);
                match self.vertical {
                    // A logarithmic axis is read off its decade lines, so the
                    // top belongs on one of them rather than on whatever the
                    // tallest channel happens to be. Rounding up also stops the
                    // peak being clipped flat against the top of the plot.
                    VerticalScale::Logarithmic => logarithmic_ceiling(peak),
                    _ => peak,
                }
            }
        }
    }

    /// Height fraction, 0.0 at the baseline and 1.0 at full scale.
    pub fn height_fraction(&self, counts: u64, full_scale: u64) -> f32 {
        let full_scale = full_scale.max(1);
        match self.vertical {
            VerticalScale::Logarithmic => {
                if counts <= self.baseline {
                    return 0.0;
                }
                let value = (counts - self.baseline) as f64;
                let top = (full_scale.saturating_sub(self.baseline)).max(1) as f64;
                ((value.ln_1p() / top.ln_1p()) as f32).clamp(0.0, 1.0)
            }
            _ => {
                if counts <= self.baseline {
                    return 0.0;
                }
                let value = (counts - self.baseline) as f64;
                let top = (full_scale.saturating_sub(self.baseline)).max(1) as f64;
                ((value / top) as f32).clamp(0.0, 1.0)
            }
        }
    }

    /// Channel under a horizontal fraction of the expanded view.
    pub fn channel_at(&self, fraction: f32) -> usize {
        let (start, end) = self.visible();
        let span = (end - start) as f32;
        let channel = start as f32 + fraction.clamp(0.0, 1.0) * span;
        (channel.round().max(0.0) as usize).min(self.length.saturating_sub(1))
    }

    /// Channel under a horizontal pixel position inside the plot.
    pub fn channel_at_x(&self, x: f32, left: f32, width: f32) -> usize {
        let fraction = ((x - left) / width.max(1.0)).clamp(0.0, 1.0);
        self.channel_at(fraction)
    }

    /// The channel range a drag covers, from where the button went down to where
    /// the pointer is now.
    ///
    /// Dragging right to left gives the same range as left to right.
    pub fn drag_range(
        &self,
        origin_x: f32,
        current_x: f32,
        left: f32,
        width: f32,
    ) -> (usize, usize) {
        let a = self.channel_at_x(origin_x, left, width);
        let b = self.channel_at_x(current_x, left, width);
        (a.min(b), a.max(b))
    }

    /// Remembers where a drag began, so the range is still known on the frame the
    /// button comes up - and whether it began in the overview inset, whose
    /// drags pan the view rather than selecting.
    pub fn begin_drag(&mut self, origin_x: f32, in_inset: bool) {
        self.drag_origin = Some(origin_x);
        self.drag_in_inset = in_inset;
    }

    /// Where the drag in progress began, if there is one.
    pub fn drag_origin(&self) -> Option<f32> {
        self.drag_origin
    }

    /// Whether the drag in progress began inside the overview inset.
    pub fn drag_in_inset(&self) -> bool {
        self.drag_origin.is_some() && self.drag_in_inset
    }

    /// Forgets the drag, once it has been dealt with.
    pub fn end_drag(&mut self) {
        self.drag_origin = None;
        self.drag_in_inset = false;
    }

    /// Horizontal fraction a channel sits at, or `None` when it is off screen.
    pub fn fraction_of(&self, channel: usize) -> Option<f32> {
        let (start, end) = self.visible();
        if channel < start || channel > end {
            return None;
        }
        let span = (end - start) as f32;
        Some(if span <= 0.0 {
            0.0
        } else {
            (channel - start) as f32 / span
        })
    }

    /// Switches between logarithmic and linear, remembering the linear scale.
    pub fn toggle_log(&mut self, spectrum: &Spectrum) {
        self.vertical = match self.vertical {
            VerticalScale::Logarithmic => VerticalScale::Linear(self.full_scale(spectrum)),
            _ => VerticalScale::Logarithmic,
        };
    }

    /// True when the whole spectrum is visible.
    pub fn is_full_view(&self) -> bool {
        let (start, end) = self.visible();
        start == 0 && end + 1 >= self.length
    }

    fn set_width_about_marker(&mut self, width: usize) {
        let width = width.clamp(MIN_VIEW_WIDTH, self.length.max(MIN_VIEW_WIDTH));
        let marker_offset = self.marker.saturating_sub(self.view_start);
        let fraction = if self.view_width > 1 {
            marker_offset as f64 / (self.view_width - 1) as f64
        } else {
            0.5
        };
        self.view_width = width;
        let offset = (fraction * (width.saturating_sub(1)) as f64).round() as usize;
        self.view_start = self.marker.saturating_sub(offset);
        self.clamp();
    }

    fn clamp(&mut self) {
        if self.length == 0 {
            self.view_start = 0;
            self.view_width = MIN_VIEW_WIDTH;
            self.marker = 0;
            return;
        }
        self.view_width = self
            .view_width
            .clamp(MIN_VIEW_WIDTH.min(self.length), self.length);
        let max_start = self.length.saturating_sub(self.view_width);
        self.view_start = self.view_start.min(max_start);
        self.marker = self.marker.min(self.length - 1);
    }
}

/// The next 1, 2 or 5 times a power of ten strictly above `peak`.
///
/// This is the top of a logarithmic axis: a round number the eye can read off
/// the gridlines, and far enough above the tallest channel that the peak has
/// room to breathe instead of being pressed flat against the frame. Strictly
/// above, because a peak landing exactly on the top line is the thing being
/// fixed.
fn logarithmic_ceiling(peak: u64) -> u64 {
    let mut step = 1u64;
    loop {
        for multiple in [1u64, 2, 5] {
            let Some(candidate) = step.checked_mul(multiple) else {
                return u64::MAX;
            };
            if candidate > peak {
                return candidate;
            }
        }
        let Some(next) = step.checked_mul(10) else {
            return u64::MAX;
        };
        step = next;
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn a_logarithmic_axis_tops_out_above_the_tallest_channel() {
        // The complaint this fixes: the peak drawn flush against the top of the
        // plot, with no room above it.
        for peak in [1u64, 9, 10, 99, 100, 1_234, 15_675, 3_558_911] {
            let top = logarithmic_ceiling(peak);
            assert!(top > peak, "{peak} needs headroom, got a top of {top}");
        }
    }

    #[test]
    fn a_logarithmic_top_is_a_round_number() {
        // One, two or five times a power of ten, so the top lands on a
        // gridline the axis already labels.
        assert_eq!(logarithmic_ceiling(1), 2);
        assert_eq!(logarithmic_ceiling(9), 10);
        assert_eq!(logarithmic_ceiling(10), 20);
        assert_eq!(logarithmic_ceiling(99), 100);
        assert_eq!(logarithmic_ceiling(100), 200);
        assert_eq!(logarithmic_ceiling(1_234), 2_000);
        assert_eq!(logarithmic_ceiling(15_675), 20_000);
        assert_eq!(logarithmic_ceiling(3_558_911), 5_000_000);
    }

    #[test]
    fn only_the_logarithmic_axis_gains_headroom() {
        // Automatic fills the window with the tallest channel on purpose;
        // giving it headroom would be changing something nobody asked about.
        let mut spectrum = Spectrum::new(64);
        spectrum.channels[10] = 1_234;
        let mut display = DisplayState::for_length(64);

        display.vertical = VerticalScale::Automatic;
        assert_eq!(display.full_scale(&spectrum), 1_234);

        display.vertical = VerticalScale::Logarithmic;
        assert_eq!(display.full_scale(&spectrum), 2_000);
    }

    #[test]
    fn the_tallest_channel_no_longer_reaches_the_top_of_a_logarithmic_plot() {
        let mut spectrum = Spectrum::new(64);
        spectrum.channels[10] = 15_675;
        let mut display = DisplayState::for_length(64);
        display.vertical = VerticalScale::Logarithmic;
        let full_scale = display.full_scale(&spectrum);
        let fraction = display.height_fraction(15_675, full_scale);
        assert!(
            fraction < 0.99,
            "the peak still touches the top: {fraction}"
        );
        assert!(
            fraction > 0.80,
            "the peak should still dominate the plot, not shrink: {fraction}"
        );
    }

    fn state() -> DisplayState {
        DisplayState::for_length(1024)
    }

    fn spectrum() -> Spectrum {
        Spectrum::from_counts((0..1024u64).map(|c| c % 100).collect())
    }

    #[test]
    fn a_new_state_shows_everything() {
        let state = state();
        assert_eq!(state.visible(), (0, 1023));
        assert!(state.is_full_view());
        assert_eq!(state.marker, 0);
    }

    #[test]
    fn zoom_in_narrows_by_a_fifth_and_keeps_the_marker() {
        let mut state = state();
        state.set_marker(500);
        state.zoom_in();
        // 1024 / 1.2 = 853
        assert_eq!(state.view_width, 853);
        let (start, end) = state.visible();
        assert!((start..=end).contains(&500), "marker left the window");
        state.zoom_in();
        assert_eq!(state.view_width, 711);
        let (start, end) = state.visible();
        assert!((start..=end).contains(&500));
    }

    #[test]
    fn every_zoom_step_changes_the_width() {
        let mut state = state();
        state.zoom_to(500, 509);
        let mut widths = Vec::new();
        for _ in 0..6 {
            state.zoom_out();
            widths.push(state.view_width);
        }
        assert!(
            widths.windows(2).all(|pair| pair[1] > pair[0]),
            "widths {widths:?} should grow every step"
        );
        for _ in 0..6 {
            let before = state.view_width;
            state.zoom_in();
            assert!(
                state.view_width < before || state.view_width == MIN_VIEW_WIDTH,
                "zooming in must narrow the window"
            );
        }
    }

    #[test]
    fn zoom_out_stops_at_the_full_spectrum() {
        let mut state = state();
        state.set_marker(500);
        state.zoom_to(480, 520);
        assert_eq!(state.view_width, 41);
        for _ in 0..40 {
            state.zoom_out();
        }
        assert!(state.is_full_view());
        assert_eq!(state.view_width, 1024);
    }

    #[test]
    fn zoom_in_stops_at_the_minimum_width() {
        let mut state = state();
        for _ in 0..80 {
            state.zoom_in();
        }
        assert_eq!(state.view_width, MIN_VIEW_WIDTH);
    }

    #[test]
    fn zooming_by_a_factor_is_available_for_pinch_and_wheel() {
        let mut state = state();
        state.zoom_by(0.5);
        assert_eq!(state.view_width, 512);
        state.zoom_by(2.0);
        assert_eq!(state.view_width, 1024);
        state.zoom_by(0.0);
        assert_eq!(state.view_width, 1024, "a zero factor is ignored");
    }

    #[test]
    fn zooming_about_a_channel_keeps_it_at_the_same_place_on_screen() {
        let mut state = state();
        state.zoom_to(200, 599); // 400 channels; channel 300 sits a quarter in
        state.zoom_about(300, 0.5);
        let (start, end) = state.visible();
        assert!(start < 300 && 300 < end, "the channel stayed visible");
        let fraction = (300 - start) as f64 / (end - start) as f64;
        assert!(
            (fraction - 0.25).abs() < 0.02,
            "channel 300 moved from a quarter in to {fraction:.2}"
        );

        // Zooming back out keeps it in place too.
        state.zoom_about(300, 2.0);
        let (start, end) = state.visible();
        let fraction = (300 - start) as f64 / (end - start) as f64;
        assert!(
            (fraction - 0.25).abs() < 0.02,
            "channel 300 drifted to {fraction:.2} on the way out"
        );
    }

    #[test]
    fn zooming_about_a_channel_never_leaves_the_spectrum() {
        let mut state = state();
        state.zoom_about(0, 0.5);
        assert_eq!(state.visible().0, 0);
        state.full_view();
        state.zoom_about(1023, 0.5);
        assert_eq!(state.visible().1, 1023);
        // A channel beyond the end clamps rather than panicking.
        state.full_view();
        state.zoom_about(5000, 0.5);
        assert!(state.visible().1 <= 1023);
    }

    #[test]
    fn the_marker_scrolls_the_window_when_it_would_leave() {
        let mut state = state();
        state.zoom_to(100, 199);
        state.set_marker(250);
        let (start, end) = state.visible();
        assert!((start..=end).contains(&250));
        assert_eq!(end, 250, "the window follows the marker to the right");
        state.set_marker(50);
        assert_eq!(state.visible().0, 50, "and to the left");
    }

    #[test]
    fn the_marker_cannot_leave_the_spectrum() {
        let mut state = state();
        state.set_marker(5000);
        assert_eq!(state.marker, 1023);
        state.move_marker(-10_000);
        assert_eq!(state.marker, 0);
    }

    #[test]
    fn centring_moves_the_window_not_the_marker() {
        let mut state = state();
        state.zoom_to(0, 99);
        state.set_marker(60);
        state.center_on_marker();
        assert_eq!(state.marker, 60);
        let (start, end) = state.visible();
        assert!((start..=end).contains(&60));
        assert!((start as i64 - 10).abs() <= 1, "start {start}");
    }

    #[test]
    fn full_view_resets_the_window_and_baseline() {
        let mut state = state();
        state.zoom_to(10, 20);
        state.baseline = 500;
        state.full_view();
        assert!(state.is_full_view());
        assert_eq!(state.baseline, 0);
    }

    #[test]
    fn automatic_scaling_follows_the_tallest_visible_channel() {
        let mut spectrum = spectrum();
        spectrum.channels[500] = 5_000;
        let mut state = state();
        state.vertical = VerticalScale::Automatic;
        assert_eq!(state.full_scale(&spectrum), 5_000);
        state.zoom_to(0, 99);
        assert_eq!(state.full_scale(&spectrum), 99, "the spike is off screen");
    }

    #[test]
    fn a_fixed_linear_scale_is_used_as_given() {
        let state = DisplayState {
            vertical: VerticalScale::Linear(2_000),
            ..state()
        };
        assert_eq!(state.full_scale(&spectrum()), 2_000);
        assert!((state.height_fraction(1_000, 2_000) - 0.5).abs() < 1e-6);
        assert_eq!(
            state.height_fraction(4_000, 2_000),
            1.0,
            "clamped at the top"
        );
    }

    #[test]
    fn the_logarithmic_scale_compresses_the_top() {
        let state = DisplayState {
            vertical: VerticalScale::Logarithmic,
            ..state()
        };
        let low = state.height_fraction(10, 10_000);
        let high = state.height_fraction(1_000, 10_000);
        assert!(low > 0.0 && low < high && high < 1.0);
        // A decade near the bottom takes more of the window than one near the top.
        let first_decade = state.height_fraction(10, 10_000);
        let last_decade =
            state.height_fraction(10_000, 10_000) - state.height_fraction(1_000, 10_000);
        assert!(first_decade > last_decade);
        assert_eq!(state.height_fraction(0, 10_000), 0.0);
    }

    #[test]
    fn the_baseline_offsets_the_bottom_of_the_window() {
        let mut state = state();
        state.vertical = VerticalScale::Linear(1_000);
        state.baseline = 500;
        assert_eq!(state.height_fraction(500, 1_000), 0.0);
        assert!((state.height_fraction(750, 1_000) - 0.5).abs() < 1e-6);
    }

    #[test]
    fn positions_and_channels_convert_both_ways() {
        let mut state = state();
        state.zoom_to(100, 199);
        assert_eq!(state.channel_at(0.0), 100);
        assert_eq!(state.channel_at(1.0), 199);
        assert_eq!(state.channel_at(0.5), 150);
        assert!((state.fraction_of(150).unwrap() - 0.505).abs() < 0.01);
        assert!(state.fraction_of(50).is_none());
    }

    #[test]
    fn a_drag_covers_everything_between_its_ends() {
        // A 100 channel window across 500 pixels starting at x = 20.
        let mut state = state();
        state.zoom_to(200, 299);
        let (left, width) = (20.0, 500.0);

        // Left to right and right to left give the same range.
        let forward = state.drag_range(120.0, 370.0, left, width);
        let backward = state.drag_range(370.0, 120.0, left, width);
        assert_eq!(forward, backward);
        // A fifth of the way in to seven tenths: channels 220 to 269.
        assert_eq!(forward, (220, 269));

        // The full width is the whole visible range.
        assert_eq!(
            state.drag_range(left, left + width, left, width),
            (200, 299)
        );
        // A drag that does not move is a single channel.
        let (low, high) = state.drag_range(200.0, 200.0, left, width);
        assert_eq!(low, high);
        // Off the ends is clamped, not wrapped.
        assert_eq!(state.drag_range(-500.0, 5000.0, left, width), (200, 299));
    }

    #[test]
    fn a_pixel_maps_to_the_channel_under_it() {
        let mut state = state();
        state.zoom_to(0, 99);
        assert_eq!(state.channel_at_x(0.0, 0.0, 100.0), 0);
        assert_eq!(state.channel_at_x(100.0, 0.0, 100.0), 99);
        assert_eq!(state.channel_at_x(50.0, 0.0, 100.0), 50);
        // The inverse agrees with fraction_of to within a channel.
        for channel in [0usize, 25, 50, 99] {
            let fraction = state.fraction_of(channel).unwrap();
            let back = state.channel_at_x(fraction * 100.0, 0.0, 100.0);
            assert!(
                back.abs_diff(channel) <= 1,
                "channel {channel} came back as {back}"
            );
        }
    }

    #[test]
    fn toggling_log_remembers_the_linear_scale() {
        let spectrum = spectrum();
        let mut state = state();
        state.vertical = VerticalScale::Automatic;
        state.toggle_log(&spectrum);
        assert_eq!(state.vertical, VerticalScale::Logarithmic);
        state.toggle_log(&spectrum);
        assert!(matches!(state.vertical, VerticalScale::Linear(_)));
    }

    #[test]
    fn changing_length_keeps_the_view_inside_the_spectrum() {
        let mut state = state();
        state.zoom_to(900, 1000);
        state.set_length(512);
        let (start, end) = state.visible();
        assert!(end < 512, "visible {start}..{end}");
        assert!(state.marker < 512);
    }

    #[test]
    fn an_empty_spectrum_is_handled() {
        let mut state = DisplayState::for_length(0);
        state.set_marker(10);
        assert_eq!(state.marker, 0);
        assert_eq!(state.visible(), (0, 0));
        state.zoom_in();
        state.zoom_out();
        state.full_view();
    }
}
