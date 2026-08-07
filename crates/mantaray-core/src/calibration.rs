//! Energy and peak-shape calibration.
//!
//! Following MAESTRO §4.3.2: two points give a linear calibration, three or more
//! give a quadratic least-squares fit, and the point table holds at most 96
//! entries. Units are free-form but limited to four characters ("keV" by
//! default), matching the Calibration Units dialog.

use serde::{Deserialize, Serialize};

use crate::error::CalibrationError;

/// Maximum number of points MAESTRO keeps in the calibration table.
pub const MAX_CALIBRATION_POINTS: usize = 96;

/// Default calibration units.
pub const DEFAULT_UNITS: &str = "keV";

/// A single entered calibration point: a channel and the value it represents.
#[derive(Clone, Copy, Debug, PartialEq, Serialize, Deserialize)]
pub struct CalibrationPoint {
    /// Channel (may be fractional: a fitted peak centroid).
    pub channel: f64,
    /// Value in calibration units, usually keV.
    pub value: f64,
}

impl CalibrationPoint {
    /// Creates a point.
    pub fn new(channel: f64, value: f64) -> Self {
        Self { channel, value }
    }
}

/// Quadratic channel-to-energy conversion: `E = c0 + c1*ch + c2*ch^2`.
#[derive(Clone, Debug, PartialEq, Serialize, Deserialize)]
pub struct EnergyCalibration {
    /// Zero, gain and quadratic coefficients.
    pub coefficients: [f64; 3],
    /// Units label (at most four characters).
    pub units: String,
}

impl Default for EnergyCalibration {
    fn default() -> Self {
        Self {
            coefficients: [0.0, 1.0, 0.0],
            units: DEFAULT_UNITS.to_string(),
        }
    }
}

impl EnergyCalibration {
    /// Builds a calibration from explicit coefficients.
    ///
    /// The units string is truncated to four characters, as MAESTRO does.
    pub fn new(coefficients: [f64; 3], units: &str) -> Self {
        Self {
            coefficients,
            units: truncate_units(units),
        }
    }

    /// Builds a linear calibration from a zero intercept and a gain per channel.
    pub fn linear(zero: f64, gain: f64) -> Self {
        Self::new([zero, gain, 0.0], DEFAULT_UNITS)
    }

    /// Energy of a (possibly fractional) channel.
    pub fn energy(&self, channel: f64) -> f64 {
        let [a, b, c] = self.coefficients;
        a + b * channel + c * channel * channel
    }

    /// Channel corresponding to an energy, or `None` when the calibration
    /// cannot be inverted there.
    pub fn channel(&self, energy: f64) -> Option<f64> {
        let [a, b, c] = self.coefficients;
        if c == 0.0 {
            if b == 0.0 {
                return None;
            }
            return Some((energy - a) / b);
        }
        let discriminant = b * b - 4.0 * c * (a - energy);
        if discriminant < 0.0 {
            return None;
        }
        let root = discriminant.sqrt();
        let r1 = (-b + root) / (2.0 * c);
        let r2 = (-b - root) / (2.0 * c);
        // Pick the branch on which energy increases with channel.
        let chosen = if c > 0.0 { r1.max(r2) } else { r1.min(r2) };
        chosen.is_finite().then_some(chosen)
    }

    /// Width in calibration units of `channels` channels centred on `channel`.
    pub fn width(&self, channel: f64, channels: f64) -> f64 {
        self.energy(channel + channels / 2.0) - self.energy(channel - channels / 2.0)
    }

    /// Polynomial order actually in use (0, 1 or 2).
    pub fn order(&self) -> usize {
        let [_, b, c] = self.coefficients;
        if c.abs() > f64::EPSILON {
            2
        } else if b.abs() > f64::EPSILON {
            1
        } else {
            0
        }
    }

    /// True for the identity calibration (channel numbers).
    pub fn is_trivial(&self) -> bool {
        self.coefficients == [0.0, 1.0, 0.0]
    }

    /// Fits a calibration to entered points, in the default units.
    ///
    /// Two points give an exact line, three or more a least-squares parabola.
    pub fn fit(points: &[CalibrationPoint]) -> Result<Self, CalibrationError> {
        Self::fit_with_units(points, DEFAULT_UNITS)
    }

