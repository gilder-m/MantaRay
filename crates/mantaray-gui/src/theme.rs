//! Colour and visual style.
//!
//! # How the palette is chosen
//!
//! A spectrum display has to carry several meanings at once - data, marked
//! regions, a comparison trace, library lines, the cursor - on top of a very dark
//! plot. The palette is built from three rules:
//!
//! 1. **The data wins.** The spectrum has the highest contrast against the plot
//!    background of anything drawn (better than 7:1), so the eye lands on it
//!    first. Everything else is deliberately quieter.
//! 2. **Hues are spread, not crowded.** The five data roles sit far apart on the
//!    wheel - aqua for the spectrum, amber for regions, violet for the comparison,
//!    pink for library lines, gold for the cursor. Aqua and amber are near
//!    opposites, which is why marked peaks read instantly against the trace.
//! 3. **Never hue alone.** Each role also differs in lightness, and in how it is
//!    drawn - regions are filled bands, the comparison is dotted, library lines
//!    are ticks - so the display still works with any colour-vision deficiency,
//!    and in print.
//!
//! Saturated red is kept out of the data palette entirely and reserved for
//! alarms, so a red thing on screen always means "look at me".
//!
//! [`SpectrumColors::contrast_report`] and the tests below check these
//! properties, for every theme, rather than trusting the eye.

use serde::{Deserialize, Serialize};

/// A colour as red, green and blue.
#[derive(Clone, Copy, Debug, PartialEq, Eq, Serialize, Deserialize)]
pub struct Rgb(pub u8, pub u8, pub u8);

impl Rgb {
    /// As an egui colour.
    pub fn to_color(self) -> egui::Color32 {
        egui::Color32::from_rgb(self.0, self.1, self.2)
    }

    /// As an egui colour with an alpha multiplier.
    pub fn with_alpha(self, alpha: f32) -> egui::Color32 {
        self.to_color().gamma_multiply(alpha.clamp(0.0, 1.0))
    }

    /// From an egui colour.
    pub fn from_color(color: egui::Color32) -> Self {
        Self(color.r(), color.g(), color.b())
    }

    /// Relative luminance, as WCAG defines it (0.0 black, 1.0 white).
    pub fn luminance(self) -> f64 {
        fn channel(value: u8) -> f64 {
            let value = value as f64 / 255.0;
            if value <= 0.03928 {
                value / 12.92
            } else {
                ((value + 0.055) / 1.055).powf(2.4)
            }
        }
        0.2126 * channel(self.0) + 0.7152 * channel(self.1) + 0.0722 * channel(self.2)
    }

    /// WCAG contrast ratio against another colour, from 1.0 to 21.0.
    pub fn contrast(self, other: Rgb) -> f64 {
        let (a, b) = (self.luminance(), other.luminance());
        let (light, dark) = if a > b { (a, b) } else { (b, a) };
        (light + 0.05) / (dark + 0.05)
    }

    /// Hue in degrees, 0 to 360.
    pub fn hue(self) -> f64 {
        let (r, g, b) = (
            self.0 as f64 / 255.0,
            self.1 as f64 / 255.0,
            self.2 as f64 / 255.0,
        );
        let max = r.max(g).max(b);
        let min = r.min(g).min(b);
        let span = max - min;
        if span <= f64::EPSILON {
            return 0.0;
        }
        let hue = if max == r {
            60.0 * (((g - b) / span) % 6.0)
        } else if max == g {
            60.0 * ((b - r) / span + 2.0)
        } else {
            60.0 * ((r - g) / span + 4.0)
        };
        (hue + 360.0) % 360.0
    }

    /// Smallest angle between two hues, in degrees.
    pub fn hue_distance(self, other: Rgb) -> f64 {
        let difference = (self.hue() - other.hue()).abs() % 360.0;
        difference.min(360.0 - difference)
    }

