//! A tiny deterministic random source.
//!
//! The simulator must be reproducible: the same seed has to give the same
//! spectrum, so tests can assert on it. This is a PCG-XSH-RR 32-bit generator
//! with Box-Muller normals and Knuth/normal-approximation Poisson draws. It is
//! not cryptographic and is not meant to be.

/// Deterministic pseudo-random generator.
#[derive(Clone, Debug)]
pub struct Rng {
    state: u64,
    increment: u64,
    spare_normal: Option<f64>,
}

const MULTIPLIER: u64 = 6_364_136_223_846_793_005;

impl Rng {
    /// Creates a generator from a seed.
    pub fn new(seed: u64) -> Self {
        let mut rng = Self {
            state: 0,
            increment: (seed << 1) | 1,
            spare_normal: None,
        };
        rng.state = rng
            .state
            .wrapping_mul(MULTIPLIER)
            .wrapping_add(rng.increment);
        rng.state = rng.state.wrapping_add(seed);
        rng.next_u32();
        rng
    }

    /// Next 32-bit value.
    pub fn next_u32(&mut self) -> u32 {
        let old = self.state;
        self.state = old.wrapping_mul(MULTIPLIER).wrapping_add(self.increment);
        let xorshifted = (((old >> 18) ^ old) >> 27) as u32;
        let rotation = (old >> 59) as u32;
        xorshifted.rotate_right(rotation)
    }

    /// Uniform value in `[0, 1)`.
    pub fn uniform(&mut self) -> f64 {
        self.next_u32() as f64 / (u32::MAX as f64 + 1.0)
    }

    /// Uniform value in `[low, high)`.
    pub fn range(&mut self, low: f64, high: f64) -> f64 {
        low + (high - low) * self.uniform()
    }

    /// Standard normal deviate.
    pub fn normal(&mut self) -> f64 {
        if let Some(value) = self.spare_normal.take() {
            return value;
        }
        // Box-Muller, guarding against log(0).
        let u1 = self.uniform().max(f64::MIN_POSITIVE);
        let u2 = self.uniform();
        let radius = (-2.0 * u1.ln()).sqrt();
        let angle = std::f64::consts::TAU * u2;
        self.spare_normal = Some(radius * angle.sin());
        radius * angle.cos()
    }

    /// Poisson deviate with mean `lambda`.
    pub fn poisson(&mut self, lambda: f64) -> u64 {
        if !lambda.is_finite() || lambda <= 0.0 {
            return 0;
        }
        if lambda < 30.0 {
            // Knuth's product method.
            let limit = (-lambda).exp();
            let mut product = self.uniform();
            let mut count = 0u64;
            while product > limit {
                count += 1;
                product *= self.uniform();
                if count > 1_000 {
                    break;
                }
            }
            return count;
        }
        // Normal approximation is accurate to better than a percent here.
        let value = lambda + lambda.sqrt() * self.normal();
        value.round().max(0.0) as u64
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn the_same_seed_repeats() {
        let mut a = Rng::new(42);
        let mut b = Rng::new(42);
        for _ in 0..100 {
            assert_eq!(a.next_u32(), b.next_u32());
        }
    }

    #[test]
    fn different_seeds_diverge() {
        let mut a = Rng::new(1);
        let mut b = Rng::new(2);
        assert_ne!(a.next_u32(), b.next_u32());
    }

    #[test]
    fn uniform_stays_in_range() {
        let mut rng = Rng::new(7);
        for _ in 0..10_000 {
            let value = rng.uniform();
            assert!((0.0..1.0).contains(&value));
        }
    }

    #[test]
    fn normals_have_the_right_moments() {
        let mut rng = Rng::new(11);
        let samples: Vec<f64> = (0..20_000).map(|_| rng.normal()).collect();
        let mean = samples.iter().sum::<f64>() / samples.len() as f64;
        let variance = samples.iter().map(|v| (v - mean) * (v - mean)).sum::<f64>()
            / (samples.len() as f64 - 1.0);
        assert!(mean.abs() < 0.05, "mean {mean}");
        assert!((variance - 1.0).abs() < 0.06, "variance {variance}");
    }

    #[test]
    fn poisson_has_the_right_mean_for_small_and_large_lambda() {
        for lambda in [0.5, 5.0, 100.0, 5_000.0] {
            let mut rng = Rng::new(3);
            let count = 4_000;
            let total: u64 = (0..count).map(|_| rng.poisson(lambda)).sum();
            let mean = total as f64 / count as f64;
            let tolerance = 5.0 * (lambda / count as f64).sqrt() + 0.02 * lambda.sqrt();
            assert!(
                (mean - lambda).abs() < tolerance.max(0.05),
                "lambda {lambda}: mean {mean}"
            );
        }
        assert_eq!(Rng::new(1).poisson(0.0), 0);
        assert_eq!(Rng::new(1).poisson(-1.0), 0);
    }
}