    /// Like [`EnergyCalibration::fit`] but with explicit units.
    pub fn fit_with_units(
        points: &[CalibrationPoint],
        units: &str,
    ) -> Result<Self, CalibrationError> {
        if points.len() < 2 {
            return Err(CalibrationError::NotEnoughPoints {
                needed: 2,
                got: points.len(),
            });
        }
        let distinct_channels = distinct_abscissae(points.iter().map(|p| p.channel));
        if distinct_channels < 2 {
            return Err(CalibrationError::Degenerate);
        }

        let pairs: Vec<(f64, f64)> = points.iter().map(|p| (p.channel, p.value)).collect();
        let solution = if distinct_channels == 2 {
            linear_fit(&pairs)
        } else {
            quadratic_fit(&pairs)
        };
        solution
            .map(|coefficients| Self::new(coefficients, units))
            .ok_or(CalibrationError::Degenerate)
    }

    /// Least-squares straight line through the points, however many there are.
    ///
    /// Auto-calibration uses this when it has exactly three matches: the
    /// candidate that won the search was a line, and an exact parabola through
    /// three points would bend to fit misses the tolerance allowed, then
    /// diverge wherever the spectrum extrapolates.
    fn fit_linear_with_units(
        points: &[CalibrationPoint],
        units: &str,
    ) -> Result<Self, CalibrationError> {
        let pairs: Vec<(f64, f64)> = points.iter().map(|p| (p.channel, p.value)).collect();
        linear_fit(&pairs)
            .map(|coefficients| Self::new(coefficients, units))
            .ok_or(CalibrationError::Degenerate)
    }
}

/// The result of matching found peaks against reference energies.
#[derive(Clone, Debug, PartialEq)]
pub struct AutoCalibration {
    /// The fitted calibration.
    pub calibration: EnergyCalibration,
    /// The peak-to-line assignments the fit is built on, as calibration points.
    pub matches: Vec<CalibrationPoint>,
}

/// Gains a germanium or scintillation spectrum plausibly has, in units per
/// channel: from 16k channels across 100 keV to 256 channels across 3 MeV.
const AUTO_GAIN_RANGE: std::ops::RangeInclusive<f64> = 0.005..=15.0;
/// How far the zero intercept may sit from 0, in energy units.
const AUTO_ZERO_LIMIT: f64 = 60.0;
/// How close a predicted energy must land to a reference line to count.
const AUTO_TOLERANCE: f64 = 2.0;

