//! How a count behaved while it was running, so drift is visible during it.
//!
//! The readout says what the rate is now. It cannot say whether it has been
//! that all along - and the things that quietly ruin a measurement are all
//! changes over time: a source that shifted, a detector warming up, a preamp
//! starting to misbehave, somebody opening the shield. Seeing it while the
//! count is still going is the difference between redoing ten minutes and
//! believing a bad number.
//!
//! Distinct from the QA control charts, which compare separate check-source
//! measurements across days. This is one acquisition, from the inside.

use std::collections::VecDeque;

/// One reading, taken as the count ran.
#[derive(Clone, Copy, Debug, PartialEq)]
pub struct Sample {
    /// Live time when it was taken, in seconds.
    pub live_time: f64,
    /// Counts per second since the previous sample, not since the beginning.
    ///
    /// The average since the start hides exactly what this is for: a rate that
    /// halves an hour in barely moves the average, but the interval rate falls
    /// off a cliff.
    pub rate: f64,
    /// Dead time at that moment, in percent.
    pub dead_time: f64,
}

/// How many readings are kept before they are thinned.
const CAPACITY: usize = 600;

/// The shortest gap between readings, in seconds of live time.
const FIRST_INTERVAL: f64 = 1.0;

/// The readings taken during one acquisition.
#[derive(Clone, Debug)]
pub struct StabilityLog {
    samples: VecDeque<Sample>,
    interval: f64,
    last_live: f64,
    last_counts: u64,
    started: bool,
}

impl Default for StabilityLog {
    fn default() -> Self {
        Self {
            samples: VecDeque::new(),
            interval: FIRST_INTERVAL,
            last_live: 0.0,
            last_counts: 0,
            started: false,
        }
    }
}

impl StabilityLog {
    /// Offers a reading, which is kept only if enough live time has passed.
    ///
    /// A live clock that has gone backwards means a Clear, or a different
    /// acquisition entirely, so the log starts again rather than drawing a
    /// line across the discontinuity as though it were data.
    pub fn record(&mut self, live_time: f64, total_counts: u64, dead_time: f64) {
        if !live_time.is_finite() || live_time < 0.0 {
            return;
        }
        if live_time < self.last_live || (self.started && total_counts < self.last_counts) {
            self.reset();
        }
        if !self.started {
            self.started = true;
            self.last_live = live_time;
            self.last_counts = total_counts;
            return;
        }
        let span = live_time - self.last_live;
        if span < self.interval {
            return;
        }
        let gained = total_counts.saturating_sub(self.last_counts);
        self.samples.push_back(Sample {
            live_time,
            rate: gained as f64 / span,
            dead_time: dead_time.clamp(0.0, 100.0),
        });
        self.last_live = live_time;
        self.last_counts = total_counts;
        if self.samples.len() > CAPACITY {
            self.thin();
        }
    }

    /// Halves the readings and doubles the gap, so a long count still fits.
    ///
    /// The alternative is dropping the beginning, which throws away the part
    /// a drift is measured against. Coarser resolution over the whole run
    /// answers "has this changed?" better than fine resolution over the last
    /// few minutes.
    fn thin(&mut self) {
        let mut kept = VecDeque::with_capacity(self.samples.len() / 2 + 1);
        for (index, sample) in self.samples.iter().enumerate() {
            if index % 2 == 1 {
                kept.push_back(*sample);
            }
        }
        self.samples = kept;
        self.interval *= 2.0;
    }

    /// Forgets everything, as a Clear does.
    pub fn reset(&mut self) {
        *self = Self::default();
    }

    /// The readings, oldest first.
    pub fn samples(&self) -> impl Iterator<Item = &Sample> {
        self.samples.iter()
    }

    /// How many readings there are.
    pub fn len(&self) -> usize {
        self.samples.len()
    }

    /// Whether nothing has been recorded yet.
    pub fn is_empty(&self) -> bool {
        self.samples.is_empty()
    }

    /// The highest rate seen, for scaling a trace.
    pub fn peak_rate(&self) -> f64 {
        self.samples
            .iter()
            .map(|sample| sample.rate)
            .fold(0.0, f64::max)
    }

