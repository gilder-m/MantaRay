//! Calculation settings (MAESTRO Calculate/Settings, §4.3.1).

use serde::{Deserialize, Serialize};

/// The three knobs on the Calculation Settings dialog.
#[derive(Clone, Copy, Debug, PartialEq, Eq, Serialize, Deserialize)]
pub struct CalculationSettings {
    /// `x` in the FW(1/x)M width report; 2..=99, default 5.
    pub fw_x: u32,
    /// Peak-search sensitivity factor; 1 (most sensitive) ..= 5, default 3.
    pub sensitivity: u32,
    /// Background channels used at each side of a region; 1..=5, default 3.
    pub background_points: usize,
}

impl Default for CalculationSettings {
    fn default() -> Self {
        Self {
            fw_x: 5,
            sensitivity: 3,
            background_points: 3,
        }
    }
}

impl CalculationSettings {
    /// Lowest accepted `x` for FW(1/x)M.
    pub const MIN_FW_X: u32 = 2;
    /// Highest accepted `x` for FW(1/x)M.
    pub const MAX_FW_X: u32 = 99;
    /// Most sensitive peak-search setting.
    pub const MIN_SENSITIVITY: u32 = 1;
    /// Least sensitive peak-search setting.
    pub const MAX_SENSITIVITY: u32 = 5;
    /// Fewest background points.
    pub const MIN_BACKGROUND_POINTS: usize = 1;
    /// Most background points.
    pub const MAX_BACKGROUND_POINTS: usize = 5;

    /// Returns a copy with `fw_x` set (clamped to 2..=99).
    pub fn with_fw_x(mut self, fw_x: u32) -> Self {
        self.fw_x = fw_x.clamp(Self::MIN_FW_X, Self::MAX_FW_X);
        self
    }

    /// Returns a copy with the sensitivity set (clamped to 1..=5).
    pub fn with_sensitivity(mut self, sensitivity: u32) -> Self {
        self.sensitivity = sensitivity.clamp(Self::MIN_SENSITIVITY, Self::MAX_SENSITIVITY);
        self
    }

    /// Returns a copy with the background-point count set (clamped to 1..=5).
    pub fn with_background_points(mut self, points: usize) -> Self {
        self.background_points =
            points.clamp(Self::MIN_BACKGROUND_POINTS, Self::MAX_BACKGROUND_POINTS);
        self
    }

    /// Clamps every field into its accepted range.
    pub fn clamped(mut self) -> Self {
        self = self
            .with_fw_x(self.fw_x)
            .with_sensitivity(self.sensitivity)
            .with_background_points(self.background_points);
        self
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn defaults_match_the_dialog() {
        let s = CalculationSettings::default();
        assert_eq!((s.fw_x, s.sensitivity, s.background_points), (5, 3, 3));
    }

    #[test]
    fn setters_clamp() {
        let s = CalculationSettings::default()
            .with_fw_x(1)
            .with_sensitivity(9)
            .with_background_points(99);
        assert_eq!((s.fw_x, s.sensitivity, s.background_points), (2, 5, 5));
    }
}