/// Finds the energy calibration that lines found peaks up with known energies.
///
/// This is what "calibrate me from this Eu-152 spectrum" needs: no starting
/// calibration, just peak centroids (strongest first) and the source's line
/// energies. Every pair of peaks is tried against every pair of lines; each
/// candidate linear calibration is scored by how many of the peaks it drops
/// within 2 keV of some line, and the winner is refitted through
/// all of its matches. At least three peaks on three different lines must
/// agree, so a lucky pair - or a doublet split across one line - cannot
/// calibrate a spectrum by itself; ties go to the candidate whose matches are
/// tightest.
pub fn auto_calibrate(
    centroids: &[f64],
    lines: &[f64],
    units: &str,
) -> Result<AutoCalibration, CalibrationError> {
    if centroids.len() < 3 || lines.len() < 3 {
        return Err(CalibrationError::NotEnoughPoints {
            needed: 3,
            got: centroids.len().min(lines.len()),
        });
    }
    // Strongest first is how callers order them; the search caps how many it
    // tries so a spectrum with a hundred regions stays quick.
    let peaks: Vec<f64> = centroids.iter().copied().take(8).collect();
    let mut references: Vec<f64> = lines.to_vec();
    references.sort_by(|a, b| a.partial_cmp(b).unwrap_or(std::cmp::Ordering::Equal));
    references.dedup_by(|a, b| (*a - *b).abs() < 1e-9);

    let mut best: Option<(usize, f64, Vec<CalibrationPoint>)> = None;
    for (i, &ci) in peaks.iter().enumerate() {
        for &cj in peaks.iter().skip(i + 1) {
            if (cj - ci).abs() < 1.0 {
                continue;
            }
            for (k, &ek) in references.iter().enumerate() {
                for &el in references.iter().skip(k + 1) {
                    // Both orders: the strongest peak is not always the lowest.
                    for (ea, eb) in [(ek, el), (el, ek)] {
                        let gain = (eb - ea) / (cj - ci);
                        if !AUTO_GAIN_RANGE.contains(&gain.abs()) || gain <= 0.0 {
                            continue;
                        }
                        let zero = ea - gain * ci;
                        if zero.abs() > AUTO_ZERO_LIMIT {
                            continue;
                        }
                        let (matches, error) = match_peaks(&peaks, &references, zero, gain);
                        if matches.len() < 3 {
                            continue;
                        }
                        let better = match &best {
                            None => true,
                            Some((count, err, _)) => {
                                matches.len() > *count || (matches.len() == *count && error < *err)
                            }
                        };
                        if better {
                            best = Some((matches.len(), error, matches));
                        }
                    }
                }
            }
        }
    }

    match best {
        Some((_, _, matches)) => {
            // Three matches validate the winning line, no more: an exact
            // parabola through them would bend to fit tolerance-sized misses
            // and diverge on extrapolation. Four or more overdetermine the
            // quadratic, which is then allowed to pick up real nonlinearity.
            let calibration = if matches.len() == 3 {
                EnergyCalibration::fit_linear_with_units(&matches, units)?
            } else {
                EnergyCalibration::fit_with_units(&matches, units)?
            };
            Ok(AutoCalibration {
                calibration,
                matches,
            })
        }
        None => Err(CalibrationError::NoMatch {
            peaks: peaks.len(),
            matched: 0,
        }),
    }
}

/// Which peaks a candidate `zero + gain*channel` drops onto a reference line,
/// and the mean absolute miss of those that land.
///
/// Each reference line may be claimed by one peak only - the closest. Without
/// that rule an unresolved doublet counts as two agreeing peaks on the same
/// line, and "at least three must agree" can be satisfied by two real lines.
fn match_peaks(
    peaks: &[f64],
    references: &[f64],
    zero: f64,
    gain: f64,
) -> (Vec<CalibrationPoint>, f64) {
    let mut claims: Vec<(f64, f64, f64)> = Vec::new();
    for &channel in peaks {
        let predicted = zero + gain * channel;
        let nearest = references
            .iter()
            .copied()
            .min_by(|a, b| {
                (a - predicted)
                    .abs()
                    .partial_cmp(&(b - predicted).abs())
                    .unwrap_or(std::cmp::Ordering::Equal)
            })
            .expect("references are non-empty");
        let miss = (nearest - predicted).abs();
        if miss <= AUTO_TOLERANCE {
            claims.push((channel, nearest, miss));
        }
    }
    claims.sort_by(|a, b| a.2.partial_cmp(&b.2).unwrap_or(std::cmp::Ordering::Equal));
    let mut matches: Vec<CalibrationPoint> = Vec::new();
    let mut total_miss = 0.0;
    for (channel, line, miss) in claims {
        if matches.iter().any(|taken| taken.value == line) {
            continue;
        }
        matches.push(CalibrationPoint::new(channel, line));
        total_miss += miss;
    }
    let mean = if matches.is_empty() {
        f64::INFINITY
    } else {
        total_miss / matches.len() as f64
    };
    matches.sort_by(|a, b| {
        a.channel
            .partial_cmp(&b.channel)
            .unwrap_or(std::cmp::Ordering::Equal)
    });
    (matches, mean)
}

/// Peak-shape calibration: `FWHM = c0 + c1*channel + c2*channel^2`, in channels.
///
/// This is the polynomial-in-channel form ORTEC software stores in the
/// `$SHAPE_CAL` block of a `.Spe` file and in the `.Chn` trailer. Real values
/// from a real HPGe file - `[2.4946, 1.745e-3, -1.9356e-7]` at
/// 0.361 keV per channel - give 1.80 keV at 662 keV and 2.27 keV at 1332 keV.
#[derive(Clone, Copy, Debug, Default, PartialEq, Serialize, Deserialize)]
pub struct ShapeCalibration {
    /// Constant, linear and quadratic coefficients, in channels.
    pub coefficients: [f64; 3],
}

