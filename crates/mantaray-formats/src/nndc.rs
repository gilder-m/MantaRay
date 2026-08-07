//! Building a nuclide library from an evaluated NNDC radiation export.
//!
//! No nuclide library is shipped with this project, deliberately: line energies
//! and emission probabilities belong to whoever evaluated them, and a table
//! carrying no evaluation, no date and nothing to cite is worse than no table
//! at all, because a result computed from it cannot be defended.
//!
//! This reads the export instead - the flat radiation table the National
//! Nuclear Data Center publishes through NuDat, over the ENSDF evaluations -
//! and turns it into a library. Every number in the result came from that
//! evaluation; nothing here invents one.
//!
//! # The shape of the export
//!
//! One row per radiation, per decay branch, per nuclide. The columns this needs
//! are found by name rather than position, so a wider export - and they vary -
//! still reads, and rows are split on the commas that separate fields rather
//! than on every comma, because some fields are quoted and hold their own:
//!
//! | Column | Used for |
//! |---|---|
//! | `A`, `Element` | the nuclide's name |
//! | `Parent E(level)` | which state decays, and its `m` suffix |
//! | `T1/2 (sec)` | half life |
//! | `Radiation` | `g` selects the photons; betas and electrons are dropped |
//! | `Rad subtype` | gamma, X-ray or annihilation |
//! | `Rad Energy` | line energy, keV |
//! | `Rad Intensity` | emission probability, percent |
//!
//! # Provenance
//!
//! [`Built::provenance`] records where the data came from and when it was
//! converted, so a report can say what it computed from. The evaluation dates
//! belong to the export and travel with it.

use std::collections::BTreeMap;

use mantaray_core::{LibraryPeak, Nuclide, NuclideLibrary, PhotonKind};

use crate::FormatError;

/// A library built from an export, with a note of where it came from.
pub struct Built {
    /// The library itself.
    pub library: NuclideLibrary,
    /// One line naming the source, for a report to cite.
    pub provenance: String,
    /// Rows read, and rows kept, so a surprising result can be explained.
    pub rows_read: usize,
    /// Lines kept after the intensity cutoff.
    pub lines_kept: usize,
}

/// How faint a line may be and still be worth carrying.
///
/// An export holds every evaluated emission, down to probabilities no detector
/// will ever see. Keeping them all makes a library that is slower to search and
/// harder to read without identifying anything more.
pub const DEFAULT_MIN_INTENSITY: f64 = 1.0;

