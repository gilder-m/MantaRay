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
///
/// Written as `"#0b1020"` when saved, because a scheme is meant to be shared
/// and edited by hand, and `[11, 16, 32]` is not a colour anybody can read.
/// Both forms are accepted when reading, so a palette saved by an earlier
/// version still loads.
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub struct Rgb(pub u8, pub u8, pub u8);

impl std::fmt::Display for Rgb {
    fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        write!(f, "#{:02x}{:02x}{:02x}", self.0, self.1, self.2)
    }
}

impl std::str::FromStr for Rgb {
    type Err = String;

    /// Reads `#rrggbb`, `rrggbb`, `#rgb` or `rgb`.
    ///
    /// The three-digit form doubles each digit, the way it does everywhere
    /// else: `#f80` is `#ff8800`.
    ///
    /// The digits are collected character by character rather than sliced out
    /// by position. This field is typed into and pasted into, so it will see
    /// text that is not hexadecimal and not even ASCII, and byte offsets into
    /// a string of anything else land in the middle of a character and panic.
    fn from_str(text: &str) -> Result<Self, Self::Err> {
        let refuse = || format!("{text:?} is not a colour - expected #rrggbb or #rgb");
        let digits: Vec<u8> = text
            .trim()
            .trim_start_matches('#')
            .chars()
            .map(|c| c.to_digit(16).map(|value| value as u8).ok_or_else(refuse))
            .collect::<Result<_, _>>()?;
        match digits[..] {
            [r, g, b] => Ok(Self(r * 17, g * 17, b * 17)),
            [r1, r0, g1, g0, b1, b0] => Ok(Self(r1 * 16 + r0, g1 * 16 + g0, b1 * 16 + b0)),
            _ => Err(refuse()),
        }
    }
}

impl Serialize for Rgb {
    fn serialize<S: serde::Serializer>(&self, serializer: S) -> Result<S::Ok, S::Error> {
        serializer.serialize_str(&self.to_string())
    }
}

impl<'de> Deserialize<'de> for Rgb {
    fn deserialize<D: serde::Deserializer<'de>>(deserializer: D) -> Result<Self, D::Error> {
        struct Either;

        impl<'de> serde::de::Visitor<'de> for Either {
            type Value = Rgb;

            fn expecting(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
                f.write_str("a colour as \"#rrggbb\" or as three numbers")
            }

            fn visit_str<E: serde::de::Error>(self, text: &str) -> Result<Rgb, E> {
                text.parse().map_err(E::custom)
            }

            /// The form earlier versions wrote. Kept so that a palette somebody
            /// tuned by hand is not thrown away by an upgrade.
            fn visit_seq<A: serde::de::SeqAccess<'de>>(self, mut seq: A) -> Result<Rgb, A::Error> {
                let mut channel = || {
                    seq.next_element::<u8>()?
                        .ok_or_else(|| serde::de::Error::custom("a colour needs three channels"))
                };
                Ok(Rgb(channel()?, channel()?, channel()?))
            }
        }

        deserializer.deserialize_any(Either)
    }
}

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
    /// Grey chrome around a black plot: what instrument software looked like
    /// when it ran on Windows 95 and nobody had thought to question it.
    Conductor,
    /// Ink on drafting paper - light, cool, and quiet enough to print.
    Blueprint,
    /// Maximum separation, for poor light and poor eyes.
    HighContrast,
}