impl ShapeCalibration {
    /// Builds a shape calibration from explicit coefficients.
    pub fn new(coefficients: [f64; 3]) -> Self {
        Self { coefficients }
    }

    /// Predicted full width at half maximum, in channels.
    pub fn fwhm(&self, channel: f64) -> f64 {
        let [a, b, c] = self.coefficients;
        let channel = channel.max(0.0);
        a + b * channel + c * channel * channel
    }

    /// Predicted standard deviation, in channels.
    pub fn sigma(&self, channel: f64) -> f64 {
        self.fwhm(channel) / crate::FWHM_PER_SIGMA
    }

    /// True when the calibration can produce a usable (positive) width.
    pub fn is_usable(&self) -> bool {
        self.coefficients.iter().any(|c| *c != 0.0)
    }

    /// Least-squares fit of measured `(channel, fwhm)` pairs.
    ///
    /// Two points give a straight line, three or more a parabola.
    pub fn fit(points: &[(f64, f64)]) -> Result<Self, CalibrationError> {
        if points.len() < 2 {
            return Err(CalibrationError::NotEnoughPoints {
                needed: 2,
                got: points.len(),
            });
        }
        let distinct = distinct_abscissae(points.iter().map(|(channel, _)| *channel));
        if distinct < 2 {
            return Err(CalibrationError::Degenerate);
        }
        let solution = if distinct == 2 {
            linear_fit(points)
        } else {
            quadratic_fit(points)
        };
        solution.map(Self::new).ok_or(CalibrationError::Degenerate)
    }
}

/// The table of calibration points MAESTRO accumulates while calibrating.
///
/// Adding points refits automatically; the table is capped at
/// [`MAX_CALIBRATION_POINTS`].
#[derive(Clone, Debug, Serialize, Deserialize)]
pub struct CalibrationTable {
    points: Vec<CalibrationPoint>,
    units: String,
    calibration: Option<EnergyCalibration>,
}

impl Default for CalibrationTable {
    fn default() -> Self {
        Self {
            points: Vec::new(),
            units: DEFAULT_UNITS.to_string(),
            calibration: None,
        }
    }
}

impl CalibrationTable {
    /// Adds a point and refits. Returns `false` when the table is full.
    pub fn add(&mut self, point: CalibrationPoint) -> bool {
        if self.points.len() >= MAX_CALIBRATION_POINTS {
            return false;
        }
        self.points.push(point);
        self.refit();
        true
    }

    /// Removes the point at `index` and refits.
    pub fn remove(&mut self, index: usize) -> Option<CalibrationPoint> {
        if index >= self.points.len() {
            return None;
        }
        let removed = self.points.remove(index);
        self.refit();
        Some(removed)
    }

    /// Clears the table and the fitted calibration ("Destroy Calibration").
    pub fn destroy(&mut self) {
        self.points.clear();
        self.calibration = None;
    }

    /// Replaces one point and refits.
    ///
    /// Editing beats deleting and re-entering: a mistyped energy is one digit
    /// wrong, and the fit should follow the correction rather than make somebody
    /// begin the whole calibration again.
    pub fn set(&mut self, index: usize, point: CalibrationPoint) -> bool {
        let Some(slot) = self.points.get_mut(index) else {
            return false;
        };
        *slot = point;
        self.refit();
        true
    }

    /// Entered points.
    pub fn points(&self) -> &[CalibrationPoint] {
        &self.points
    }

    /// Number of entered points.
    pub fn len(&self) -> usize {
        self.points.len()
    }

    /// True when no points have been entered.
    pub fn is_empty(&self) -> bool {
        self.points.is_empty()
    }

    /// The current fit, if the table holds at least two usable points.
    pub fn calibration(&self) -> Option<&EnergyCalibration> {
        self.calibration.as_ref()
    }

    /// Sets the units label and refits so the calibration carries it.
    pub fn set_units(&mut self, units: &str) {
        self.units = truncate_units(units);
        self.refit();
    }

    /// Current units label.
    pub fn units(&self) -> &str {
        &self.units
    }

