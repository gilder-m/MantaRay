//! Nuclide libraries: the list of nuclides and gamma lines used to identify
//! peaks and compute activities (MAESTRO Services/Library, §7.2).

use serde::{Deserialize, Serialize};

/// Origin of a library line.
#[derive(Clone, Copy, Debug, Default, PartialEq, Eq, Serialize, Deserialize)]
pub enum PhotonKind {
    /// Nuclear transition.
    #[default]
    Gamma,
    /// Atomic transition.
    XRay,
    /// The 511 keV annihilation line.
    Positron,
    /// Single-escape peak (full energy minus 511 keV).
    SingleEscape,
    /// Double-escape peak (full energy minus 1022 keV).
    DoubleEscape,
}

impl PhotonKind {
    /// One-letter code as used by the library editor.
    pub fn code(&self) -> char {
        match self {
            Self::Gamma => 'G',
            Self::XRay => 'X',
            Self::Positron => 'P',
            Self::SingleEscape => 'S',
            Self::DoubleEscape => 'D',
        }
    }

    /// Parses a one-letter code.
    pub fn from_code(code: char) -> Option<Self> {
        match code.to_ascii_uppercase() {
            'G' => Some(Self::Gamma),
            'X' => Some(Self::XRay),
            'P' => Some(Self::Positron),
            'S' => Some(Self::SingleEscape),
            'D' => Some(Self::DoubleEscape),
            _ => None,
        }
    }

    /// True when the line may be used for activity calculations.
    pub fn usable_for_activity(&self) -> bool {
        matches!(self, Self::Gamma | Self::XRay | Self::Positron)
    }
}

/// A single gamma line of a nuclide.
#[derive(Clone, Copy, Debug, PartialEq, Serialize, Deserialize)]
pub struct LibraryPeak {
    /// Line energy in keV.
    pub energy: f64,
    /// Gammas per 100 disintegrations (the "percent" of equation 22).
    pub yield_percent: f64,
    /// Where the line comes from.
    pub photon: PhotonKind,
    /// Key line: must be present for the nuclide to be reported.
    pub key_line: bool,
    /// Excluded from the weighted mean activity.
    pub not_in_average: bool,
}

impl LibraryPeak {
    /// A gamma line.
    pub fn new(energy: f64, yield_percent: f64) -> Self {
        Self {
            energy,
            yield_percent,
            photon: PhotonKind::Gamma,
            key_line: false,
            not_in_average: false,
        }
    }

    /// Marks the line as a key line.
    pub fn key(mut self) -> Self {
        self.key_line = true;
        self
    }
}

/// The eight nuclide flags of the library editor, in order `T F I N P C M A`.
#[derive(Clone, Copy, Debug, Default, PartialEq, Eq, Serialize, Deserialize)]
pub struct NuclideFlags {
    /// (T) produced by thermal neutron activation.
    pub thermal_neutron_activation: bool,
    /// (F) produced by fast neutron activation.
    pub fast_neutron_activation: bool,
    /// (I) fission product.
    pub fission_product: bool,
    /// (N) naturally occurring isotope.
    pub naturally_occurring: bool,
    /// (P) photon reaction product.
    pub photon_reaction: bool,
    /// (C) charged-particle reaction product.
    pub charged_particle: bool,
    /// (M) excluded from MDA reporting.
    pub no_mda: bool,
    /// (A) activity not included in the total.
    pub not_in_total: bool,
}

impl NuclideFlags {
    const CODES: [char; 8] = ['T', 'F', 'I', 'N', 'P', 'C', 'M', 'A'];

    /// Eight-character code string, a dot where a flag is not set.
    pub fn to_code_string(&self) -> String {
        let set = [
            self.thermal_neutron_activation,
            self.fast_neutron_activation,
            self.fission_product,
            self.naturally_occurring,
            self.photon_reaction,
            self.charged_particle,
            self.no_mda,
            self.not_in_total,
        ];
        set.iter()
            .zip(Self::CODES)
            .map(|(on, code)| if *on { code } else { '.' })
            .collect()
    }