    /// How colourful this is, from 0.0 (grey) to 1.0.
    ///
    /// Hue means nothing for a near-grey, so comparisons skip them.
    pub fn chroma(self) -> f64 {
        let (r, g, b) = (
            self.0 as f64 / 255.0,
            self.1 as f64 / 255.0,
            self.2 as f64 / 255.0,
        );
        r.max(g).max(b) - r.min(g).min(b)
    }

    /// This colour blended `amount` of the way towards another one.
    ///
    /// Plain sRGB interpolation: it is used for shading, where being perceptualy
    /// exact matters less than being cheap and monotonic.
    pub fn mix(self, other: Rgb, amount: f32) -> Rgb {
        let amount = amount.clamp(0.0, 1.0) as f64;
        let blend = |a: u8, b: u8| (a as f64 + (b as f64 - a as f64) * amount).round() as u8;
        Rgb(
            blend(self.0, other.0),
            blend(self.1, other.1),
            blend(self.2, other.2),
        )
    }

    /// True for a red, orange or yellow hue - the family that the common forms
    /// of colour-vision deficiency compress together.
    pub fn is_warm(self) -> bool {
        let hue = self.hue();
        self.chroma() > 0.15 && !(100.0..=330.0).contains(&hue)
    }
}

/// The named colour schemes.
#[derive(Clone, Copy, Debug, Default, PartialEq, Eq, Serialize, Deserialize)]
pub enum Theme {
    /// Dark blue, aqua data, amber regions. The default.
    #[default]
    DeepSpace,
    /// Very dark neutral with a cooler, higher-contrast data colour.
    Midnight,
    /// A phosphor look: amber data on near black.
    AmberCrt,
    /// Light, for bright rooms and printing.
    Paper,
}

impl Theme {
    /// Every theme, for the picker.
    pub fn all() -> &'static [Theme] {
        &[
            Theme::DeepSpace,
            Theme::Midnight,
            Theme::AmberCrt,
            Theme::Paper,
        ]
    }

    /// Name for the picker.
    pub fn label(&self) -> &'static str {
        match self {
            Theme::DeepSpace => "Deep space",
            Theme::Midnight => "Midnight",
            Theme::AmberCrt => "Amber CRT",
            Theme::Paper => "Paper",
        }
    }

    /// Whether the scheme is a dark one.
    pub fn is_dark(&self) -> bool {
        !matches!(self, Theme::Paper)
    }

    /// The colours of the scheme.
    pub fn colors(&self) -> SpectrumColors {
        match self {
            Theme::DeepSpace => SpectrumColors::deep_space(),
            Theme::Midnight => SpectrumColors::midnight(),
            Theme::AmberCrt => SpectrumColors::amber_crt(),
            Theme::Paper => SpectrumColors::paper(),
        }
    }
}

/// The colours of the spectrum display.
#[derive(Clone, Copy, Debug, PartialEq, Eq, Serialize, Deserialize)]
pub struct SpectrumColors {
    /// Behind the spectrum.
    pub background: Rgb,
    /// The spectrum itself.
    pub foreground: Rgb,
    /// Channels inside a region of interest.
    pub roi: Rgb,
    /// The comparison spectrum.
    pub compare: Rgb,
    /// Where the starting spectrum rises above the comparison.
    pub composite: Rgb,
    /// Axes, tick marks and grid.
    pub axes: Rgb,
    /// The marker line.
    pub marker: Rgb,
    /// Library line markers.
    pub library: Rgb,
    /// The area the expanded view covers, in the overview.
    pub view_box: Rgb,
    /// Panels and window chrome.
    pub panel: Rgb,
    /// Alarms and errors. Kept out of the data palette on purpose.
    pub alarm: Rgb,
    /// Healthy status.
    pub healthy: Rgb,
}

impl Default for SpectrumColors {
    fn default() -> Self {
        Self::deep_space()
    }
}