/// Builds a library from an NNDC radiation export in CSV form.
///
/// `min_intensity` drops lines below that emission probability, in percent.
/// Pass `0.0` to keep everything the export holds.
pub fn build(text: &str, min_intensity: f64) -> Result<Built, FormatError> {
    let mut lines = text.lines();
    let header = lines
        .next()
        .ok_or_else(|| FormatError::missing("the export's header row"))?;
    let columns = Columns::find(header)?;

    // Grouped by nuclide, and by energy within it, so the result is stable
    // whatever order the export happens to be in - two runs over the same file
    // must produce the same library, byte for byte.
    let mut found: BTreeMap<(String, Level), Entry> = BTreeMap::new();
    let mut rows_read = 0usize;
    let mut lines_kept = 0usize;

    // Reused across a quarter of a million rows rather than allocated per row.
    let mut fields: Vec<String> = Vec::new();
    for row in lines {
        if row.trim().is_empty() {
            continue;
        }
        rows_read += 1;
        split_row(row, &mut fields);
        let get = |at: usize| fields.get(at).map(|value| value.trim()).unwrap_or("");

        // Photons only. The export carries betas, conversion electrons and
        // alphas too, and none of them make a peak in a gamma spectrum.
        if !get(columns.radiation).eq_ignore_ascii_case("g") {
            continue;
        }
        let Ok(energy) = get(columns.energy).parse::<f64>() else {
            continue;
        };
        let Ok(intensity) = get(columns.intensity).parse::<f64>() else {
            continue;
        };
        // NaN fails every one of these, which is the intent: a line with no
        // energy or no measured probability is not a line.
        if energy.is_nan() || energy <= 0.0 {
            continue;
        }
        if intensity.is_nan() || intensity <= 0.0 || intensity < min_intensity {
            continue;
        }

        let Some(name) = nuclide_name(get(columns.element), get(columns.mass)) else {
            continue;
        };
        // Keyed by the state the radiation comes from, not only by the
        // nuclide: two states of one nuclide have different half lives and
        // different intensities for a line they both emit.
        let level = Level::read(get(columns.level));
        // The export writes -1 where the half life has not been determined,
        // and a negative half life would give a decay correction that grows
        // with time. Unknown is zero here, which the reader below treats as
        // "not yet known" rather than as a measurement.
        let half_life = match get(columns.half_life).parse::<f64>() {
            Ok(seconds) if seconds > 0.0 => seconds,
            _ => 0.0,
        };
        let entry = found.entry((name, level)).or_insert_with(|| Entry {
            half_life_seconds: half_life,
            peaks: BTreeMap::new(),
        });
        // A nuclide's half life is repeated on every one of its rows; the first
        // sensible one is as good as any, and a zero never displaces a real one.
        if entry.half_life_seconds <= 0.0 && half_life > 0.0 {
            entry.half_life_seconds = half_life;
        }
        // The same line can appear under more than one decay branch. Keeping
        // the strongest is what a spectroscopist would do by hand.
        let key = (energy * 1_000.0).round() as i64;
        let peak = LibraryPeak {
            energy,
            yield_percent: intensity,
            photon: photon_kind(get(columns.subtype)),
            key_line: false,
            not_in_average: false,
        };
        entry
            .peaks
            .entry(key)
            .and_modify(|kept| {
                if peak.yield_percent > kept.yield_percent {
                    *kept = peak;
                }
            })
            .or_insert(peak);
        lines_kept += 1;
    }

    let mut library = NuclideLibrary::new("NNDC");
    for (name, entry) in name_states(found) {
        let mut peaks: Vec<LibraryPeak> = entry.peaks.into_values().collect();
        // Strongest first: the line a nuclide is recognised by should lead, and
        // the strongest gamma is the one to key the identification on.
        peaks.sort_by(|a, b| {
            b.yield_percent
                .partial_cmp(&a.yield_percent)
                .unwrap_or(std::cmp::Ordering::Equal)
        });
        if let Some(first) = peaks
            .iter_mut()
            .find(|peak| peak.photon == PhotonKind::Gamma)
        {
            first.key_line = true;
        }
        library.push(Nuclide::new(&name, entry.half_life_seconds, peaks));
    }

    Ok(Built {
        provenance: format!(
            "National Nuclear Data Center (NuDat/ENSDF) radiation export, \
             converted {}; lines at or above {min_intensity}% emission probability",
            chrono::Utc::now().format("%Y-%m-%d")
        ),
        library,
        rows_read,
        lines_kept,
    })
}

/// Splits one row on the commas that separate fields, leaving quoted ones whole.
///
/// Splitting on every comma is wrong here, and wrong silently. The spin-parity
/// column carries values like `"(0-,1-)"` - the ordinary way to write an
/// undecided assignment - and a comma inside those quotes shifts every column
/// after it. The four columns this reader needs sit at the end of a forty-two
/// column row, so all four move together: the row stops looking like a gamma
/// and is dropped without a word. Against the published export that quietly
/// costs 550 evaluated lines and 31 nuclides outright, among them Bi-218 from
/// the radon chain, Rh-102 and Pm-148m.
///
/// A doubled quote inside a quoted field is one literal quote, as the format
/// has it. Rows are still whole lines: a field may hold a comma, but nothing
/// in this export holds a newline.
fn split_row(row: &str, into: &mut Vec<String>) {
    into.clear();
    let mut field = String::new();
    let mut quoted = false;
    let mut rest = row.chars().peekable();
    while let Some(character) = rest.next() {
        match character {
            '"' if quoted && rest.peek() == Some(&'"') => {
                field.push('"');
                rest.next();
            }
            '"' => quoted = !quoted,
            ',' if !quoted => into.push(std::mem::take(&mut field)),
            _ => field.push(character),
        }
    }
    into.push(field);
}

