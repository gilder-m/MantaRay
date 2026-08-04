//! Efficiency calibration and source decay, the basis of quantitative results.
//!
//! MAESTRO reports a count rate ("cA") from the library yield alone; an
//! efficiency curve turns net areas into activities.

use serde::{Deserialize, Serialize};

use crate::calibration::least_squares_n;
use crate::error::CalibrationError;

/// A measured absolute efficiency at one energy.
#[derive(Clone, Copy, Debug, PartialEq, Serialize, Deserialize)]
pub struct EfficiencyPoint {
    /// Line energy in keV.
    pub energy: f64,
    /// Absolute efficiency as a fraction (counts per emitted photon).
    pub efficiency: f64,
    /// One-sigma uncertainty of the efficiency, as a fraction.
    pub uncertainty: f64,
}

impl EfficiencyPoint {
    /// A point with no stated uncertainty.
    pub fn new(energy: f64, efficiency: f64) -> Self {
        Self {
            energy,
            efficiency,
            uncertainty: 0.0,
        }
    }

    /// A point with an uncertainty.
    pub fn with_uncertainty(energy: f64, efficiency: f64, uncertainty: f64) -> Self {
        Self {
            energy,
            efficiency,
            uncertainty,
        }
    }
}

/// How efficiency is interpolated or extrapolated between calibration points.
#[derive(Clone, Debug, Default, PartialEq, Serialize, Deserialize)]
pub enum EfficiencyModel {
    /// No efficiency information: activities cannot be computed.
    #[default]
    None,
    /// One efficiency for every energy.
    Constant(f64),
    /// Straight lines between the points in log(energy)/log(efficiency) space,
    /// clamped to the end points outside the calibrated range.
    Interpolated,
    /// `ln(eff) = a0 + a1*ln(E) + a2*ln(E)^2 + ...`, the usual gamma-spectroscopy
    /// efficiency curve.
    LogLogPolynomial(Vec<f64>),
}

/// An efficiency curve plus the points it was built from.
#[derive(Clone, Debug, Default, PartialEq, Serialize, Deserialize)]
pub struct EfficiencyCalibration {
    /// The curve itself.
    pub model: EfficiencyModel,
    /// Calibration points, sorted by energy (kept for display and refitting).
    pub points: Vec<EfficiencyPoint>,
}

impl EfficiencyCalibration {
    /// A flat efficiency, useful for quick estimates and for tests.
    pub fn constant(efficiency: f64) -> Self {
        Self {
            model: EfficiencyModel::Constant(efficiency),
            points: Vec::new(),
        }
    }

    /// Log-log linear interpolation through the given points.
    pub fn interpolated(mut points: Vec<EfficiencyPoint>) -> Self {
        points.sort_by(|a, b| {
            a.energy
                .partial_cmp(&b.energy)
                .unwrap_or(std::cmp::Ordering::Equal)
        });
        Self {
            model: EfficiencyModel::Interpolated,
            points,
        }
    }

    /// Least-squares fit of `ln(eff)` against a polynomial in `ln(E)`.
    pub fn fit_log_log(points: &[EfficiencyPoint], order: usize) -> Result<Self, CalibrationError> {
        let terms = order + 1;
        if points.len() < terms {
            return Err(CalibrationError::NotEnoughPoints {
                needed: terms,
                got: points.len(),
            });
        }
        if points
            .iter()
            .any(|p| p.energy <= 0.0 || p.efficiency <= 0.0)
        {
            return Err(CalibrationError::Degenerate);
        }
        let rows: Vec<(Vec<f64>, f64)> = points
            .iter()
            .map(|p| {
                let ln_e = p.energy.ln();
                let basis: Vec<f64> = (0..terms).map(|power| ln_e.powi(power as i32)).collect();
                (basis, p.efficiency.ln())
            })
            .collect();
        let coefficients = least_squares_n(&rows, terms).ok_or(CalibrationError::Degenerate)?;
        let mut sorted = points.to_vec();
        sorted.sort_by(|a, b| {
            a.energy
                .partial_cmp(&b.energy)
                .unwrap_or(std::cmp::Ordering::Equal)
        });
        Ok(Self {
            model: EfficiencyModel::LogLogPolynomial(coefficients),
            points: sorted,
        })
    }

