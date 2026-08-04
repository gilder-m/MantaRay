//! Nuclide library files.
//!
//! Three shapes are read. The first is ORTEC's own binary `.Lib`, exactly as
//! ULI, GammaVision and MAESTRO write it; the layout is section 6.1 of the
//! *ORTEC Software File Structure Manual* (part 753800), and [`read_ortec`]
//! records what the manual leaves out. The second is JSON, which is what
//! ortseam writes (lossless). The third is a CSV table that is easy to build
//! in a spreadsheet:
//!
//! ```csv
//! nuclide,half_life_s,uncertainty_percent,flags,energy_kev,yield_percent,photon,key_line,not_in_average
//! Co-60,166344000,0.1,T.......,1173.228,99.85,G,1,0
//! Co-60,166344000,0.1,T.......,1332.492,99.9826,G,1,0
//! ```
//!
//! A shorter four-column form (`nuclide,half_life_s,energy_kev,yield_percent`)
//! is also accepted. Rows are grouped by nuclide, keeping the order of first
//! appearance, because reports follow library order.

use std::path::Path;

use ortseam_core::{LibraryPeak, Nuclide, NuclideFlags, NuclideLibrary, PhotonKind};

use crate::FormatError;

/// CSV header row written by [`write_csv`].
pub const CSV_HEADER: &str = "nuclide,half_life_s,uncertainty_percent,flags,energy_kev,yield_percent,photon,key_line,not_in_average";

/// Serialises a library as JSON.
pub fn write_json(library: &NuclideLibrary) -> Result<String, FormatError> {
    Ok(serde_json::to_string_pretty(library)?)
}

/// Parses a JSON library.
pub fn read_json(text: &str) -> Result<NuclideLibrary, FormatError> {
    Ok(serde_json::from_str(text)?)
}

/// Serialises a library as CSV, one row per line.
pub fn write_csv(library: &NuclideLibrary) -> String {
    let mut out = String::from(CSV_HEADER);
    out.push('\n');
    for nuclide in library.iter() {
        for peak in &nuclide.peaks {
            out.push_str(&format!(
                "{},{},{},{},{},{},{},{},{}\n",
                nuclide.name,
                nuclide.half_life_seconds,
                nuclide.uncertainty_percent,
                nuclide.flags.to_code_string(),
                peak.energy,
                peak.yield_percent,
                peak.photon.code(),
                u8::from(peak.key_line),
                u8::from(peak.not_in_average),
            ));
        }
    }
    out
}

/// Parses a CSV library.
pub fn read_csv(text: &str) -> Result<NuclideLibrary, FormatError> {
    let mut library = NuclideLibrary::new("csv");
    let mut rows = 0usize;
    for (number, raw) in text.lines().enumerate() {
        let line = raw.trim();
        if line.is_empty() || line.starts_with('#') || line.starts_with("//") {
            continue;
        }
        let fields: Vec<&str> = line.split(',').map(|field| field.trim()).collect();
        if fields[0].eq_ignore_ascii_case("nuclide") {
            continue; // header row
        }
        if fields.len() < 4 {
            return Err(FormatError::invalid(
                format!("library row {}", number + 1),
                format!("expected at least 4 columns, found {}", fields.len()),
            ));
        }
        let name = fields[0].to_string();
        let half_life: f64 = fields[1].parse().unwrap_or(0.0);
        let (uncertainty, flags, energy_index) = if fields.len() >= 9 {
            (
                fields[2].parse().unwrap_or(0.0),
                NuclideFlags::from_code_string(fields[3]),
                4,
            )
        } else {
            (0.0, NuclideFlags::default(), 2)
        };
        let energy: f64 = fields[energy_index].parse().map_err(|_| {
            FormatError::invalid(
                format!("library row {}", number + 1),
                format!("energy {:?} is not a number", fields[energy_index]),
            )
        })?;
        let yield_percent: f64 = fields
            .get(energy_index + 1)
            .and_then(|value| value.parse().ok())
            .unwrap_or(0.0);
        let mut peak = LibraryPeak::new(energy, yield_percent);
        if let Some(code) = fields.get(energy_index + 2)
            && let Some(kind) = code.chars().next().and_then(PhotonKind::from_code)
        {
            peak.photon = kind;
        }
        peak.key_line = truthy(fields.get(energy_index + 3));
        peak.not_in_average = truthy(fields.get(energy_index + 4));

        match library.nuclide_mut(&name) {
            Some(existing) => existing.peaks.push(peak),
            None => library.push(Nuclide {
                name,
                half_life_seconds: half_life,
                uncertainty_percent: uncertainty,
                flags,
                peaks: vec![peak],
            }),
        }
        rows += 1;
    }
    if rows == 0 {
        return Err(FormatError::missing("library rows"));
    }
    Ok(library)
}

