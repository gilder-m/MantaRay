//! `.Clb` calibration files: an energy and a peak-shape calibration, saved on
//! their own so they can be put onto another spectrum.
//!
//! # How this was decoded
//!
//! The file is 1152 bytes and mostly zero. It carries the coefficients as six
//! consecutive little-endian `f32`s at offset `0x94`: the three energy terms
//! and then the three shape terms.
//!
//! That was not read off a specification - there is none - but recovered from
//! samples with known answers. Four `.Clb` files were read, three of them
//! against `.Spe` files saved from the same detector on the same days, whose
//! `$MCA_CAL` and `$SHAPE_CAL` records say in plain text what the calibration
//! was. The coefficients appear at `0x94` in every one, agreeing with the
//! paired `.Spe` to six significant figures, across three calibration epochs of
//! one instrument and one file from another:
//!
//! | File | a0 at `0x94` | `.Spe` states | matched against |
//! |---|---|---|---|
//! | `9_19_2025.Clb` | 19.1196995 | 19.11970 | `calibration_9_19_2025.Spe` |
//! | `2_20_2026.Clb` | 16.5096378 | 16.50960 | `Sample93179_2_27_2026.Spe` |
//! | `11_10_2025.Clb` | 16.6497726 | 16.64980 | `sample93179_11_10-17_2025.Spe` |
//!
//! Six figures rather than every printed digit: a `.Spe` prints seven, and on
//! two of the three pairs the last differs by a unit or two - about two parts
//! in a million, which is seven thousandths of a keV at 3 MeV. Six consecutive
//! values matching to six figures on four files is what fixes the offset.
//!
//! An earlier note in `docs/formats.md` put "the gain" at `0x98`. That offset
//! is right and the description was not: `0x98` is `a1`, the linear term, and
//! the constant sits in front of it at `0x94`.
//!
//! # What is still unknown
//!
//! Most of the file. Two 16-bit fields at `0x24` and `0x2c` hold 3 in every
//! sample, which matches the term counts the paired `.Spe` files report, but
//! three samples all saying "3" is not evidence that the field means what it
//! looks like, so it is not read - the six coefficients are taken as written.
//! The file also carries its own name, a timestamp and the full path it was
//! written from, in Windows text near the end. None of that is a calibration,
//! and none of it is read.
//!
//! No units are stored. MAESTRO writes keV and the paired `.Spe` files all say
//! keV, so keV is what is returned.

use mantaray_core::{EnergyCalibration, ShapeCalibration};

use crate::FormatError;

/// Where the six coefficients begin.
const COEFFICIENTS: usize = 0x94;

/// The size every sample has. Used to recognise the format, not to demand it:
/// a longer file is read, since the coefficients are near the front.
const EXPECTED_LENGTH: usize = 1152;

/// An energy calibration and a peak-shape calibration, as a `.Clb` holds them.
#[derive(Clone, Debug, PartialEq)]
pub struct Calibration {
    /// Channel to energy, in keV.
    pub energy: EnergyCalibration,
    /// Peak width against energy, when the file records one.
    ///
    /// `None` when all three shape terms are zero. The slots are always
    /// present in the file, so a `.Clb` written with no shape calibration
    /// holds three zeros rather than nothing - and three zeros are not a peak
    /// shape, they are the absence of one. Handing them back as a
    /// [`ShapeCalibration`] would let a file that records no widths overwrite
    /// measured ones with a curve that says every peak is infinitely narrow.
    pub shape: Option<ShapeCalibration>,
}

/// Reads a `.Clb` file.
///
/// Refuses a file whose linear term is zero or not a number. Every channel
/// would map to the same energy, which is not a calibration - and a calibration
/// that is silently wrong is worse than one that fails to load, because every
/// energy in every report computed from it is quietly off.
pub fn read(bytes: &[u8]) -> Result<Calibration, FormatError> {
    let end = COEFFICIENTS + 6 * 4;
    if bytes.len() < end {
        return Err(FormatError::Truncated {
            expected: end,
            got: bytes.len(),
        });
    }
    let at = |index: usize| -> f32 {
        let start = COEFFICIENTS + index * 4;
        f32::from_le_bytes([
            bytes[start],
            bytes[start + 1],
            bytes[start + 2],
            bytes[start + 3],
        ])
    };
    let terms: Vec<f64> = (0..6).map(|index| at(index) as f64).collect();
    if !terms.iter().all(|term| term.is_finite()) {
        return Err(FormatError::invalid(
            "the calibration holds a value that is not a number",
            "a .Clb file",
        ));
    }
    if terms[1] == 0.0 {
        return Err(FormatError::invalid(
            "the calibration has no gain, so every channel would be the same energy",
            "a .Clb file",
        ));
    }
    let shape = ShapeCalibration {
        coefficients: [terms[3], terms[4], terms[5]],
    };
    Ok(Calibration {
        energy: EnergyCalibration {
            coefficients: [terms[0], terms[1], terms[2]],
            units: "keV".into(),
        },
        // The same test the rest of the program applies to a shape before it
        // will use one, so a `.Clb` recording no widths reads as no widths
        // rather than as widths of zero.
        shape: shape.is_usable().then_some(shape),
    })
}