impl SpectrumColors {
    /// Dark blue with aqua data and amber regions.
    pub fn deep_space() -> Self {
        Self {
            background: Rgb(11, 16, 32),
            foreground: Rgb(64, 224, 208),
            roi: Rgb(255, 176, 32),
            compare: Rgb(167, 139, 250),
            composite: Rgb(251, 146, 130),
            axes: Rgb(138, 148, 172),
            marker: Rgb(255, 247, 220),
            library: Rgb(244, 114, 182),
            view_box: Rgb(96, 116, 168),
            panel: Rgb(20, 24, 34),
            alarm: Rgb(239, 68, 68),
            healthy: Rgb(52, 211, 153),
        }
    }

    /// Near-black neutral with a cooler data colour.
    pub fn midnight() -> Self {
        Self {
            background: Rgb(9, 10, 13),
            foreground: Rgb(96, 205, 255),
            roi: Rgb(255, 158, 60),
            compare: Rgb(87, 201, 138),
            composite: Rgb(244, 164, 96),
            axes: Rgb(130, 136, 148),
            marker: Rgb(255, 245, 200),
            library: Rgb(216, 148, 255),
            view_box: Rgb(88, 96, 112),
            panel: Rgb(15, 17, 21),
            alarm: Rgb(248, 81, 73),
            healthy: Rgb(63, 185, 130),
        }
    }

    /// A phosphor display: amber data on near black.
    pub fn amber_crt() -> Self {
        Self {
            background: Rgb(12, 9, 4),
            foreground: Rgb(255, 176, 48),
            roi: Rgb(70, 183, 224),
            compare: Rgb(127, 191, 95),
            composite: Rgb(255, 120, 90),
            axes: Rgb(150, 126, 88),
            marker: Rgb(255, 244, 214),
            library: Rgb(255, 130, 200),
            view_box: Rgb(140, 110, 60),
            panel: Rgb(20, 15, 8),
            alarm: Rgb(255, 70, 60),
            healthy: Rgb(150, 230, 120),
        }
    }

    /// Light, for bright rooms and printing.
    pub fn paper() -> Self {
        Self {
            background: Rgb(252, 252, 250),
            foreground: Rgb(11, 78, 115),
            roi: Rgb(160, 72, 10),
            compare: Rgb(96, 60, 180),
            composite: Rgb(178, 60, 60),
            axes: Rgb(90, 96, 110),
            marker: Rgb(30, 30, 34),
            library: Rgb(160, 40, 130),
            view_box: Rgb(140, 150, 175),
            panel: Rgb(238, 238, 236),
            alarm: Rgb(190, 30, 30),
            healthy: Rgb(20, 120, 80),
        }
    }

    /// The far end of the plot's background wash: the background carrying a
    /// trace of the data hue, so the plot has depth.
    ///
    /// Deliberately a whisper - anything stronger would compete with the trace
    /// drawn over it, which is the one thing that must stay the most prominent.
    pub fn background_wash(&self) -> Rgb {
        self.background.mix(self.foreground, 0.09)
    }

    /// Pairs of data roles that would be hard to tell apart, with how far apart
    /// their hues are.
    ///
    /// Hand-edited colours can end up too close together; this is what the
    /// preferences dialog warns with.
    pub fn clashes(&self) -> Vec<(&'static str, &'static str, f64)> {
        let roles = [
            ("spectrum", self.foreground),
            ("regions", self.roi),
            ("comparison", self.compare),
            ("library", self.library),
            ("marker", self.marker),
        ];
        let mut found = Vec::new();
        for (index, (name, colour)) in roles.iter().enumerate() {
            for (other_name, other) in roles.iter().skip(index + 1) {
                // A near-grey has no hue to compare, so only lightness counts.
                let comparable = colour.chroma() > 0.15 && other.chroma() > 0.15;
                let hue = colour.hue_distance(*other);
                let lightness = colour.contrast(*other);
                // Two warm colours are the hardest pair to tell apart, so they
                // are held to a stricter lightness difference.
                let needed_lightness = if colour.is_warm() && other.is_warm() {
                    1.6
                } else {
                    1.4
                };
                let too_close = if comparable {
                    hue < 25.0 && lightness < needed_lightness
                } else {
                    lightness < 1.4
                };
                if too_close {
                    found.push((*name, *other_name, hue));
                }
            }
        }
        found
    }