/// One 64-word record of ORTEC's binary library, in bytes.
///
/// Section 6.1: "All types have 64-word records", and every word is a 16-bit
/// little-endian integer. Record numbers are 1-based and record 1 is the
/// header.
const ORTEC_RECORD: usize = 128;

/// Words in a nuclide record: three fit in a 64-word record, one word spare.
const ORTEC_NUCLIDE_WORDS: usize = 21;

/// Words in a peak record: four fit in a 64-word record exactly.
const ORTEC_PEAK_WORDS: usize = 16;

/// The 16-bit word `number` of a record, 1-based as the manual numbers them.
fn ortec_word(record: &[u8], number: usize) -> u16 {
    let at = (number - 1) * 2;
    u16::from_le_bytes([record[at], record[at + 1]])
}

/// The R*4 starting at word `number`: an IEEE 754 float, little-endian.
///
/// The manual only says "R*4"; the sample libraries settle what that means,
/// because Be-7's half-life reads back as 53.29 days in no other encoding.
fn ortec_real(record: &[u8], number: usize) -> f32 {
    let at = (number - 1) * 2;
    f32::from_le_bytes([record[at], record[at + 1], record[at + 2], record[at + 3]])
}

/// Seconds per half-life unit (section 6.1.2: S=1, M=2, H=3, D=4, Y=5).
fn ortec_half_life_unit(code: u16) -> f64 {
    match code {
        2 => 60.0,
        3 => 3_600.0,
        4 => 86_400.0,
        5 => 365.25 * 86_400.0,
        _ => 1.0,
    }
}

/// The eight production flags of nuclide word 12, in the manual's bit order.
fn ortec_nuclide_flags(word: u16) -> NuclideFlags {
    let bit = |n: u16| word & (1 << n) != 0;
    NuclideFlags {
        thermal_neutron_activation: bit(0),
        fast_neutron_activation: bit(1),
        fission_product: bit(2),
        naturally_occurring: bit(3),
        photon_reaction: bit(4),
        charged_particle: bit(5),
        no_mda: bit(6),
        not_in_total: bit(7),
    }
}

/// What kind of line peak word 5 describes.
///
/// Old ULI files (GvDemo.Lib is from 1988) leave the whole word at zero, so a
/// line with no kind bit at all is a gamma, not an error.
fn ortec_photon_kind(flags: u16) -> PhotonKind {
    if flags & 0b00010 != 0 {
        PhotonKind::XRay
    } else if flags & 0b00100 != 0 {
        PhotonKind::Positron
    } else if flags & 0b01000 != 0 {
        PhotonKind::SingleEscape
    } else if flags & 0b10000 != 0 {
        PhotonKind::DoubleEscape
    } else {
        PhotonKind::Gamma
    }
}