    /// Absolute efficiency at an energy; 0.0 when no curve is defined.
    pub fn efficiency(&self, energy: f64) -> f64 {
        match &self.model {
            EfficiencyModel::None => 0.0,
            EfficiencyModel::Constant(value) => *value,
            EfficiencyModel::LogLogPolynomial(coefficients) => {
                if energy <= 0.0 || coefficients.is_empty() {
                    return 0.0;
                }
                let ln_e = energy.ln();
                let ln_eff: f64 = coefficients
                    .iter()
                    .enumerate()
                    .map(|(power, a)| a * ln_e.powi(power as i32))
                    .sum();
                let eff = ln_eff.exp();
                if eff.is_finite() { eff } else { 0.0 }
            }
            EfficiencyModel::Interpolated => self.interpolate(energy),
        }
    }

    fn interpolate(&self, energy: f64) -> f64 {
        // The explicit NaN check matters: a NaN energy sails through
        // `energy <= 0.0` and would come back out as a NaN efficiency.
        if self.points.is_empty() || energy.is_nan() || energy <= 0.0 {
            return 0.0;
        }
        let first = self.points[0];
        let last = self.points[self.points.len() - 1];
        if energy <= first.energy {
            return first.efficiency;
        }
        if energy >= last.energy {
            return last.efficiency;
        }
        let index = self
            .points
            .partition_point(|p| p.energy < energy)
            .clamp(1, self.points.len() - 1);
        let (low, high) = (self.points[index - 1], self.points[index]);
        if low.efficiency <= 0.0 || high.efficiency <= 0.0 {
            // Fall back to linear interpolation when a log is not available.
            let t = (energy - low.energy) / (high.energy - low.energy);
            return low.efficiency + t * (high.efficiency - low.efficiency);
        }
        let t = (energy.ln() - low.energy.ln()) / (high.energy.ln() - low.energy.ln());
        (low.efficiency.ln() + t * (high.efficiency.ln() - low.efficiency.ln())).exp()
    }

    /// Polynomial order, or 0 for the other models.
    pub fn order(&self) -> usize {
        match &self.model {
            EfficiencyModel::LogLogPolynomial(coefficients) => coefficients.len().saturating_sub(1),
            _ => 0,
        }
    }

    /// True when no efficiency information is available.
    pub fn is_empty(&self) -> bool {
        matches!(self.model, EfficiencyModel::None)
    }

    /// Energy range covered by the calibration points.
    pub fn range(&self) -> Option<(f64, f64)> {
        let first = self.points.first()?;
        let last = self.points.last()?;
        Some((first.energy, last.energy))
    }
}

/// Absolute efficiency implied by a certified source:
/// `eff = rate / (activity * yield/100)`.
pub fn efficiency_from_source(net_rate_cps: f64, activity_bq: f64, yield_percent: f64) -> f64 {
    let emission_rate = activity_bq * yield_percent / 100.0;
    if emission_rate <= 0.0 {
        return 0.0;
    }
    net_rate_cps / emission_rate
}

/// Fraction of a nuclide left after `elapsed_seconds`.
///
/// A non-positive half life means "no decay".
pub fn decay_factor(half_life_seconds: f64, elapsed_seconds: f64) -> f64 {
    if half_life_seconds <= 0.0 {
        return 1.0;
    }
    (-std::f64::consts::LN_2 * elapsed_seconds / half_life_seconds).exp()
}

/// Activity of a source `elapsed_seconds` after its certified activity.
pub fn source_activity_at(
    certified_activity_bq: f64,
    half_life_seconds: f64,
    elapsed_seconds: f64,
) -> f64 {
    certified_activity_bq * decay_factor(half_life_seconds, elapsed_seconds)
}

/// A certified calibration source.
#[derive(Clone, Debug, PartialEq, Serialize, Deserialize)]
pub struct CertifiedSource {
    /// Nuclide name, matching the library.
    pub nuclide: String,
    /// Certified activity in becquerel at the reference date.
    pub activity_bq: f64,
    /// One-sigma uncertainty of the certified activity, in percent.
    pub uncertainty_percent: f64,
    /// Seconds between the certificate date and the measurement.
    pub elapsed_seconds: f64,
    /// Half life in seconds.
    pub half_life_seconds: f64,
}

impl CertifiedSource {
    /// Activity at the time of the measurement.
    pub fn activity_now(&self) -> f64 {
        source_activity_at(
            self.activity_bq,
            self.half_life_seconds,
            self.elapsed_seconds,
        )
    }

    /// Efficiency point from a measured net count rate for one line.
    pub fn efficiency_point(
        &self,
        energy: f64,
        net_rate_cps: f64,
        yield_percent: f64,
    ) -> EfficiencyPoint {
        let efficiency = efficiency_from_source(net_rate_cps, self.activity_now(), yield_percent);
        EfficiencyPoint::with_uncertainty(
            energy,
            efficiency,
            efficiency * self.uncertainty_percent / 100.0,
        )
    }
}