/// Gives every state its name: the ground one plain, the excited ones `m`.
///
/// Which state is `m` cannot be decided a row at a time - it depends on what
/// else the nuclide has. Ordered by level, the ground state keeps the bare
/// name and the excited ones take `m`, `m2`, `m3` in ascending energy, which
/// is how the nomenclature reads: Tc-99 and Tc-99m, Ba-137 and Ba-137m. A
/// nuclide the export knows only as an excited state - Ba-137m appears with no
/// ground-state rows, because the ground state is stable and emits nothing -
/// is still `m`, not the bare name.
fn name_states(found: BTreeMap<(String, Level), Entry>) -> Vec<(String, Entry)> {
    let mut named: Vec<(String, Entry)> = Vec::with_capacity(found.len());
    let mut isomer = 0usize;
    let mut current: Option<String> = None;
    // The map is ordered by name and then by level, so the states of one
    // nuclide arrive together, ground first.
    for ((name, level), entry) in found {
        if current.as_deref() != Some(name.as_str()) {
            current = Some(name.clone());
            isomer = 0;
        }
        let named_as = match level {
            Level::Ground => name.clone(),
            _ => {
                isomer += 1;
                match isomer {
                    1 => format!("{name}m"),
                    other => format!("{name}m{other}"),
                }
            }
        };
        named.push((named_as, entry));
    }
    named
}

/// One nuclide as it is being assembled.
struct Entry {
    half_life_seconds: f64,
    peaks: BTreeMap<i64, LibraryPeak>,
}

/// Where each needed column sits in this particular export.
struct Columns {
    element: usize,
    mass: usize,
    level: usize,
    half_life: usize,
    radiation: usize,
    subtype: usize,
    energy: usize,
    intensity: usize,
}

impl Columns {
    fn find(header: &str) -> Result<Self, FormatError> {
        let mut fields = Vec::new();
        split_row(header, &mut fields);
        let names: Vec<String> = fields
            .iter()
            .map(|name| name.trim().to_ascii_lowercase())
            .collect();
        let at = |wanted: &str| -> Result<usize, FormatError> {
            names.iter().position(|name| name == wanted).ok_or_else(|| {
                FormatError::invalid("the export is missing a column", wanted.to_string())
            })
        };
        Ok(Self {
            element: at("element")?,
            mass: at("a")?,
            level: at("parent e(level)")?,
            half_life: at("t1/2 (sec)")?,
            radiation: at("radiation")?,
            subtype: at("rad subtype")?,
            energy: at("rad energy")?,
            intensity: at("rad intensity")?,
        })
    }
}

/// `Cs` and `137` become `Cs-137`. The isomer suffix is added later, by
/// [`name_states`], because which state is `m` depends on the others.
fn nuclide_name(element: &str, mass: &str) -> Option<String> {
    let element = element.trim();
    let mass: u32 = mass.trim().parse().ok()?;
    if element.is_empty() || !element.chars().all(|c| c.is_ascii_alphabetic()) {
        return None;
    }
    let mut name = String::with_capacity(8);
    let mut letters = element.chars();
    if let Some(first) = letters.next() {
        name.extend(first.to_uppercase());
    }
    name.extend(letters.flat_map(|letter| letter.to_lowercase()));
    name.push('-');
    name.push_str(&mass.to_string());
    Some(name)
}