    /// Parses an eight-character code string; unknown characters are ignored.
    pub fn from_code_string(text: &str) -> Self {
        let mut flags = Self::default();
        let upper = text.to_ascii_uppercase();
        for code in upper.chars() {
            match code {
                'T' => flags.thermal_neutron_activation = true,
                'F' => flags.fast_neutron_activation = true,
                'I' => flags.fission_product = true,
                'N' => flags.naturally_occurring = true,
                'P' => flags.photon_reaction = true,
                'C' => flags.charged_particle = true,
                'M' => flags.no_mda = true,
                'A' => flags.not_in_total = true,
                _ => {}
            }
        }
        flags
    }
}

/// A nuclide and its lines.
#[derive(Clone, Debug, PartialEq, Serialize, Deserialize)]
pub struct Nuclide {
    /// Name, up to eight characters (e.g. `Co-60`).
    pub name: String,
    /// Half life in seconds (0 for a stable or unspecified nuclide).
    pub half_life_seconds: f64,
    /// Two-sigma uncertainty of the library data, in percent.
    pub uncertainty_percent: f64,
    /// Production and reporting flags.
    pub flags: NuclideFlags,
    /// Gamma lines, in library order.
    pub peaks: Vec<LibraryPeak>,
}

impl Nuclide {
    /// A nuclide with a single gamma line.
    pub fn new(name: &str, half_life_seconds: f64, peaks: Vec<LibraryPeak>) -> Self {
        Self {
            name: name.to_string(),
            half_life_seconds,
            uncertainty_percent: 0.0,
            flags: NuclideFlags::default(),
            peaks,
        }
    }

    /// Lowest-energy line, used for sorting by energy.
    pub fn first_energy(&self) -> f64 {
        self.peaks
            .iter()
            .map(|p| p.energy)
            .fold(f64::INFINITY, f64::min)
    }

    /// Decay constant in reciprocal seconds, when the half life is known.
    pub fn decay_constant(&self) -> Option<f64> {
        (self.half_life_seconds > 0.0).then(|| std::f64::consts::LN_2 / self.half_life_seconds)
    }

    /// Fraction of the nuclide remaining after `seconds`.
    pub fn decay_factor(&self, seconds: f64) -> f64 {
        match self.decay_constant() {
            Some(lambda) => (-lambda * seconds).exp(),
            None => 1.0,
        }
    }

    /// Half life rendered in the largest sensible unit, e.g. `30.08 y`.
    pub fn half_life_display(&self) -> String {
        let seconds = self.half_life_seconds;
        if seconds <= 0.0 {
            return "stable".to_string();
        }
        const MINUTE: f64 = 60.0;
        const HOUR: f64 = 3600.0;
        const DAY: f64 = 86_400.0;
        const YEAR: f64 = 365.25 * DAY;
        let (value, unit) = if seconds >= YEAR {
            (seconds / YEAR, "y")
        } else if seconds >= DAY {
            (seconds / DAY, "d")
        } else if seconds >= HOUR {
            (seconds / HOUR, "h")
        } else if seconds >= MINUTE {
            (seconds / MINUTE, "m")
        } else {
            (seconds, "s")
        };
        format!("{} {unit}", significant(value))
    }
}

/// How many figures of a number are worth showing.
///
/// Four. The evaluations state about that many, and past them the digits are
/// arithmetic rather than measurement - K-40's half life written out in full
/// ends in the length of a year, not in anything anybody measured.
const FIGURES: i32 = 4;

