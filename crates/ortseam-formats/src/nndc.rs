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
//! | `Metastable` | the `m` suffix |
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

use ortseam_core::{LibraryPeak, Nuclide, NuclideLibrary, PhotonKind};

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
    let mut found: BTreeMap<String, Entry> = BTreeMap::new();
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

        let Some(name) = nuclide_name(get(columns.element), get(columns.mass), get(columns.meta))
        else {
            continue;
        };
        let half_life = get(columns.half_life).parse::<f64>().unwrap_or(0.0);
        let entry = found.entry(name).or_insert_with(|| Entry {
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
    for (name, entry) in found {
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

/// One nuclide as it is being assembled.
struct Entry {
    half_life_seconds: f64,
    peaks: BTreeMap<i64, LibraryPeak>,
}

/// Where each needed column sits in this particular export.
struct Columns {
    element: usize,
    mass: usize,
    meta: usize,
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
            meta: at("metastable")?,
            half_life: at("t1/2 (sec)")?,
            radiation: at("radiation")?,
            subtype: at("rad subtype")?,
            energy: at("rad energy")?,
            intensity: at("rad intensity")?,
        })
    }
}

/// `Cs`, `137`, `False` becomes `Cs-137`; a metastable one gains an `m`.
fn nuclide_name(element: &str, mass: &str, metastable: &str) -> Option<String> {
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
    if metastable.trim().eq_ignore_ascii_case("true") {
        name.push('m');
    }
    Some(name)
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
Isotope,A,Element,Z,N,Metastable,T1/2 (sec),Daughter,Radiation,Rad subtype,Rad Energy,Rad Intensity
Cs137,137,Cs,55,82,False,949252608.0,137Ba,g,,661.657,85.1
Cs137,137,Cs,55,82,False,949252608.0,137Ba,g,XR ka1,32.194,3.64
Cs137,137,Cs,55,82,False,949252608.0,137Ba,bm,,514.03,94.7
Co60,60,Co,27,33,False,166344000.0,60Ni,g,,1173.228,99.85
Co60,60,Co,27,33,False,166344000.0,60Ni,g,,1332.492,99.9826
Co60,60,Co,27,33,False,166344000.0,60Ni,g,,347.14,0.0075
Na22,22,Na,11,11,False,82053000.0,22Ne,g,Annihil.,511.0,180.7
Al24,24,Al,13,11,True,131.3,24Mg,g,,1368.6,100.0
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

    /// The shape the real export has: a quoted spin-parity holding a comma,
    /// with the columns that matter sitting after it.
    const QUOTED: &str = "\
Isotope,A,Element,JPi,Metastable,T1/2 (sec),Radiation,Rad subtype,Rad Energy,Rad Intensity
N22,22,N,\"(0-,1-)\",False,0.02,g,,1221.0,2.3
Cs137,137,Cs,7/2+,False,949252608.0,g,,661.657,85.1
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