/// Which state of a nuclide a row belongs to.
///
/// The export gives one row per radiation per *parent state*, and two states of
/// one nuclide have different half lives and different intensities for a line
/// they both emit. Merging them takes whichever half life arrives first and,
/// for the shared line, whichever intensity is larger: a 30% line from an
/// isomer is not a 30% line from the ground state, and an understated yield
/// overstates the activity computed from it.
///
/// The `Metastable` column cannot tell the states apart, and reading it as
/// though it could is a quiet way to get that wrong. It is set on every row of
/// a nuclide that *has* an isomer rather than on the isomer's own rows - Bi-190
/// carries it on both its ground state and its 191 keV state - and it is false
/// on states that plainly are not the ground one, such as Sc-56's `0+X`. The
/// level a row decays from is what identifies its state, so that is the only
/// thing read here.
#[derive(Clone, Debug, PartialEq, Eq, PartialOrd, Ord)]
enum Level {
    /// The ground state: the level column places it at zero.
    Ground,
    /// An excited state, at this energy in thousandths of a keV, followed by
    /// the letter of any offset the evaluation has not determined.
    ///
    /// Ordered by energy and then by that letter, which is what decides `m`
    /// from `m2`. An undetermined offset sits on top of the energy that *is*
    /// known - `169.56+X` is a state somewhere above 169.56 keV, not one near
    /// the ground state - so the known part has to be read out and ordered on,
    /// or Ho-160's two isomers come out the wrong way round.
    Excited(i64, String),
}

impl Level {
    /// Reads the level column.
    ///
    /// Four shapes appear in the export and all four are handled here: a plain
    /// energy (`661.659`), an energy carrying an undetermined offset
    /// (`169.56+X`), that offset alone (`X`, an unknown height above zero),
    /// and the same thing written the other way round (`X+0.0`).
    fn read(level: &str) -> Self {
        let mut energy = 0.0f64;
        let mut undetermined = String::new();
        for part in level.split('+') {
            let part = part.trim();
            if part.is_empty() {
                continue;
            }
            match part.parse::<f64>() {
                Ok(value) => energy += value,
                Err(_) => undetermined.push_str(part),
            }
        }
        if energy <= 0.0 && undetermined.is_empty() {
            Self::Ground
        } else {
            Self::Excited((energy * 1_000.0).round() as i64, undetermined)
        }
    }
}