/// True when the bytes carry ORTEC's binary library layout.
///
/// The header is unmistakable: LIBID is 4 (16 for the `.PBC` sibling, which is
/// recognised here so it can be refused by name), the record length is always
/// 64 words, a nuclide record 21, a peak record 16.
pub fn is_ortec_binary(bytes: &[u8]) -> bool {
    bytes.len() >= ORTEC_RECORD
        && matches!(ortec_word(bytes, 1), 4 | 16)
        && ortec_word(bytes, 3) == 64
        && ortec_word(bytes, 10) as usize == ORTEC_NUCLIDE_WORDS
        && ortec_word(bytes, 18) as usize == ORTEC_PEAK_WORDS
}

/// Reads ORTEC's binary germanium library (File Structure Manual, section 6.1).
///
/// The file is a little record store: a header, then nuclide records three to
/// a 128-byte block, then peak records four to a block. Nothing is read in
/// file order, because file order lies: records are chained by ordinal
/// numbers, deleted records stay in place on a free list, and after an edit
/// the chain can start anywhere ("Mixed Gamma.Lib" ships with its first
/// nuclide ninth on disk). The walk starts at header word 11 and follows each
/// record's fore pointer, which is exactly what GammaVision displays as
/// library order.
pub fn read_ortec(bytes: &[u8]) -> Result<NuclideLibrary, FormatError> {
    if bytes.len() < ORTEC_RECORD {
        return Err(FormatError::Truncated {
            expected: ORTEC_RECORD,
            got: bytes.len(),
        });
    }
    let header = &bytes[..ORTEC_RECORD];
    match ortec_word(header, 1) {
        4 => {}
        16 => {
            return Err(FormatError::Unsupported {
                detail: "a .PBC background-correction file, not a nuclide library".into(),
            });
        }
        other => {
            return Err(FormatError::BadMagic {
                expected: "ORTEC library (LIBID 4)".into(),
                found: format!("LIBID {other}"),
            });
        }
    }
    for (number, name, must) in [
        (3, "record length", 64_usize),
        (10, "nuclide record length", ORTEC_NUCLIDE_WORDS),
        (18, "peak record length", ORTEC_PEAK_WORDS),
    ] {
        let found = ortec_word(header, number) as usize;
        if found != must {
            return Err(FormatError::invalid(
                name,
                format!("{found} words where the format fixes {must}"),
            ));
        }
    }
    let nuclide_max = ortec_word(header, 9) as usize; // ISOMAX
    let nuclide_start = ortec_word(header, 11) as usize; // ISOSTR
    let nuclide_base = ortec_word(header, 16) as usize; // ISOBAS
    let peak_max = ortec_word(header, 17) as usize; // PKMAX
    let peak_base = ortec_word(header, 24) as usize; // PKBAS
    if nuclide_base < 2 || peak_base < 2 {
        return Err(FormatError::invalid(
            "record layout",
            format!(
                "nuclide records at record {nuclide_base}, peaks at {peak_base}; \
                 record 1 is the header, so neither can come before record 2"
            ),
        ));
    }

    // A stored record by its ordinal, bounds-checked against the actual file.
    let stored = |base: usize, per_record: usize, words: usize, ordinal: usize| {
        let at = (base - 1 + (ordinal - 1) / per_record) * ORTEC_RECORD
            + ((ordinal - 1) % per_record) * words * 2;
        let end = at + words * 2;
        if bytes.len() < end {
            return Err(FormatError::Truncated {
                expected: end,
                got: bytes.len(),
            });
        }
        Ok(&bytes[at..end])
    };

    let mut library = NuclideLibrary::new("");
    // Fore pointers in a corrupt file can loop; a visited set makes the walk
    // finite without trusting any count the header claims.
    let mut seen_nuclides = vec![false; nuclide_max + 1];
    let mut ordinal = nuclide_start;
    while (1..=nuclide_max).contains(&ordinal) && !seen_nuclides[ordinal] {
        seen_nuclides[ordinal] = true;
        let record = stored(nuclide_base, 3, ORTEC_NUCLIDE_WORDS, ordinal)?;
        let name = String::from_utf8_lossy(&record[..8])
            .trim_end_matches([' ', '\0'])
            .to_string();
        let mut peaks = Vec::new();
        let mut seen_peaks = vec![false; peak_max + 1];
        let mut peak_ordinal = ortec_word(record, 19) as usize;
        while (1..=peak_max).contains(&peak_ordinal) && !seen_peaks[peak_ordinal] {
            seen_peaks[peak_ordinal] = true;
            let peak = stored(peak_base, 4, ORTEC_PEAK_WORDS, peak_ordinal)?;
            let flags = ortec_word(peak, 5);
            peaks.push(LibraryPeak {
                energy: ortec_real(peak, 1) as f64,
                // GAMPRD is already per hundred: Co-60 stores 99.97, not 0.9997.
                yield_percent: ortec_real(peak, 3) as f64,
                photon: ortec_photon_kind(flags),
                key_line: flags & (1 << 5) != 0,
                not_in_average: flags & (1 << 6) != 0,
            });
            // Word 16 chains this nuclide's peaks; word 14 chains all peaks by
            // energy, which is a different walk.
            peak_ordinal = ortec_word(peak, 16) as usize;
        }
        // A blank name means the chain wandered onto a deleted record; there
        // is nothing there worth showing, but its pointer still leads on.
        if !name.is_empty() {
            library.push(Nuclide {
                name,
                half_life_seconds: ortec_real(record, 5) as f64
                    * ortec_half_life_unit(ortec_word(record, 9)),
                uncertainty_percent: ortec_real(record, 7) as f64,
                flags: ortec_nuclide_flags(ortec_word(record, 12)),
                peaks,
            });
        }
        ordinal = ortec_word(record, 21) as usize;
    }
    if library.is_empty() {
        return Err(FormatError::missing("nuclide records"));
    }
    Ok(library)
}

