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
        let value = format!("{value:.4}");
        let value = value.trim_end_matches('0').trim_end_matches('.');
        format!("{value} {unit}")
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

    /// A small library of the nuclides used for calibration and quality checks.
    ///
    /// Energies and yields are the accepted evaluated values; they are meant for
    /// demonstration and peak identification, not for certified assay work.
    pub fn standard() -> Self {
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