impl Theme {
    /// Every theme, for the picker.
    pub fn all() -> &'static [Theme] {
        &[
            Theme::DeepSpace,
            Theme::Midnight,
            Theme::AmberCrt,
            Theme::Conductor,
            Theme::HighContrast,
            Theme::Paper,
            Theme::Blueprint,
        ]
    }

    /// Name for the picker.
    pub fn label(&self) -> &'static str {
        match self {
            Theme::DeepSpace => "Deep space",
            Theme::Midnight => "Midnight",
            Theme::AmberCrt => "Amber CRT",
            Theme::Paper => "Paper",
            Theme::Conductor => "Conductor",
            Theme::Blueprint => "Blueprint",
            Theme::HighContrast => "High contrast",
        }
    }

    /// How the scheme draws, which is as much of its look as the colours are.
    pub fn style(&self) -> SchemeStyle {
        match self {
            // The one scheme reproducing something that existed: solid fill,
            // no grid, no glow, square corners, no shadows.
            Theme::Conductor => SchemeStyle::period(),
            _ => SchemeStyle::default(),
        }
    }

    /// The colours of the scheme.
    pub fn colors(&self) -> SpectrumColors {
        match self {
            Theme::DeepSpace => SpectrumColors::deep_space(),
            Theme::Midnight => SpectrumColors::midnight(),
            Theme::AmberCrt => SpectrumColors::amber_crt(),
            Theme::Paper => SpectrumColors::paper(),
            Theme::Conductor => SpectrumColors::conductor(),
            Theme::Blueprint => SpectrumColors::blueprint(),
            Theme::HighContrast => SpectrumColors::high_contrast(),
        }
    }
}