    /// Seeds the table from an existing calibration (for recalibration).
    pub fn with_units(units: &str) -> Self {
        Self {
            units: truncate_units(units),
            ..Self::default()
        }
    }

    fn refit(&mut self) {
        self.calibration = EnergyCalibration::fit_with_units(&self.points, &self.units).ok();
    }
}

fn truncate_units(units: &str) -> String {
    units.chars().take(4).collect()
}

/// How many of the `x` values are meaningfully different from each other.
fn distinct_abscissae(xs: impl Iterator<Item = f64>) -> usize {
    let mut sorted: Vec<f64> = xs.collect();
    sorted.sort_by(|a, b| a.partial_cmp(b).unwrap_or(std::cmp::Ordering::Equal));
    sorted.dedup_by(|a, b| (*a - *b).abs() < 1e-9);
    sorted.len()
}

/// Least-squares straight line `y = c0 + c1*x`, as polynomial coefficients.
///
/// Repeated `x` values average instead of breaking the fit, which is what a
/// calibration point entered twice deserves. Centred on the mean, so the
/// arithmetic is well conditioned at any channel count.
fn linear_fit(points: &[(f64, f64)]) -> Option<[f64; 3]> {
    let n = points.len() as f64;
    let mean_x = points.iter().map(|(x, _)| x).sum::<f64>() / n;
    let mean_y = points.iter().map(|(_, y)| y).sum::<f64>() / n;
    let mut cross = 0.0;
    let mut spread = 0.0;
    for (x, y) in points {
        let d = x - mean_x;
        cross += d * (y - mean_y);
        spread += d * d;
    }
    if spread <= 0.0 {
        return None;
    }
    let gain = cross / spread;
    let zero = mean_y - gain * mean_x;
    (gain.is_finite() && zero.is_finite()).then_some([zero, gain, 0.0])
}

/// Least-squares parabola `y = c0 + c1*x + c2*x^2`.
///
/// The normal equations are solved in a centred, scaled coordinate: on the raw
/// channel basis a 16k-channel spectrum puts entries from 1 to 10^17 in the
/// same matrix, and the quadratic coefficient comes back shredded.
fn quadratic_fit(points: &[(f64, f64)]) -> Option<[f64; 3]> {
    let n = points.len() as f64;
    let mean = points.iter().map(|(x, _)| x).sum::<f64>() / n;
    let scale = points
        .iter()
        .map(|(x, _)| (x - mean).abs())
        .fold(0.0, f64::max);
    if scale <= 0.0 {
        return None;
    }
    let rows: Vec<([f64; 3], f64)> = points
        .iter()
        .map(|(x, y)| {
            let t = (x - mean) / scale;
            ([1.0, t, t * t], *y)
        })
        .collect();
    let [a, b, c] = least_squares3(&rows)?;
    // Expand y = a + b*t + c*t^2 with t = (x - m)/s back to powers of x.
    let (m, s) = (mean, scale);
    let c2 = c / (s * s);
    let c1 = b / s - 2.0 * m * c / (s * s);
    let c0 = a - b * m / s + c * m * m / (s * s);
    let coefficients = [c0, c1, c2];
    coefficients
        .iter()
        .all(|v| v.is_finite())
        .then_some(coefficients)
}

/// Solves a 3-parameter linear least-squares problem via normal equations.
pub(crate) fn least_squares3(rows: &[([f64; 3], f64)]) -> Option<[f64; 3]> {
    let mut ata = [[0.0f64; 3]; 3];
    let mut atb = [0.0f64; 3];
    for (basis, y) in rows {
        for i in 0..3 {
            for j in 0..3 {
                ata[i][j] += basis[i] * basis[j];
            }
            atb[i] += basis[i] * y;
        }
    }
    solve3(ata, atb)
}

/// Solves an `n`-parameter linear least-squares problem via normal equations.
pub(crate) fn least_squares_n(rows: &[(Vec<f64>, f64)], n: usize) -> Option<Vec<f64>> {
    if n == 0 || rows.len() < n {
        return None;
    }
    let mut ata = vec![vec![0.0f64; n]; n];
    let mut atb = vec![0.0f64; n];
    for (basis, y) in rows {
        if basis.len() != n {
            return None;
        }
        for i in 0..n {
            for j in 0..n {
                ata[i][j] += basis[i] * basis[j];
            }
            atb[i] += basis[i] * y;
        }
    }
    solve_n(ata, atb)
}

