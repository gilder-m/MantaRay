//! Energy units for formats that can only mean keV.
//!
//! `.Chn` and `.Spc` have no units field: ORTEC software reads their
//! coefficients as keV. SpecUtils also takes a `.Spe` `$MCA_CAL` line only
//! when it ends in keV. A calibration in eV or MeV is converted, which is
//! exact up to float rounding; any other units are refused by name rather
//! than stored as if they were keV.

use mantaray_core::EnergyCalibration;

use crate::FormatError;

/// How many keV one of `units` is, or `None` for units that are not energy.
fn kev_per_unit(units: &str) -> Option<f64> {
    match units.trim().to_ascii_lowercase().as_str() {
        "" | "kev" => Some(1.0),
        "mev" => Some(1000.0),
        "ev" => Some(0.001),
        _ => None,
    }
}

/// The calibration with its coefficients in keV, or `None` for units that
/// are not an energy.
pub(crate) fn in_kev(calibration: &EnergyCalibration) -> Option<EnergyCalibration> {
    let factor = kev_per_unit(&calibration.units)?;
    Some(EnergyCalibration::new(
        calibration.coefficients.map(|c| c * factor),
        "keV",
    ))
}

/// [`in_kev`] for a binary format, which refuses what it cannot carry.
pub(crate) fn in_kev_for(
    format: &str,
    calibration: &EnergyCalibration,
) -> Result<EnergyCalibration, FormatError> {
    in_kev(calibration).ok_or_else(|| FormatError::Unsupported {
        detail: format!(
            "{format} stores an energy calibration in keV only; this one is in \"{}\". \
             Save it as .Spe or native JSON, or recalibrate in keV",
            calibration.units
        ),
    })
}