/// What the overview is drawn in when a palette predates it having its own
/// colour: a neutral silver, which is what it was already being drawn as on
/// every scheme that had no opinion.
fn default_overview() -> Rgb {
    Rgb(160, 160, 160)
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
    /// The whole spectrum drawn small, in the overview inset.
    ///
    /// Its own colour rather than the trace's. The overview answers a different
    /// question - where am I in the whole thing - and a scheme may want it
    /// quiet and neutral while the trace stays loud.
    #[serde(default = "default_overview")]
    pub overview: Rgb,
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
            overview: Rgb(120, 148, 190),
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
            overview: Rgb(132, 140, 152),
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
            overview: Rgb(190, 158, 100),
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
            overview: Rgb(120, 132, 156),
            panel: Rgb(238, 238, 236),
            alarm: Rgb(190, 30, 30),
            healthy: Rgb(20, 120, 80),
        }
    }

    /// Whether the chrome around the plot wants light text on it.
    ///
    /// Read from the panel colour rather than carried alongside it, because a
    /// scheme somebody edited has no preset left to ask - and a flag that can
    /// disagree with the colours it describes will, eventually. It is the panel
    /// rather than the plot because these are the colours the menus, sidebar
    /// and dialogs are drawn on: a scheme can legitimately put light chrome
    /// around a black plot, which is what most instrument software did.
    pub fn chrome_is_dark(&self) -> bool {
        self.panel.luminance() < 0.35
    }

    /// The look of the software that used to drive these instruments.
    ///
    /// Not reconstructed from memory: the values were sampled from a screenshot
    /// of the real program running on this bench. Cyan filled to the baseline
    /// over a navy field, red regions, a white cursor, and the overview drawn
    /// in silver with a yellow cursor of its own - the VGA sixteen, which is
    /// what everything of that era was drawn in.
    ///
    /// One departure. The original leaves red meaning both "region" and
    /// "alarm"; here the alarm is taken round to crimson, because two reds
    /// meaning two things is the collision the rules in this module exist to
    /// prevent. It stays far enough from the regions to be told apart and
    /// close enough to still read as an alarm.
    pub fn conductor() -> Self {
        Self {
            background: Rgb(0, 0, 64),
            foreground: Rgb(0, 255, 255),
            roi: Rgb(255, 0, 0),
            compare: Rgb(192, 192, 192),
            composite: Rgb(0, 128, 128),
            axes: Rgb(128, 128, 128),
            marker: Rgb(255, 255, 255),
            library: Rgb(255, 0, 255),
            // Not the original's yellow. That yellow is a thin cursor line
            // inside the overview; this role is the rectangle covering
            // everything the expanded view shows, which on a spectrum viewed
            // whole is the entire inset - and in yellow it flooded it.
            view_box: Rgb(140, 140, 170),
            // The overview drawn in silver, which is what the original does -
            // and the reason this colour is a role of its own.
            overview: Rgb(192, 192, 192),
            panel: Rgb(240, 240, 240),
            alarm: Rgb(200, 0, 80),
            healthy: Rgb(0, 128, 0),
        }
    }

    /// Ink on drafting paper: light and cool, and quiet enough to print.
    pub fn blueprint() -> Self {
        Self {
            background: Rgb(238, 243, 248),
            foreground: Rgb(12, 62, 122),
            roi: Rgb(168, 74, 0),
            compare: Rgb(104, 44, 160),
            composite: Rgb(176, 52, 52),
            axes: Rgb(104, 116, 132),
            marker: Rgb(20, 26, 36),
            library: Rgb(158, 26, 116),
            view_box: Rgb(150, 168, 190),
            overview: Rgb(126, 146, 170),
            panel: Rgb(224, 231, 238),
            alarm: Rgb(184, 24, 24),
            healthy: Rgb(14, 110, 74),
        }
    }

    /// Maximum separation: dark ink on white, for bright rooms and projectors.
    ///
    /// Light rather than dark on purpose. On black every "maximum contrast"
    /// colour ends up crowding white, and the cursor - which has to be the one
    /// thing always findable - was within 1.25:1 in lightness of the trace it
    /// sits on. White leaves room underneath for four saturated hues and a
    /// black cursor that none of them can be confused with.
    pub fn high_contrast() -> Self {
        Self {
            background: Rgb(255, 255, 255),
            foreground: Rgb(0, 0, 160),
            roi: Rgb(170, 60, 0),
            compare: Rgb(90, 0, 140),
            composite: Rgb(160, 40, 40),
            axes: Rgb(60, 60, 60),
            marker: Rgb(0, 0, 0),
            library: Rgb(150, 0, 90),
            view_box: Rgb(120, 120, 130),
            overview: Rgb(90, 90, 100),
            panel: Rgb(245, 245, 245),
            alarm: Rgb(200, 0, 0),
            healthy: Rgb(0, 110, 50),
        }
    }

    /// The fill of a card floating over the plot, such as the peak readout.
    ///
    /// Lifted off the plot background rather than derived from the panel. The
    /// card is drawn on the plot, so the plot is what it has to be legible
    /// against, and everything written on it is already chosen to contrast
    /// with that. Taking it from the panel tied the two together for no
    /// reason, and ruled out a scheme with light chrome around a dark plot -
    /// which is what most instrument software of the era actually looked like,
    /// and would have put near-white text on a near-white card.
    pub fn card(&self) -> Rgb {
        // Toward the panel, so the card still belongs to the scheme, but only
        // a quarter of the way: the peak behind it must stay visible.
        self.background.mix(self.panel, 0.25).mix(
            if self.chrome_is_dark() {
                Rgb(255, 255, 255)
            } else {
                Rgb(0, 0, 0)
            },
            0.10,
        )
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
                let hue = colour.hue_distance(*other);
                let lightness = colour.contrast(*other);
                // Two warm colours are the hardest pair to tell apart, so they
                // are held to a stricter lightness difference.
                let needed_lightness = if colour.is_warm() && other.is_warm() {
                    1.6
                } else {
                    1.4
                };
                let too_close = if colour.chroma() > 0.15 && other.chroma() > 0.15 {
                    // Both carry a hue, so the angle between them is meaningful.
                    hue < 25.0 && lightness < needed_lightness
                } else if (colour.chroma() - other.chroma()).abs() > 0.35 {
                    // One is a grey and the other is not, which is a difference
                    // the eye reads immediately whatever their lightness. A
                    // white cursor over a cyan trace is the case: barely 1.25:1
                    // apart in lightness, and nobody has ever confused them.
                    false
                } else {
                    // Two greys, or two colours of similar faintness. Neither
                    // hue nor saturation separates them, so lightness must.
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

/// How the area under the trace is filled.
#[derive(Clone, Copy, Debug, Default, PartialEq, Eq, Serialize, Deserialize)]
pub enum FillStyle {
    /// Bright at the trace, fading to nothing at the baseline.
    ///
    /// Reads as depth on a dark plot and keeps the trace itself the brightest
    /// thing drawn, which is why it is the default.
    #[default]
    Gradient,
    /// One flat colour from the trace down to the baseline.
    ///
    /// What instrument software has always done, and what prints and
    /// photocopies without turning to mud.
    Solid,
    /// Nothing under the trace; the line alone.
    None,
}

impl FillStyle {
    /// Every style, for the picker.
    pub fn all() -> &'static [FillStyle] {
        &[FillStyle::Gradient, FillStyle::Solid, FillStyle::None]
    }

    /// Name for the picker.
    pub fn label(&self) -> &'static str {
        match self {
            FillStyle::Gradient => "Gradient",
            FillStyle::Solid => "Solid",
            FillStyle::None => "Outline",
        }
    }

    /// The alpha at the trace and at the baseline.
    pub fn alphas(&self) -> (f32, f32) {
        match self {
            FillStyle::Gradient => (0.80, 0.10),
            FillStyle::Solid => (1.0, 1.0),
            FillStyle::None => (0.0, 0.0),
        }
    }
}