/// Gaussian elimination with partial pivoting for a dense `n x n` system.
pub(crate) fn solve_n(mut a: Vec<Vec<f64>>, mut b: Vec<f64>) -> Option<Vec<f64>> {
    let n = b.len();
    for col in 0..n {
        let mut pivot = col;
        for row in (col + 1)..n {
            if a[row][col].abs() > a[pivot][col].abs() {
                pivot = row;
            }
        }
        if a[pivot][col].abs() < 1e-14 {
            return None;
        }
        a.swap(col, pivot);
        b.swap(col, pivot);
        for row in (col + 1)..n {
            let factor = a[row][col] / a[col][col];
            let pivot_row = a[col].clone();
            for (target, source) in a[row].iter_mut().zip(pivot_row.iter()).skip(col) {
                *target -= factor * source;
            }
            b[row] -= factor * b[col];
        }
    }
    let mut x = vec![0.0f64; n];
    for row in (0..n).rev() {
        let mut sum = b[row];
        for col in (row + 1)..n {
            sum -= a[row][col] * x[col];
        }
        x[row] = sum / a[row][row];
    }
    x.iter().all(|v| v.is_finite()).then_some(x)
}

/// Gaussian elimination with partial pivoting for a 3x3 system.
pub(crate) fn solve3(mut a: [[f64; 3]; 3], mut b: [f64; 3]) -> Option<[f64; 3]> {
    for col in 0..3 {
        // Pivot.
        let mut pivot = col;
        for row in (col + 1)..3 {
            if a[row][col].abs() > a[pivot][col].abs() {
                pivot = row;
            }
        }
        if a[pivot][col].abs() < 1e-12 {
            return None;
        }
        a.swap(col, pivot);
        b.swap(col, pivot);

        for row in (col + 1)..3 {
            let factor = a[row][col] / a[col][col];
            let pivot_row = a[col];
            for (target, source) in a[row].iter_mut().zip(pivot_row.iter()).skip(col) {
                *target -= factor * source;
            }
            b[row] -= factor * b[col];
        }
    }
    let mut x = [0.0f64; 3];
    for row in (0..3).rev() {
        let mut sum = b[row];
        for col in (row + 1)..3 {
            sum -= a[row][col] * x[col];
        }
        x[row] = sum / a[row][row];
    }
    x.iter().all(|v| v.is_finite()).then_some(x)
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn solve3_handles_a_known_system() {
        let a = [[2.0, 0.0, 0.0], [0.0, 3.0, 0.0], [0.0, 0.0, 4.0]];
        let x = solve3(a, [2.0, 9.0, 8.0]).unwrap();
        assert_eq!(x, [1.0, 3.0, 2.0]);
    }

    #[test]
    fn solve3_rejects_a_singular_system() {
        let a = [[1.0, 1.0, 1.0], [1.0, 1.0, 1.0], [1.0, 1.0, 1.0]];
        assert!(solve3(a, [1.0, 2.0, 3.0]).is_none());
    }

    #[test]
    fn a_point_entered_twice_averages_instead_of_breaking_the_fit() {
        // The same peak recorded twice with slightly different energies: the
        // fit should pass through the average, not divide by zero.
        let fit = EnergyCalibration::fit(&[
            CalibrationPoint::new(100.0, 50.0),
            CalibrationPoint::new(100.0, 55.0),
            CalibrationPoint::new(500.0, 250.0),
        ])
        .expect("two distinct channels are enough");
        assert!((fit.energy(100.0) - 52.5).abs() < 1e-9);
        assert!((fit.energy(500.0) - 250.0).abs() < 1e-9);
    }

    #[test]
    fn a_duplicated_channel_still_counts_every_point() {
        // Least squares over all three points, not a line through the first
        // two: at channel 500 the best answer is the average of 250 and 270.
        let fit = EnergyCalibration::fit(&[
            CalibrationPoint::new(100.0, 50.0),
            CalibrationPoint::new(500.0, 250.0),
            CalibrationPoint::new(500.0, 270.0),
        ])
        .expect("two distinct channels are enough");
        assert!(
            (fit.energy(500.0) - 260.0).abs() < 1e-9,
            "got {}",
            fit.energy(500.0)
        );
    }

    #[test]
    fn the_quadratic_fit_survives_sixteen_thousand_channels() {
        // On the raw basis 1, ch, ch^2 the normal equations span seventeen
        // orders of magnitude at these channels and shred the curvature.
        let truth = [3.0, 0.35, 1.0e-7];
        let energy = |ch: f64| truth[0] + truth[1] * ch + truth[2] * ch * ch;
        let points: Vec<CalibrationPoint> = (0..9)
            .map(|i| {
                let ch = 8000.0 + 1000.0 * i as f64;
                CalibrationPoint::new(ch, energy(ch))
            })
            .collect();
        let fit = EnergyCalibration::fit(&points).expect("a clean parabola");
        for point in &points {
            assert!(
                (fit.energy(point.channel) - point.value).abs() < 1e-6,
                "channel {} should read {}, got {}",
                point.channel,
                point.value,
                fit.energy(point.channel)
            );
        }
    }

    #[test]
    fn one_reference_line_takes_one_peak_only() {
        // Channels 100 and 100.2 both land within tolerance of the 500 line;
        // only the closer may claim it, so two lines are matched, not three.
        let (matches, _) = match_peaks(&[100.0, 100.2, 300.0], &[500.0, 1500.0], 0.0, 5.0);
        assert_eq!(matches.len(), 2);
        assert_eq!(matches[0].channel, 100.0);
        assert_eq!(matches[1].channel, 300.0);
    }

    #[test]
    fn three_auto_calibration_matches_fit_a_line_not_a_parabola() {
        // Peaks sitting a hair off a perfect line: an exact quadratic through
        // them would pick up curvature from the misses alone.
        let centroids = [200.1, 600.0, 1199.8];
        let lines = [200.0, 600.0, 1200.0];
        let result = auto_calibrate(&centroids, &lines, "keV").expect("should calibrate");
        assert!(
            result.calibration.order() <= 1,
            "coefficients {:?}",
            result.calibration.coefficients
        );
    }
}