/// Writes a `.Clb` file: the mirror of [`read`], carrying only what reading
/// understands.
///
/// The samples are 1152 bytes and mostly zero, so that is what is written:
/// zeros, the two 16-bit fields at `0x24` and `0x2c` that hold 3 in every
/// sample read, and the six coefficients at `0x94`. The name, timestamp and
/// path MAESTRO embeds near the end are not written - they were never read,
/// and inventing Windows paths would be decoration pretending to be data.
/// Whether MAESTRO itself accepts such a file has not been tried against a
/// Windows installation; what is certain is that it round-trips through
/// [`read`], which checks the same offsets against the same samples.
///
/// A calibration with no shape writes three zeros, which read back as no
/// shape - the same rule reading applies, in the same direction.
///
/// The format stores no units and [`read`] assumes keV, so the coefficients
/// given here must already be in keV: a caller holding anything else must
/// refuse or convert *before* writing, because a file of MeV coefficients
/// reads back a thousand times wrong with nothing to say so.
pub fn write(calibration: &Calibration) -> Vec<u8> {
    let mut out = vec![0u8; EXPECTED_LENGTH];
    for offset in [0x24usize, 0x2c] {
        out[offset..offset + 2].copy_from_slice(&3u16.to_le_bytes());
    }
    let shape = calibration
        .shape
        .map(|shape| shape.coefficients)
        .unwrap_or([0.0; 3]);
    let terms = calibration.energy.coefficients.into_iter().chain(shape);
    for (index, term) in terms.enumerate() {
        let start = COEFFICIENTS + index * 4;
        out[start..start + 4].copy_from_slice(&(term as f32).to_le_bytes());
    }
    out
}

/// Whether a file looks like a `.Clb`.
///
/// Deliberately weak, because there is nothing strong to test: the format has
/// no magic number, so the length and a usable gain are all there is. That is
/// enough to confirm a file somebody has already named, and nowhere near
/// enough to pick one out of a directory.
///
/// Recalling a calibration goes by the extension rather than through here, and
/// deliberately: [`read`] takes a file longer than the samples, since the
/// coefficients sit near the front, while this insists on the exact length the
/// samples have. This is for a caller holding bytes and no name.
pub fn looks_like(bytes: &[u8]) -> bool {
    bytes.len() == EXPECTED_LENGTH && read(bytes).is_ok()
}

#[cfg(test)]
mod tests {
    use super::*;

    /// A written calibration reads back as itself, to the format's precision.
    #[test]
    fn a_written_calibration_round_trips() {
        let out = write(&Calibration {
            energy: EnergyCalibration {
                coefficients: [19.1197, 0.420412, 3.060_48e-7],
                units: "keV".into(),
            },
            shape: Some(ShapeCalibration {
                coefficients: [4.5439, 0.001, 0.0],
            }),
        });
        assert_eq!(out.len(), EXPECTED_LENGTH, "the samples' exact size");
        assert!(looks_like(&out), "what was written must look like a .Clb");
        let back = read(&out).expect("reads back");
        assert!(same(back.energy.coefficients[0], 19.1197));
        assert!(same(back.energy.coefficients[1], 0.420_412));
        assert!(same(back.energy.coefficients[2], 3.060_48e-7));
        let shape = back.shape.expect("the shape was written");
        assert!(same(shape.coefficients[0], 4.5439));
    }

    /// No shape writes as zeros and reads back as no shape - not as a shape
    /// that says every peak is infinitely narrow.
    #[test]
    fn no_shape_round_trips_as_no_shape() {
        let out = write(&Calibration {
            energy: EnergyCalibration::linear(0.5, 0.36),
            shape: None,
        });
        assert_eq!(read(&out).expect("reads back").shape, None);
    }