/// How open spectra are arranged in the working area.
#[derive(Clone, Copy, Debug, Default, PartialEq, Eq, Serialize, Deserialize)]
pub enum Layout {
    /// One at a time, filling the area, chosen from a strip of tabs.
    ///
    /// The default, because it is what a spectrum wants: the whole area, with
    /// nothing overlapping the trace. Any one of them can still be pulled out
    /// into a window when two need to be seen at once.
    #[default]
    Tabs,
    /// Free-floating windows that can overlap, be tiled and be cascaded.
    Windows,
}

impl Layout {
    /// Both arrangements, for the picker.
    pub fn all() -> &'static [Layout] {
        &[Layout::Tabs, Layout::Windows]
    }

    /// Name for the picker.
    pub fn label(&self) -> &'static str {
        match self {
            Layout::Tabs => "Tabs",
            Layout::Windows => "Windows",
        }
    }
}

/// How a scheme draws, as distinct from what it draws in.
///
/// A palette on its own cannot reproduce a look. The programs these
/// instruments shipped with filled solid to the baseline, drew no grid, put no
/// glow under the trace and had square corners and no shadows - none of which
/// is a colour, and all of which is more of the difference than the colours
/// are.
#[derive(Clone, Copy, Debug, PartialEq, Eq, Serialize, Deserialize)]
pub struct SchemeStyle {
    /// How open spectra are arranged in the working area.
    #[serde(default)]
    pub layout: Layout,
    /// How the area under the trace is filled.
    #[serde(default)]
    pub fill: FillStyle,
    /// Whether gridlines are drawn across the plot.
    #[serde(default = "yes")]
    pub grid: bool,
    /// Whether the plot background carries a trace of the data hue.
    ///
    /// A whisper of colour across the field reads as depth on a modern
    /// display. A scheme reproducing one that had exactly as many colours as
    /// it had, and no gradients at all, turns it off for a flat field.
    #[serde(default = "yes")]
    pub wash: bool,
    /// Whether a wide faint stroke sits under the trace, reading as a glow.
    #[serde(default = "yes")]
    pub glow: bool,
    /// Corner rounding of windows, menus and buttons, in points. Zero is square.
    #[serde(default = "default_corners")]
    pub corners: u8,
    /// Whether windows and menus cast a shadow.
    #[serde(default = "yes")]
    pub shadows: bool,
}

/// The default for a flag that is on unless a scheme says otherwise.
fn yes() -> bool {
    true
}

/// Rounded, unless a scheme asks for square.
fn default_corners() -> u8 {
    7
}

impl Default for SchemeStyle {
    fn default() -> Self {
        Self {
            layout: Layout::Tabs,
            fill: FillStyle::Gradient,
            grid: true,
            wash: true,
            glow: true,
            corners: 7,
            shadows: true,
        }
    }
}

impl SchemeStyle {
    /// Square, flat and unadorned: the way instrument software was drawn.
    ///
    /// Windows rather than tabs, because that software put each spectrum in
    /// one and the arrangement is as much of the look as the colours.
    pub fn period() -> Self {
        Self {
            layout: Layout::Windows,
            fill: FillStyle::Solid,
            grid: false,
            wash: false,
            glow: false,
            corners: 0,
            shadows: false,
        }
    }
}