    /// How far the rate has drifted, as a fraction of the first readings.
    ///
    /// Compares the first and last fifth of the run rather than single
    /// readings, because counting statistics alone move any one reading
    /// around. `None` until there is enough to compare.
    pub fn drift(&self) -> Option<f64> {
        if self.samples.len() < 10 {
            return None;
        }
        let fifth = (self.samples.len() / 5).max(1);
        let mean = |values: &[Sample]| -> f64 {
            values.iter().map(|sample| sample.rate).sum::<f64>() / values.len() as f64
        };
        let ordered: Vec<Sample> = self.samples.iter().copied().collect();
        let first = mean(&ordered[..fifth]);
        let last = mean(&ordered[ordered.len() - fifth..]);
        (first > 0.0).then(|| (last - first) / first)
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    /// A steady count: one thousand counts every second of live time.
    fn steady(log: &mut StabilityLog, seconds: usize) {
        for second in 0..=seconds {
            log.record(second as f64, second as u64 * 1_000, 3.0);
        }
    }

    #[test]
    fn readings_are_taken_no_faster_than_the_interval() {
        let mut log = StabilityLog::default();
        // Ten offers inside one second of live time yield one reading at most.
        for tenth in 0..=10 {
            log.record(tenth as f64 / 10.0, tenth as u64 * 100, 2.0);
        }
        assert_eq!(log.len(), 1, "one second of live time is one reading");
    }

    #[test]
    fn the_rate_is_the_interval_rate_not_the_average() {
        let mut log = StabilityLog::default();
        // Ten seconds at 1000/s, then ten at 100/s. The average would still
        // read high; the interval rate has to show the fall.
        steady(&mut log, 10);
        let mut counts = 10_000u64;
        for second in 11..=20 {
            counts += 100;
            log.record(second as f64, counts, 3.0);
        }
        let rates: Vec<f64> = log.samples().map(|sample| sample.rate).collect();
        assert!((rates[0] - 1_000.0).abs() < 1e-9, "{rates:?}");
        assert!(
            (rates[rates.len() - 1] - 100.0).abs() < 1e-9,
            "the last interval is the slow one: {rates:?}"
        );
    }

    #[test]
    fn a_cleared_instrument_starts_the_log_again() {
        let mut log = StabilityLog::default();
        steady(&mut log, 30);
        assert!(log.len() > 10);
        // Clear: the live clock and the counts both go back to nothing.
        log.record(0.0, 0, 0.0);
        assert!(
            log.is_empty(),
            "a line drawn across a Clear would be a lie about the data"
        );
    }

    #[test]
    fn a_long_count_is_thinned_rather_than_truncated() {
        let mut log = StabilityLog::default();
        steady(&mut log, CAPACITY * 3);
        assert!(log.len() <= CAPACITY, "{} readings", log.len());
        assert!(log.len() > CAPACITY / 4, "still a useful trace");
        // The beginning is what a drift is measured against, so it must
        // survive: the first reading is still from early in the run.
        let first = log.samples().next().expect("a first reading");
        assert!(
            first.live_time < (CAPACITY * 3) as f64 / 2.0,
            "the start was thrown away: first at {}",
            first.live_time
        );
    }

    #[test]
    fn a_steady_count_shows_no_drift_and_a_falling_one_does() {
        let mut steady_log = StabilityLog::default();
        steady(&mut steady_log, 60);
        let drift = steady_log.drift().expect("enough readings");
        assert!(
            drift.abs() < 0.01,
            "a steady count should not drift: {drift}"
        );

        // A source that has moved away: the rate halves across the run.
        let mut falling = StabilityLog::default();
        let mut counts = 0u64;
        for second in 0..=60 {
            let rate = 1_000.0 - 8.0 * second as f64;
            counts += rate as u64;
            falling.record(second as f64, counts, 3.0);
        }
        let drift = falling.drift().expect("enough readings");
        assert!(drift < -0.3, "the fall should show: {drift}");
    }

    #[test]
    fn drift_waits_for_enough_to_compare() {
        let mut log = StabilityLog::default();
        steady(&mut log, 5);
        assert_eq!(log.drift(), None, "five readings is not a trend");
    }
}