#[cfg(test)]
mod editing_tests {
    use super::*;

    /// Three points on a known straight line, the middle one mistyped.
    fn mistyped() -> CalibrationTable {
        let mut set = CalibrationTable::default();
        set.add(CalibrationPoint::new(100.0, 50.0));
        set.add(CalibrationPoint::new(500.0, 2500.0)); // should be 250
        set.add(CalibrationPoint::new(900.0, 450.0));
        set
    }

    #[test]
    fn correcting_a_point_refits_without_starting_again() {
        let mut set = mistyped();
        let before = set
            .calibration()
            .expect("a fit even from a bad point")
            .energy(500.0);
        assert!(
            (before - 250.0).abs() > 100.0,
            "the mistyped point should pull the fit badly off, got {before}"
        );

        assert!(set.set(1, CalibrationPoint::new(500.0, 250.0)));

        assert_eq!(set.len(), 3, "correcting must not lose the point");
        let fit = set.calibration().expect("a fit");
        for (channel, energy) in [(100.0, 50.0), (500.0, 250.0), (900.0, 450.0)] {
            assert!(
                (fit.energy(channel) - energy).abs() < 1e-6,
                "channel {channel} should read {energy}, got {}",
                fit.energy(channel)
            );
        }
    }

    #[test]
    fn editing_a_point_that_is_not_there_changes_nothing() {
        let mut set = mistyped();
        let before = set.points().to_vec();
        assert!(!set.set(99, CalibrationPoint::new(1.0, 1.0)));
        assert_eq!(set.points(), before.as_slice());
    }

    #[test]
    fn removing_the_bad_point_leaves_the_others_fitting() {
        let mut set = mistyped();
        assert!(set.remove(1).is_some());
        let fit = set.calibration().expect("two points still fit");
        assert!((fit.energy(100.0) - 50.0).abs() < 1e-6);
        assert!((fit.energy(900.0) - 450.0).abs() < 1e-6);
    }
}