/// A named palette, as it is saved, shared and edited by hand.
///
/// The built-in themes are presets that produce one of these; everything past
/// that point treats a scheme the same whether it came from a preset, from the
/// colour editor, or from a file somebody was sent.
///
/// ```json
/// {
///   "name": "Bench",
///   "colors": { "background": "#0b1020", "foreground": "#40e0d0", ... }
/// }
/// ```
#[derive(Clone, Debug, PartialEq, Eq, Serialize, Deserialize)]
pub struct Scheme {
    /// What to call it in the picker.
    pub name: String,
    /// The palette itself.
    pub colors: SpectrumColors,
    /// How it draws. Absent from a scheme written before styles existed, and
    /// defaulted then, so an older file still loads and looks as it did.
    #[serde(default)]
    pub style: SchemeStyle,
}

impl Scheme {
    /// A named scheme, drawn the default way.
    pub fn new(name: impl Into<String>, colors: SpectrumColors) -> Self {
        Self {
            name: name.into(),
            colors,
            style: SchemeStyle::default(),
        }
    }

    /// The same, drawn some other way.
    pub fn styled(name: impl Into<String>, colors: SpectrumColors, style: SchemeStyle) -> Self {
        Self {
            name: name.into(),
            colors,
            style,
        }
    }

    /// The scheme as a file, indented so it can be read and edited by hand.
    pub fn to_json(&self) -> String {
        serde_json::to_string_pretty(self).unwrap_or_default()
    }

    /// Reads a scheme from a file somebody was sent.
    ///
    /// The error is meant to be shown to whoever opened the file, so it says
    /// what was wrong rather than that something was.
    pub fn from_json(text: &str) -> Result<Self, String> {
        let scheme: Self = serde_json::from_str(text).map_err(|error| {
            // serde puts the position at the end of its own message; the line
            // is worth keeping and the rest of that suffix is not, since it is
            // about to be repeated.
            let message = error.to_string();
            let reason = message.split(" at line").next().unwrap_or(&message);
            format!("line {}: {reason}", error.line())
        })?;
        if scheme.name.trim().is_empty() {
            return Err("the scheme has no name".into());
        }
        Ok(scheme)
    }

    /// What is wrong with this palette, in the words the editor uses.
    ///
    /// A scheme is not refused for failing these - somebody may have a reason,
    /// and a program that argues with its operator about colour is worse than
    /// one that lets them see the problem. It is reported, not enforced.
    pub fn complaints(&self) -> Vec<String> {
        let mut found = Vec::new();
        for (role, contrast) in self.colors.contrast_report() {
            if contrast < 4.5 {
                found.push(format!(
                    "{role} is only {contrast:.1}:1 against the plot, which will be hard to see"
                ));
            }
        }
        for (first, second, hue) in self.colors.clashes() {
            found.push(format!(
                "{first} and {second} are only {hue:.0} degrees apart and equally light"
            ));
        }
        found
    }
}