/// Saves a library, choosing JSON or CSV by extension.
pub fn save(library: &NuclideLibrary, path: impl AsRef<Path>) -> Result<(), FormatError> {
    let path = path.as_ref();
    match extension(path).as_str() {
        "csv" | "txt" => std::fs::write(path, write_csv(library))?,
        "json" | "lib" => std::fs::write(path, write_json(library)?)?,
        other => {
            return Err(FormatError::Unsupported {
                detail: format!("library format .{other} (use .json or .csv)"),
            });
        }
    }
    Ok(())
}

/// Loads a library, choosing the reader by extension and contents.
///
/// A `.lib` file may be ORTEC's binary layout, a JSON export or a CSV; the
/// first 128 bytes decide, so libraries from other tools can be renamed
/// without editing.
pub fn load(path: impl AsRef<Path>) -> Result<NuclideLibrary, FormatError> {
    let path = path.as_ref();
    let bytes = std::fs::read(path)?;
    let text = || String::from_utf8_lossy(&bytes).into_owned();
    let mut library = match extension(path).as_str() {
        "csv" | "txt" => read_csv(&text())?,
        "json" => read_json(&text())?,
        "lib" if is_ortec_binary(&bytes) => read_ortec(&bytes)?,
        "lib" => read_json(&text()).or_else(|_| read_csv(&text()))?,
        other => {
            return Err(FormatError::Unsupported {
                detail: format!("library format .{other} (use .json or .csv)"),
            });
        }
    };
    if library.name.is_empty() || library.name == "csv" {
        library.name = path
            .file_stem()
            .map(|stem| stem.to_string_lossy().to_string())
            .unwrap_or_default();
    }
    Ok(library)
}

fn extension(path: &Path) -> String {
    path.extension()
        .and_then(|value| value.to_str())
        .unwrap_or_default()
        .to_ascii_lowercase()
}

fn truthy(field: Option<&&str>) -> bool {
    matches!(
        field
            .map(|value| value.trim().to_ascii_lowercase())
            .as_deref(),
        Some("1" | "y" | "yes" | "true" | "k")
    )
}