    /// Contrast of each data colour against the plot background.
    ///
    /// Used by the tests, and shown in the preferences dialog so a hand-edited
    /// colour can be checked rather than guessed at.
    pub fn contrast_report(&self) -> [(&'static str, f64); 5] {
        [
            ("spectrum", self.foreground.contrast(self.background)),
            ("regions", self.roi.contrast(self.background)),
            ("comparison", self.compare.contrast(self.background)),
            ("library", self.library.contrast(self.background)),
            ("marker", self.marker.contrast(self.background)),
        ]
    }
}

/// Applies a scheme to egui's own widgets, so the interface and the spectrum
/// agree with each other.
pub fn apply(ctx: &egui::Context, theme: Theme) {
    let colors = theme.colors();
    let mut visuals = if theme.is_dark() {
        egui::Visuals::dark()
    } else {
        egui::Visuals::light()
    };

    let panel = colors.panel.to_color();
    let window = if theme.is_dark() {
        colors.panel.with_alpha(1.0).gamma_multiply(1.35)
    } else {
        egui::Color32::from_rgb(248, 248, 246)
    };
    visuals.panel_fill = panel;
    visuals.window_fill = window;
    visuals.extreme_bg_color = colors.background.to_color();
    visuals.faint_bg_color = if theme.is_dark() {
        panel.gamma_multiply(1.5)
    } else {
        egui::Color32::from_rgb(242, 242, 240)
    };
    visuals.window_corner_radius = 7.into();
    visuals.menu_corner_radius = 5.into();
    visuals.window_shadow = egui::epaint::Shadow {
        offset: [0, 8],
        blur: 24,
        spread: 0,
        color: egui::Color32::from_black_alpha(if theme.is_dark() { 150 } else { 40 }),
    };
    visuals.popup_shadow = visuals.window_shadow;

    // The accent is the data colour, so selections and links belong to the same
    // family as the spectrum.
    let accent = colors.foreground.to_color();
    visuals.selection.bg_fill = accent.gamma_multiply(0.35);
    visuals.selection.stroke = egui::Stroke::new(1.0, accent);
    visuals.hyperlink_color = accent;
    visuals.warn_fg_color = colors.roi.to_color();
    visuals.error_fg_color = colors.alarm.to_color();

    let stroke_colour = colors.axes.with_alpha(0.5);
    visuals.widgets.noninteractive.bg_stroke = egui::Stroke::new(1.0, stroke_colour);
    visuals.widgets.inactive.bg_fill = if theme.is_dark() {
        panel.gamma_multiply(1.9)
    } else {
        egui::Color32::from_rgb(228, 228, 226)
    };
    visuals.widgets.inactive.weak_bg_fill = visuals.widgets.inactive.bg_fill;
    visuals.widgets.hovered.bg_fill = accent.gamma_multiply(0.28);
    visuals.widgets.hovered.weak_bg_fill = accent.gamma_multiply(0.22);
    visuals.widgets.hovered.bg_stroke = egui::Stroke::new(1.0, accent.gamma_multiply(0.8));
    visuals.widgets.active.bg_fill = accent.gamma_multiply(0.45);
    visuals.widgets.active.weak_bg_fill = accent.gamma_multiply(0.4);
    visuals.widgets.active.bg_stroke = egui::Stroke::new(1.0, accent);
    ctx.set_visuals(visuals);

    ctx.all_styles_mut(|style| {
        style.spacing.item_spacing = egui::vec2(7.0, 5.0);
        style.spacing.button_padding = egui::vec2(7.0, 3.0);
        style.spacing.menu_margin = egui::Margin::same(6);
        style.visuals.striped = true;
    });
}

#[cfg(test)]
mod tests {
    use super::*;