/// A number at four significant figures, in the form that stays readable.
///
/// Plain digits while they can be taken in at a glance, and `1.248e9` once
/// they cannot. Significant figures rather than decimal places, which is the
/// whole point: an activity runs from millibecquerels to gigabecquerels, and
/// any fixed number of decimals is wrong at one end or the other - a wall of
/// digits for a strong source, or `0.000` for a weak one.
///
/// Not round-trip safe, and cannot be: `significant(166_344_000.0)` is
/// `"1.663e8"`, which reads back as a different number. Anything that shows a
/// stored value through this and takes it back again has to keep the original
/// - see the library dialog's half-life field, which does.
pub fn significant(value: f64) -> String {
    // Neither has a four-figure form, and inventing one would print an
    // arithmetic accident - the old code turned an infinity into
    // "NaNe2147483647" - where a plain word tells the truth.
    if !value.is_finite() {
        return format!("{value}");
    }
    if value == 0.0 {
        return "0".to_string();
    }
    // Rust's own scientific formatting does the rounding, carry and all: it
    // is what turns 9.9999e8 into 1.000e9 rather than the "10e8" that doing
    // this by hand invites. So the exponent it prints is the one the value
    // has *at four figures*, which is what decides the form below.
    let decimals = (FIGURES - 1) as usize;
    let scientific = format!("{value:.decimals$e}");
    let exponent: i32 = scientific
        .split('e')
        .nth(1)
        .and_then(|digits| digits.parse().ok())
        .unwrap_or(0);
    // Where plain digits still read at a glance. Below a thousandth they are
    // mostly leading zeros; at a hundred thousand and up they are a run too
    // long to count, which is the complaint that started this.
    if !(-3..5).contains(&exponent) {
        let (mantissa, exponent) = scientific.split_once('e').unwrap_or((&scientific, "0"));
        return format!("{}e{exponent}", trimmed(mantissa));
    }
    // Four figures means the decimals left over after the whole part has had
    // its share, and none at all once the whole part has used them up.
    let places = (FIGURES - 1 - exponent).max(0) as usize;
    trimmed(&format!("{value:.places$}"))
}

/// Drops the zeros a fixed-point format leaves behind, and the lone point.
///
/// Only where there is a point to trim towards: `"1200"` has trailing zeros
/// that are digits of the number, and trimming those would make it twelve.
fn trimmed(text: &str) -> String {
    if text.contains('.') {
        text.trim_end_matches('0').trim_end_matches('.').to_string()
    } else {
        text.to_string()
    }
}

/// A library line together with the nuclide that owns it.
#[derive(Clone, Copy, Debug)]
pub struct LibraryHit<'a> {
    /// Owning nuclide.
    pub nuclide: &'a Nuclide,
    /// The line.
    pub peak: &'a LibraryPeak,
}

/// The result of a nearest-line lookup.
#[derive(Clone, Copy, Debug)]
pub struct LibraryMatch<'a> {
    /// Owning nuclide.
    pub nuclide: &'a Nuclide,
    /// Matched line.
    pub peak: &'a LibraryPeak,
    /// Query energy minus line energy, in keV.
    pub delta: f64,
}

/// A working nuclide library.
#[derive(Clone, Debug, Default, PartialEq, Serialize, Deserialize)]
pub struct NuclideLibrary {
    /// Library name (usually the file stem).
    pub name: String,
    /// Nuclides in library order; reports follow this order.
    pub nuclides: Vec<Nuclide>,
}

impl NuclideLibrary {
    /// An empty library.
    pub fn new(name: &str) -> Self {
        Self {
            name: name.to_string(),
            nuclides: Vec::new(),
        }
    }

    /// Number of nuclides.
    pub fn len(&self) -> usize {
        self.nuclides.len()
    }

    /// True when the library holds no nuclides.
    pub fn is_empty(&self) -> bool {
        self.nuclides.is_empty()
    }