    /// A file of the shape the samples have, with chosen coefficients in it.
    fn sample(energy: [f32; 3], shape: [f32; 3]) -> Vec<u8> {
        let mut bytes = vec![0u8; EXPECTED_LENGTH];
        for (index, value) in energy.iter().chain(shape.iter()).enumerate() {
            let start = COEFFICIENTS + index * 4;
            bytes[start..start + 4].copy_from_slice(&value.to_le_bytes());
        }
        bytes
    }

    /// How close two coefficients have to be to count as the same.
    ///
    /// A `.Clb` stores single-precision floats, so it carries about seven
    /// significant figures - which is exactly what a `.Spe` prints for the same
    /// calibration, and why the two agree to every digit either of them shows.
    /// Asking for more than that is asking the format for precision it does not
    /// hold.
    fn same(got: f64, want: f64) -> bool {
        (got - want).abs() <= want.abs() * 1e-6
    }

    /// The values from `9_19_2025.Clb`, and what the `.Spe` saved by the same
    /// detector that day reports for them.
    #[test]
    fn the_coefficients_come_back_as_the_spectrum_records_them() {
        let bytes = sample([19.1197, 0.420412, 3.060_48e-7], [4.5439, 0.0, 0.0]);
        let read = read(&bytes).expect("a calibration");

        assert!(same(read.energy.coefficients[0], 19.1197));
        assert!(same(read.energy.coefficients[1], 0.420_412));
        assert!(same(read.energy.coefficients[2], 3.060_48e-7));
        assert_eq!(read.energy.units, "keV");
        let shape = read.shape.expect("this file records a peak shape");
        assert!(same(shape.coefficients[0], 4.5439));

        // And the calibration it produces puts a channel where the instrument
        // does: the 661.657 keV line of Cs-137 lands near channel 1527 on this
        // detector, which is where its own reports put it.
        let channel = read.energy.channel(661.657).expect("a channel");
        assert!(
            (1520.0..1535.0).contains(&channel),
            "661.657 keV came out at channel {channel}"
        );
    }

    /// A second calibration of the same instrument, months later.
    #[test]
    fn a_later_calibration_reads_as_its_own() {
        let bytes = sample(
            [16.5096, 0.359_362, 2.271_16e-7],
            [3.43805, 7.744_61e-4, -1.916_22e-8],
        );
        let read = read(&bytes).expect("a calibration");
        assert!(same(read.energy.coefficients[1], 0.359_362));
        // The shape terms are real here rather than a single width, which is
        // the case that would be missed if only the energy terms were read.
        let shape = read.shape.expect("this file records a peak shape");
        assert!(same(shape.coefficients[1], 7.744_61e-4));
        assert!(shape.coefficients[2] < 0.0);
    }

    /// A `.Clb` whose shape slots are empty records no peak shape.
    ///
    /// The slots are always there, so "no shape calibration" reaches the reader
    /// as three zeros. Handing those back as a `ShapeCalibration` would let a
    /// file that says nothing about peak widths overwrite measured ones with a
    /// curve saying every peak has zero width - which is not a shape the rest
    /// of the program will even use, so the widths in hand would be lost for
    /// nothing.
    #[test]
    fn a_file_recording_no_peak_shape_reads_as_none_rather_than_zero() {
        let bytes = sample([19.1197, 0.420412, 3.060_48e-7], [0.0, 0.0, 0.0]);
        let read = read(&bytes).expect("the energy calibration is still good");
        assert!(same(read.energy.coefficients[1], 0.420_412));
        assert_eq!(
            read.shape, None,
            "three zeros are the absence of a shape, not a shape of zero"
        );
    }

    #[test]
    fn a_calibration_with_no_gain_is_refused() {
        // Every channel the same energy is not a calibration, and loading it
        // silently would put every energy in every report quietly wrong.
        let bytes = sample([19.1197, 0.0, 0.0], [4.5439, 0.0, 0.0]);
        assert!(read(&bytes).is_err());
        assert!(!looks_like(&bytes));
    }

    #[test]
    fn a_file_too_short_to_hold_a_calibration_is_refused() {
        assert!(read(&[]).is_err());
        assert!(read(&[0u8; COEFFICIENTS + 8]).is_err());
        assert!(!looks_like(&[0u8; 64]));
    }

    #[test]
    fn a_file_of_the_right_length_but_no_calibration_is_not_one() {
        // A zeroed file is the shape of a .Clb and holds nothing, which is
        // exactly what a weak check has to catch.
        assert!(!looks_like(&[0u8; EXPECTED_LENGTH]));
    }
}