/// The export names X-rays and the annihilation line in its subtype column;
/// a blank subtype is a nuclear transition.
fn photon_kind(subtype: &str) -> PhotonKind {
    let subtype = subtype.trim().to_ascii_lowercase();
    if subtype.is_empty() {
        PhotonKind::Gamma
    } else if subtype.starts_with("xr") {
        PhotonKind::XRay
    } else if subtype.starts_with("annihil") {
        PhotonKind::Positron
    } else {
        PhotonKind::Gamma
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    /// Rows in the shape the NNDC export has, with the real Cs-137 and Co-60
    /// values so a wrong column would show as wrong physics.
    const EXPORT: &str = "\
Isotope,A,Element,Z,N,Parent E(level),Metastable,T1/2 (sec),Daughter,Radiation,Rad subtype,Rad Energy,Rad Intensity
Cs137,137,Cs,55,82,0.0,False,949252608.0,137Ba,g,,661.657,85.1
Cs137,137,Cs,55,82,0.0,False,949252608.0,137Ba,g,XR ka1,32.194,3.64
Cs137,137,Cs,55,82,0.0,False,949252608.0,137Ba,bm,,514.03,94.7
Co60,60,Co,27,33,0.0,False,166344000.0,60Ni,g,,1173.228,99.85
Co60,60,Co,27,33,0.0,False,166344000.0,60Ni,g,,1332.492,99.9826
Co60,60,Co,27,33,0.0,False,166344000.0,60Ni,g,,347.14,0.0075
Na22,22,Na,11,11,0.0,False,82053000.0,22Ne,g,Annihil.,511.0,180.7
Al24,24,Al,13,11,0.0,False,2.053,24Mg,g,,1368.6,100.0
Al24,24,Al,13,11,425.81,True,0.1307,24Mg,g,,426.0,98.0
";

    fn built(min: f64) -> Built {
        build(EXPORT, min).expect("the export reads")
    }

    #[test]
    fn the_evaluated_values_arrive_intact() {
        let library = built(1.0).library;
        let caesium = library.nuclide("Cs-137").expect("Cs-137");
        assert!((caesium.half_life_seconds - 949_252_608.0).abs() < 1.0);
        let line = caesium
            .peaks
            .iter()
            .find(|peak| (peak.energy - 661.657).abs() < 1e-6)
            .expect("the 661.657 keV line");
        assert!((line.yield_percent - 85.1).abs() < 1e-9);
        assert_eq!(line.photon, PhotonKind::Gamma);
    }

    #[test]
    fn only_photons_are_kept() {
        // The export carries betas and conversion electrons; none of them make
        // a peak in a gamma spectrum.
        let library = built(1.0).library;
        let caesium = library.nuclide("Cs-137").expect("Cs-137");
        assert!(
            caesium
                .peaks
                .iter()
                .all(|peak| (peak.energy - 514.03).abs() > 1e-6),
            "a beta end-point became a line"
        );
    }

    #[test]
    fn x_rays_and_the_annihilation_line_are_told_apart() {
        let library = built(1.0).library;
        let caesium = library.nuclide("Cs-137").expect("Cs-137");
        let x_ray = caesium
            .peaks
            .iter()
            .find(|peak| (peak.energy - 32.194).abs() < 1e-6)
            .expect("the barium K X-ray");
        assert_eq!(x_ray.photon, PhotonKind::XRay);

        let sodium = library.nuclide("Na-22").expect("Na-22");
        let annihilation = sodium.peaks.first().expect("the 511 line");
        assert_eq!(annihilation.photon, PhotonKind::Positron);
    }

    #[test]
    fn the_cutoff_drops_lines_no_detector_would_see() {
        // Co-60's 347 keV line is evaluated at 0.0075%, which is real and
        // useless: it identifies nothing and lengthens every search.
        let kept = built(1.0).library;
        let cobalt = kept.nuclide("Co-60").expect("Co-60");
        assert_eq!(cobalt.peaks.len(), 2, "{:?}", cobalt.peaks);

        let everything = built(0.0).library;
        let cobalt = everything.nuclide("Co-60").expect("Co-60");
        assert_eq!(cobalt.peaks.len(), 3, "a cutoff of zero keeps them all");
    }

    #[test]
    fn a_metastable_nuclide_is_named_as_one() {
        let library = built(1.0).library;
        assert!(
            library.nuclide("Al-24m").is_some(),
            "names: {:?}",
            library.names()
        );
    }

    #[test]
    fn the_strongest_gamma_leads_and_is_the_key_line() {
        let library = built(1.0).library;
        let cobalt = library.nuclide("Co-60").expect("Co-60");
        assert!((cobalt.peaks[0].yield_percent - 99.9826).abs() < 1e-9);
        assert!(cobalt.peaks[0].key_line, "the strongest gamma keys it");
    }

    #[test]
    fn the_result_says_where_it_came_from() {
        let built = built(1.0);
        assert!(built.provenance.contains("National Nuclear Data Center"));
        assert!(built.provenance.contains("ENSDF"));
        assert!(built.rows_read > 0 && built.lines_kept > 0);
    }

    /// Two states of one nuclide, as the real export has them.
    ///
    /// Sc-56 decays from the ground state and from `0+X` - an excited state the
    /// evaluation cannot place - and Y-98 from `0.0` and from `465.7`. Neither
    /// of the excited ones carries the `Metastable` flag, which is exactly why
    /// keying on that flag was wrong: the two collapsed into one nuclide, which
    /// then took whichever half life arrived first and, for the line both emit,
    /// whichever intensity was larger.
    const TWO_STATES: &str = "\
Isotope,A,Element,Parent E(level),Metastable,T1/2 (sec),Radiation,Rad subtype,Rad Energy,Rad Intensity
Sc56,56,Sc,0,False,0.026,g,,1128.7,18.0
Sc56,56,Sc,0+X,False,0.075,g,,1128.7,30.0
Sc56,56,Sc,775.1,True,0.00000029,g,,775.1,50.0
Ba137,137,Ba,661.659,True,153.12,g,,661.657,89.9
";

    #[test]
    fn two_states_of_one_nuclide_stay_apart() {
        let library = build(TWO_STATES, 1.0).expect("the export reads").library;

        // The ground state keeps its own half life and its own intensity for
        // the line both states emit - 18%, not the isomer's 30%.
        let ground = library.nuclide("Sc-56").expect("the ground state");
        assert!((ground.half_life_seconds - 0.026).abs() < 1e-9);
        let line = ground
            .peaks
            .iter()
            .find(|peak| (peak.energy - 1128.7).abs() < 1e-6)
            .expect("the 1128.7 keV line");
        assert!(
            (line.yield_percent - 18.0).abs() < 1e-9,
            "the isomer's 30% must not be reported as the ground state's: {}",
            line.yield_percent
        );

        // The unplaced state is an isomer in its own right, with its own half
        // life, even though the export does not flag it as metastable.
        let isomer = library
            .nuclide("Sc-56m")
            .expect("the 0+X state, which the Metastable flag calls False");
        assert!((isomer.half_life_seconds - 0.075).abs() < 1e-9);
        assert!(
            (isomer.peaks[0].yield_percent - 30.0).abs() < 1e-9,
            "{:?}",
            isomer.peaks
        );

        // A second isomer is numbered rather than colliding with the first.
        let second = library.nuclide("Sc-56m2").expect("the 775.1 keV state");
        assert!((second.half_life_seconds - 2.9e-7).abs() < 1e-12);

        // A nuclide the export knows only as an excited state is still `m`:
        // Ba-137's ground state is stable and emits nothing, so only the
        // isomer appears - and it is Ba-137m, not Ba-137.
        assert!(
            library.nuclide("Ba-137m").is_some(),
            "names: {:?}",
            library.names()
        );
        assert!(
            library.nuclide("Ba-137").is_none(),
            "the stable ground state emits nothing and must not be invented"
        );
    }

    /// A half life the evaluation has not determined, written as -1.
    #[test]
    fn an_undetermined_half_life_is_not_a_negative_one() {
        const NO_HALF_LIFE: &str = "\
Isotope,A,Element,Parent E(level),T1/2 (sec),Radiation,Rad subtype,Rad Energy,Rad Intensity
Ni58,58,Ni,16795,-1.0,g,,1454.0,20.0
";
        let library = build(NO_HALF_LIFE, 1.0).expect("the export reads").library;
        let nuclide = library.nuclide("Ni-58m").expect("the 16795 keV state");
        assert_eq!(
            nuclide.half_life_seconds, 0.0,
            "a negative half life would make a decay correction grow with time"
        );
    }

    /// Levels the evaluation has not fully placed.
    ///
    /// Ho-160's two isomers are written `59.98` and `169.56+X`, and Os-183's
    /// are `170.7` and `4180.2+X`. An undetermined offset does not mean "near
    /// the ground state": it sits on top of the energy that is known, so the
    /// known part decides the order. Reading `169.56+X` as though it were `0+X`
    /// numbered Ho-160's isomers the wrong way round and gave `Ho-160m` a half
    /// life of 3.2 s, which belongs to the other state.
    const UNPLACED: &str = "\
Isotope,A,Element,Parent E(level),T1/2 (sec),Radiation,Rad subtype,Rad Energy,Rad Intensity
Ho160,160,Ho,59.98,18072.0,g,,879.0,30.0
Ho160,160,Ho,169.56+X,3.2,g,,197.0,40.0
Os183,183,Os,0.0,46800.0,g,,381.7,90.0
Os183,183,Os,170.7,35640.0,g,,1101.9,50.0
Os183,183,Os,4180.2+X,0.00000003,g,,102.0,20.0
Ta178,178,Ta,X+0.0,8496.0,g,,213.4,80.0
";

    #[test]
    fn an_undetermined_offset_is_ordered_by_the_energy_it_sits_on() {
        let library = build(UNPLACED, 1.0).expect("the export reads").library;

        // Ho-160 has no ground-state rows, so both states are isomers - and the
        // 59.98 keV one is the first of them, not the one at 169.56+X.
        let first = library.nuclide("Ho-160m").expect("the 59.98 keV state");
        assert!(
            (first.half_life_seconds - 18072.0).abs() < 1e-9,
            "169.56+X sorted below 59.98 keV: {} s",
            first.half_life_seconds
        );
        let second = library.nuclide("Ho-160m2").expect("the 169.56+X state");
        assert!((second.half_life_seconds - 3.2).abs() < 1e-9);

        // Os-183 does have a ground state, so the numbering starts after it.
        assert!(library.nuclide("Os-183").is_some());
        let isomer = library.nuclide("Os-183m").expect("the 170.7 keV state");
        assert!(
            (isomer.half_life_seconds - 35640.0).abs() < 1e-9,
            "4180.2+X sorted below 170.7 keV: {} s",
            isomer.half_life_seconds
        );
        assert!(library.nuclide("Os-183m2").is_some());

        // The same notation written the other way round is the same level.
        assert_eq!(Level::read("X+0.0"), Level::read("0.0+X"));
        assert!(
            library.nuclide("Ta-178m").is_some(),
            "an offset alone is still an excited state: {:?}",
            library.names()
        );
    }

    /// Two unknown offsets are two different states, not one.
    #[test]
    fn different_offsets_are_different_states() {
        assert_ne!(Level::read("0.0+X"), Level::read("0.0+Y"));
        assert_eq!(Level::read("Y"), Level::read("0.0+Y"));
        assert_eq!(Level::read("0"), Level::Ground);
        assert_eq!(Level::read("0.0"), Level::Ground);
        assert_eq!(Level::read(""), Level::Ground);
        // A placed level sorts before the same energy carrying an offset.
        assert!(Level::read("59.98") < Level::read("59.98+X"));
        assert!(Level::read("0.0+X") < Level::read("59.98"));
    }

    /// The shape the real export has: a quoted spin-parity holding a comma,
    /// with the columns that matter sitting after it.
    const QUOTED: &str = "\
Isotope,A,Element,JPi,Parent E(level),Metastable,T1/2 (sec),Radiation,Rad subtype,Rad Energy,Rad Intensity
N22,22,N,\"(0-,1-)\",0.0,False,0.02,g,,1221.0,2.3
Cs137,137,Cs,7/2+,0.0,False,949252608.0,g,,661.657,85.1
";

    #[test]
    fn a_comma_inside_a_quoted_field_does_not_shift_the_columns() {
        let library = build(QUOTED, 1.0).expect("the export reads").library;
        let nitrogen = library
            .nuclide("N-22")
            .expect("N-22, whose row carries a quoted spin-parity");
        let line = nitrogen
            .peaks
            .iter()
            .find(|peak| (peak.energy - 1221.0).abs() < 1e-6)
            .expect("the 1221 keV line, which a naive split loses entirely");
        assert!((line.yield_percent - 2.3).abs() < 1e-9);
        // The plain row beside it must still read: the fix cannot have traded
        // one shape of row for the other.
        assert!(library.nuclide("Cs-137").is_some(), "the unquoted row too");
    }

    #[test]
    fn a_doubled_quote_is_one_literal_quote() {
        let mut fields = Vec::new();
        split_row("a,\"b,c\",\"say \"\"hi\"\"\",d", &mut fields);
        assert_eq!(fields, ["a", "b,c", "say \"hi\"", "d"]);
    }

    #[test]
    fn an_export_without_the_columns_says_which_one_is_missing() {
        let wrong = "Isotope,A,Element\nCs137,137,Cs\n";
        let error = match build(wrong, 1.0) {
            Ok(_) => panic!("an export missing its columns cannot be read"),
            Err(error) => error,
        };
        assert!(
            error.to_string().to_lowercase().contains("metastable")
                || error.to_string().to_lowercase().contains("column"),
            "{error}"
        );
    }
}