    /// Iterates the nuclides in library order.
    pub fn iter(&self) -> std::slice::Iter<'_, Nuclide> {
        self.nuclides.iter()
    }

    /// Nuclide names in library order.
    pub fn names(&self) -> Vec<&str> {
        self.nuclides.iter().map(|n| n.name.as_str()).collect()
    }

    /// Appends a nuclide.
    pub fn push(&mut self, nuclide: Nuclide) {
        self.nuclides.push(nuclide);
    }

    /// Inserts a nuclide at `index` (clamped to the end).
    pub fn insert(&mut self, index: usize, nuclide: Nuclide) {
        let index = index.min(self.nuclides.len());
        self.nuclides.insert(index, nuclide);
    }

    /// Removes and returns the nuclide at `index` (the editor's Cut).
    pub fn cut(&mut self, index: usize) -> Option<Nuclide> {
        (index < self.nuclides.len()).then(|| self.nuclides.remove(index))
    }

    /// Case-insensitive lookup by name.
    pub fn nuclide(&self, name: &str) -> Option<&Nuclide> {
        self.nuclides
            .iter()
            .find(|n| n.name.eq_ignore_ascii_case(name))
    }

    /// Looks a nuclide up from whatever the operator typed.
    ///
    /// The name is read with [`parse_nuclide_name`] first, so `na22`, `22Na`,
    /// `Na 22` and `Na-22` all find the same entry, and only then matched
    /// against the library. Falls back to a plain lookup so a library using
    /// some other naming can still be searched by typing a name from it
    /// exactly.
    pub fn find_typed(&self, typed: &str) -> Option<&Nuclide> {
        nuclide_readings(typed)
            .iter()
            .find_map(|name| self.nuclide(name))
            .or_else(|| self.nuclide(typed.trim()))
    }

    /// Mutable case-insensitive lookup by name.
    pub fn nuclide_mut(&mut self, name: &str) -> Option<&mut Nuclide> {
        self.nuclides
            .iter_mut()
            .find(|n| n.name.eq_ignore_ascii_case(name))
    }

    /// Sorts nuclides alphabetically.
    pub fn sort_by_name(&mut self) {
        self.nuclides.sort_by(|a, b| {
            a.name
                .to_ascii_uppercase()
                .cmp(&b.name.to_ascii_uppercase())
        });
    }

    /// Sorts nuclides by their lowest line energy.
    pub fn sort_by_energy(&mut self) {
        self.nuclides.sort_by(|a, b| {
            a.first_energy()
                .partial_cmp(&b.first_energy())
                .unwrap_or(std::cmp::Ordering::Equal)
        });
    }

    /// Every line in an energy window, sorted by energy.
    pub fn peaks_in(&self, low: f64, high: f64) -> Vec<LibraryHit<'_>> {
        let mut hits: Vec<LibraryHit<'_>> = self
            .nuclides
            .iter()
            .flat_map(|nuclide| {
                nuclide
                    .peaks
                    .iter()
                    .filter(move |peak| peak.energy >= low && peak.energy <= high)
                    .map(move |peak| LibraryHit { nuclide, peak })
            })
            .collect();
        hits.sort_by(|a, b| {
            a.peak
                .energy
                .partial_cmp(&b.peak.energy)
                .unwrap_or(std::cmp::Ordering::Equal)
        });
        hits
    }

    /// Nearest line to `energy` within `tolerance` keV.
    pub fn best_match(&self, energy: f64, tolerance: f64) -> Option<LibraryMatch<'_>> {
        let mut best: Option<LibraryMatch<'_>> = None;
        for nuclide in &self.nuclides {
            for peak in &nuclide.peaks {
                let delta = energy - peak.energy;
                if delta.abs() > tolerance {
                    continue;
                }
                let better = match &best {
                    Some(current) => delta.abs() < current.delta.abs(),
                    None => true,
                };
                if better {
                    best = Some(LibraryMatch {
                        nuclide,
                        peak,
                        delta,
                    });
                }
            }
        }
        best
    }

    /// A handful of nuclides for the test suite to work against.
    ///
    /// **Not a library, and never presented as one.** The numbers here were
    /// entered by hand and carry no provenance: no evaluation, no date, no
    /// uncertainty, nothing to cite. They are close to the accepted values,
    /// which is exactly what makes shipping them dangerous - a table that
    /// looks authoritative and answers to nobody is worse than no table at
    /// all, because a result computed from it cannot be defended.
    ///
    /// mantaray therefore ships no nuclide data. A library comes from a `.Lib`
    /// file the operator supplies, or is built from an evaluated source. This
    /// exists so the tests have something deterministic to compute against.
    pub fn sample_for_tests() -> Self {
        const HOUR: f64 = 3_600.0;
        const DAY: f64 = 86_400.0;
        const YEAR: f64 = 365.25 * DAY;
        let mut lib = Self::new("standard");
        lib.push(Nuclide {
            uncertainty_percent: 0.5,
            flags: NuclideFlags {
                thermal_neutron_activation: true,
                ..NuclideFlags::default()
            },
            ..Nuclide::new(
                "Am-241",
                432.6 * YEAR,
                vec![
                    LibraryPeak::new(59.541, 35.9).key(),
                    LibraryPeak::new(26.345, 2.27),
                ],
            )
        });
        lib.push(Nuclide::new(
            "Ba-133",
            10.551 * YEAR,
            vec![
                LibraryPeak::new(80.998, 34.06),
                LibraryPeak::new(276.400, 7.16),
                LibraryPeak::new(302.853, 18.34),
                LibraryPeak::new(356.017, 62.05).key(),
                LibraryPeak::new(383.851, 8.94),
            ],
        ));
        lib.push(Nuclide::new(
            "Co-57",
            271.74 * DAY,
            vec![
                LibraryPeak::new(122.061, 85.60).key(),
                LibraryPeak::new(136.474, 10.68),
                LibraryPeak::new(14.413, 9.16),
            ],
        ));
        lib.push(Nuclide {
            uncertainty_percent: 0.1,
            ..Nuclide::new(
                "Co-60",
                5.2711 * YEAR,
                vec![
                    LibraryPeak::new(1173.228, 99.85).key(),
                    LibraryPeak::new(1332.492, 99.9826).key(),
                ],
            )
        });
        lib.push(Nuclide {
            flags: NuclideFlags {
                fission_product: true,
                ..NuclideFlags::default()
            },
            ..Nuclide::new(
                "Cs-137",
                30.08 * YEAR,
                vec![LibraryPeak::new(661.657, 85.10).key()],
            )
        });
        lib.push(Nuclide::new(
            "Eu-152",
            13.517 * YEAR,
            vec![
                LibraryPeak::new(121.782, 28.53).key(),
                LibraryPeak::new(244.697, 7.55),
                LibraryPeak::new(344.279, 26.59),
                LibraryPeak::new(778.905, 12.93),
                LibraryPeak::new(964.057, 14.51),
                LibraryPeak::new(1085.837, 10.11),
                LibraryPeak::new(1112.076, 13.67),
                LibraryPeak::new(1408.013, 20.87),
            ],
        ));
        lib.push(Nuclide {
            flags: NuclideFlags {
                naturally_occurring: true,
                ..NuclideFlags::default()
            },
            ..Nuclide::new(
                "K-40",
                1.248e9 * YEAR,
                vec![LibraryPeak::new(1460.822, 10.66).key()],
            )
        });
        lib.push(Nuclide::new(
            "Mn-54",
            312.20 * DAY,
            vec![LibraryPeak::new(834.848, 99.976).key()],
        ));
        lib.push(Nuclide {
            ..Nuclide::new(
                "Na-22",
                2.6018 * YEAR,
                vec![
                    LibraryPeak::new(1274.537, 99.94).key(),
                    LibraryPeak {
                        photon: PhotonKind::Positron,
                        ..LibraryPeak::new(511.0, 180.7)
                    },
                ],
            )
        });
        lib.push(Nuclide::new(
            "Zn-65",
            243.93 * DAY,
            vec![LibraryPeak::new(1115.539, 50.04).key()],
        ));
        lib.push(Nuclide::new(
            "I-131",
            8.0252 * DAY,
            vec![
                LibraryPeak::new(364.489, 81.5).key(),
                LibraryPeak::new(636.989, 7.16),
                LibraryPeak::new(284.305, 6.14),
            ],
        ));

        // Natural series. Every real spectrum contains these lines, so a library
        // without them mis-assigns background peaks to the source.
        let natural = NuclideFlags {
            naturally_occurring: true,
            ..NuclideFlags::default()
        };
        lib.push(Nuclide {
            flags: natural,
            ..Nuclide::new(
                "Ra-226",
                1600.0 * YEAR,
                vec![LibraryPeak::new(186.211, 3.64).key()],
            )
        });
        lib.push(Nuclide {
            flags: natural,
            ..Nuclide::new(
                "Pb-214",
                26.916 * 60.0,
                vec![
                    LibraryPeak::new(241.997, 7.25),
                    LibraryPeak::new(295.224, 18.42),
                    LibraryPeak::new(351.932, 35.60).key(),
                ],
            )
        });
        lib.push(Nuclide {
            flags: natural,
            ..Nuclide::new(
                "Bi-214",
                19.9 * 60.0,
                vec![
                    LibraryPeak::new(609.312, 45.49).key(),
                    LibraryPeak::new(1120.287, 14.91),
                    LibraryPeak::new(1238.110, 5.831),
                    LibraryPeak::new(1764.494, 15.31),
                    LibraryPeak::new(2204.210, 4.913),
                ],
            )
        });
        lib.push(Nuclide {
            flags: natural,
            ..Nuclide::new(
                "Pb-212",
                10.64 * HOUR,
                vec![
                    LibraryPeak::new(238.632, 43.6).key(),
                    LibraryPeak::new(300.087, 3.30),
                ],
            )
        });
        lib.push(Nuclide {
            flags: natural,
            ..Nuclide::new(
                "Bi-212",
                60.55 * 60.0,
                vec![
                    LibraryPeak::new(727.330, 6.67).key(),
                    LibraryPeak::new(1620.500, 1.51),
                ],
            )
        });
        lib.push(Nuclide {
            flags: natural,
            ..Nuclide::new(
                "Ac-228",
                6.15 * HOUR,
                vec![
                    LibraryPeak::new(338.320, 11.27),
                    LibraryPeak::new(911.204, 25.8).key(),
                    LibraryPeak::new(964.766, 4.99),
                    LibraryPeak::new(968.971, 15.8),
                    LibraryPeak::new(1588.200, 3.22),
                ],
            )
        });
        lib.push(Nuclide {
            flags: natural,
            ..Nuclide::new(
                "Tl-208",
                183.18,
                vec![
                    LibraryPeak::new(583.191, 85.0),
                    LibraryPeak::new(860.564, 12.5),
                    LibraryPeak::new(2614.511, 99.754).key(),
                ],
            )
        });
        lib
    }
}