/// Applies a scheme to egui's own widgets, so the interface and the spectrum
/// agree with each other.
///
/// Takes the colours in use rather than the preset they came from. Deriving
/// them from the preset meant an edited palette reached the plot and stopped
/// there: the trace changed colour while every selection, hover, link and
/// warning around it stayed the shade the preset had chosen.
pub fn apply(ctx: &egui::Context, colors: &SpectrumColors, style: &SchemeStyle) {
    let dark = colors.chrome_is_dark();
    let mut visuals = if dark {
        egui::Visuals::dark()
    } else {
        egui::Visuals::light()
    };

    let panel = colors.panel.to_color();
    // A window sits above the panels, so it is lifted away from them - upward
    // on a dark scheme and downward on a light one, since white has no room
    // left above it.
    let window = if dark {
        colors.panel.with_alpha(1.0).gamma_multiply(1.35)
    } else {
        colors.panel.with_alpha(1.0).gamma_multiply(1.06)
    };
    visuals.panel_fill = panel;
    visuals.window_fill = window;
    // What egui fills a text field with. Taken from the chrome rather than
    // from the plot: they are usually the same darkness and it never showed,
    // until a scheme put light chrome around a black plot and every field in
    // the sidebar became a black box with dark text written on it. A field is
    // part of the chrome, and sinks below it - toward white where the chrome
    // is light, because that is what a light interface does.
    visuals.extreme_bg_color = if dark {
        colors.panel.mix(Rgb(0, 0, 0), 0.45).to_color()
    } else {
        colors.panel.mix(Rgb(255, 255, 255), 0.6).to_color()
    };
    visuals.faint_bg_color = if dark {
        panel.gamma_multiply(1.5)
    } else {
        colors.panel.mix(Rgb(255, 255, 255), 0.35).to_color()
    };
    visuals.window_corner_radius = style.corners.into();
    visuals.menu_corner_radius = style.corners.saturating_sub(2).into();
    // A shadow lifts a window off what is behind it. A scheme reproducing
    // software that never had one turns it off rather than living with an
    // anachronism under every menu.
    visuals.window_shadow = if style.shadows {
        egui::epaint::Shadow {
            offset: [0, 8],
            blur: 24,
            spread: 0,
            color: egui::Color32::from_black_alpha(if dark { 150 } else { 40 }),
        }
    } else {
        egui::epaint::Shadow::NONE
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
    // A button is a surface raised off the panel, and it has to look like one.
    // Lifting it ninety per cent worked on a dark scheme and the same code on a
    // light one moved it ten per cent the other way, which is a difference
    // nobody can see: every button in the toolbar read as bare text on grey.
    // Light chrome gets a fill lifted toward white and a drawn edge, which is
    // how a raised control was always made to look - highlight above, shadow
    // below.
    let raised = if dark {
        panel.gamma_multiply(1.9)
    } else {
        colors.panel.mix(Rgb(255, 255, 255), 0.45).to_color()
    };
    let edge = if dark {
        stroke_colour
    } else {
        colors.panel.mix(Rgb(0, 0, 0), 0.35).to_color()
    };
    visuals.widgets.inactive.bg_fill = raised;
    visuals.widgets.inactive.weak_bg_fill = raised;
    visuals.widgets.inactive.bg_stroke = egui::Stroke::new(1.0, edge);
    visuals.widgets.noninteractive.bg_stroke = egui::Stroke::new(1.0, edge);
    visuals.widgets.hovered.bg_fill = accent.gamma_multiply(0.28);
    visuals.widgets.hovered.weak_bg_fill = accent.gamma_multiply(0.22);
    visuals.widgets.hovered.bg_stroke = egui::Stroke::new(1.0, accent.gamma_multiply(0.8));
    visuals.widgets.active.bg_fill = accent.gamma_multiply(0.45);
    visuals.widgets.active.weak_bg_fill = accent.gamma_multiply(0.4);
    visuals.widgets.active.bg_stroke = egui::Stroke::new(1.0, accent);

    // Widgets follow the same corner as the windows, so a square scheme is
    // square all the way down rather than square windows full of round buttons.
    let corner: egui::CornerRadius = style.corners.saturating_sub(3).into();
    for widget in [
        &mut visuals.widgets.noninteractive,
        &mut visuals.widgets.inactive,
        &mut visuals.widgets.hovered,
        &mut visuals.widgets.active,
        &mut visuals.widgets.open,
    ] {
        widget.corner_radius = corner;
    }
    ctx.set_visuals(visuals);

    ctx.all_styles_mut(|ui_style| {
        ui_style.spacing.item_spacing = egui::vec2(7.0, 5.0);
        ui_style.spacing.button_padding = egui::vec2(7.0, 3.0);
        ui_style.spacing.menu_margin = egui::Margin::same(6);
        ui_style.visuals.striped = true;
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

    /// A theme that is not in `all()` is in no picker and in no test below.
    #[test]
    fn every_theme_is_offered_and_named_distinctly() {
        // Listed here as well as in `all()` on purpose: adding a variant
        // already fails to compile in `label()` and `colors()`, which are
        // exhaustive matches, and this is the third place that has to agree.
        let every = [
            Theme::DeepSpace,
            Theme::Midnight,
            Theme::AmberCrt,
            Theme::Conductor,
            Theme::HighContrast,
            Theme::Paper,
            Theme::Blueprint,
        ];
        for theme in every {
            assert!(
                Theme::all().contains(&theme),
                "{} is not offered in the picker",
                theme.label()
            );
        }
        assert_eq!(
            Theme::all().len(),
            every.len(),
            "the picker and this list have drifted apart"
        );
        // Two schemes with one name is a picker nobody can use.
        let mut labels: Vec<&str> = Theme::all().iter().map(|theme| theme.label()).collect();
        labels.sort_unstable();
        let before = labels.len();
        labels.dedup();
        assert_eq!(labels.len(), before, "two themes share a name");
    }

    /// A scheme with light chrome around a dark plot is a real arrangement.
    #[test]
    fn chrome_darkness_follows_the_panel_not_the_plot() {
        // Conductor is the case this exists for: a black plot inside grey
        // chrome, which is what the software these instruments shipped with
        // looked like. Reading darkness from the plot would put light text on
        // that grey and make every menu unreadable.
        let conductor = SpectrumColors::conductor();
        assert!(
            !conductor.chrome_is_dark(),
            "grey chrome wants dark text on it"
        );
        assert!(
            conductor.background.luminance() < 0.05,
            "even though the plot behind it is black"
        );
        for theme in [Theme::DeepSpace, Theme::Midnight, Theme::AmberCrt] {
            assert!(theme.colors().chrome_is_dark(), "{}", theme.label());
        }
        for theme in [Theme::Paper, Theme::Blueprint] {
            assert!(!theme.colors().chrome_is_dark(), "{}", theme.label());
        }
    }

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
        // Asked of the card colour itself rather than of a copy of the formula
        // that produces it, so that changing how the card is mixed cannot leave
        // this test checking something the program no longer draws. On Paper
        // this once failed for real: white text on a white card.
        for theme in Theme::all() {
            let colors = theme.colors();
            let card = colors.card();
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
    fn a_scheme_survives_being_written_out_and_read_back() {
        let scheme = Scheme::new("Bench", SpectrumColors::conductor());
        let text = scheme.to_json();
        // Readable by a person, because the point of the format is sharing it.
        assert!(text.contains("\"name\": \"Bench\""), "{text}");
        assert!(text.contains("\"background\": \"#000040\""), "{text}");
        assert_eq!(Scheme::from_json(&text).expect("read back"), scheme);
    }

    #[test]
    fn a_scheme_file_that_is_wrong_says_what_is_wrong_with_it() {
        // Nameless: it would appear in the picker as a blank row.
        let nameless = r#"{"name": "  ", "colors": {}}"#;
        assert!(Scheme::from_json(nameless).is_err());

        // Truncated, misspelt, or simply not a scheme at all. The message
        // reaches the operator, so it has to say something they can act on.
        for text in ["", "{}", "not json", r#"{"name": "x"}"#] {
            let error = Scheme::from_json(text).expect_err("should be refused");
            assert!(!error.is_empty(), "{text:?} gave an empty reason");
        }

        // A colour that is not a colour names the field it was found in.
        let bad_colour = format!(
            r#"{{"name": "x", "colors": {}}}"#,
            serde_json::to_string(&SpectrumColors::deep_space())
                .expect("a palette")
                .replace("\"#0b1020\"", "\"not a colour\"")
        );
        let error = Scheme::from_json(&bad_colour).expect_err("should be refused");
        assert!(
            error.contains("colour") || error.contains("background"),
            "unhelpful message: {error}"
        );
    }

    #[test]
    fn a_scheme_reports_what_is_wrong_without_refusing_it() {
        // Every built-in is beyond reproach, by the rules the tests above set.
        for theme in Theme::all() {
            let scheme = Scheme::new(theme.label(), theme.colors());
            assert!(
                scheme.complaints().is_empty(),
                "{}: {:?}",
                theme.label(),
                scheme.complaints()
            );
        }
        // A palette somebody made unreadable is described, not rejected: they
        // may have a reason, and arguing with an operator about colour is
        // worse than letting them see the problem.
        let mut washed = SpectrumColors::deep_space();
        washed.foreground = washed.background;
        let scheme = Scheme::new("Invisible", washed);
        assert!(!scheme.complaints().is_empty());
        assert!(
            scheme
                .complaints()
                .iter()
                .any(|note| note.contains("spectrum")),
            "{:?}",
            scheme.complaints()
        );
    }

    #[test]
    fn a_colour_is_written_as_hex_and_read_back() {
        let colour = Rgb(11, 16, 32);
        assert_eq!(colour.to_string(), "#0b1020");
        let json = serde_json::to_string(&colour).expect("write");
        assert_eq!(json, "\"#0b1020\"");
        assert_eq!(serde_json::from_str::<Rgb>(&json).expect("read"), colour);
    }

    #[test]
    fn a_colour_is_read_however_it_is_written() {
        for (text, expected) in [
            ("#0b1020", Rgb(11, 16, 32)),
            ("0b1020", Rgb(11, 16, 32)),
            ("#FFFFFF", Rgb(255, 255, 255)),
            ("  #40e0d0  ", Rgb(64, 224, 208)),
            // The short form doubles each digit, the way it does everywhere.
            ("#f80", Rgb(255, 136, 0)),
            ("fff", Rgb(255, 255, 255)),
        ] {
            assert_eq!(text.parse::<Rgb>(), Ok(expected), "{text:?}");
        }
        // And a typo is refused rather than silently becoming black, which
        // would be a scheme that looks broken with nothing to explain it.
        for text in ["", "#", "#12", "#12345", "#gggggg", "not a colour"] {
            assert!(text.parse::<Rgb>().is_err(), "{text:?} should be refused");
        }
        // Refused, not panicked on. This is a field people paste into, so it
        // will see text that is neither hexadecimal nor ASCII. Slicing it by
        // byte offset put the cut inside a character: "#a\u{20ac}12" is six
        // bytes and four characters, so it took the six-digit branch and split
        // the euro sign in half.
        for text in [
            "#a\u{20ac}12",
            "#\u{e9}\u{e9}\u{e9}",
            "\u{20ac}\u{20ac}",
            "#\u{1f600}",
        ] {
            assert!(
                text.parse::<Rgb>().is_err(),
                "{text:?} should be refused rather than panic"
            );
        }
    }

    /// A palette somebody tuned by hand must survive the change of format.
    #[test]
    fn the_older_array_form_still_loads() {
        assert_eq!(
            serde_json::from_str::<Rgb>("[11, 16, 32]").expect("the older form"),
            Rgb(11, 16, 32)
        );
        // Including inside a whole scheme, which is how it is actually stored.
        let older = r#"{
            "background": [11, 16, 32], "foreground": [64, 224, 208],
            "roi": [255, 176, 32], "compare": [167, 139, 250],
            "composite": [251, 146, 130], "axes": [138, 148, 172],
            "marker": [255, 247, 220], "library": [244, 114, 182],
            "view_box": [96, 116, 168], "panel": [20, 24, 34],
            "alarm": [239, 68, 68], "healthy": [52, 211, 153]
        }"#;
        let colors: SpectrumColors = serde_json::from_str(older).expect("an older palette");
        // Every colour that was in the file arrives unchanged.
        assert_eq!(
            SpectrumColors {
                overview: colors.overview,
                ..colors
            },
            SpectrumColors {
                overview: colors.overview,
                ..SpectrumColors::deep_space()
            }
        );
        // And one added since gets a neutral default rather than this scheme's
        // own, because a file written before it existed cannot have an opinion
        // about it and guessing one would change how an old palette looks.
        assert_eq!(colors.overview, default_overview());
        assert_ne!(colors.overview, SpectrumColors::deep_space().overview);
    }

    #[test]
    fn colours_round_trip_through_egui() {
        let colour = Rgb(64, 224, 208);
        assert_eq!(Rgb::from_color(colour.to_color()), colour);
    }
}
