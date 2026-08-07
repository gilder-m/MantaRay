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
//! samples with known answers. Four `.Clb` files were compared against `.Spe`
//! files saved from the same detector on the same days, whose `$MCA_CAL` and
//! `$SHAPE_CAL` records say in plain text what the calibration was. The
//! coefficients appear at `0x94` in every one of them, to every digit the
//! `.Spe` prints, across two different calibration epochs of the same
//! instrument:
//!
//! | File | a0 | a1 | a2 | matched against |
//! |---|---|---|---|---|
//! | `9_19_2025.Clb` | 19.1197 | 0.420412 | 3.06048e-7 | `calibration_9_19_2025.Spe` |
//! | `2_20_2026.Clb` | 16.5096 | 0.359362 | 2.27116e-7 | `Sample93179_2_27_2026.Spe` |
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
    /// Peak width against energy.
    pub shape: ShapeCalibration,
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
    Ok(Calibration {
        energy: EnergyCalibration {
            coefficients: [terms[0], terms[1], terms[2]],
            units: "keV".into(),
        },
        shape: ShapeCalibration {
            coefficients: [terms[3], terms[4], terms[5]],
        },
    })
}

/// Whether a file looks like a `.Clb`.
///
/// Deliberately weak, because there is nothing strong to test: the format has
/// no magic number. The length and a usable gain are all there is, so this is
/// only ever used to confirm a file the operator has already named.
pub fn looks_like(bytes: &[u8]) -> bool {
    bytes.len() == EXPECTED_LENGTH && read(bytes).is_ok()
}

#[cfg(test)]
mod tests {
    use super::*;

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
        assert!(same(read.shape.coefficients[0], 4.5439));

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
        assert!(same(read.shape.coefficients[1], 7.744_61e-4));
        assert!(read.shape.coefficients[2] < 0.0);
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