impl<'a> IntoIterator for &'a NuclideLibrary {
    type Item = &'a Nuclide;
    type IntoIter = std::slice::Iter<'a, Nuclide>;

    fn into_iter(self) -> Self::IntoIter {
        self.nuclides.iter()
    }
}

/// Reads a nuclide name the way somebody would write one, into `Na-22`.
///
/// There is no single convention. The symbol comes first in a lab notebook and
/// on a source's label, last in a journal, and the separator is a hyphen, a
/// space or nothing at all depending on who is typing. `Na22`, `22Na`, `Na-22`,
/// `Na 22`, `na 22` and `22-NA` are all the same nuclide, and asking an
/// operator to remember which one this program wants is asking them to get it
/// wrong while holding a source.
///
/// A metastable state keeps its suffix wherever it is written: `Ba137m`,
/// `137mBa` and `Ba-137m` all read as `Ba-137m`, and `m2` survives too.
///
/// Returns the most likely reading, or `None` when there is no symbol, no mass
/// number, or anything else in the text - a typo is refused rather than turned
/// into a lookup for something the operator did not ask about. Where a name
/// has more than one reading, [`NuclideLibrary::find_typed`] tries them all.
pub fn parse_nuclide_name(typed: &str) -> Option<String> {
    nuclide_readings(typed).into_iter().next()
}