    /// Contrast a data colour needs against the plot background to be read
    /// comfortably. WCAG asks 4.5 for body text; a 1-pixel trace needs more.
    const MIN_DATA_CONTRAST: f64 = 4.5;
    /// The spectrum itself should stand out further still.
    const MIN_SPECTRUM_CONTRAST: f64 = 7.0;
    /// How far apart two data hues must sit to be told apart at a glance.
    const MIN_HUE_SEPARATION: f64 = 28.0;

    #[test]
    fn every_theme_puts_the_data_above_the_background() {
        for theme in Theme::all() {
            let colors = theme.colors();
            for (role, contrast) in colors.contrast_report() {
                assert!(
                    contrast >= MIN_DATA_CONTRAST,
                    "{}: {role} has only {contrast:.2}:1 against the background",
                    theme.label()
                );
            }
            let spectrum = colors.foreground.contrast(colors.background);
            assert!(
                spectrum >= MIN_SPECTRUM_CONTRAST,
                "{}: the spectrum has only {spectrum:.2}:1",
                theme.label()
            );
        }
    }

    #[test]
    fn the_background_wash_stays_a_background() {
        for theme in Theme::all() {
            let colors = theme.colors();
            let wash = colors.background_wash();
            let against_background = wash.contrast(colors.background);
            assert!(
                against_background < 1.35,
                "{}: the wash is {against_background:.2}:1 from the background, which reads as a band",
                theme.label()
            );
            // The trace must still stand clear at the washed end of the plot.
            let trace = colors.foreground.contrast(wash);
            assert!(
                trace >= MIN_DATA_CONTRAST,
                "{}: the spectrum has only {trace:.2}:1 over the wash",
                theme.label()
            );
        }
    }

    #[test]
    fn mixing_moves_between_two_colours() {
        let black = Rgb(0, 0, 0);
        let white = Rgb(255, 255, 255);
        assert_eq!(black.mix(white, 0.0), black);
        assert_eq!(black.mix(white, 1.0), white);
        assert_eq!(black.mix(white, 0.5), Rgb(128, 128, 128));
        // Out-of-range amounts clamp rather than overshoot.
        assert_eq!(black.mix(white, 2.0), white);
        assert_eq!(black.mix(white, -1.0), black);
    }

    #[test]
    fn the_peak_card_text_is_readable_on_every_theme() {
        // The card behind the peak information is the panel colour brightened;
        // every colour written on it must survive that. On Paper this once
        // failed silently: white text on a white card.
        for theme in Theme::all() {
            let colors = theme.colors();
            let card = Rgb(
                ((colors.panel.0 as f32 * 1.45).min(255.0)) as u8,
                ((colors.panel.1 as f32 * 1.45).min(255.0)) as u8,
                ((colors.panel.2 as f32 * 1.45).min(255.0)) as u8,
            );
            for (role, colour) in [
                ("numbers (spectrum colour)", colors.foreground),
                ("headline (marker colour)", colors.marker),
                ("hint (axes colour)", colors.axes),
            ] {
                let contrast = colour.contrast(card);
                assert!(
                    contrast >= 2.5,
                    "{}: the card's {role} has only {contrast:.2}:1 against the card",
                    theme.label()
                );
            }
        }
    }

    #[test]
    fn the_spectrum_is_the_most_prominent_thing_drawn() {
        // Only the marker, which is a single line, may match it.
        for theme in Theme::all() {
            let colors = theme.colors();
            let spectrum = colors.foreground.contrast(colors.background);
            for (role, contrast) in colors.contrast_report() {
                if role == "spectrum" || role == "marker" {
                    continue;
                }
                assert!(
                    contrast <= spectrum,
                    "{}: {role} ({contrast:.2}) outshines the spectrum ({spectrum:.2})",
                    theme.label()
                );
            }
        }
    }

    /// The data roles of a scheme, as name and colour.
    fn data_roles(colors: &SpectrumColors) -> [(&'static str, Rgb); 5] {
        [
            ("spectrum", colors.foreground),
            ("regions", colors.roi),
            ("comparison", colors.compare),
            ("library", colors.library),
            ("marker", colors.marker),
        ]
    }

    #[test]
    fn data_hues_are_spread_out() {
        for theme in Theme::all() {
            let colors = theme.colors();
            let roles = data_roles(&colors);
            for (index, (name, colour)) in roles.iter().enumerate() {
                for (other_name, other) in roles.iter().skip(index + 1) {
                    // Hue is meaningless for a near-grey, so those are skipped;
                    // they are separated by lightness instead.
                    if colour.chroma() < 0.15 || other.chroma() < 0.15 {
                        continue;
                    }
                    let distance = colour.hue_distance(*other);
                    let lightness = colour.contrast(*other);
                    assert!(
                        distance >= MIN_HUE_SEPARATION || lightness >= 1.6,
                        "{}: {name} and {other_name} are only {distance:.0} degrees apart \
                         and differ in lightness by just {lightness:.2}",
                        theme.label()
                    );
                }
            }
        }
    }

    #[test]
    fn warm_roles_also_differ_in_lightness() {
        // Protanopia and deuteranopia compress reds, oranges and yellows
        // together, so two warm roles must not also be equally light.
        for theme in Theme::all() {
            let colors = theme.colors();
            let roles = data_roles(&colors);
            for (index, (name, colour)) in roles.iter().enumerate() {
                for (other_name, other) in roles.iter().skip(index + 1) {
                    if !colour.is_warm() || !other.is_warm() {
                        continue;
                    }
                    let ratio = colour.contrast(*other);
                    assert!(
                        ratio >= 1.25,
                        "{}: {name} and {other_name} are both warm and only {ratio:.2} apart \
                         in lightness",
                        theme.label()
                    );
                }
            }
        }
    }

    #[test]
    fn the_spectrum_and_the_regions_are_near_opposites() {
        // Marked peaks have to read against the trace they sit on.
        for theme in Theme::all() {
            let colors = theme.colors();
            let distance = colors.foreground.hue_distance(colors.roi);
            assert!(
                distance >= 90.0,
                "{}: spectrum and regions are only {distance:.0} degrees apart",
                theme.label()
            );
        }
    }

    #[test]
    fn alarm_red_is_not_used_for_data() {
        for theme in Theme::all() {
            let colors = theme.colors();
            for (role, colour) in [
                ("spectrum", colors.foreground),
                ("regions", colors.roi),
                ("comparison", colors.compare),
                ("library", colors.library),
            ] {
                assert!(
                    colour.hue_distance(colors.alarm) > 18.0 || colour == colors.alarm,
                    "{}: {role} is too close to the alarm colour",
                    theme.label()
                );
            }
        }
    }

    #[test]
    fn luminance_and_hue_are_computed_correctly() {
        assert!((Rgb(255, 255, 255).luminance() - 1.0).abs() < 1e-9);
        assert!(Rgb(0, 0, 0).luminance() < 1e-9);
        assert!((Rgb(255, 255, 255).contrast(Rgb(0, 0, 0)) - 21.0).abs() < 0.01);
        assert!((Rgb(255, 0, 0).hue() - 0.0).abs() < 0.5);
        assert!((Rgb(0, 255, 0).hue() - 120.0).abs() < 0.5);
        assert!((Rgb(0, 0, 255).hue() - 240.0).abs() < 0.5);
        assert!((Rgb(0, 255, 255).hue_distance(Rgb(255, 0, 0)) - 180.0).abs() < 0.5);
        assert_eq!(Rgb(10, 10, 10).hue(), 0.0, "grey has no hue");
    }

    #[test]
    fn colours_round_trip_through_egui() {
        let colour = Rgb(64, 224, 208);
        assert_eq!(Rgb::from_color(colour.to_color()), colour);
    }
}