/// Every way the typed text could be read, likeliest first.
///
/// `22mg` is Mg-22 to anyone writing an element symbol first, and G-22m to
/// anyone writing the state before the symbol. Both readings are returned and
/// the library decides, which is more reliable than a rule: there is no reading
/// of the text alone that gets Mg-22 and Ba-137m both right.
fn nuclide_readings(typed: &str) -> Vec<String> {
    /// A run of letters or a run of digits. Nothing else survives the scan.
    enum Token {
        Letters(String),
        Digits(String),
    }

    let mut tokens: Vec<Token> = Vec::new();
    // A separator between two runs of digits is a slip of the finger, not a
    // name: closing the gap in `Na-22-3` reads it as Na-223, which is a
    // different nuclide and no way to find out you mistyped. Between runs of
    // letters it is just punctuation - `137m-Ba` is how a lot of people write
    // it - so those are joined and sorted out below.
    let mut after_separator = false;
    for character in typed.trim().chars() {
        // The separator is whatever the writer felt like, including none.
        if matches!(character, '-' | '_' | ' ') {
            after_separator = true;
            continue;
        }
        let (is_digit, is_letter) = (character.is_ascii_digit(), character.is_ascii_alphabetic());
        if !is_digit && !is_letter {
            return Vec::new();
        }
        if after_separator && is_digit && matches!(tokens.last(), Some(Token::Digits(_))) {
            return Vec::new();
        }
        after_separator = false;
        match tokens.last_mut() {
            Some(Token::Digits(run)) if is_digit => run.push(character),
            Some(Token::Letters(run)) if is_letter => run.push(character),
            _ if is_digit => tokens.push(Token::Digits(character.to_string())),
            _ => tokens.push(Token::Letters(character.to_string())),
        }
    }

    // The shapes a name can take, once the separators are gone. `Ba137m2` is
    // letters-digits-letters-digits; `137m2Ba` is the same four the other way
    // round. Anything else is not a nuclide name.
    use Token::{Digits, Letters};
    let (symbol, mass, state): (&str, &str, String) = match tokens.as_slice() {
        [Letters(symbol), Digits(mass)] => (symbol, mass, String::new()),
        [Letters(symbol), Digits(mass), Letters(state)] => (symbol, mass, state.clone()),
        [
            Letters(symbol),
            Digits(mass),
            Letters(state),
            Digits(number),
        ] => (symbol, mass, format!("{state}{number}")),
        [Digits(mass), Letters(symbol)] => (symbol, mass, String::new()),
        [
            Digits(mass),
            Letters(state),
            Digits(number),
            Letters(symbol),
        ] => (symbol, mass, format!("{state}{number}")),
        _ => return Vec::new(),
    };

    let mut readings = Vec::new();
    // As written, if the symbol is a symbol and the state is a state.
    if let Some(name) = assemble(symbol, mass, &state) {
        readings.push(name);
    }
    // And the other reading of `137mBa`: the leading letter of the symbol is
    // the state, written before the symbol rather than after it. Only worth
    // considering when nothing else is claiming to be the state.
    if state.is_empty()
        && matches!(tokens.as_slice(), [Digits(_), Letters(_)])
        && symbol.len() > 1
        && symbol.starts_with(['m', 'M'])
        && let Some(name) = assemble(&symbol[1..], mass, "m")
    {
        readings.push(name);
    }
    readings
}

/// Puts a symbol, a mass number and a state together, or refuses them.
fn assemble(symbol: &str, mass: &str, state: &str) -> Option<String> {
    // Checked against the element table rather than merely counted: `Xy-137`
    // is a typo, and an empty result would leave the operator wondering
    // whether the library was at fault.
    let symbol = crate::elements::normalise(symbol)?;
    if mass.is_empty() {
        return None;
    }
    // A leading zero is a typo, not a mass number: `Na-022` must not find
    // Na-22, because the two would be indistinguishable in a report.
    if mass.len() > 1 && mass.starts_with('0') {
        return None;
    }
    // The only state a gamma library names is a metastable one, numbered from
    // the second onwards: `m`, `m2`, `m3`.
    let state = state.to_ascii_lowercase();
    if !state.is_empty() {
        let mut letters = state.chars();
        if letters.next() != Some('m') || !letters.all(|c| c.is_ascii_digit()) {
            return None;
        }
    }

    Some(format!("{symbol}-{mass}{state}"))
}
